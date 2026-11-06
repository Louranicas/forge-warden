//! Independent finite policy model, compiled only by the laws integration test.
//! This fixture never imports the production matcher, evaluator or vocabulary.

#[derive(Clone, Debug)]
pub(super) struct File {
    pub(super) id: usize,
    pub(super) group: u8, // 0 source, 1 tests, 2 docs
    pub(super) score: u32,
    pub(super) secret: bool,
    pub(super) shape: u8, // 0 ordinary, 1 deleted, 2 ambiguous binary, 3 new binary, 4 deleted binary
    pub(super) unknown: Option<(bool, String)>, // status or signal, always a future_ label
}
#[derive(Clone, Copy, Debug)]
pub(super) enum CheckSpec {
    Forbid,
    Protect,
    Companion,
    FileScore(u32),
    ChangeScore(u32),
    FileCount(std::num::NonZeroU32),
}
#[derive(Clone, Debug)]
pub(super) struct Rule {
    pub(super) kind: CheckSpec,
    pub(super) fail: bool,
    pub(super) scope: u8, // 0 all, 1 source, 2 docs
}
#[derive(Clone, Debug)]
pub(super) struct Case {
    pub(super) files: Vec<File>,
    pub(super) rules: Vec<Rule>,
    pub(super) waiver: u8, // none, today, tomorrow, yesterday; always for rule-0, source paths
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ResultSpec {
    Clear,
    Exempt,
    Review,
    Reject,
}

fn in_scope(scope: u8, file: &File) -> bool {
    scope == 0 || (scope == 1 && file.group == 0) || (scope == 2 && file.group == 2)
}
fn removed(file: &File) -> bool {
    matches!(file.shape, 1 | 4)
}

pub(super) fn reference(case: &Case) -> (u8, Vec<ResultSpec>) {
    let states: Vec<ResultSpec> = case
        .rules
        .iter()
        .enumerate()
        .map(|(index, rule)| {
            // Per-file failures carry an identity; aggregate budget failures cannot be waived.
            let violating: Vec<&File> = case
                .files
                .iter()
                .filter(|file| match rule.kind {
                    CheckSpec::Forbid => in_scope(rule.scope, file) && file.secret,
                    CheckSpec::Protect => in_scope(rule.scope, file),
                    CheckSpec::Companion => {
                        file.group == 0
                            && !removed(file)
                            && !case
                                .files
                                .iter()
                                .any(|other| other.group == 1 && matches!(other.shape, 0 | 3))
                    }
                    CheckSpec::FileScore(limit) => in_scope(rule.scope, file) && file.score > limit,
                    CheckSpec::ChangeScore(_) | CheckSpec::FileCount(_) => false,
                })
                .collect();
            let aggregate = match rule.kind {
                CheckSpec::ChangeScore(limit) => {
                    case.files
                        .iter()
                        .filter(|f| in_scope(rule.scope, f))
                        .map(|f| u64::from(f.score))
                        .sum::<u64>()
                        > u64::from(limit)
                }
                CheckSpec::FileCount(limit) => {
                    case.files
                        .iter()
                        .filter(|f| in_scope(rule.scope, f))
                        .count()
                        > usize::try_from(limit.get()).unwrap()
                }
                CheckSpec::Forbid
                | CheckSpec::Protect
                | CheckSpec::Companion
                | CheckSpec::FileScore(_) => false,
            };
            if !aggregate && violating.is_empty() {
                ResultSpec::Clear
            } else if !aggregate
                && index == 0
                && matches!(case.waiver, 1 | 2)
                && violating.iter().all(|f| f.group == 0)
            {
                ResultSpec::Exempt
            } else if rule.fail {
                ResultSpec::Reject
            } else {
                ResultSpec::Review
            }
        })
        .collect();
    let floor = if case.waiver == 3 {
        2
    } else {
        u8::from(case.files.is_empty() || case.files.iter().any(|f| f.unknown.is_some()))
    };
    let overall = states
        .iter()
        .map(|state| match state {
            ResultSpec::Clear | ResultSpec::Exempt => 0,
            ResultSpec::Review => 1,
            ResultSpec::Reject => 2,
        })
        .chain(std::iter::once(floor))
        .max()
        .unwrap();
    (overall, states)
}
