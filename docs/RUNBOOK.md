# Operator runbook: an authority-mode deployment

This is the procedure for running forge-warden in **authority mode** on one Linux machine: the only
mode whose verdict can be `pass`. Each step says what to run and how to check it worked; do them in
order and stop at the first check that fails. Advisory mode (a supplied ranking, at best `hold`) needs
steps 1, 2, 3 and 5 and the advisory commands at the end of step 6; it needs no Git allowlist.

Two steps are owner acts that need root: installing the Git allowlist (step 4) and, if you choose,
pinning a different deep-diff-forge (step 3). Everything else runs as an ordinary user.

## 1. Check the machine

forge-warden runs on Linux only (it reads `/proc`). Authority mode also needs Git and a SHA-256 tool.

```bash
uname -s                      # Linux
test -d /proc/self/fd && echo ok
command -v git sha256sum
git --version
```

## 2. Install forge-warden at a pinned revision

**NOT YET RUN** as written: no published revision carries 0.2.0 yet, and the verifier ran a local build of
the branch instead.

Pin what you deploy: a release tag, or a commit you have reviewed.

```bash
cargo install --git https://github.com/Louranicas/forge-warden --rev <commit> forge-warden-cli --locked
forge-warden schema exit-codes   # the exit codes your automation will see
```

## 3. Get the ranker and record its digest

Authority mode runs deep-diff-forge itself and admits it only by the SHA-256 of its bytes. Use the
released binary: a binary built from the same source on another machine has other bytes (its build
paths are embedded) and will not match.

```bash
v=v0.8.2; a=deep-diff-forge-$v-x86_64-unknown-linux-gnu
curl --fail --location --proto '=https' -O "https://github.com/Louranicas/deep-diff-forge/releases/download/$v/$a.tar.gz"
curl --fail --location --proto '=https' -O "https://github.com/Louranicas/deep-diff-forge/releases/download/$v/$a.tar.gz.sha256"
sha256sum --check "$a.tar.gz.sha256"          # must print OK
tar --extract --gzip --file "$a.tar.gz" "$a/deep-diff-forge"
sha256sum "$a/deep-diff-forge"                # the digest the policy will list
install -D -m 0755 "$a/deep-diff-forge" "$HOME/.local/share/forge-warden/deep-diff-forge"
```

For v0.8.2 the archive digest is `f3898cc9065f856f151e9f2ecd72bfe8818c5e8bc0a3004bae654fad592f056b`
and the binary's is `298e07511310b18051a4c8ed56c18fafa03e40dc8fc88bedf74edba0e2ba41ad`. The last line
installs it where step 6 runs it: a regular file only you can write (the door refuses a symlink, a FIFO
or a file writable by group or others). Do not modify it afterwards.

## 4. Install the Git allowlist (owner, root)

**NOT YET RUN** on any machine: it is the owner's step, and no allowlist is installed yet.

Authority mode admits Git only if `/etc/forge-warden/git-allowlist.toml` lists the SHA-256 of the
Git it runs (`--git`, default `/usr/bin/git`). Without the file, authority mode refuses (exit 3).
See [the Git runtime](GIT_RUNTIME.md) and ADR 0011.

```bash
sha256sum /usr/bin/git                        # put this digest in the file below
sudo install -d -o root -g root -m 0755 /etc/forge-warden
printf 'schema = "forge-warden.git-allowlist.v1"\nrevision = 1\nsha256 = ["%s"]\n' \
  "$(sha256sum /usr/bin/git | cut -d' ' -f1)" > git-allowlist.toml
sudo install -o root -g root -m 0644 git-allowlist.toml /etc/forge-warden/git-allowlist.toml
stat -c '%u:%g %a %n' / /etc /etc/forge-warden /etc/forge-warden/git-allowlist.toml
```

Check: every line of the `stat` output shows `0:0` and no group or other write bit. The file must be a
regular file (not a symlink), and so must Git; every directory above each of them must be owned by
root and not writable by group or others. Inside a rootless container, uid 0 is a subordinate uid of
your own account, so an allowlist there is only as strong as that account: prefer the host.

## 5. Write and check the policy

If the repository already has a `warden.toml` (you are upgrading), keep it and skip `init`: the
redirection below would overwrite it.

```bash
forge-warden init > warden.toml               # a starter policy; only when there is none yet
forge-warden lint warden.toml                 # lists every problem at once; exit 0 when valid
```

For authority mode, add the ranker's digest from step 3 to the policy, then lint it again:

```toml
[producer]
sha256 = ["298e07511310b18051a4c8ed56c18fafa03e40dc8fc88bedf74edba0e2ba41ad"]
```

The policy lives in the repository it judges and should be owned there (CODEOWNERS and a
`protect_paths` rule on the policy itself), so a change to it is reviewed like code.

## 6. Judge a change

**Run once in authority mode, not yet as written by an owner.** On 2026-11-11 a verifier ran it on the
0.2.0 release candidate inside a toolbox container, with an allowlist an agent installed, on the owner's
instruction, through the container's root (which is the user's own uid on the host): a docs-only change gave `pass` (exit 0), a
change to a protected path `hold` (exit 8), a wrong `[producer]` digest exit 4, and an unowned or
unenrolled Git exit 3. That proves the commands and the refusals, not an owner-held allowlist. The
advisory commands below were run as written.

```bash
git diff --binary --no-renames --no-relative --src-prefix=a/ --dst-prefix=b/ main... > change.patch
forge-warden check --policy warden.toml --patch change.patch \
  --ranker "$HOME/.local/share/forge-warden/deep-diff-forge" --json > verdict.json
echo "exit $?"
```

Run it from the top level of the repository. The prefix options fix the `a/` and `b/` every path
in the patch starts with: your own Git configuration can change or drop them (`diff.noprefix`),
and a patch without them is refused as unverifiable (exit 8, `patch_unverifiable`), because Git
would read `scripts/x` as `x`. `--no-relative` matters as much and nothing can check it for you:
with `diff.relative` set and the command run in a subdirectory, Git writes a well-formed patch
that leaves out every change outside that directory, and forge-warden judges what it is given.

What a verdict is about: these patch bytes, with paths read from the repository root.
forge-warden does not check that the patch is the whole change between two commits; `--base`
and `--head` are recorded as your claims, unverified. Whoever makes the patch is part of the
trust boundary, so make it with this command or with `scripts/dogfood.sh`, not by hand.

Exit 0 is `pass`, 7 `fail` and 8 `hold` (a person decides); anything else is a refusal with its
next step on stderr, and `verdict.json` is then empty. `forge-warden schema exit-codes` lists them all.

Advisory mode instead (no allowlist needed; the best result is `hold`, exit 8): rank the patch
yourself and hand the ranking over.

```bash
"$HOME/.local/share/forge-warden/deep-diff-forge" --stdin-patch --rank --json < change.patch > rank.json
forge-warden check --policy warden.toml --patch change.patch --advisory --rank rank.json --json > verdict.json
```

## 7. Check that the verdict is an authority verdict

**Not yet run as written.** The verdict of the authority run described in step 6 records what this step
checks (provenance `produced`, the pinned ranker digest, Git admission `approved` and the allowlist's
digest), but the commands below were not run on it, and not on a host with the owner's allowlist (step 4).

```bash
python3 -c 'import json,sys; v=json.load(open("verdict.json")); s=v["seals"]; print(v["verdict"], s["provenance"], s["ranked_by"]["sha256"], s["git"]["admission"], s["git"]["allowlist_sha256"])'
sha256sum /etc/forge-warden/git-allowlist.toml
```

Run it only after exit 0, 7 or 8: on a refusal `verdict.json` is empty. Check: provenance is `produced`; `ranked_by.sha256` is the digest from step 3; `git.admission` is
`approved`; and `git.allowlist_sha256` equals the file's SHA-256. A verdict without all four is not an
authority verdict, whatever it says.

## 8. Changes over time

- **Git is updated** (a distribution update changes `/usr/bin/git`): authority mode refuses until the
  allowlist lists the new digest. Add it with `revision` increased by one, keep the old digest until
  every machine is updated, then remove it in a later revision.
- **A new deep-diff-forge**: repeat step 3 and change `[producer] sha256` in a reviewed commit. Keep
  the old digest listed only while you still run the old binary.
- **A new forge-warden**: repeat step 2 at the new revision; read the CHANGELOG's notes for that
  release before switching automation over.

## 9. When it refuses

| You see | Meaning | Do |
|---|---|---|
| exit 3, `no Git allowlist enrolled` | step 4 not done on this machine. Git is admitted before the ranker is checked, so until step 4 every authority run stops here, even with a wrong ranker | install the allowlist, or run advisory mode |
| exit 3, `executable digest is not enrolled` | Git changed (step 8) | add the new digest, new revision |
| exit 4, the ranker's digest `does not list` | the binary is not the one the policy pins | use the pinned binary, or change the policy in review |
| exit 4, `pinning …` | the ranker file is a symlink, a FIFO, or writable by others | install it as a plain file only you can write |
| exit 8 | a rule or note holds: a person must decide | read `verdict.json`; a signed HOLD acknowledgement is the reviewed way past it |

**Roll back** by removing `/etc/forge-warden/git-allowlist.toml` (authority mode then refuses; advisory
mode still runs and records `unpinned`), or by reinstalling the previous forge-warden revision.

## Not covered here

Hosted CI runs dogfood in advisory mode by decision, because a hosted runner has no owner-enrolled Git
allowlist. Network containment of a self-hosted runner, signing keys for HOLD acknowledgements
(`.github/hold-reviewers/README.md`) and releases are separate owner procedures.
