#!/usr/bin/env python3
"""Offline acknowledgement attacks using disposable fixture keys, never deployment keys."""
import json
import shutil
import subprocess
import sys
import tempfile
import unittest
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


class BoundAck(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="ack-test-")
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        (self.root / "scripts").mkdir()
        self.keys = self.root / ".github/hold-reviewers"
        self.keys.mkdir(parents=True)
        self.helper = self.root / "scripts/hold_ack.py"
        shutil.copyfile(ROOT / "scripts/hold_ack.py", self.helper)
        self.base, self.head = "a" * 40, "b" * 40
        self.verdict = json.loads((ROOT / "fixtures/demo.verdict.json").read_bytes())
        self.verdict["verdict"] = "hold"
        self.verdict["exit"] = {"code": 8, "name": "policy_hold", "action": "review_evidence"}
        self.verdict["summary"]["failed"] = 0
        self.verdict["summary"]["held"] = 5
        self.verdict["rules"][2]["severity"] = "hold"
        self.verdict["rules"][2]["outcome"] = "hold"
        self.write_verdict()
        self.new_key("private")
        public = (self.root / "private.pub").read_text().strip()
        (self.keys / "allowed_signers").write_text('fixture namespaces="forge-warden-ack" ' + public + "\n")
        prepared = self.run_helper("prepare", "--signer", "fixture")
        self.assertEqual(prepared.returncode, 0, prepared.stderr)
        (self.root / "ack.json").write_bytes(prepared.stdout)
        self.sign()

    def new_key(self, name):
        subprocess.run(["/usr/bin/ssh-keygen", "-q", "-t", "ed25519", "-N", "",
                        "-f", str(self.root / name)], check=True, capture_output=True)

    def write_verdict(self):
        (self.root / "verdict.json").write_text(json.dumps(self.verdict))

    def sign(self, key="private", namespace="forge-warden-ack"):
        signed = subprocess.run(["/usr/bin/ssh-keygen", "-Y", "sign", "-f", str(self.root / key),
                                 "-n", namespace, "-"], input=(self.root / "ack.json").read_bytes(),
                                capture_output=True, check=True)
        (self.root / "ack.sig").write_bytes(signed.stdout)

    def run_helper(self, command, *args):
        if command == "prepare":
            args = (*args, "--expires-at", str(int(time.time()) + 3600))
        return subprocess.run([sys.executable, "-I", str(self.helper), command, "--base", self.base,
                               "--head", self.head, "--verdict", str(self.root / "verdict.json"), *args],
                              capture_output=True)

    def verify(self, success=False):
        out = self.run_helper("verify", "--ack", str(self.root / "ack.json"),
                              "--signature", str(self.root / "ack.sig"))
        self.assertEqual(out.returncode, 0 if success else 8, out.stderr)
        if not success:
            self.assertEqual(out.stdout, b"")

    def test_valid_receipt_and_new_push(self):
        self.verify(True)
        self.head = "c" * 40
        self.verify()

    def test_new_base_invalidates(self):
        self.base = "d" * 40
        self.verify()

    def test_exact_verdict_bytes(self):
        with (self.root / "verdict.json").open("ab") as stream:
            stream.write(b"\n")
        self.verify()

    def test_wrong_key_signature(self):
        self.new_key("other")
        self.sign(key="other")
        self.verify()

    def test_unknown_or_revoked_reviewer(self):
        (self.keys / "allowed_signers").write_text("")
        self.verify()

    def test_wrong_namespace(self):
        self.sign(namespace="another-application")
        self.verify()

    def test_expired_receipt(self):
        receipt = json.loads((self.root / "ack.json").read_bytes())
        receipt["expires_at"] = 1
        (self.root / "ack.json").write_text(json.dumps(receipt, sort_keys=True, separators=(",", ":")) + "\n")
        self.sign()
        self.verify()

    def test_candidate_allowed_signers_cannot_enroll_key(self):
        self.new_key("candidate-key")
        candidate = self.root / "candidate/.github/hold-reviewers"
        candidate.mkdir(parents=True)
        (candidate / "allowed_signers").write_text("fixture " + (self.root / "candidate-key.pub").read_text())
        self.sign(key="candidate-key")
        self.verify()

    def test_noncanonical_signed_receipt(self):
        receipt = json.loads((self.root / "ack.json").read_bytes())
        (self.root / "ack.json").write_text(json.dumps(receipt, indent=2))
        self.sign()
        self.verify()

    def test_signature_tamper(self):
        (self.root / "ack.sig").write_bytes(bytes(64))
        self.verify()

    def test_signed_incomplete_rules_and_extra_fields(self):
        original = (self.root / "ack.json").read_bytes()
        for field, value in [("held_rules", []), ("held_rules", ["extra-rule"]), ("repository", "other/repo"), ("extra", True)]:
            with self.subTest(field=field):
                receipt = json.loads(original)
                receipt[field] = value
                (self.root / "ack.json").write_text(json.dumps(receipt, sort_keys=True, separators=(",", ":")) + "\n")
                self.sign()
                self.verify()

    def test_duplicate_json_keys(self):
        raw = (self.root / "ack.json").read_bytes()
        (self.root / "ack.json").write_bytes(b'{"head":"' + self.head.encode() + b'",' + raw[1:])
        self.sign()
        self.verify()

    def test_unknown_evidence_cannot_be_prepared(self):
        self.verdict["notes"] = [{"code": "rank_unverified"}]
        self.write_verdict()
        self.assertEqual(self.run_helper("prepare", "--signer", "fixture").returncode, 8)

    def test_fail_pass_unknown_schema_and_unverified_refuse(self):
        original = json.dumps(self.verdict)
        for label in ("fail", "pass", "schema", "unverified", "expired"):
            with self.subTest(label=label):
                self.verdict = json.loads(original)
                if label in ("fail", "pass"):
                    self.verdict["verdict"] = label
                elif label == "schema":
                    self.verdict["schema"] = "forge-warden.verdict.v1"
                elif label == "unverified":
                    self.verdict["seals"]["patch_verified"] = False
                else:
                    self.verdict["expired_waivers"] = ["expired"]
                self.write_verdict()
                self.assertEqual(self.run_helper("prepare", "--signer", "fixture").returncode, 8)

    def test_malformed_oversized_and_path_signer_refuse(self):
        for data in (b"[]", b"{", b"x" * 16385, b'{"signer":"../../other"}'):
            with self.subTest(data=data[:20]):
                (self.root / "ack.json").write_bytes(data)
                self.verify()


if __name__ == "__main__":
    unittest.main()
