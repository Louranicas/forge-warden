# Patch prefixes: what Git writes, what Git reports, what is believed

One change (a line added to `scripts/gate.sh`), written by Git 2.56.0 under each prefix setting.
`NAME.patch` is the patch; `NAME.numstat` is what `git apply --numstat -z` reported for it. The
summary output was empty for every case. Replayed by
`crates/forge-warden-core/tests/hostile_input.rs::the_patch_prefix_corpus_is_believed_only_under_the_default_prefixes`.

| Case | Made with | Header names | Git reports | Believed |
| --- | --- | --- | --- | --- |
| `default` | `git diff` | `a/scripts/gate.sh b/scripts/gate.sh` | `scripts/gate.sh` | yes |
| `reverse` | `git diff -R` | `b/scripts/gate.sh a/scripts/gate.sh` | `scripts/gate.sh` | yes |
| `noprefix` | `diff.noprefix=true` | `scripts/gate.sh scripts/gate.sh` | `gate.sh` | no: refused |
| `prefix-old-new` | `diff.srcPrefix=old/`, `diff.dstPrefix=new/` | `old/scripts/gate.sh new/scripts/gate.sh` | `scripts/gate.sh` | no: refused |
| `mnemonic-worktree` | `diff.mnemonicPrefix=true`, index against work tree | `i/scripts/gate.sh w/scripts/gate.sh` | `scripts/gate.sh` | no: refused |
| `prefix-a-x` | `diff.srcPrefix=a/x/`, `diff.dstPrefix=b/x/` | `a/x/scripts/gate.sh b/x/scripts/gate.sh` | `x/scripts/gate.sh` | yes, as `x/scripts/gate.sh` |

`git apply` removes one leading path component whatever it is. With `diff.noprefix` that
component is part of the real path, so Git, and deep-diff-forge 0.8.2 with it, report `gate.sh`
for a change to `scripts/gate.sh`; before this corpus existed, authority mode passed such a patch
for a protected file because no path rule matched `gate.sh`.

The last row is a limit, not a fix: prefixes that themselves begin `a/` and `b/` give a patch
that is well formed for another path, and nothing in the patch says otherwise. The path believed
is the one the patch names. The commands this repository documents, and `scripts/dogfood.sh`,
pass `--src-prefix=a/ --dst-prefix=b/` so that no configuration can change the prefixes.

The two refused cases whose reported path is right (`prefix-old-new`, `mnemonic-worktree`) are
refused all the same: a reader cannot know which component a prefix was.
