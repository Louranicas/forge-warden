## What changes, and why

<!-- One paragraph. Link the issue. -->

## Evidence

<!-- Claims need construction, not assertion. For each claim, the command that proves it. -->

- [ ] `just ci` passes locally (fmt, clippy, tests, docs; deny, zizmor and mutants if installed)
- [ ] New behaviour has a test that fails without the change (say which)
- [ ] Schema or exit-code changes: `CHANGELOG.md` updated, `scripts/bless.sh` run and the golden diff reviewed
- [ ] No new `#[expect]`, or each one's `reason` is in this description
- [ ] The dogfood verdict is pass, or every held rule is explained here

## What a reviewer should look at first

<!-- The riskiest hunk. `deep-diff-forge --stdin-patch --outline` ranks them for you. -->
