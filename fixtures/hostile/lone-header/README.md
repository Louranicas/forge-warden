# A lone `diff --git` header over a section for another path

A `diff --git` header with no `---`/`+++` lines of its own, followed by a traditional section
(`--- PATH`, `+++ PATH`) that names a different path. Git reads the traditional section and
reports its path; a reader that believed the header would judge the change on the header's path.
forge-warden believes Git's path only when the patch names it in exactly one
`diff --git a/PATH b/PATH` header, so each shape below is refused by forge-warden's own header
check, whatever the ranker makes of it. Shapes measured by Touchstone (M7-HEADER-OWN, Git 2.56.1);
`NAME.numstat` is what `git apply --numstat -z` reported, and `git apply --summary` was empty for
every case. Replayed by
`crates/forge-warden-core/tests/hostile_input.rs::a_lone_git_header_over_another_path_is_never_believed`.

| Case | Header names | Section names | Git reports | Believed |
| --- | --- | --- | --- | --- |
| `control-good` | `docs/ok.md` | `docs/ok.md` | `docs/ok.md` | yes |
| `lone-then-traditional` | `docs/ok.md` | `scripts/x.sh` | `scripts/x.sh` | no: refused |
| `lone-index-then-traditional` | `docs/ok.md`, with an `index` line | `scripts/x.sh` | `scripts/x.sh` | no: refused |
| `good-then-lone-then-traditional` | `docs/ok.md` twice: a real section, then `docs/n.md` | `docs/ok.md`, `scripts/x.sh` | both | no: refused on `docs/n.md` |
| `lone-then-creation` | `docs/ok.md` | `/dev/null` to `scripts/new.sh` | `scripts/new.sh` | no: refused |
| `lone-blank-unprefixed` | `docs/ok.md`, then a blank line | `scripts/x.sh` without prefixes | `x.sh` | no: refused |

Two more shapes Touchstone measured are left out because Git itself refuses them ("inconsistent
old filename"): a second `diff --git` header right after the lone one, and a binary section
followed by the lone header. Git's refusal holds them before this check is reached.
