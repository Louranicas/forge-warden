#!/usr/bin/env python3
"""Require the normal compiler to accept the specimen before testing the pinned MSRV."""
import subprocess


def main():
    args = ['check', '--workspace', '--all-targets', '--locked', '--offline']
    normal = subprocess.run(['cargo', *args], capture_output=True, text=True, timeout=120)
    if normal.returncode:
        print('ERROR_NORMAL_COMPILER:', normal.stderr)
        return 1
    old = subprocess.run(['cargo', '+1.88.0', *args], capture_output=True, text=True, timeout=120)
    if old.returncode == 0:
        print('MSRV_POSITIVE_CONTROL')
        return 0
    if old.returncode == 101 and 'error[E0658]' in old.stderr and 'round_char_boundary' in old.stderr:
        print(old.stderr)
        print('MSRV_BOUNDARY_REJECTED')
        return 1
    print('ERROR_MSRV_PROBE:', old.returncode, old.stderr)
    return 1


if __name__ == '__main__':
    raise SystemExit(main())
