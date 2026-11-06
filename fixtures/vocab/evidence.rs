// Synthetic stand-in for deep-diff-forge's crates/deep-diff-forge-graph/src/evidence.rs, used by
// crates/forge-warden-cli/tests/check_vocab.rs.
pub enum EvidenceSignal {
    UnsafeAdded,
    ProcessSpawn,
    NetworkFetch,
    DynamicEval,
    CiPrivilege,
    BuildHook,
    SecretLike,
    CheckSuppressed,
    TestRemoved,
}

impl EvidenceSignal {
    pub const fn label(self) -> &'static str {
        match self {
            Self::UnsafeAdded => "unsafe_added",
            Self::ProcessSpawn => "process_spawn",
            Self::NetworkFetch => "network_fetch",
            Self::DynamicEval => "dynamic_eval",
            Self::CiPrivilege => "ci_privilege",
            Self::BuildHook => "build_hook",
            Self::SecretLike => "secret_like",
            Self::CheckSuppressed => "check_suppressed",
            Self::TestRemoved => "test_removed",
        }
    }
}
