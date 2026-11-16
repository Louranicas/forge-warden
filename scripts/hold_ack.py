#!/usr/bin/env python3
"""Bound HOLD receipts. Run only from trusted code with independently supplied subject IDs."""
import argparse
import base64
import hashlib
import json
import os
import re
import subprocess
import sys
import time
import tempfile
import stat
import tomllib
from pathlib import Path

LIMIT = 64 * 1024 * 1024
ROOT = Path(__file__).resolve().parent.parent
ENV = {"LC_ALL": "C", "PATH": "/usr/bin:/bin"}
NAMESPACE = "forge-warden-ack"
GIT_POLICY = Path("/etc/forge-warden/git-allowlist.toml")
GIT_POLICY_ROOT = Path("/")
GIT_POLICY_UID = 0


def require(condition, message):
    if not condition:
        raise ValueError(message)


def read(path, limit):
    with Path(path).open("rb") as stream:
        data = stream.read(limit + 1)
    require(len(data) <= limit, "input exceeds byte limit")
    return data


def pairs(items):
    result = {}
    for key, value in items:
        require(key not in result, "duplicate JSON key")
        result[key] = value
    return result


def decode(data):
    return json.loads(data, object_pairs_hook=pairs,
                      parse_constant=lambda _: (_ for _ in ()).throw(ValueError("non-finite JSON")))


def current_git_policy():
    """Read one bounded snapshot from the fixed owner-controlled policy, never candidate data."""
    current = GIT_POLICY_ROOT
    for part in ("", *GIT_POLICY.parent.relative_to(GIT_POLICY_ROOT).parts):
        current = current / part
        info = current.lstat()
        require(stat.S_ISDIR(info.st_mode) and info.st_uid == GIT_POLICY_UID
                and not info.st_mode & 0o022, "unsafe Git policy parent")
    descriptor = os.open(GIT_POLICY, os.O_RDONLY | os.O_NONBLOCK | os.O_NOFOLLOW)
    with os.fdopen(descriptor, "rb") as stream:
        info = os.fstat(stream.fileno())
        require(stat.S_ISREG(info.st_mode) and info.st_uid == GIT_POLICY_UID
                and not info.st_mode & 0o022 and info.st_size <= 65536,
                "unsafe Git policy file")
        raw = stream.read(65537)
    require(len(raw) <= 65536, "Git policy exceeds byte limit")
    policy = tomllib.loads(raw.decode("utf-8"))
    require(set(policy) == {"schema", "revision", "sha256"}
            and policy["schema"] == "forge-warden.git-allowlist.v1"
            and type(policy["revision"]) is int and policy["revision"] > 0,
            "invalid Git policy structure")
    digests = policy["sha256"]
    require(isinstance(digests, list) and digests
            and all(isinstance(digest, str) and re.fullmatch(r"[0-9a-f]{64}", digest)
                    for digest in digests) and len(digests) == len(set(digests)),
            "invalid Git policy digests")
    return set(digests), hashlib.sha256(raw).hexdigest()


def admitted_git(verdict):
    git = verdict["seals"]["git"]
    require(isinstance(git, dict) and git.get("admission") == "approved",
            "only owner-approved Git may authorize acknowledgement")
    digests, policy_sha256 = current_git_policy()
    require(git.get("sha256") in digests and git.get("allowlist_sha256") == policy_sha256,
            "Git admission no longer matches the current owner policy")


def expected(args, signer, expires_at):
    require(type(expires_at) is int and expires_at > int(time.time()), "acknowledgement expired or invalid expiry")
    require(re.fullmatch(r"[a-z0-9][a-z0-9-]{0,63}", signer), "invalid signer ID")
    # SHA-1 (40) or SHA-256 (64) object ids, both the same length: bound mode verifies claims in either
    # object format (Bastion's LOW on PATCH-BOUND b: a SHA-256 hold could not be acknowledged).
    for commit in (args.base, args.head):
        require(re.fullmatch(r"[0-9a-f]{40}|[0-9a-f]{64}", commit), "full lowercase commit IDs required")
    require(len(args.base) == len(args.head), "base and head must be object ids of one length")
    raw = read(args.verdict, LIMIT)
    verdict = decode(raw)
    require(verdict["schema"] == "forge-warden.verdict.v3", "unsupported verdict schema")
    # v3 (ADR 0012): verified claims bind the patch to base and head; they must be these commits.
    claims = verdict["seals"].get("claims")
    # The merge base is not compared with anything: the acknowledgement names base and head, and the
    # merge base is what forge-warden computed from those two in a verified copy (ADR 0012 D3), so it
    # adds no claim the signer makes. It is still bound, through verdict_sha256, like every other field.
    if isinstance(claims, dict) and claims.get("verified") is True:
        require(claims.get("base") == args.base and claims.get("head") == args.head,
                "the verdict's verified claims name other commits")
    admitted_git(verdict)
    require(verdict["verdict"] == "hold" and verdict["exit"]["code"] == 8,
            "only HOLD may be acknowledged")
    require(verdict["notes"] == [] and verdict["expired_waivers"] == [],
            "unknown evidence or expired waivers cannot be acknowledged")
    # v2 (ADR 0005 and ADR 0011): only a ranking forge-warden produced itself, from an allowlisted ranker, can
    # be acknowledged; a supplied ranking (advisory mode) is never authority.
    require(verdict["seals"]["provenance"] == "produced"
            and isinstance(verdict["seals"]["ranked_by"]["sha256"], str)
            and re.fullmatch(r"[0-9a-f]{64}", verdict["seals"]["ranked_by"]["sha256"]),
            "a supplied ranking cannot be acknowledged")
    held = []
    ids = set()
    for rule in verdict["rules"]:
        require(isinstance(rule["id"], str) and rule["id"] not in ids, "invalid or duplicate rule ID")
        ids.add(rule["id"])
        require(rule["outcome"] in ("pass", "waived", "hold"), "failed or unknown rule cannot be acknowledged")
        if rule["outcome"] == "hold":
            require(rule["severity"] == "hold", "only explicit policy holds may be acknowledged")
            held.append(rule["id"])
    require(held and verdict["summary"]["held"] == len(held) and verdict["summary"]["failed"] == 0,
            "inconsistent HOLD summary")
    return {"schema": "forge-warden.hold-ack.v1", "repository": "Louranicas/forge-warden",
            "base": args.base, "head": args.head, "verdict_sha256": hashlib.sha256(raw).hexdigest(),
            "held_rules": sorted(held), "signer": signer, "expires_at": expires_at}


def canonical(receipt):
    return (json.dumps(receipt, sort_keys=True, separators=(",", ":")) + "\n").encode("utf-8")


def mismatch(receipt, wanted):
    """Name the receipt fields that differ from the current subject, for the refusal message only."""
    differ = [key for key in sorted(wanted) if receipt.get(key) != wanted[key]]
    extra = len(set(receipt) - set(wanted))
    why = ", ".join(differ + ([f"{extra} extra field(s)"] if extra else []))
    if differ == ["verdict_sha256"] and not extra:
        why += ("; a verdict carries its UTC evaluation date, so a receipt matches only runs"
                " on the UTC day it was prepared for")
    return why


def verify(args):
    # Snapshot all untrusted bytes before parsing or signature verification.
    raw = read(args.ack, 16384)
    receipt = decode(raw)
    wanted = expected(args, receipt["signer"], receipt["expires_at"])
    try:
        require(receipt == wanted, "acknowledgement does not match current subject and verdict")
    except ValueError as error:
        raise ValueError(f"{error}: {mismatch(receipt, wanted)}") from None
    require(raw == canonical(receipt), "receipt must use canonical encoding")
    signers = ROOT / ".github" / "hold-reviewers" / "allowed_signers"
    require(signers.resolve() == signers, "allowed_signers path must not contain symlinks")
    allowed = read(signers, 65536)
    signature = read(args.signature, 4096)
    with tempfile.TemporaryDirectory(prefix="warden-ack-") as directory:
        tmp = Path(directory)
        (tmp / "allowed_signers").write_bytes(allowed)
        (tmp / "ack.sig").write_bytes(signature)
        ssh_keygen = Path("/usr/bin/ssh-keygen")
        require(ssh_keygen.is_file() and os.access(ssh_keygen, os.X_OK),
                "HOLD_ACK_TOOL_UNAVAILABLE: install executable /usr/bin/ssh-keygen (OpenSSH client) before verifying acknowledgements")
        checked = subprocess.run([str(ssh_keygen), "-Y", "verify", "-f", str(tmp / "allowed_signers"),
                                  "-I", receipt["signer"], "-n", NAMESPACE, "-s", str(tmp / "ack.sig")],
                                 input=raw, env=ENV, capture_output=True, timeout=5, check=False)
        require(checked.returncode == 0,
                "signature is not valid for an enrolled reviewer: /usr/bin/ssh-keygen -Y verify refused it"
                f" (signer {receipt['signer']}, namespace {NAMESPACE}, exit {checked.returncode})")
    return {"receipt": receipt, "signed_bytes_base64": base64.b64encode(raw).decode("ascii"),
            "signature_base64": base64.b64encode(signature).decode("ascii"),
            "allowed_signers_sha256": hashlib.sha256(allowed).hexdigest()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    for name in ("prepare", "verify"):
        command = commands.add_parser(name)
        command.add_argument("--base", required=True)
        command.add_argument("--head", required=True)
        command.add_argument("--verdict", required=True)
        if name == "prepare":
            command.add_argument("--signer", required=True)
            command.add_argument("--expires-at", required=True, type=int)
        else:
            command.add_argument("--ack", required=True)
            command.add_argument("--signature", required=True)
    args = parser.parse_args()
    try:
        receipt = expected(args, args.signer, args.expires_at) if args.command == "prepare" else verify(args)
    except (OSError, ValueError, KeyError, TypeError, RecursionError, subprocess.SubprocessError) as error:
        print("hold_ack: refused: " + str(error), file=sys.stderr)
        return 8
    sys.stdout.buffer.write(canonical(receipt))
    return 0


if __name__ == "__main__":
    sys.exit(main())
