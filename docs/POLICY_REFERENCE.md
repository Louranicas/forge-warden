# Policy reference (`forge-warden.policy.v0`)

The authority is `forge-warden lint`. The JSON Schema `schemas/policy.v0.schema.json` gives
editors the same rules as you type; a test keeps the two equal.

## Top level

| Key | Required | Meaning |
| --- | --- | --- |
| `schema` | yes | Exactly `"forge-warden.policy.v0"`. |
| `[[rule]]` | yes, one or more | An empty policy is refused: it would pass every change. |
| `[[waiver]]` | no | Dated, owned exemptions. |

Unknown keys anywhere are refused, not ignored: a misspelt key would otherwise silently disable
what it meant to configure.

## Every rule

| Key | Meaning |
| --- | --- |
| `id` | Kebab-case (`[a-z][a-z0-9]*(-[a-z0-9]+)*`), at most 64 characters, unique. Appears in verdicts and waivers. |
| `why` | At least 12 characters. Shown to whoever the rule stops; write it for them. |
| `kind` | One of the kinds below. |
| `severity` | `fail` (exit 7) or `hold` (exit 8). No default. |

## Kinds

### `forbid_signal`
Violated by each in-scope file carrying any of `signals`. Each violation cites deep-diff-forge's
evidence for that signal (hunk address, side, line) when deep-diff-forge listed any.
Fields: `signals` (required), `paths` (default `["**"]`), `except`.

### `require_companion`
If any non-deleted file in `when` (minus `except`) changes, at least one file matching `require`
must change too, and that companion must still exist afterwards and have a knowable change. A
file is deleted when its status is `deleted` **or** it carries the `deleted_file` signal
(deep-diff-forge labels a deleted binary `binary_changed`). A `binary_changed` file counts as a
companion only if it is a `new_file`. One violation per triggering file.
Fields: `when`, `require` (both required), `except`.

### `protect_paths`
Violated by each in-scope file that changes in any way, deletion included.
Fields: `paths` (required), `except`.

### `max_score`
`per = "file"`: violated by each in-scope file whose risk score exceeds `limit`.
`per = "change"`: violated once (no path) when the summed score of in-scope files exceeds `limit`.
Fields: `limit`, `per` (both required), `paths`, `except`.

### `max_files`
Violated once (no path) when more than `limit` in-scope files change. `limit = 0` is refused:
use `protect_paths` to say "nothing may change here".
Fields: `limit` (required, at least 1), `paths`, `except`.

## Signals

deep-diff-forge 0.7.0's vocabulary. Path and shape: `public_api_surface`, `new_file`,
`deleted_file`, `binary_change`, `large_change`, `many_hunks`, `config_or_lockfile`, `test_only`,
`generated_or_vendored`, `hazardous_text`, `symlink_change`, `executable_bit`. Code evidence:
`unsafe_added`, `process_spawn`, `network_fetch`, `dynamic_eval`, `ci_privilege`, `build_hook`,
`secret_like`, `check_suppressed`, `test_removed`. deep-diff-forge's README documents what each
detects; its code evidence is lexical, so it finds tokens and does not prove a change safe.

## Globs

Relative to the repository root; a leading `/` or `./` is refused. `*` matches within one path
segment, `**` across segments, `?` one character, `[abc]` a class, `{a,b}` alternatives, and `\`
escapes. `src/*.rs` matches `src/lib.rs` but not `src/a/lib.rs`.

## Waivers

| Key | Meaning |
| --- | --- |
| `rule` | The id of a rule in this policy. |
| `paths` | Globs. The waiver covers a violation whose path matches. |
| `reason` | At least 12 characters: why the exemption is acceptable. |
| `owner` | Who answers for it. |
| `expires` | `YYYY-MM-DD`, the last day it applies. |

A violation is covered by the first active waiver (today on or before `expires`) for its rule
whose `paths` match. Change-wide violations (`max_score` per change, `max_files`) have no path
and cannot be waived. From the day after `expires`, the waiver fails the verdict until it is
renewed or deleted. An active waiver that covered nothing is listed in `unused_waivers`.
