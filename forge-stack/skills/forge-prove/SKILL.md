---
name: forge-prove
description: Run the gate and produce an evidence-backed summary before declaring any change finished. Use before saying done, ready, fixed or complete, before a commit, and before writing a pull request description.
---

# Prove it

"Done" is a claim. This is how it becomes a fact.

## 1. Run the project's own checks

Its tests, linter and build, whatever the repository uses. Record each command and its result.
A step you could not run is **not verified**, never passed.

## 2. Run the gate

```bash
forge gate              # exit 0 pass · 7 fail · 8 hold    (--json for the full document)
```

| Result | What it means | What you do |
| --- | --- | --- |
| PASS | Nothing the policy, the integrity checks or the scope object to | Go to step 3 |
| FAIL | The change must not ship as it is | Fix the **code** named in each item, then run the gate again |
| HOLD | A person must decide | Do not retry. Report each hold with its path, line and reason |

The gate judges everything since the session began, including new files. The policy comes from
the base commit, so editing `warden.toml` or `forge-stack.toml` changes nothing except adding a
protected-path hold.

What never fixes a FAIL: skipping or deleting the test, loosening its assertion, adding a
suppression, moving the code somewhere the check does not look. Each of those is itself a FAIL.

## 3. Write the summary

- **What changed and why**, in two or three sentences, matching the declared scope.
- **Verified**: each command you ran and its result (tests, `forge gate` with the patch id it
  prints).
- **Believed, not verified**: anything you reasoned about but did not run.
- **Holds**: every HOLD item, with what the person needs to look at.
- **The test that fails without this change**, by name. If there is none, say so.

The stop hook runs the same gate. A failing change cannot end the session quietly, so finding
out here is faster.
