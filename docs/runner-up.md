# Self-hosted CI on runner-up (WP-RUNNER-UP)

One of this repository's CI jobs, the full-workspace mutation baseline, can run on a self-hosted pool. The pool is
[runner-up](https://github.com/kunchenguid/runner-up) v0.1.12, label `runner-up-kinoite`: three
fresh KVM guests (4 cores, 8 GB) on the owner's Fedora desktop. Everything else runs on GitHub's
hosted `ubuntu-latest`.

## What runs where

| Job | Runner | Why |
| --- | --- | --- |
| `route` (mutation-baseline.yml) | always hosted | picks the runner for `full-workspace`, with exact bash comparisons |
| every ci.yml job (`gate`, `msrv`, ...) | always hosted | fw-verify's hosted_parity models only the reviewed `ubuntu-latest` gate shape, and the gate is a few minutes hosted |
| `full-workspace` (mutation-baseline.yml) | runner-up for a schedule or dispatch on protected `main`; hosted otherwise | the longest job (a reviewed 120-minute bound, docs/CI_BOUNDS.md) |
| `mutation`, `dogfood` | always hosted | they run only for pull requests and merge_group, which is unmerged code |
| `supply-chain`, `workflow-audit`, `vocabulary` | always hosted | under a minute each; they keep a hosted witness that does not depend on the desktop |

The pool is chosen in one place: the hosted `route` job of mutation-baseline.yml. It runs before any
pool place is allocated, and compares the event and ref with exact, case-sensitive bash string tests.
GitHub expression `==` ignores case, so a protected branch named `MAIN` would match `refs/heads/main`
in an expression, which is why the decision is not made in an expression. Routed jobs take the
route job's output. If that output is empty, or the route job failed, they still run, hosted.
`scripts/tests/runner_routing.py` runs the route script itself, with bash, over a truth table, and
requires every routed job to provision the guest right after checkout.
- Every `pull_request` (same-repository, fork or Dependabot) and every `merge_group` stays hosted.
- So do `pull_request_target`, `workflow_run`, comment events and any unknown event, and any ref that is not protected `main`.
- The ref must be exactly `refs/heads/main` (case-sensitive) and protected: `MAIN`, `Main` and
  `main-next` are other branches and stay hosted, even if a ruleset protects them.
- Check names do not change, so the ruleset's required checks are unaffected.

## Residual risk (read this before adding a job)

**runner-up jobs can reach host loopback services; only merged main code runs there**, with one
exception: anyone with write access can push a branch whose workflow names the pool directly (see
"The routing is not the trust boundary" below).
- A KVM place uses QEMU user networking. The guest reaches the host at `10.0.2.2`, which means every service the desktop listens on at 127.0.0.1, including its loopback SSH listener, and the LAN.
- The desktop's Samba (SMB, TCP 139 and 445) listens on all interfaces (0.0.0.0), so a guest can reach it as well.
- A fresh VM per job does not stop that.
- Real containment needs a guest bridge with nftables rules, which is a runner-up provider change. That is deferred to a separate, owner-approved host WP (specified in Bastion's GUEST-ISOLATION-SPEC).
- Until it is proven, pull-request code never routes here: a new dependency's `build.rs` in an unmerged PR would otherwise run beside those services.

**The routing is not the trust boundary.**
- A workflow runs from the ref it triggers on, so anyone who can push a branch can also edit `runs-on` on that branch and name the pool directly, bypassing the route job.
- The boundary that actually holds is repository write access plus runner-up's fork guard. The guard refuses jobs from another repository's pull request, a fork's follow-on `workflow_run`, and any job whose event it cannot read.
- Grant write access accordingly.

## Guest bootstrap and supply chain

The pool's image is a stock Ubuntu 24.04 cloud image. Before anything else, a routed job runs
`scripts/guest_bootstrap.sh`, only when `runner.environment == 'self-hosted'`. It does three things:
- it installs `build-essential` from the image's Ubuntu archive, if there is no `cc`;
- it installs `rustup-init` at the version pinned in `scripts/tool-versions.toml`, refused unless its sha256 matches the pin, with no default toolchain. Each job then installs its own pinned toolchain, as on hosted runners;
- it logs the image release and the tool versions.

The trade-off: the Ubuntu archive packages are not pinned, unlike a baked image with recorded build inputs. A baked image is the better follow-up, but it is a change to the runner-up deployment.

## Other trade-offs

- **Availability:** a single desktop.
  - If it is off, routed jobs wait for `startTimeoutSeconds` (600 s) before failing.
  - Pull requests and merge_group never wait, because they are hosted.
- **Cost and speed:** runner-up places have no GitHub-minute cost. Cache restores go over the network, as on hosted runners.
- **Fork pull requests:** routed to hosted runners, so a fork's checks still run rather than being refused by the guard.
