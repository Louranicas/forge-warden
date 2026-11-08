# Glossary

These terms describe the current contracts without renaming Rust types or wire fields.
`Outcome` remains the internal Rust verdict-level type; `RuleOutcome` is a separate per-rule
type, and `Verdict` is the complete document. Capitalized PASS/HOLD/FAIL in human output and
lowercase JSON values name the same levels. `policy_hold` names exit 8, not another level.

The table is also the reviewed exact-phrase lint list for the complete source Markdown inventory,
including SECURITY, docs, and the skills/agents under .claude. The filesystem inventory covers
tracked and untracked Markdown and works in Git-less exports. Exact historical/quoted-text
exemptions, with reasons, are reviewed in `docs/glossary-exemptions.toml`. No directory or glob
is exempt; current ADR text is checked unless an exact record is explicitly admitted there.
Matching folds ASCII case and collapses whitespace, then checks complete phrase boundaries.
It does not interpret intent, execute examples, or reject arbitrary occurrences of Rust type
names. A semicolon separates forbidden phrases; a dash means no forbidden phrase for that row.

| Concept | Canonical term | Forbidden phrases | Meaning |
| --- | --- | --- | --- |
| document | verdict | result document; outcome document | Complete sealed evaluation document, including its level, rules, notes and seals. |
| level | verdict level | outcome tier; verdict tier; result type | The overall pass, hold or fail value serialized in the verdict field. |
| rule | rule outcome | rule verdict | One rule's pass, hold, fail or waived result; it is not the complete verdict. |
| effect | effect door | lint door; lint doors; minting door | A reviewed function in doors.rs that performs an external effect; lint guards enforce that boundary. |
| review | hold | soft pass; conditional pass | A level requiring review evidence; it grants no approval. A hold rule's severity and exit policy_hold are distinct fields. |
| uncertainty | note | warning-only note | Structured uncertainty that makes the overall level at least hold; it cannot loosen fail. |
| digest | seal | signed seal | Recorded input digest or runtime metadata; a seal is not a digital signature. |
| origin | provenance | trust tier | Whether the ranking was produced or supplied; this is not a correctness certificate. |
| authoritative | authority mode | trusted mode | Mode that runs the admitted ranker on the patch and can reach pass when all contracts hold. |
| informative | advisory mode | approval advice | Mode consuming a supplied ranking, which can hold or fail and cannot pass. |
| producer data | ranking | risk verdict | The producer's sealed description of changed paths, signals, evidence and risk. |
