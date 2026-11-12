# Threat model

## Scope and evidence status

Scope (2026-11-10): this record was written for commit `a68506f` and has not been
re-baselined since. Later changes (the effect-door split, Git admission, verdict v2, runner
routing) are described by their ADRs and tests, not here; the risk table below links current
tests, which are the check. The record is re-measured on the release commit before 0.2.0.

This WP-D5 design record describes merged hub main `a68506f`, including C1c v4.4,
D6/D8, B5, E3a and the reviewed expectation/data ledger. Source integration is
accepted; this does not assert deployment or acceptance of later work packages.
The coordination record reports the owner's L1 option-A golden acceptance and
completion of the L3 DDF v0.8.0 release decision. These are recorded owner decisions,
not independent release-signature or deployment verification by this document.

The C1c v4.4 boundary review passed on `8b4e76c`; Touchstone's independent stress
verdict records 1000/1000 complete workspace invocations (176000 test observations,
zero failures, ignored tests or retries) on that subject. Lodestar's exact merged-tree
gate on `a68506f`, parent `d214a8b`, is GREEN: 192 tests, zero ignored, MSRV checked;
120 mutants comprise 66 caught, 54 unviable and zero missed. These receipts have
different subjects and must not be conflated. Evidence: the project's coordination records
(not in this repository): the mission-4 work board, Touchstone's C1c v4.4 verdict and the
C1c-stack merge receipt.

Repository links below identify implementation and repeatable tests. Mission evidence
paths are relative to the separate forge-warden coordination root, not files shipped
in this repository. A fixture proves the property it exercises, not hosted identity,
all possible input behavior, or safety of a compromised runtime. This document adds
no runtime control and performs no new adversarial probes.

## Assets and actors

| Asset | Property to preserve | Authority and limit |
| --- | --- | --- |
| Verdict, notes and byte seals | Exact policy, ranking and patch identification; deterministic outcome; no incomplete analysis presented as PASS | The trusted judge produces the verdict. SHA-256 identifies bytes, not a signer or approval. |
| Policy, including `[producer].sha256` | Reviewed rules, waivers and executable allowlist | Trusted base/release policy selected by an external controller; candidate policy is data for review. |
| Ranker executable identity | The descriptor executed names the bytes admitted by policy | Digest admission and descriptor pinning; no build provenance or same-uid write protection. |
| HOLD receipt and allowed signers | Consent for exact repository/base/head/verdict/rule set, before expiry | Enrolled reviewer signature plus trusted controller subject and clock; private key custody is external. |
| Protected-path manifest, lint floor and expectation/data ledger | Reviewed guardrails cannot drift silently between representations | Owned manifests and checked equality; simultaneous edits to checker and manifest still need trusted enforcement/review. |
| CI judge, driver and evidence | Candidate changes cannot choose their current judge; failures propagate | Trusted base code and separate evidence directory; hosted merge-admission identity is an external requirement. |
| Review contract and candidate bytes | Instructions come from the trusted contract; candidate content remains evidence | Local bundle selection is checked; provider isolation must be established separately. |

The patch author controls diff bytes, paths and content. The PR author may additionally
change every candidate file, including workflow YAML, agent instructions, policy,
build configuration and proposed tests. Neither is an authority for the current
judgment. DDF is an admitted producer whose output can still be wrong or incomplete.
The CI runner supplies the trusted runtime and must keep candidate execution separate.
Maintainers choose future policy/code and enroll reviewers; an acknowledger may consent
only within the signed-receipt contract. Claude Code hooks and managed reviewers assist
work but do not grant merge permission. Other local processes with the same uid, host
administrators and a compromised runtime are outside the containment claim.

## Enforcement rungs

The project ladder is: **1 architecture**, **2 types**, **3 runtime admission**,
**4 lint/build invariant**, **5 behavioral test**, **6 review**. A test that enforces
repository structure is rung 4; a test demonstrating behavior is evidence for the
underlying control, not an upgrade of that control. The table names the highest
implemented control for each property and why a stronger claim is unavailable.

## Trust boundaries and proving evidence

| Boundary | Control and failure behavior | Rung and why not higher | Evidence / design |
| --- | --- | --- | --- |
| Candidate PR → current judge | Base driver checks full subject IDs, clean base and separate candidate/evidence trees; builds base code/policy; bootstrap refuses without supported driver. D8 explicitly selects authority when producer policy is present; malformed policy refuses, with no fallback. | 1 separation, 3 identity admission. Filesystem/process identity needs runtime checks; a candidate-defined workflow cannot authenticate itself. | [dogfood fixtures](../scripts/tests/dogfood.sh), [mode/exit fixtures](../scripts/tests/ci_local.sh), [ADR 0006](adr/0006-trusted-dogfood-driver.md). L4 remains external. |
| DDF or supplied rank → authority verdict | CLI requires patch bytes. Authority runs the admitted ranker; supplied-only mode has no PASS outcome. A mismatching optional supplied ranking adds a HOLD note while the produced bytes are judged/sealed. | 2 provenance/outcome types, 3 producer admission, 4 mint-site lint. External provenance cannot be inferred from a deserialized string; public library attestation is not OS authentication. | [authority types/tests](../crates/forge-warden-core/tests/authority.rs), [authority-mode tests](../crates/forge-warden-cli/tests/authority_mode.rs), [mint lint](../crates/forge-warden-cli/tests/mint_lint.rs), [ADR 0005](adr/0005-coverage-is-verified-not-claimed.md). |
| Git executable and owner policy → CLI execution | Fixed root-owned `/etc/forge-warden/git-allowlist.toml`, protected ancestors, bounded regular ELF descriptor, digest membership and one policy snapshot. Every Git invocation uses that descriptor. Missing policy permits only explicitly unpinned advisory evidence; authority refuses. | 2 private admitted descriptor; 3 runtime metadata/digest checks. In-place owner mutation, loader and support files remain trusted. | `git_admission_binds_descriptor_and_exact_policy_bytes`, refusal tests in [doors/mod.rs](../crates/forge-warden-cli/src/doors/mod.rs), current-policy checks in [hold_ack.py](../scripts/hold_ack.py); [enrollment and limits](GIT_RUNTIME.md), ADR 0011 with Mission 6 R2. |
| Patch bytes → Git facts → coverage | One bounded patch snapshot feeds Git and ranker. Exact paths and supported kind/mode facts are compared; unsupported/ambiguous evidence becomes unverifiable/HOLD. Git-derived change-kind signals only add to producer signals. FAIL is never weakened to HOLD. | 2 checked coverage/domain values; 3 total admission/comparison. Types cannot establish truth of external bytes without runtime evidence. | [coverage corpus](../crates/forge-warden-core/tests/coverage.rs), `git_still_checks_the_producer_and_refusals_hold` in [authority tests](../crates/forge-warden-cli/tests/authority_mode.rs); ADR 0005 §§2.2, 2.8–2.9. |
| Executable path → executed ranker | Open once with no-follow/nonblocking admission; require regular file, acceptable owner/mode, bounded hashing and policy digest; execute the pinned descriptor. Scripts and unsupported execution environments refuse rather than downgrade. | 3 descriptor/runtime admission. A Rust path type cannot prevent filesystem replacement or establish binary trust. | `a_pinned_executable_runs_the_hashed_bytes_after_its_path_is_swapped`, `every_file_door_refuses_a_fifo_at_once_not_waited_on`, pin/refusal tests in [doors/mod.rs](../crates/forge-warden-cli/src/doors/mod.rs); ADR 0005 §2.4. |
| Parent environment / candidate Git state → Git or ranker child | Cleared environment, absolute program, argv/stdin separation, private cwd/HOME. Temporary base input must be absolute; its canonicalized path must be UTF-8 and colon-free. Git does not discover the surrounding candidate repository. D3 uses controller-owned object lookups, never candidate worktree operations. | 1 data/code separation plus 3 runtime admission. The host, trusted Git and controller repository remain assumptions. | `a_candidate_repository_around_the_private_dir_is_never_consulted`, `the_private_base_is_admitted_only_if_safe_for_cwd_home_and_ceiling` in [doors/mod.rs](../crates/forge-warden-cli/src/doors/mod.rs), [bundle fixtures](../scripts/tests/managed_review_bundle.py), [ADR 0010](adr/0010-isolated-git-data-contract.md). |
| Child process → judge resources/results | Bounded stdin/stdout/stderr and wall time; concurrently drain streams; cap/timeout/nonzero status is a failure, not successful truncation. Signal the group with SIGKILL; reap the direct child. Grandchildren remaining in the group are killed but not reaped by forge-warden; their init/subreaper performs reaping. Ranker failure gives no verdict; unavailable Git evidence prevents PASS. | 3 runtime limits. Types cannot bound wall time or contain processes; process groups are not isolation. | Timeout, grandchild, cap, signal and large-input tests in [doors/mod.rs](../crates/forge-warden-cli/src/doors/mod.rs); ADR 0005 §2.5. |
| Raw policy/ranking → pure evaluation | Private validated fields, canonical RepoPath, nonempty policy, typed rule kinds and exhaustive dispatch. Unknown vocabulary, duplicate ranked paths and incomplete evidence cannot silently approve. | 2 types after 3 parsing. Raw bytes must be admitted; type correctness does not prove DDF semantic correctness. | [policy tests](../crates/forge-warden-core/src/policy/tests.rs), [path tests](../crates/forge-warden-core/src/path.rs), [core adversarial tests](../crates/forge-warden-core/tests/adversarial.rs), [CLI adversarial tests](../crates/forge-warden-cli/tests/adversarial.rs), B5 [evaluation laws](../crates/forge-warden-core/tests/laws.rs) and [reference model](../fixtures/adversarial/reference_model.rs). |
| Effectful CLI → core / authority mint | Core effect/dependency restrictions; named doors; Clippy forbids attestation/raw judging outside the intended binary door, with parsed suppression checks. | 1 crate dependency direction; 4 lint/build checks for forbidden effects and mint calls. Rust visibility does not authenticate an external library caller. | [repo invariants](../crates/forge-warden-cli/tests/repo_invariants.rs), [door lint checks](../crates/forge-warden-cli/tests/door_lints.rs), [ADR 0001](adr/0001-pure-core-and-effect-doors.md), [ADR 0002](adr/0002-the-lint-wall.md). |
| HOLD evidence → acknowledgement admission | Detached OpenSSH signature, enrolled signer, exact subject/verdict/rule set, canonical bounded receipt and expiry. Notes/incomplete evidence, FAIL and expired waivers cannot be acknowledged. Verdict remains HOLD; admission is a separate audit result. | 3 signature/subject admission, supported by 1 separate trusted key/code ownership. Types cannot prove authorship, current remote head or revocation. | [receipt fixtures](../scripts/tests/hold_ack.py), [driver fixtures](../scripts/tests/dogfood.sh), [ADR 0007](adr/0007-subject-bound-hold-acknowledgements.md). |
| Agent tool request → repository | Hook refuses malformed requests and checks structured/literal edits; it is convenience only. Build checks enforce lint inheritance, effect and configuration invariants under a trusted gate. | Hook 3 limited admission; enforcing repo checks 4. Shell text inspection cannot provide OS write containment. | [hook tests](../.claude/hooks/test_guard.py), [repo invariants](../crates/forge-warden-cli/tests/repo_invariants.rs), [guardrail fixtures](../scripts/tests/guardrails.py). |
| Candidate instructions/tests → managed reviewer | Bundle base instructions/policy as object bytes outside both trees; refuse missing base data and candidate fallback. Candidate tests belong in a separate disposable worker; advice is not approval. | 1 intended separation, 3 local bundle admission. Provider mounts, permissions and instruction discovery are deployment requirements, not proven by prompts. | [bundle fixtures](../scripts/tests/managed_review_bundle.py), [managed contract](../agents/managed/REVIEW_CONTRACT.md), [ADR 0008](adr/0008-external-managed-review-contract.md). |
| Compiler inputs → source/expectation coverage | Exact workspace membership and dep-info admission; refuse outside-root/symlink sources and unsupported inputs; reviewed non-Rust data hashes. Production classification uses artifact kind/profile, not filenames; production `tests` components refuse. D6 compares both classifiers on the same build evidence. | 3 input admission, 4 coverage/ledger/agreement checks. Build evidence depends on trusted compiler/config/dependencies; agreement alone does not prove correctness. | [classifier agreement](../crates/forge-warden-cli/tests/classifier_agreement.rs), [source fixtures](../scripts/tests/guardrails.py), [Rust classifier](../crates/forge-warden-cli/tests/common/mod.rs), [ADR 0009](adr/0009-owned-guardrail-manifests.md). |
| Proposed guardrail/dependency/toolchain edits → future trusted base | Protected-path equality, lint-floor/ledger checks, dependency restrictions, pinned toolchain, lockfile and reviewed changes. Owner review remains necessary for coordinated changes to the controls themselves. | 4 build checks plus 6 policy intent review. Configuration cannot independently approve its own replacement. | [guardrail checker](../scripts/check_guardrails.py), [deny policy](../deny.toml), [toolchain pin](../rust-toolchain.toml), [repo invariants](../crates/forge-warden-cli/tests/repo_invariants.rs); ADR 0009. |
| Result bytes → human/automation | Escaped terminal rendering, deterministic JSON/seals, typed exit and note-code registries and exact script propagation. Schema tests compare the declared note-code vocabulary; output fixtures anchor the rendered/serialized contract. A success exit from help/lint is not a policy approval. Date-dependent waivers use the supplied date/runtime clock contract. | 2 typed exits, 3 admission/escaping, 5 determinism/date/propagation evidence. Semantic intent of a command is not encoded by exit zero alone. | [render tests](../crates/forge-warden-core/src/render.rs), [exit tests](../crates/forge-warden-core/src/exit.rs), [evaluation tests](../crates/forge-warden-core/tests/eval.rs), [CLI tests](../crates/forge-warden-cli/tests/cli.rs), E3a [schema parity](../crates/forge-warden-cli/tests/schema_tracks_code.rs), [output anchors](../crates/forge-warden-core/tests/adversarial_output.rs), [ADR 0004](adr/0004-sealed-deterministic-verdicts.md). |

## Explicit non-claims and deployment obligations

- **The effect wall is a list of spellings, not a closed capability.** Outside the door
  modules, clippy refuses the std and first-party functions that `clippy.toml` names
  (the reviewed names are pinned by a test, and clippy's refusal of each is measured).
  A std or third-party effect the list does not name is not refused. The closed design,
  a doors crate with the rest of the code under `forbid` (ledger Q26), is deferred to 0.3.0.
- **A patch is judged as it names its own paths.** A path is believed only under the default
  `a/` and `b/` prefixes (a reverse diff's `b/` and `a/` included); a patch made with
  `diff.noprefix` or other prefixes is refused, because Git and the ranker would agree on a
  path with one component missing (found in the freeze review: an authority-mode pass for a
  protected file). Prefixes that themselves begin `a/` and `b/` (`--src-prefix=a/x/`) give a
  well-formed patch for another path, and nothing in the patch says so. Nor does a patch say
  what tree it is a diff of: with `diff.relative` and a command run in a subdirectory, Git
  writes a well-formed patch that omits every change outside that directory, and authority
  mode passes it (found by the attack on this fix). **A verdict is about the patch bytes it was
  given, with paths read from the repository root. forge-warden does not check that the patch
  is the whole change between two commits; `--base` and `--head` are recorded as unverified
  claims.** Whoever makes the patch is inside the trust boundary: `scripts/dogfood.sh` makes it
  in a repository of its own with every option fixed, and the documented commands pass
  `--no-relative --src-prefix=a/ --dst-prefix=b/` from the top level. A patch made by hand
  with other settings can still pass for less than the change it claims to be.
- **This is not a sandbox.** Process-group cleanup covers descendants remaining in
  that group. It does not contain a ranker that leaves the group. The admitted
  ranker is trusted to execute; OS isolation is a separate deployment decision.
- **Same-uid writers are not contained.** Descriptor pinning prevents path replacement
  from selecting a different executable; it does not promise immutable inode bytes,
  loader/libraries, evidence, policy or toolchain against an arbitrary same-uid writer.
  Root, the kernel, Git, Python, OpenSSH, Cargo/rustc and the authenticated runner are
  trusted runtime components, not attested by a verdict digest.
- **L4 is not closed by repository code.** An owner-controlled external workflow or
  admission service must select current base/head, judge and policy, authenticate the
  check identity, keep candidate jobs separate, transport evidence, manage reviewer
  keys/revocation and recheck the subject atomically at admission. A same-named hosted
  check, a YAML file or a passing local fixture does not establish this. Hosted CI
  identity and managed-provider isolation remain unmeasured here. No private keys or
  deployment credentials are provisioned by this document.
- **The L3 release decision is complete; producer correctness remains external.**
  The owner recorded DDF v0.8.0 release acceptance. Git fact comparison checks the supported
  identity/kind model, not all semantic signals or scores. The historical DDF 0.7.0
  version string did not uniquely identify the parser-fix revision. Digest enrollment
  identifies an executable; the owner must qualify its provenance and behavior.
  Warden does not fix or certify DDF's parser, upstream release, or all code findings.
- Patch seals do not prove applicability to a specific base tree. CLI base/head claims
  are caller assertions; an external controller must bind the actual diff and subject.
  A signed HOLD receipt is scoped consent, not a signature on every verdict or a way
  to turn missing evidence into PASS.
- Source checks cover the supported gate platform, default features/all-targets and
  explicit admitted inputs. They are not proof of arbitrary cfg/procedural-macro
  expansion or safe execution of candidate builds. Unsupported first-party build
  scripts/generated inputs refuse; trusted build tooling is still required.
- DDF MCP `rank-summary.v0` is not full `rank.v0` and is refused. The historical
  large-response summary limit is an upstream transport limitation, not something
  this stack repairs. Use full ranking bytes for advisory inspection, or the authority
  subprocess path; never treat a summary as verified complete coverage.
- Review ownership, unsigned byte hashes and local hub branch names are not deployment
  authentication. L1 golden approval and the integrator's exact merged-tree gate
  are recorded for the baseline above; future changes need their own acceptance. No claim of zero future defects follows from zero missed mutants.
- Four gate checks are narrower than their names (deep review 2026-11-10, C13; accepted as low): the `unexpected_cfgs` check-cfg list in `Cargo.toml` is not held by `lint-floor.toml`, so an added cfg passes the floor check; `scripts/tests/perimeter.py` has no timeout entry for the `route` job of `mutation-baseline.yml`; each crate's `[lints] workspace = true` is checked as text, not parsed; `scripts/check_vocab.sh` accepts any revision Git resolves, a branch name included, not only a commit id.

## Mission 3 finding traceability

“Implemented” below names the in-tree control, not blanket deployment closure.
R-20 is the exit-registry follow-up recorded in WP-B4; the original consolidated
Mission 3 report enumerated R-01 through R-19. Evidence is linked to the tests in
this stack; historical receipts retain their own exact subject identities.

| Finding | Closing WP / status | Highest relevant rung | Evidence path and residual |
| --- | --- | --- | --- |
| R-01 ranking coverage/authenticity | C0, C1a–C1c implemented and stack accepted; deployment separate | 2 provenance/coverage types + 3 facts/producer admission; 4 mint lint | [authority_mode.rs](../crates/forge-warden-cli/tests/authority_mode.rs), [coverage.rs](../crates/forge-warden-core/tests/coverage.rs), ADR 0005; L3 semantic limits remain. |
| R-02 PR controls its judge | D1 + D8 implemented; deployment open | 1 trusted-base separation + 3 identity admission | [dogfood.sh fixtures](../scripts/tests/dogfood.sh), ADR 0006; L4 external identity remains. |
| R-03 bare-label acknowledgement | D2 implemented; deployment open | 3 signature and exact-subject admission | [hold_ack.py fixtures](../scripts/tests/hold_ack.py), ADR 0007; key custody/revocation/current-head admission remain L4. |
| R-04 lexical hook protection | D4 honest limited hook, D6 enforcing configuration checks | 4 build checks; hook is convenience | [test_guard.py](../.claude/hooks/test_guard.py), [guardrails.py](../scripts/tests/guardrails.py); no write-containment claim. |
| R-05 PASS without patch | C1c implemented | 3 required CLI patch; 2 advisory outcome excludes PASS | `omitting_the_patch_requires_review_or_refusal` in [adversarial.rs](../crates/forge-warden-cli/tests/adversarial.rs). |
| R-06 noncanonical paths | B2 implemented | 2 RepoPath after 3 admission | [path.rs](../crates/forge-warden-core/src/path.rs), `dot_prefixed_paths_are_refused_at_the_cli_boundary` in [adversarial.rs](../crates/forge-warden-cli/tests/adversarial.rs). |
| R-07 rule-kind drift | B1 implemented | 2 Kind enum/exhaustive dispatch | [kind.rs](../crates/forge-warden-core/src/policy/kind.rs), [policy tests](../crates/forge-warden-core/src/policy/tests.rs). |
| R-08 unpinned/red toolchain | A1 implemented | 4 pinned build/lint gate | [rust-toolchain.toml](../rust-toolchain.toml), [ci_fast.sh](../scripts/ci_fast.sh); pin does not authenticate tool distribution. |
| R-09 vocabulary revision not attested | B3 + B3.1 implemented | 3 revision resolution/byte comparison | `the_script_attests_exactly_what_the_commit_labels` in [check_vocab.rs](../crates/forge-warden-cli/tests/check_vocab.rs). The script resolves its argument with `rev-parse`, so a branch name is accepted and attests whatever that branch points to; CI passes the full pinned commit (`DDF_REV`). |
| R-10 swallowed local failures | A2 + D8 implemented | 3 exact exit propagation, 5 regression evidence | [ci_local.sh fixtures](../scripts/tests/ci_local.sh), including pre-push, judge/ranker and early-failure controls. |
| R-11 malformed hook input passes | D4 implemented | 3 request parsing | [test_guard.py](../.claude/hooks/test_guard.py); rejection is not shell containment. |
| R-12 semantic edits evade hook | D4 + B6 + D6 checks implemented | 4 compiler/lint/repository invariants | [repo_invariants.rs](../crates/forge-warden-cli/tests/repo_invariants.rs), [guardrails.py](../scripts/tests/guardrails.py); trusted enforcement required. |
| R-13 candidate reviewer contract/fallback | D3 implemented locally; provider deployment open | 1 base object bundle + 3 admission | [managed_review_bundle.py fixtures](../scripts/tests/managed_review_bundle.py), ADR 0008; provider isolation unmeasured. |
| R-14 unsigned seals/producer-version ambiguity | C1c provenance + D2 scoped acknowledgement | 3 digest/producer admission and scoped signature | [authority tests](../crates/forge-warden-core/tests/authority.rs), [hold_ack.py fixtures](../scripts/tests/hold_ack.py); verdict seals remain unsigned byte identities; L3 release decision complete, producer semantic limits remain; L4 open. |
| R-15 nonexistent test references | B3 corrected references | 6 documentation accuracy, supported by actual test paths | [vocab.rs](../crates/forge-warden-core/src/vocab.rs), [repo_invariants.rs](../crates/forge-warden-cli/tests/repo_invariants.rs); review must keep prose and tests aligned. |
| R-16 incomplete vocabulary scan | B3 implemented | 3 total supported-label parsing | `every_label_form_the_script_cannot_read_exactly_fails_closed` in [check_vocab.rs](../crates/forge-warden-cli/tests/check_vocab.rs). |
| R-17 missing many_hunks corpus coverage | B3 implemented | 5 corpus coverage test | [corpus.rs](../crates/forge-warden-cli/tests/corpus.rs), [many-hunks ranking](../fixtures/corpus/shape-many-hunks.rank.json). |
| R-18 MCP summary incompatibility | D5 documents refusal/non-claim; upstream limitation remains | 3 schema admission + 6 transport documentation | [rank.rs](../crates/forge-warden-core/src/rank.rs) (`refusals_name_the_problem` tests an unsupported schema); summary-specific refusal is inferred from strict schema admission, not a new MCP test. No MCP integration repair claimed. |
| R-19 mutable unvalidated library values | B2 implemented | 2 private validated types/nonempty policy | [core adversarial tests](../crates/forge-warden-core/tests/adversarial.rs), [policy model](../crates/forge-warden-core/src/policy/model.rs); library provenance attestation still trusts its caller. |
| R-20 literal/misspelled exit routes | B4 implemented | 2 Exit enum + 4 sole exit-door invariant | [exit.rs](../crates/forge-warden-core/src/exit.rs), `exit_statuses_come_from_the_registry` in [repo_invariants.rs](../crates/forge-warden-cli/tests/repo_invariants.rs), over-budget test in [cli.rs](../crates/forge-warden-cli/tests/cli.rs). |

Historical review context, in the project's coordination records (not in this repository):
the mission-3 review report, the mission-4 work board and Bastion's C1c v4.4 boundary verdict
(boundary PASS with exact subject receipt). These records are not runtime authority inputs.
