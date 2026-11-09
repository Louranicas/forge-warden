#!/usr/bin/env python3
"""Independent measured-capacity and failure tests for the mutation planner."""
import importlib.util
from pathlib import Path
import unittest
import os
import subprocess
import sys
from unittest.mock import patch

# Dynamic imports must not leave bytecode in the committed subject before plants.
sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('mutation_ci', ROOT / 'scripts/mutation_ci.py')
M = importlib.util.module_from_spec(spec)
spec.loader.exec_module(M)

class Budget(unittest.TestCase):
    def test_integration_diff_fits_bounded_shards(self):
        plan = M.matrix(305, 'a' * 64)
        rows = plan['include']
        self.assertEqual(len(rows), 16)
        self.assertEqual([r['index'] for r in rows], list(range(16)))
        self.assertTrue(all(r['shards'] == 16 and r['total'] == 305 for r in rows))
        self.assertTrue(all(r['minutes'] == 110 for r in rows))
        # Serial worst-observed-cost replay exceeds both old60m and GitHub6h.
        self.assertGreater(361 + 305 * 194, 6 * 3600)
        self.assertLess((361 + 20 * 194) * 1.5 + 180, 110 * 60)

    def test_boundary_sizes_and_matrix_limit(self):
        for total, shards in ((0, 1), (1, 1), (20, 1), (21, 2), (5120, 256)):
            rows = M.matrix(total, 'a' * 64)['include']
            self.assertEqual(len(rows), shards)
            self.assertTrue(all(0 < r['minutes'] <= 110 for r in rows))
        for total in (-1, 5121, True, 1.5):
            with self.assertRaises(ValueError):
                M.matrix(total, 'a' * 64)

    def test_every_non_success_dependency_fails(self):
        for a in ('success', 'failure', 'cancelled', 'skipped', '', 'unknown'):
            for b in ('success', 'failure', 'cancelled', 'skipped', '', 'unknown'):
                self.assertEqual(M.aggregate(a,b), a == b == 'success')

class Execution(unittest.TestCase):
    def supplied(self):
        return dict(SHARD_INDEX='0', SHARD_COUNT='16', MUTANT_TOTAL='305',
                    INVENTORY_DIGEST='a'*64, SHARD_MINUTES='110')

    def test_drift_refuses_before_running_mutants(self):
        changes = dict(SHARD_INDEX='16', SHARD_COUNT='15', MUTANT_TOTAL='304',
                       INVENTORY_DIGEST='b'*64, SHARD_MINUTES='111')
        for key,value in changes.items():
            env=self.supplied();env[key]=value
            with self.subTest(key=key), patch.dict(os.environ,env,clear=True), \
                 patch.object(M,'inventory',return_value=(['cargo','mutants'],{},305,'a'*64)), \
                 patch.object(M.subprocess,'run') as run:
                with self.assertRaises(ValueError): M.main(['run','diff'])
                run.assert_not_called()

    def test_missed_failure_and_timeout_are_not_success(self):
        for code in (2,3,4,124):
            with self.subTest(code=code), patch.dict(os.environ,self.supplied(),clear=True), \
                 patch.object(M,'inventory',return_value=(['cargo','mutants'],{},305,'a'*64)), \
                 patch.object(M.subprocess,'run',side_effect=subprocess.CalledProcessError(code,['cargo'])) as run:
                with self.assertRaises(subprocess.CalledProcessError): M.main(['run','diff'])
                self.assertTrue(run.call_args.kwargs['check'])
                self.assertEqual(run.call_args.args[0][-3:], ['--shard','0/16','-vV'])

    def test_actual_empty_diff_is_not_invalid_json_or_full_workspace(self):
        with patch.dict(os.environ,{'BASE_SHA':'a'*40},clear=True), \
             patch.object(M.Path,'write_bytes'), \
             patch.object(M.subprocess,'check_output',return_value=b'') as read:
            command,env,total,digest=M.inventory('diff')
            self.assertEqual(total,0)
            self.assertIn('--in-diff',command)
            self.assertEqual(read.call_count,1)  # git diff only; cargo emits no JSON here.
        with patch.dict(os.environ,{'BASE_SHA':'a'*40},clear=True), \
             patch.object(M.Path,'write_bytes'), \
             patch.object(M.subprocess,'check_output',side_effect=[b'nonempty',b'']):
            with self.assertRaises(ValueError): M.inventory('diff')

    def test_both_scopes_keep_honest_flags(self):
        for scope in ('diff','full'):
            with self.subTest(scope=scope), patch.dict(os.environ,{'BASE_SHA':'a'*40,'CARGO_TARGET_DIR':'/inherited'},clear=True), \
                 patch.object(M.Path,'write_bytes'), \
                 patch.object(M.subprocess,'check_output',side_effect=([b'diff',b'[]'] if scope=='diff' else [b'[]'])) as read:
                command,env,total,digest=M.inventory(scope)
                self.assertEqual(total,0)
                self.assertNotIn('CARGO_TARGET_DIR',env)
                for flag in ('--no-config','--cap-lints','--no-shuffle','--workspace','--cargo-arg=--locked'):
                    self.assertIn(flag,command)
                self.assertEqual(command[command.index('--cap-lints')+1],'true')
                self.assertEqual(command[command.index('--jobs')+1],'1')
                self.assertEqual('--in-diff' in command,scope=='diff')
                self.assertFalse(any('baseline' in arg for arg in command))
                self.assertEqual(read.call_args.args[0][-2:],['--list','--json'])

if __name__ == '__main__':
    unittest.main()
