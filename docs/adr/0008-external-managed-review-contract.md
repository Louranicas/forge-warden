# ADR 0008: Managed review instructions come from outside the candidate

Status: proposed implementation, WP-D3. Author: Bastion (O4).

The old managed definition read candidate CLAUDE.md first, installed candidate-selected tooling,
ran candidate CI in its own harness, and fell back to candidate warden.toml if the base lacked one.
A cold session did not make those inputs trustworthy.

The owner-controlled launcher obtains the authenticated base/head and runs the base's bundle script.
The script checks clean separate checkouts and exact HEADs; requires regular base instruction and
policy blobs; emits Git-object bytes plus a subject/digest manifest into a new private directory
outside both trees. It disables Git fsmonitor for status checks so candidate-local config cannot
execute a monitor. Missing policy/instructions or an unsupported base refuses; no candidate fallback.
The provider receives the bundled JSON, with the bundle mounted read-only at a fixed external path.

The system instruction reads only that external contract and base CLAUDE.md as project authority.
Candidate files are review data. Candidate tests execute only in a separate disposable worker;
the reviewer consumes their logs as data. Trusted judgment uses ADR0006. Model recommendations
remain advisory; external admission and any signed HOLD receipt remain separate controls.

| Guarantee | Rung | Evidence / remaining boundary |
|---|---|---|
| Bundle bytes come from base, not candidate | architectural separation plus runtime identity admission | real Git substitution fixtures; output hash manifest is not authentication |
| Missing base policy/instructions refuse | admission | missing-file and symlink tests |
| Candidate-local config does not run fsmonitor | admission | executable marker fixture; candidate-specific status disables hook |
| Bundle cannot be inside candidate/trusted trees | admission | candidate output-path refusal test |
| Runtime ignores candidate instructions/hooks | deployment architecture required | UNMEASURED on managed provider; owner must disable discovery and pin initial agent definition |
| Candidate tests cannot alter judge/bundle | deployment architecture required | UNMEASURED; separate isolated workers, no shared writable caches/credentials |
| Agent advice is not merge permission | authority architecture | external owner-controlled admission; no merge/signing capability provisioned here |

An instruction telling a model to ignore hostile text is not isolation. This patch establishes the
local bundle selection boundary and removes the unsafe recipe; it does not claim the provider
actually enforces mounts, tool permissions or instruction discovery. The README specifies the
launcher contract and acceptance checks without guessing hosted API fields. Do not ship an old
candidate-mounted launcher and call this closed because the template changed.
