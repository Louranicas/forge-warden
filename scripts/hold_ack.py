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
from pathlib import Path

LIMIT = 64 * 1024 * 1024
ROOT = Path(__file__).resolve().parent.parent
ENV = {"LC_ALL": "C", "PATH": "/usr/bin:/bin"}
NAMESPACE = "forge-warden-ack"


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


def expected(args, signer, expires_at):
    require(type(expires_at) is int and expires_at > int(time.time()), "acknowledgement expired or invalid expiry")
    require(re.fullmatch(r"[a-z0-9][a-z0-9-]{0,63}", signer), "invalid signer ID")
    for commit in (args.base, args.head):
        require(re.fullmatch(r"[0-9a-f]{40}", commit), "full lowercase commit IDs required")
    raw = read(args.verdict, LIMIT)
    verdict = decode(raw)
    require(verdict["schema"] == "forge-warden.verdict.v1", "unsupported verdict schema")
    require(verdict["verdict"] == "hold" and verdict["exit"]["code"] == 8,
            "only HOLD may be acknowledged")
    require(verdict["notes"] == [] and verdict["expired_waivers"] == [],
            "unknown evidence or expired waivers cannot be acknowledged")
    # v1 (ADR 0005): only a ranking forge-warden produced itself, from an allowlisted ranker, can
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


def verify(args):
    # Snapshot all untrusted bytes before parsing or signature verification.
    raw = read(args.ack, 16384)
    receipt = decode(raw)
    wanted = expected(args, receipt["signer"], receipt["expires_at"])
    require(receipt == wanted, "acknowledgement does not match current subject and verdict")
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
        subprocess.run([str(ssh_keygen), "-Y", "verify", "-f", str(tmp / "allowed_signers"),
                        "-I", receipt["signer"], "-n", NAMESPACE, "-s", str(tmp / "ack.sig")],
                       input=raw, env=ENV, capture_output=True, timeout=5, check=True)
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
