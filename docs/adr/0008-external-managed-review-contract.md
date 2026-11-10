# ADR 0008: Managed review instructions come from outside the candidate

Status: Accepted
Owner-deployment: Required

Accepted = design contract, not deployment; see [ADR status](../ADR_STATUS.md).

Historical status record: Status: proposed implementation, WP-D3. Author: Bastion (O4).

The old managed definition read candidate CLAUDE.md first, installed candidate-selected tooling,
ran candidate CI in its own harness, and fell back to candidate warden.toml if the base lacked one.
A cold session did not make those inputs trustworthy.

The owner-controlled launcher obtains the authenticated base/head and runs the base's bundle script.
The controller imports authenticated base/head objects into its own trusted repository. The script
checks the base HEAD, verifies the head commit object there and checks its own bytes against the
base helper blob; then requires regular base instruction/policy blobs and emits their Git-object
bytes plus a subject/digest manifest outside both trees. It never invokes Git in the candidate
checkout and never runs status, checkout, index refresh, fetch or upload-pack. Dirty worktree
copies are irrelevant; the candidate directory is only an output-containment boundary. Missing policy/instructions or an unsupported base refuses; no candidate fallback.
The provider receives the bundled JSON, with the bundle mounted read-only at a fixed external path.

The system instruction reads only that external contract and base CLAUDE.md as project authority.
Candidate files are review data. Candidate tests execute only in a separate disposable worker;
the reviewer consumes their logs as data. Trusted judgment uses ADR0006. Model recommendations
remain advisory; external admission and any signed HOLD receipt remain separate controls.

| Guarantee | Rung | Evidence / remaining boundary |
|---|---|---|
| Bundle bytes come from base, not candidate | architectural separation plus runtime identity admission | real Git substitution fixtures; output hash manifest is not authentication |
| Missing base policy/instructions refuse | admission | missing-file and symlink tests |
| Candidate config cannot select a Git extension | architectural separation | no candidate Git invocation; clean/smudge/textconv/diff/hooks/include marker fixtures with positive controls |
| Bundle cannot be inside candidate/trusted trees | admission | candidate output-path refusal test |
| Runtime ignores candidate instructions/hooks | deployment architecture required | UNMEASURED on managed provider; owner must disable discovery and pin initial agent definition |
| Candidate tests cannot alter judge/bundle | deployment architecture required | UNMEASURED; separate isolated workers, no shared writable caches/credentials |
| Agent advice is not merge permission | authority architecture | external owner-controlled admission; no merge/signing capability provisioned here |

An instruction telling a model to ignore hostile text is not isolation. This patch establishes the
local bundle selection boundary and removes the unsafe recipe; it does not claim the provider
actually enforces mounts, tool permissions or instruction discovery. The README specifies the
launcher contract and acceptance checks without guessing hosted API fields. Do not ship an old
candidate-mounted launcher and call this closed because the template changed.

## Independent counterexample and correction

Touchstone proved the first implementation ran a candidate clean filter during git status even
with fsmonitor disabled. The correction removes the worktree operation rather than enumerating
filter names. The trusted Git wrapper clears inherited environment, disables user/system config,
attributes, replace objects, hooks and fsmonitor, and accesses only controller-owned repository
configuration. See ADR0010 for the shared D3/C1 contract. Import of objects and authentication of
the helper remain controller responsibilities; this source-consistency check is not its own
authority root. No candidate cleanliness or hosted deployment guarantee is claimed.
