# Operator runbook: an authority-mode deployment

This is the procedure for running forge-warden in **authority mode** on one Linux machine: the only
mode whose verdict can be `pass`. Each step says what to run and how to check it worked; do them in
order and stop at the first check that fails. Advisory mode (a supplied ranking, at best `hold`) needs
only steps 1, 2 and 5.

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
```

For v0.8.2 the archive digest is `f3898cc9065f856f151e9f2ecd72bfe8818c5e8bc0a3004bae654fad592f056b`
and the binary's is `298e07511310b18051a4c8ed56c18fafa03e40dc8fc88bedf74edba0e2ba41ad`. Install the
binary somewhere only you can write, and do not modify it afterwards.

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

```bash
forge-warden init > warden.toml               # a starter policy
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

**NOT YET RUN** on any machine: it needs the owner's Git allowlist (step 4), which is not installed yet.

```bash
git diff --binary --no-renames main... > change.patch
forge-warden check --policy warden.toml --patch change.patch \
  --ranker /opt/deep-diff-forge/deep-diff-forge --json > verdict.json
echo "exit $?"
```

Exit 0 is `pass`, 7 `fail` and 8 `hold` (a person decides); anything else is a refusal with its
next step on stderr. `forge-warden schema exit-codes` lists them all.

## 7. Check that the verdict is an authority verdict

**NOT YET RUN** on any machine: it needs the owner's Git allowlist (step 4), which is not installed yet.

```bash
python3 -c 'import json,sys; v=json.load(open("verdict.json")); s=v["seals"]; print(v["verdict"], s["provenance"], s["ranked_by"]["sha256"], s["git"]["admission"], s["git"]["allowlist_sha256"])'
sha256sum /etc/forge-warden/git-allowlist.toml
```

Check: provenance is `produced`; `ranked_by.sha256` is the digest from step 3; `git.admission` is
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
| exit 3, `no Git allowlist enrolled` | step 4 not done on this machine | install the allowlist, or run advisory mode |
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
