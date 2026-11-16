# A text file sent as a binary section

`text-as-binary-rs.patch` changes `build.rs`, adding an `unsafe` block. The change also commits a
`.gitattributes` with `*.rs binary`, and the patch was made by hand from the work tree at the head, so
Git wrote `build.rs` as a `GIT binary patch` section (Git 2.56.1; the `.gitattributes` section itself is
left out of this file to keep one path). `text-as-binary-rs.numstat` and `.summary` are what
`git apply --numstat -z` and `--summary` reported; `text-as-binary-rs.rank.json` is what the pinned
deep-diff-forge 0.8.2 ranks: `binary_changed` with only `binary_change`. Ranked as text, the same change
carries `unsafe_added` and `build_hook`. This is finding W1 (ledger Q62): the documented patch maker now
reads attributes from the empty tree, and rule `binary-text-is-hidden-text` (ledger Q63) holds the
section for any other patch maker. Replayed by
`crates/forge-warden-core/tests/authority.rs::a_text_file_sent_as_a_binary_section_is_held_by_the_starter`.
