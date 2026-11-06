// Synthetic stand-in for deep-diff-forge's crates/deep-diff-forge-graph/src/risk.rs, used by
// crates/forge-warden-cli/tests/check_vocab.rs. Same shape: an enum and its `label` match.
pub enum RiskSignal {
    PublicApiSurface,
    NewFile,
    DeletedFile,
    BinaryChange,
    LargeChange,
    ManyHunks,
    ConfigOrLockfile,
    TestOnly,
    GeneratedOrVendored,
    HazardousText,
    SymlinkChange,
    ExecutableBit,
    Code(EvidenceSignal),
}

impl RiskSignal {
    pub fn label(self) -> &'static str {
        match self {
            Self::PublicApiSurface => "public_api_surface",
            Self::NewFile => "new_file",
            Self::DeletedFile => "deleted_file",
            Self::BinaryChange => "binary_change",
            Self::LargeChange => "large_change",
            Self::ManyHunks => "many_hunks",
            Self::ConfigOrLockfile => "config_or_lockfile",
            Self::TestOnly => "test_only",
            Self::GeneratedOrVendored => "generated_or_vendored",
            Self::HazardousText => "hazardous_text",
            Self::SymlinkChange => "symlink_change",
            Self::ExecutableBit => "executable_bit",
            Self::Code(signal) => signal.label(),
        }
    }
}
