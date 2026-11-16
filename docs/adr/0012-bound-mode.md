# ADR 0012: Bound mode — forge-warden makes the diff it judges, and refuses a patch that differs

Status: Proposed
Owner-deployment: Required

Historical status record: drafted by Keel (O2) 2026-11-16 (M7-PATCH-BOUND, 0.3.0); security review by Bastion (O4):
CHANGES (M1 forged objects, M2 control characters in the path, M3 multi-pack index; S1-S5), then accepted for code with
one correction to D3.3 (measured, receipts 64ec6308 and 8a6ce212). Closes, when built: ledger Q55; removes by
construction the patch-maker half of Q54 and Q59. Amends ADR 0010 when accepted (section "Amendment to ADR 0010").

## Context

forge-warden judges the bytes of `--patch`. `--base` and `--head` are recorded as claims with `"verified": false`
and are never used. So a verdict says nothing about which change the patch is: a patch made in a subdirectory with
`diff.relative` is well formed, drops every change outside that directory, and gets an authority-mode PASS for a
protected change (Q55, measured on 6682ed53). No check of the patch bytes alone can see this. 0.2.0 shipped a
mitigation (the trusted `scripts/make_patch.sh` and fixed documented commands) and a stated non-claim.

The closed fix: when the caller names the repository and two full commit ids, forge-warden makes the patch itself,
the way `make_patch.sh` does, and refuses the run unless `--patch` is byte-equal to it. Then `claims.verified` can
be true, and it means: these patch bytes are exactly `git diff base...head` with the fixed options, computed by the
admitted Git (ADR 0011) from the objects in that repository.

## Decision

### D1. The interface
- New optional `--repo DIR` on `check`. With `--repo`, both `--base` and `--head` are required and must be FULL object
  ids: 40 lowercase hex (SHA-1) or 64 (SHA-256), both the same length. Abbreviations and names are refused (exit 2,
  usage), because resolving a name reads refs, which this mode never reads.
- `--patch` stays required. Bound mode compares; it does not replace the supplied patch. The bytes a reviewer reads,
  the bytes the ranker reads and the bytes the verdict seals stay one and the same file.
- Bound mode is allowed in authority mode only. `--advisory` with `--repo` is a usage error (advisory can at best
  hold; binding it would add an effect for nothing).
- Without `--repo`, nothing changes: claims are recorded, `verified` is false, exactly as in 0.2.x.

### D2. What the door reads, and what it never reads
The door reads the caller's object directory and nothing else of the caller's repository.
1. Admit `DIR` like the temporary base (ADR 0010): absolute after `canonicalize`, UTF-8, no `:`, **and no control
   character (U+0000-U+001F, U+007F)**: the alternates file is line-based, so a newline in the path would make a second
   alternate the caller chose (review M2; refusal row `DIR = /…/a\n/…/elsewhere/.git`). Then:
   - `DIR/.git` is a directory: objects = `DIR/.git/objects`; or `DIR` is a bare repository (`DIR/objects` and
     `DIR/HEAD` exist, `DIR/.git` absent): objects = `DIR/objects`.
   - `DIR/.git` is a file (a linked worktree or a submodule's `gitdir:` pointer): **refused** (exit 3, environment)
     with "pass the repository's git directory: --repo \"$(git rev-parse --git-common-dir)\"". Following the pointer
     means parsing a file the checkout controls; not in this version. RUNBOOK and README show that exact form (S5).
2. Refused before Git runs (exit 3, environment, each with its own message and refusal row):
   - `objects/info/alternates` or `objects/info/http-alternates` present: Git would follow them to object stores
     outside `DIR`.
   - `DIR/.git/shallow` (or `DIR/shallow`) present: a shallow history can give a wrong merge base silently.
   - the objects path, or any of its parents up to `DIR`, is a symlink (read with no-follow, as the Git door does).
3. Never read, never passed to Git: the caller's config, refs, `HEAD`, index, hooks, `info/` (grafts, attributes,
   excludes), work tree, `.gitmodules`, `.gitattributes`, `.mailmap`. The caller's index files (commit-graph under
   `objects/info/`, multi-pack-index and `.idx`/bitmaps under `objects/pack/`) are read only while enumerating in the
   staging repository and are never trusted: the merge base and the diff are computed in the verified repository,
   which holds only objects Git re-hashed on receipt (D3, review M1/M3). This is what makes M7-WORKTREE-ATTACK's list moot for bound mode:
   nothing from a work tree is consulted.

### D3. How the diff is made: copy the objects through Git's receive-side checks, then diff
Git hashes an object when it RECEIVES it (index-pack, unpack-objects), not when it reads it. A loose blob whose bytes are
not its name passes `git diff` and `cat-file` (Bastion, receipt 64ec6308, Git 2.56.1), and an `.idx` or multi-pack index
can map an id to another object's bytes. A step that ran candidate code in the CI checkout before the gate can write the
store, and `make_patch.sh` reading the same store would agree with the forged bytes. So the diff never reads the
caller's store. Two repositories inside the private 0700 directory (ADR 0010 `Isolation`, same environment, same ceiling):

1. **Staging** `<private>/staging.git`: `git init --quiet --bare --template= --object-format=<sha1|sha256 from the id
   length>` (`--template=`: no hook samples; measured: config, HEAD, objects, refs only). Its `objects/info/alternates`
   is the admitted objects path (one line; control characters were refused in D2.1). Every staging command runs with
   `-c core.commitGraph=false -c core.multiPackIndex=false`. Staging only enumerates and packs; nothing it says is trusted.
2. In staging: `cat-file -t` of base and head must print `commit` (else exit 4, "base is not a commit in the
   repository"); `merge-base --all base head` must print exactly one line, the candidate merge base `mb` (else exit 4,
   "base and head have N merge bases; name the merge base as --base"). `git diff A...B` picks one of several with only a
   stderr warning and exit 0 (measured, Git 2.56.1: "warning: a...b: multiple merge bases, using 14c81e2…").
3. **The copy:** `git pack-objects --stdout` in staging, fed the commits of `^mb^@ base head` and the complete trees
   (every blob) of `mb` and `head`, piped into plain `git index-pack --stdin` in **verified** `<private>/verified.git`
   (same `init` flags, **no alternate**). index-pack names every object by the hash of its content, so a forged object
   lands under another name and the id the diff asks for is absent. forge-warden then writes **its own**
   `verified.git/shallow`: `mb` and every copied commit that has a parent outside the copy (from `rev-list --parents`
   of the copy list), so history in verified stops at the copy's edge. `mb` alone is not enough: head can reach `mb`'s
   ancestors along another path (a merge), and verified's merge-base then walks into an uncopied commit and fails on an
   honest store (measured on forge-warden's own 7bd9e655..26330b93, the M7-PATCH-BOUND Q-C record, 2026-11-16). This is not a caller's shallow
   repository (D2.2 still refuses those); it is the cut forge-warden makes after the copy was re-hashed. Every edge left
   in verified is a re-hashed, real edge, and a cut only removes ancestry, so verified's merge base is a real common
   ancestor, `mb` or older; step 4 still requires it to equal staging's, and an older one could only widen the patch. `--strict` is NOT used: its
   connectivity check wants all of `mb`'s ancestry and refuses every honest store with this list (Bastion, receipt
   8a6ce212, Git 2.56.1), and its fsck checks can refuse honest repositories with historic object quirks.
   Any object missing in staging enumeration, in verified's merge-base or in the diff: exit 3 (environment: "the
   repository's object store does not hold what its ids name"), no verdict.
4. In **verified**: `merge-base --all base head` again; it must equal `mb` from step 2, else exit 3. A commit forged to
   move the merge base fails the hash in step 3 or disagrees here.
5. The diff, in **verified** only, with exactly the script's options plus hardening that does not change output for an
   honest store: `git --git-dir=<verified.git> --attr-source=<E> -c core.attributesFile=/dev/null -c core.quotePath=true
   -c core.hooksPath=/dev/null -c core.fsmonitor=false diff --no-ext-diff --no-textconv --no-color --no-relative
   --binary --no-renames --full-index --src-prefix=a/ --dst-prefix=b/ --submodule=short --ignore-submodules=none <mb>
   <head>`. `--full-index`: without it Git abbreviates the `index` line ids to a length that depends on the repository's
   object count, and verified holds only the copy (measured: 7 against 10 characters on a 100k-commit repository;
   the M7-PATCH-BOUND Q-C record, 2026-11-16). Where
   `<E>` is the empty tree computed in that repository (`git hash-object -t tree /dev/null`: `4b825dc…` for SHA-1,
   `6ef19b4…` for SHA-256; never a constant; review S3). No promisor remote exists in either repository, so a missing
   object is an error, never a fetch.
6. Bounds for every Git child of bound mode (staging enumeration, pack-objects, index-pack, merge-base, diff): stdout
   cap `MAX_INPUT_BYTES` for the diff; a time limit `GIT_BOUND_TIMEOUT` = 60 s for the whole bound run (measured, warm
   cache: worst 1.8 s, a 100k-commit repository with a 99,896-object copy; the M7-PATCH-BOUND Q-C record, 2026-11-16); **and a memory ceiling**
   RLIMIT_DATA = 1 GiB per Git child (worst measured peak RSS 174 MB): RLIMIT_DATA
   (RLIMIT_AS also counts Git's pack mmap windows, `core.packedGitWindowSize`/`packedGitLimit`, and would turn into false
   refusals on large packs; Bastion's re-read), value measured on the 100k-commit fixture; a loose object's header states
   its size and index-pack inflates every object (review S2). Overflow, timeout or the ceiling: exit 3 (environment), no verdict.
7. Cleanup: bounded, no-follow recursive removal of `<private>/staging.git` and `<private>/verified.git` only
   (openat/unlinkat on descriptors), then the existing empty-directory removal.

### D4. The comparison
- Equal bytes: the run continues exactly as today on those bytes (`git apply --numstat/--summary`, ranker, evaluate),
  and the verdict records `claims = {base, head, merge_base, verified: true}`.
- Different bytes: **refused, exit 4 (invalid_input)**, no verdict, message "the patch is not `git diff base...head`
  of the repository: first difference at byte N (patch L bytes, diff M bytes); make it with
  `scripts/make_patch.sh BASE HEAD`; forge-warden's Git is VERSION (the admitted one); a patch made by another Git
  version can differ in diff output (review S4)". No patch content is echoed. A plain refusal needs no new note code and no
  HOLD a reviewer could acknowledge into a PASS: a mismatched patch is a wrong input, not a risky change.

### D5. The verdict schema: `forge-warden.verdict.v3`
`claims.verified` is the constant `false` in v2 (`"const": false`), so a strict v2 reader rejects `true`. Per ADR
0011's rule (the smallest honest wire change, never new meaning under an old label), bound mode needs a new schema id.
- v3 = v2 with: `claims.verified` a boolean; when `true`, `base` and `head` are required and full-length (40 or 64
  hex), and a new required `claims.merge_base` (same length). When `false`, v3 claims are exactly v2 claims.
- Emitted unconditionally (the ADR 0011 amendment lesson: dual emission was never built). 0.3.0 is a minor release
  before 1.0; UPGRADING and CHANGELOG say v2 -> v3 and that only `claims` changed.
- `Unverified` (a type that can only be false) leaves the public API; `Claims` gains a verified form constructible only
  by the CLI's bound path (a private-field type, like `AdmittedGit`), so the pure core cannot be told "verified" by a
  caller-built value. The public-API snapshot changes; semver says 0.3.0.
- Consumers changed in the same package: `scripts/hold_ack.py` (schema id check), `scripts/dogfood.sh` (switches to
  bound mode: `--repo "$trusted_or_candidate" --base B --head H`), `forge-warden schema`, goldens.
- Golden fixtures change (`demo.verdict.json`'s schema id). Regenerating them is a blessing; the owner decides who
  blesses (L1 precedent). Not done by the builder silently.

### D6. Threat model wording
The non-claim "base and head are caller assertions; a verdict is about the patch bytes" stays for runs without
`--repo`. For bound runs it becomes: "the patch is `git diff merge-base..head` of the named commits, made by the
admitted Git from objects it re-hashed when copying them out of the named repository: a store that holds bytes other
than its ids name is refused, not believed. forge-warden does not attest that those commits are the ones your pull
request names (that is your CI's checkout and the ids it passes)." ADR 0010/0011's boundary (the runtime, the admitted
Git and forge-warden's own binary are trusted) is unchanged.

### D7. `make_patch.sh` follows
- Refuses several merge bases (D3 step 2) with the same message.
- Writes `--full-index`, like the door (D3.5), and so do the documented by-hand commands. A patch made by a 0.2.x
  `make_patch.sh` then differs from the bound diff in its `index` lines: UPGRADING says to make it again.
- Stays the one documented way to make the patch; RUNBOOK step 6 and README show `forge-warden check ... --repo
  "$(git rev-parse --git-common-dir)" --base B --head H` with the patch from the script.

### D8. A gate can require bound mode (review S1)
Without `--repo`, authority mode still passes a sub-tree patch (Q55 itself), so a CI line that drops `--repo` silently
goes back to unbound. A policy can forbid that:
- New optional policy key `[claims] require_verified = true` (policy schema change, 0.3.0). With it, an authority run
  without `--repo` gets the v3 note `claims_unverified` and can at best HOLD.
- `scripts/hold_ack.py` refuses to acknowledge a verdict whose policy requires bound mode and whose claims are not
  verified (hold_ack already refuses verdicts with notes; the note makes this explicit).
- `scripts/dogfood.sh` runs bound and asserts `claims.verified == true` in the verdict it judged; forge-warden's own
  `warden.toml` sets `require_verified = true`.

## Amendment to ADR 0010 (to be appended to 0010 when this is accepted)
ADR 0010 says: never discover a repository; no candidate `.git` config, index or work tree is consulted; object
alternates are never inherited. Bound mode (ADR 0012) adds exactly one effect: the CLI Git door may READ the object
directory of a repository the caller names with `--repo`, through an alternates file in a STAGING repository
forge-warden creates in its private directory, only to enumerate and pack objects. Every object the judgement uses is
copied into a second private repository with no alternate through `index-pack`, which names each object by the hash
of its content; forge-warden writes that repository's own `shallow` file at the merge base (never a caller's shallow
file, which stays refused); the merge base and the diff are computed there. The door still never discovers a repository, never inherits an alternate from
the environment, never reads that repository's config, refs, index, hooks, `info/` or work tree, refuses alternates
chains, shallow repositories, `gitdir:` pointers and control characters in the path, and turns off commit-graph and
multi-pack-index reading in staging. The private directory may hold the two repositories Git wrote; its cleanup is a
bounded no-follow removal of exactly those two. Every Git child of bound mode has a time limit and a memory ceiling.

## Rejected
- **Judge the diff forge-warden made, with no `--patch`.** Then the ranker reads bytes no reviewer saw, and a reviewer
  who reads `change.patch` reads something else. Byte equality keeps one artefact. (Could come later as `--patch -`.)
- **A mismatch as a HOLD with a new note code.** A hold can be acknowledged into a pass; a wrong input should not be.
- **Keep v2 and add a field.** `additionalProperties: false` everywhere: any new field or value breaks a strict v2 reader.
- **Run `git rev-parse`/`git diff` in the caller's repository** (as `make_patch.sh` does for rev-parse): it reads the
  caller's config; ADR 0010 forbids it in the product door. Full ids make rev-parse unnecessary.
- **Follow `.git` files and alternates.** More reach for a first version; refused with a clear message instead.

## Acceptance evidence before READY (Bastion verifies, Touchstone attacks)
1. Byte equality: on this repository, the three N3 patches' commits, Touchstone's relative-patch commits and 20
   generated commit pairs, the door's bytes equal `make_patch.sh`'s (SHA-1 and SHA-256 repositories), including one run
   where the user's Git is not the admitted Git (review S4; a difference is a refusal that names both versions).
2. **Forged stores (review M1), each refused (exit 3 or 4), no verdict, and each a row that the rev 1 design passes:**
   a forged loose blob in head's tree; a forged tree; a forged commit whose changed parent moves the merge base; a pack
   whose `.idx` maps an id to another object's offset; a lying multi-pack index; a lying commit-graph.
3. Refusals, each with a test that fails on the old code and a refusal row: abbreviated or mixed-length ids; advisory +
   `--repo`; `.git` file (message shows the `--git-common-dir` form); a newline or CR in `DIR` (M2); alternates /
   http-alternates; shallow; symlinked objects path; non-commit id; several merge bases; staging and verified merge bases
   differ; a patch differing by one byte (prefix, relative, submodule-omitted, `-U5`, trailing newline); diff over
   budget; time limit (a fixture Git that sleeps); memory ceiling (a loose object whose header states a huge size).
4. Hostile caller repository settings have no effect on the bytes: `diff.noprefix`, `diff.relative`, `diff.external`,
   textconv drivers, `core.quotePath=false`, `.gitattributes` `-diff`, `.gitmodules ignore = all`, `info/grafts`,
   replace refs.
5. No fetch: a partial clone with a missing blob in the diff refuses (exit 3) with no network attempt (gate image,
   `--network none`).
6. Cleanup: nothing left in the temporary base after success, mismatch, timeout, the memory ceiling and a killed Git;
   a symlink planted in either private repository is not followed.
7. Schema: v3 validates bound and unbound verdicts; v2 fixtures kept as history; `verified: true` cannot be produced
   without the comparison (no public constructor). D8: `require_verified` without `--repo` holds with
   `claims_unverified`; hold_ack refuses it; dogfood asserts `verified == true`.
8. Plants (each CAUGHT): comparison removed; the copy skipped (diff through the alternate: the forged-blob row);
   merge base taken from staging without the recheck; forge-warden's `shallow` file not written (the honest row is
   refused: the row is live); control-character check removed; `--template=` removed; merge-base
   `--all` check removed; memory ceiling removed; `require_verified` ignored.
9. In-diff mutants zero missed; ci_fast; MSRV; dogfood in authority bound mode on this repository.
