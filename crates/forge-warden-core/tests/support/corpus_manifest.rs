//! Reviewed corpus identities/content; Rust sources pin membership, never implementation bytes.
use std::collections::BTreeMap;

type Entries = Vec<(String, String)>;

fn unique<T>(
    entries: impl IntoIterator<Item = (String, T)>,
) -> Result<BTreeMap<String, T>, String> {
    let mut result = BTreeMap::new();
    for (name, value) in entries {
        if result.insert(name.clone(), value).is_some() {
            return Err(format!("duplicate protected manifest identity: {name}"));
        }
    }
    Ok(result)
}

/// Compare names and SHA-256 with the protected manifest; duplicate names/groups fail.
/// Entries use arrays of pairs so JSON object decoding cannot silently discard duplicate keys.
pub(crate) fn check(
    group: &str,
    entries: impl IntoIterator<Item = (String, Vec<u8>)>,
) -> Result<(), String> {
    let actual = unique(
        entries
            .into_iter()
            .map(|(name, bytes)| (name, forge_warden_core::seal::sha256_hex(&bytes))),
    )?;
    let groups: Vec<(String, Entries)> =
        serde_json::from_str(include_str!("../../../../fixtures/coverage/MANIFEST"))
            .map_err(|error| format!("invalid protected corpus manifest: {error}"))?;
    let mut checked = Vec::new();
    for (name, expected_entries) in groups {
        if expected_entries.is_empty() {
            return Err(format!("empty protected corpus group: {name}"));
        }
        checked.push((name, unique(expected_entries)?));
    }
    let manifest = unique(checked)?;
    let expected = manifest
        .get(group)
        .ok_or_else(|| format!("missing protected corpus group: {group}"))?;
    if &actual != expected {
        return Err(format!(
            "corpus identities/content differ: {group}; actual={actual:?}; expected={expected:?}"
        ));
    }
    Ok(())
}
