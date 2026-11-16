# Release checklist: forge-warden 0.2.1

This is the checklist for the 0.2.1 release. 0.2.1 is 0.2.0 (`8f18db3`, tag `v0.2.0`) plus the packages
below, each merged through the integrator gate. Each line that is done names the commit that did it. The
lines at the end can only be done by the repository's owner and are not ticked by anyone else.

0.2.1 is a patch release: no published schema or exit code changes, and a 0.2.0 user acts only on
two starter-policy rules ([Upgrading](UPGRADING.md), "0.2.0 to 0.2.1").

## Decisions recorded before the tag

- [x] **Verdict schema.** It stays `forge-warden.verdict.v2`; `schemas/` is unchanged since `8f18db3`.
- [x] **Policy schema.** It stays `forge-warden.policy.v0`. The starter policy gains rule
  `binary-text-is-hidden-text` and two `.claude` globs in `ci-is-owned`; a policy adopted from the 0.2.0
  starter keeps its own rules until it is edited.
- [x] **Exit codes.** Unchanged. Two refusal details read differently (a type change; strict numstat
  counts); the note code is the same (`patch_unverifiable`).

## What 0.2.1 carries (merged on hub main)

- [x] Q57: a test keeps the string vocabulary predicates equal to the typed vocabulary (`7800dad`). Test only.
- [x] Q58: numstat counts read strictly (1 to 20 digits, or `-` `-`); the no-header refusal names
  `scripts/make_patch.sh`; the dogfood download stays on HTTPS through redirects (`e1ee50b`).
- [x] Q61 and HOLD-REVIEWER: who can acknowledge a hold, for how long; the owner's hold reviewer key
  enrolled (`9cb7513`).
- [x] PROTECT-CLAUDE: the starter's `ci-is-owned` holds `.claude` and everything under it, any letter case (`acaa280`).
- [x] Q60: a file/symlink type change is refused as a type change (`933ca9f`).
- [x] HEADER-OWN: forge-warden's own header check refuses a lone `diff --git` header; five hostile
  shapes and a control under `fixtures/hostile/lone-header/`, each with the numstat Git reported (`3f5054b`). Test and fixtures only.
- [x] Q53: the plants README count is derived from the catalogue (`5e75cc3`). Test only.
- [x] BINARY-TEXT-HOLD (Q63): rule `binary-text-is-hidden-text` in the starter and in `warden.toml` (`26330b9`).
  Not covered, stated: an executable with a name the rule does not list (Q64; verdict v3).
- [x] G9: the gate image's commands are checked against every tracked script (`92a9ac9`). Test and fast gate only.

## The release preparation

- [x] Workspace version and both `Cargo.lock` entries 0.2.0 to 0.2.1 (this preparation).
- [x] `fixtures/demo.verdict.json`: one line changed, the tool version; regenerated from the built binary by
  the bless path; its reviewed digest in `lint-expectations.toml` updated (this preparation). Byte-equal by another
  lane: the verifier's.
- [x] README: the demo block shows what the 0.2.1 binary prints (run in the gate image, which has no Git
  allowlist); the install line pins `v0.2.1` (this preparation).
- [x] CHANGELOG: `[0.2.1]` under an empty `[Unreleased]`, with the schema and exit-code statement (this preparation).
- [x] [The runbook](RUNBOOK.md), step 2: what was run for v0.2.0, and that 0.2.1's install run comes after
  the tag (this preparation).
- [x] [Upgrading](UPGRADING.md): "0.2.0 to 0.2.1" (this preparation).
- [x] `EVIDENCE.md`: a 0.2.1 section with the numbers of the release candidate (this preparation); the integrator
  gate's receipt on the release commit is the one line left for the integrator.

## Gate

- [x] Train A: integrator gate GREEN on `acaa280` (Q57, Q58, Q61, HOLD-REVIEWER, PROTECT-CLAUDE); changed
  lines' mutants 9: 8 caught, 1 not buildable, none missed.
- [x] Train A2: GREEN on `5e75cc3` (Q60, HEADER-OWN, Q53); 10 mutants: 9 caught, 1 not buildable, none missed.
- [x] Train A3b: GREEN on `26330b9` (BINARY-TEXT-HOLD); no source mutation sites. Train A3 before it was red
  on the hosted-parity export (G9's reader needed Git there); G9 was fixed and went in its own train.
- [x] Train A4: GREEN on `92a9ac9` (G9), the hosted-parity export phase included; no Rust change.
- [ ] The gate on the release commit (this preparation).

## After the tag, by a lane

- [ ] [The runbook](RUNBOOK.md), steps 2 and 6 with the tagged release: install from `--tag v0.2.1` into a
  scratch root with a fresh `CARGO_HOME`, and the README's advisory sample byte for byte in the gate image
  without network (as was done for v0.2.0).

## Only the owner

- [ ] Merge the release commit to GitLab `main` through a merge request (GitLab, Gitea, Forgejo only; nothing
  at GitHub).
- [ ] Create the signed tag `v0.2.1` on that commit: `mission6/tools/fw-release-tag.sh 0.2.1 <main sha>` (it
  signs with the release key and pushes the tag to GitLab, Gitea and Forgejo). Until the tag exists the
  README's install line names a tag that is not there.
- [ ] Make the changelog date (now 2026-11-16, the date of the release commit) the date of the tag.
- [ ] crates.io: not planned for 0.2.1.
