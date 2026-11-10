#!/usr/bin/env python3
"""Compile-negative external consumers: unchecked RankDoc values must stay impossible.

Two probes, so each guard is tested for its own cause (Q-TYPES-1 v4: `#[non_exhaustive]` on RankDoc
now refuses a literal by itself, which would hide a field made public):
- literal: `RankDoc { .. }` must fail only with E0639 (non-exhaustive) or E0451 (private field);
- fields: writing each of the three fields of an existing RankDoc must fail with E0616 (private
  field) exactly once per field. This is the field-privacy cause, independent of non_exhaustive.
Any other outcome is a probe error, never a pass: an arbitrary compile failure proves nothing.
Both probes use type-correct values (a Sha256Hex seal since Q-TYPES-2), so privacy alone decides.
"""
import json
from pathlib import Path
import subprocess
import tempfile

FIELDS = ('input_sha256', 'tool_version', 'files')
LITERAL = ('fn main() { let _invalid = forge_warden_core::rank::RankDoc { '
           'input_sha256: forge_warden_core::seal::sha256_hex(b"x"), tool_version: String::new(), '
           'files: vec![] }; }\n')
WRITES = ('pub fn forge(doc: &mut forge_warden_core::rank::RankDoc) { '
          'doc.input_sha256 = forge_warden_core::seal::sha256_hex(b""); doc.tool_version = String::new(); '
          'doc.files = Vec::new(); }\nfn main() {}\n')


def check(root, name, source):
    """cargo check one external consumer; returns (returncode, errors, messages, stderr)."""
    with tempfile.TemporaryDirectory(prefix='a4-private-') as directory:
        probe = Path(directory)
        (probe / 'src').mkdir()
        (probe / 'Cargo.toml').write_text(
            '[package]\nname="a4-private-' + name + '"\nversion="0.0.0"\nedition="2024"\n'
            '[dependencies]\nforge-warden-core={path=' + json.dumps(str(root / 'crates/forge-warden-core'))
            + '}\n[workspace]\n')
        (probe / 'Cargo.lock').write_bytes((root / 'Cargo.lock').read_bytes())
        (probe / 'src/main.rs').write_text(source)
        result = subprocess.run(['cargo', 'check', '--offline', '--message-format=json'],
                                cwd=probe, capture_output=True, text=True, timeout=120)
        messages = [json.loads(line) for line in result.stdout.splitlines()]
        errors = [m['message'] for m in messages if m.get('reason') == 'compiler-message'
                  and m['message']['level'] == 'error']
        return result.returncode, errors, messages, result.stderr


def codes(errors):
    return [(m.get('code') or {}).get('code') for m in errors]


def compiled(returncode, errors, messages):
    return returncode == 0 and not errors and any(
        m.get('reason') == 'build-finished' and m.get('success') for m in messages)


def main():
    root = Path(__file__).resolve().parents[1]
    literal = check(root, 'literal', LITERAL)
    writes = check(root, 'fields', WRITES)
    print('EXTERNAL_CONSTRUCTOR_ERRORS:', json.dumps(codes(literal[1])))
    print('EXTERNAL_FIELD_WRITE_ERRORS:', json.dumps(codes(writes[1])))
    if compiled(*literal[:3]) or compiled(*writes[:3]):
        print('PRIVATE_CONSTRUCTION_ESCAPED')
        return 1
    literal_refused = (literal[0] == 101 and literal[1]
                       and all(code in ('E0639', 'E0451') for code in codes(literal[1]))
                       and all('RankDoc' in m.get('rendered', '') for m in literal[1]))
    named = sorted(field for m in writes[1] for field in FIELDS
                   if f'field `{field}`' in m.get('rendered', ''))
    writes_refused = (writes[0] == 101 and codes(writes[1]) == ['E0616'] * len(FIELDS)
                      and named == sorted(FIELDS))
    if literal_refused and writes_refused:
        print('PRIVATE_CONSTRUCTION_REFUSED')
        return 0
    print('ERROR_CONSTRUCTOR_PROBE:', literal[3], writes[3])
    for error in literal[1] + writes[1]:
        print(error.get('rendered', ''))
    return 1


if __name__ == '__main__':
    raise SystemExit(main())
