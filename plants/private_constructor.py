#!/usr/bin/env python3
"""A compile-negative external consumer: unchecked RankDoc literals must stay impossible."""
import json
from pathlib import Path
import subprocess
import tempfile


def main():
    root = Path(__file__).resolve().parents[1]
    with tempfile.TemporaryDirectory(prefix='a4-private-') as directory:
        probe = Path(directory)
        (probe / 'src').mkdir()
        (probe / 'Cargo.toml').write_text(
            '[package]\nname="a4-private-probe"\nversion="0.0.0"\nedition="2024"\n'
            '[dependencies]\nforge-warden-core={path=' + json.dumps(str(root / 'crates/forge-warden-core'))
            + '}\n[workspace]\n')
        (probe / 'Cargo.lock').write_bytes((root / 'Cargo.lock').read_bytes())
        (probe / 'src/main.rs').write_text(
            'fn main() { let _invalid = forge_warden_core::rank::RankDoc { '
            'input_sha256: "invalid".to_owned(), tool_version: String::new(), files: vec![] }; }\n')
        result = subprocess.run(['cargo', 'check', '--offline', '--message-format=json'],
                                cwd=probe, capture_output=True, text=True, timeout=120)
        messages = [json.loads(line) for line in result.stdout.splitlines()]
        errors = [m['message'] for m in messages if m.get('reason') == 'compiler-message'
                  and m['message']['level'] == 'error']
        codes = [(m.get('code') or {}).get('code') for m in errors]
        print('EXTERNAL_CONSTRUCTOR_ERRORS:', json.dumps(codes))
        if result.returncode == 101 and errors and all(code == 'E0451' for code in codes):
            if all('RankDoc' in m.get('rendered', '') for m in errors):
                print('PRIVATE_CONSTRUCTION_REFUSED')
                return 0
        if result.returncode == 0 and not errors and any(
                m.get('reason') == 'build-finished' and m.get('success') for m in messages):
            print('PRIVATE_CONSTRUCTION_ESCAPED')
            return 1
        print('ERROR_CONSTRUCTOR_PROBE:', result.stderr)
        for error in errors:
            print(error.get('rendered', ''))
        return 1


if __name__ == '__main__':
    raise SystemExit(main())
