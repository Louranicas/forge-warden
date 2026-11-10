//! JEV Z3 independent authority oracle (WP-JEV-Z3; Touchstone's CASES.md 9e2497f1, rows P01-P20
//! and N01-N28). Standard library only: it compiles alone under `scripts/oracle_boundary.py`, so
//! it cannot reach the production comparison, signal union, evaluator, outcome fold, vocabulary or
//! exit registry. It owns plain case records, emits every input byte the actual side parses (the
//! patch, Git's numstat/summary output, the ranking JSON and the policy TOML) and the handwritten
//! expectations; the frozen files and manifest under `fixtures/z3/` are exactly what it emits.
//! All paths, contents and digests are invented local examples, not measured producer output.

/// The judging date D of every case unless a row moves it.
pub(super) const D: &str = "2026-11-09";
const D_PLUS_1: &str = "2026-11-10";
const D_MINUS_1: &str = "2026-11-08";
/// The invented ranker version every ranking states.
pub(super) const TOOL_VERSION: &str = "0.0.0-z3.synthetic";
const CLAIMED_VERSION: &str = "9.9.9-z3.claimed";
const MAX: u32 = u32::MAX;
/// The full implementation base these expectations were frozen against (frozen v2; Touchstone
/// T-Z3F1-03, OWNER-RESPONSE-1174 R1). It supersedes, without editing, the design packet's
/// `SOURCE-IDENTITY.json` at 6c968ad; a rebase re-pins in a new frozen revision.
const IMPLEMENTATION_BASE: &str = "04bbf7b5522129c4b8a749ba5297eecd94f692f0";
/// SHA-256 of each source the expectations were reviewed against, at that base.
const SOURCE_IDENTITY: [(&str, &str); 15] = [
    (
        "crates/forge-warden-core/src/authority.rs",
        "6d26b6393d5bf1d00cc5620a0035316ef939a7d7fc5b6aa52fb644e317e7f61c",
    ),
    (
        "crates/forge-warden-core/src/facts.rs",
        "e4705f77d4111e86bb2c34464b8c356e6a626dd48b62296bc109a7eca006a372",
    ),
    (
        "crates/forge-warden-core/src/eval.rs",
        "e0260645332a07213690210de451fbda814e36870375ad03dab3b7cbd9ee8dc3",
    ),
    (
        "crates/forge-warden-core/src/verdict.rs",
        "041a4cbe4b08c671a953adf4f0c9a4828e4c337a3d73f33b99469c818ba045f4",
    ),
    (
        "crates/forge-warden-core/src/exit.rs",
        "feccec631411e28f5a72df01b7a32d725ad7ce3b5041ce01cb898085c677398f",
    ),
    (
        "crates/forge-warden-core/src/rank.rs",
        "fa35fa8a59c2989984e30e3904626e61b19061a64bf023146ad9e76d6d309cd8",
    ),
    (
        "crates/forge-warden-core/src/policy/validate.rs",
        "1c7c4e7b427eb5c62e4a94d98bd48f1cb63abb47b8a601dce8a31a8c673cdebf",
    ),
    (
        "crates/forge-warden-core/src/policy/model.rs",
        "633dc4909fd402ab00538d91790f6ed5ca5a185302e349145dc9c3012adac8cb",
    ),
    (
        "crates/forge-warden-core/src/seal.rs",
        "1ce8e21c4ab06403a87fb35a1094f3807988a09b9751fe846f50dbc490a9aaa8",
    ),
    (
        "crates/forge-warden-core/tests/authority.rs",
        "9e8e0d11194fae4ebaff66b4c16841f00bcf0570bdfe61af8cdd637dc05eccdd",
    ),
    (
        "crates/forge-warden-core/tests/laws.rs",
        "1fc1edfe9f3d4498f02b9c52f1e739990d05fac08f7dfbd52f8d9c45907d3d16",
    ),
    (
        "fixtures/adversarial/reference_model.rs",
        "366a371ff0b040bff0bb25b4753ed66e4235dc739404d073567ede924ea6b3e1",
    ),
    (
        "docs/adr/0005-coverage-is-verified-not-claimed.md",
        "4663916ecafd9ecf7384346db2358dd0005f785956b2aaf72be692e3289d3fb1",
    ),
    (
        "scripts/oracle_boundary.py",
        "6bceb49dc91c7d557b6082be371ddb030c62787b9a5b6d96a88142b09fc3d99f",
    ),
    (
        "fixtures/quality-contract.json",
        "213cfd9637a203df105f0eba42bf1ce51a851976b93e00af74fbe7c1ff15430d",
    ),
];

/// How Git sees one file. `old`/`new` are its modes; an `Edit` with two different modes is a
/// content plus mode change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Shape {
    Edit,
    ModeOnly,
    Add,
    Del,
    BinEdit,
    BinAdd,
    BinDel,
    LinkAdd,
}

#[derive(Clone, Debug)]
pub(super) struct GitFile {
    pub(super) path: &'static str,
    pub(super) shape: Shape,
    pub(super) old: &'static str,
    pub(super) new: &'static str,
}

/// One piece of ranked evidence.
#[derive(Clone, Debug)]
pub(super) struct Ev {
    pub(super) signal: &'static str,
    pub(super) address: String,
    pub(super) side: &'static str,
    pub(super) line: u64,
}

#[derive(Clone, Debug)]
pub(super) struct Ranked {
    pub(super) path: &'static str,
    pub(super) status: &'static str,
    pub(super) score: u32,
    pub(super) signals: Vec<&'static str>,
    pub(super) evidence: Vec<Ev>,
}

/// A rule's check. Every rule is severity `fail` over all paths; a companion rule requires
/// `tests/**` when `src/**` changes.
#[derive(Clone, Copy, Debug)]
pub(super) enum Check {
    Forbid(&'static str),
    MaxFiles(u32),
    FileScore(u32),
    SumScore(u32),
    Companion,
}

#[derive(Clone, Debug)]
pub(super) struct Rule {
    pub(super) id: &'static str,
    pub(super) check: Check,
}

#[derive(Clone, Debug)]
pub(super) struct Waiver {
    pub(super) rule: &'static str,
    pub(super) paths: &'static str,
    pub(super) expires: &'static str,
}

/// What the caller hands the judge besides the produced ranking.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Supply {
    Absent,
    Identical,
    /// The ranking bytes plus one trailing LF.
    TrailingLf,
    /// The base ranking before the row's signal change.
    Different,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Stage {
    Judged,
    PolicyRefused,
    AdmissionRefused,
}

#[derive(Clone, Debug)]
pub(super) struct ViolExp {
    pub(super) path: Option<&'static str>,
    pub(super) signal: Option<&'static str>,
    pub(super) waived_by: Option<usize>,
    pub(super) evidence: Vec<Ev>,
    pub(super) message_contains: Option<&'static str>,
}

#[derive(Clone, Debug)]
pub(super) struct RuleExp {
    pub(super) id: &'static str,
    pub(super) outcome: &'static str,
    pub(super) violations: Vec<ViolExp>,
}

#[derive(Clone, Debug)]
pub(super) struct NoteExp {
    pub(super) code: &'static str,
    pub(super) path: Option<&'static str>,
    pub(super) detail_contains: Option<&'static str>,
}

/// One frozen case: inputs, then the handwritten expectation.
#[derive(Clone, Debug)]
pub(super) struct Case {
    pub(super) id: &'static str,
    pub(super) base: &'static str,
    pub(super) changes: &'static str,
    pub(super) today: &'static str,
    pub(super) git: Vec<GitFile>,
    /// A literal Git triple (patch, numstat -z, summary) instead of one emitted from `git`.
    pub(super) raw: Option<(&'static str, &'static str, &'static str)>,
    /// Facts unavailable: the adapter passes `GitRefused(reason)` instead of parsing a triple.
    pub(super) refused: Option<&'static str>,
    pub(super) rank: Vec<Ranked>,
    pub(super) rules: Vec<Rule>,
    pub(super) waivers: Vec<Waiver>,
    pub(super) supplied_origin: bool,
    pub(super) copy: Supply,
    /// The caller's false provenance claim (P20): supplied, producer B, a claimed version.
    pub(super) false_claim: bool,
    /// Ask admission for producer B, which the allowlist lacks (N28).
    pub(super) admit_b: bool,
    pub(super) stage: Stage,
    pub(super) coverage: Option<bool>,
    pub(super) verdict: &'static str,
    pub(super) results: Vec<RuleExp>,
    pub(super) notes: Vec<NoteExp>,
    pub(super) expired: Vec<usize>,
    pub(super) unused: Vec<usize>,
    pub(super) named: Vec<&'static str>,
    /// The exact facts refusal kind `from_git` is specified to return (N20/N21; Lodestar's
    /// Z3-N20-N21 ruling): a specificity assertion, separate from the safety ones.
    pub(super) refusal_kind: Option<&'static str>,
}

// ---------------------------------------------------------------- SHA-256 (FIPS 180-4), std only

const K: [u32; 64] = [
    0x428a_2f98,
    0x7137_4491,
    0xb5c0_fbcf,
    0xe9b5_dba5,
    0x3956_c25b,
    0x59f1_11f1,
    0x923f_82a4,
    0xab1c_5ed5,
    0xd807_aa98,
    0x1283_5b01,
    0x2431_85be,
    0x550c_7dc3,
    0x72be_5d74,
    0x80de_b1fe,
    0x9bdc_06a7,
    0xc19b_f174,
    0xe49b_69c1,
    0xefbe_4786,
    0x0fc1_9dc6,
    0x240c_a1cc,
    0x2de9_2c6f,
    0x4a74_84aa,
    0x5cb0_a9dc,
    0x76f9_88da,
    0x983e_5152,
    0xa831_c66d,
    0xb003_27c8,
    0xbf59_7fc7,
    0xc6e0_0bf3,
    0xd5a7_9147,
    0x06ca_6351,
    0x1429_2967,
    0x27b7_0a85,
    0x2e1b_2138,
    0x4d2c_6dfc,
    0x5338_0d13,
    0x650a_7354,
    0x766a_0abb,
    0x81c2_c92e,
    0x9272_2c85,
    0xa2bf_e8a1,
    0xa81a_664b,
    0xc24b_8b70,
    0xc76c_51a3,
    0xd192_e819,
    0xd699_0624,
    0xf40e_3585,
    0x106a_a070,
    0x19a4_c116,
    0x1e37_6c08,
    0x2748_774c,
    0x34b0_bcb5,
    0x391c_0cb3,
    0x4ed8_aa4a,
    0x5b9c_ca4f,
    0x682e_6ff3,
    0x748f_82ee,
    0x78a5_636f,
    0x84c8_7814,
    0x8cc7_0208,
    0x90be_fffa,
    0xa450_6ceb,
    0xbef9_a3f7,
    0xc671_78f2,
];

/// Lowercase hex SHA-256 of `data`, independent of the production seal.
pub(super) fn sha256(data: &[u8]) -> String {
    let mut state: [u32; 8] = [
        0x6a09_e667,
        0xbb67_ae85,
        0x3c6e_f372,
        0xa54f_f53a,
        0x510e_527f,
        0x9b05_688c,
        0x1f83_d9ab,
        0x5be0_cd19,
    ];
    let bits = u64::try_from(data.len())
        .unwrap_or(u64::MAX)
        .wrapping_mul(8);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len().checked_rem(64) != Some(56) {
        msg.push(0);
    }
    msg.extend_from_slice(&bits.to_be_bytes());
    let (blocks, _) = msg.as_chunks::<64>();
    for block in blocks {
        let (words, _) = block.as_chunks::<4>();
        let mut schedule: Vec<u32> = words.iter().map(|word| u32::from_be_bytes(*word)).collect();
        while schedule.len() < 64 {
            let back = |n: usize| -> u32 {
                schedule
                    .len()
                    .checked_sub(n)
                    .and_then(|at| schedule.get(at))
                    .copied()
                    .unwrap_or(0)
            };
            let (w15, w2, w16, w7) = (back(15), back(2), back(16), back(7));
            let s0 = w15.rotate_right(7) ^ w15.rotate_right(18) ^ (w15 >> 3);
            let s1 = w2.rotate_right(17) ^ w2.rotate_right(19) ^ (w2 >> 10);
            schedule.push(w16.wrapping_add(s0).wrapping_add(w7).wrapping_add(s1));
        }
        let mut work = state;
        for (round, word) in K.iter().zip(&schedule) {
            let [va, vb, vc, vd, ve, vf, vg, vh] = work;
            let big1 = ve.rotate_right(6) ^ ve.rotate_right(11) ^ ve.rotate_right(25);
            let choose = (ve & vf) ^ (!ve & vg);
            let t1 = vh
                .wrapping_add(big1)
                .wrapping_add(choose)
                .wrapping_add(*round)
                .wrapping_add(*word);
            let big0 = va.rotate_right(2) ^ va.rotate_right(13) ^ va.rotate_right(22);
            let majority = (va & vb) ^ (va & vc) ^ (vb & vc);
            let t2 = big0.wrapping_add(majority);
            work = [
                t1.wrapping_add(t2),
                va,
                vb,
                vc,
                vd.wrapping_add(t1),
                ve,
                vf,
                vg,
            ];
        }
        for (slot, add) in state.iter_mut().zip(work) {
            *slot = slot.wrapping_add(add);
        }
    }
    state
        .iter()
        .fold(String::new(), |digits, word| cat(&[&digits, &hex8(*word)]))
}

/// The invented digest of synthetic producer A (allowlisted) or B (not).
pub(super) fn producer(b: bool) -> String {
    sha256(if b {
        b"forge-warden JEV-Z3 synthetic producer B"
    } else {
        b"forge-warden JEV-Z3 synthetic producer A"
    })
}

fn address(case: &str) -> String {
    sha256(cat(&[&"forge-warden JEV-Z3 invented hunk address ", &case]).as_bytes())
}

// ---------------------------------------------------------------- text helpers (no macros)

/// Concatenate each piece's `Display` text: the oracle's only string builder.
fn cat(pieces: &[&dyn std::fmt::Display]) -> String {
    pieces.iter().map(ToString::to_string).collect()
}

/// Lowercase hexadecimal of the low `width` nibbles of `value`, zero padded.
fn hex(value: u32, width: u32) -> String {
    (0..width)
        .rev()
        .map(|nibble| {
            let digit = value.checked_shr(nibble.saturating_mul(4)).unwrap_or(0) & 0xf;
            char::from_digit(digit, 16).unwrap_or('?')
        })
        .collect()
}

fn hex8(value: u32) -> String {
    hex(value, 8)
}

fn hex4(value: u32) -> String {
    hex(value, 4)
}

// ---------------------------------------------------------------- emitters (wire bytes)

fn file_patch(f: &GitFile) -> String {
    let p = f.path;
    let head = cat(&[&"diff --git a/", &p, &" b/", &p, &"\n"]);
    let edit = cat(&[
        &"--- a/",
        &p,
        &"\n+++ b/",
        &p,
        &"\n@@ -1 +1 @@\n-old\n+new\n",
    ]);
    let add = cat(&[&"--- /dev/null\n+++ b/", &p, &"\n@@ -0,0 +1 @@\n+new\n"]);
    let del = cat(&[&"--- a/", &p, &"\n+++ /dev/null\n@@ -1 +0,0 @@\n-old\n"]);
    let (old, new) = (f.old, f.new);
    let body = match f.shape {
        Shape::Edit if old == new => cat(&[&"index 1111111..2222222 ", &old, &"\n", &edit]),
        Shape::Edit => cat(&[
            &"old mode ",
            &old,
            &"\nnew mode ",
            &new,
            &"\nindex 1111111..2222222\n",
            &edit,
        ]),
        Shape::ModeOnly => cat(&[&"old mode ", &old, &"\nnew mode ", &new, &"\n"]),
        Shape::Add => cat(&[&"new file mode ", &new, &"\nindex 0000000..2222222\n", &add]),
        Shape::Del => cat(&[
            &"deleted file mode ",
            &old,
            &"\nindex 1111111..0000000\n",
            &del,
        ]),
        Shape::BinEdit => cat(&[
            &"index 1111111..2222222 ",
            &old,
            &"\nBinary files a/",
            &p,
            &" and b/",
            &p,
            &" differ\n",
        ]),
        Shape::BinAdd => cat(&[
            &"new file mode ",
            &new,
            &"\nindex 0000000..2222222\nBinary files /dev/null and b/",
            &p,
            &" differ\n",
        ]),
        Shape::BinDel => cat(&[
            &"deleted file mode ",
            &old,
            &"\nindex 1111111..0000000\nBinary files a/",
            &p,
            &" and /dev/null differ\n",
        ]),
        Shape::LinkAdd => cat(&[
            &"new file mode ",
            &new,
            &"\nindex 0000000..2222222\n--- /dev/null\n+++ b/",
            &p,
            &"\n@@ -0,0 +1 @@\n+target\n\\ No newline at end of file\n",
        ]),
    };
    cat(&[&head, &body])
}

fn numstat_record(f: &GitFile) -> String {
    let counts = match f.shape {
        Shape::Edit => "1\t1",
        Shape::ModeOnly => "0\t0",
        Shape::Add | Shape::LinkAdd => "1\t0",
        Shape::Del => "0\t1",
        Shape::BinEdit | Shape::BinAdd | Shape::BinDel => "-\t-",
    };
    cat(&[&counts, &"\t", &f.path, &"\0"])
}

fn summary_line(f: &GitFile) -> String {
    let p = f.path;
    match f.shape {
        Shape::Add | Shape::BinAdd | Shape::LinkAdd => {
            cat(&[&" create mode ", &f.new, &" ", &p, &"\n"])
        }
        Shape::Del | Shape::BinDel => cat(&[&" delete mode ", &f.old, &" ", &p, &"\n"]),
        Shape::Edit | Shape::ModeOnly if f.old != f.new => {
            cat(&[&" mode change ", &f.old, &" => ", &f.new, &" ", &p, &"\n"])
        }
        Shape::Edit | Shape::ModeOnly | Shape::BinEdit => String::new(),
    }
}

fn json_str(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out = cat(&[&out, &"\\u", &hex4(u32::from(c))]),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn json_list<T>(items: &[T], each: impl Fn(&T) -> String) -> String {
    cat(&[
        &"[",
        &(items.iter().map(each).collect::<Vec<_>>().join(",")),
        &"]",
    ])
}

fn opt_str(v: Option<&str>) -> String {
    v.map_or_else(|| "null".to_owned(), json_str)
}

fn opt_num(v: Option<usize>) -> String {
    v.map_or_else(|| "null".to_owned(), |n| n.to_string())
}

fn evidence_json(e: &Ev) -> String {
    cat(&[
        &"{\"signal\":",
        &(json_str(e.signal)),
        &",\"address\":",
        &(json_str(&e.address)),
        &",\"side\":",
        &(json_str(e.side)),
        &",\"line\":",
        &e.line,
        &"}",
    ])
}

fn rank_json(seal: &str, rank: &[Ranked]) -> String {
    let files = json_list(rank, |r| {
        cat(&[
            &"{\"path\":",
            &(json_str(r.path)),
            &",\"status\":",
            &(json_str(r.status)),
            &",\"score\":",
            &r.score,
            &",\"signals\":",
            &(json_list(&r.signals, |s| json_str(s))),
            &",\"evidence\":",
            &(json_list(&r.evidence, evidence_json)),
            &"}",
        ])
    });
    cat(&[
        &"{\"schema\":\"deep-diff-forge.rank.v0\",\"input_sha256\":",
        &(json_str(seal)),
        &",\"tool\":{\"name\":\"deep-diff-forge\",\"version\":",
        &(json_str(TOOL_VERSION)),
        &"},\"ranked\":",
        &files,
        &"}\n",
    ])
}

fn rule_toml(r: &Rule) -> String {
    let fields = match r.check {
        Check::Forbid(signal) => cat(&[
            &"kind=\"forbid_signal\"\nsignals=[\"",
            &signal,
            &"\"]\npaths=[\"**\"]",
        ]),
        Check::MaxFiles(limit) => {
            cat(&[&"kind=\"max_files\"\nlimit=", &limit, &"\npaths=[\"**\"]"])
        }
        Check::FileScore(limit) => cat(&[
            &"kind=\"max_score\"\nper=\"file\"\nlimit=",
            &limit,
            &"\npaths=[\"**\"]",
        ]),
        Check::SumScore(limit) => cat(&[
            &"kind=\"max_score\"\nper=\"change\"\nlimit=",
            &limit,
            &"\npaths=[\"**\"]",
        ]),
        Check::Companion => {
            "kind=\"require_companion\"\nwhen=[\"src/**\"]\nrequire=[\"tests/**\"]".to_owned()
        }
    };
    cat(&[
        &"[[rule]]\nid=\"",
        &r.id,
        &"\"\nwhy=\"Z3 fixed rule.\"\nseverity=\"fail\"\n",
        &fields,
        &"\n",
    ])
}

fn policy_toml(case: &Case) -> String {
    let mut out = cat(&[
        &"schema=\"forge-warden.policy.v0\"\n\n[producer]\nsha256=[\"",
        &(producer(false)),
        &"\"]\n",
    ]);
    for r in &case.rules {
        out.push('\n');
        out.push_str(&rule_toml(r));
    }
    for w in &case.waivers {
        out = cat(&[
            &out,
            &"\n[[waiver]]\nrule=\"",
            &w.rule,
            &"\"\npaths=[\"",
            &w.paths,
            &"\"]\nreason=\"Frozen Z3 boundary\"\nowner=\"Test owner\"\nexpires=\"",
            &w.expires,
            &"\"\n",
        ]);
    }
    out
}

impl Case {
    /// The patch bytes.
    pub(super) fn patch(&self) -> String {
        match self.raw {
            Some((patch, _, _)) => patch.to_owned(),
            None => self.git.iter().map(file_patch).collect(),
        }
    }

    /// Git's `--numstat -z` and `--summary` bytes, absent when the facts are refused.
    pub(super) fn facts(&self) -> Option<(String, String)> {
        if self.refused.is_some() {
            return None;
        }
        Some(match self.raw {
            Some((_, numstat, summary)) => (numstat.to_owned(), summary.to_owned()),
            None => (
                self.git.iter().map(numstat_record).collect(),
                self.git.iter().map(summary_line).collect(),
            ),
        })
    }

    /// Every input file, by its name under `fixtures/z3/cases/<id>/`.
    pub(super) fn files(&self) -> Vec<(&'static str, String)> {
        let patch = self.patch();
        let rank = rank_json(&sha256(patch.as_bytes()), &self.rank);
        let mut out = std::vec![("patch", patch.clone())];
        if let Some((numstat, summary)) = self.facts() {
            out.push(("numstat-z", numstat));
            out.push(("summary", summary));
        }
        let supplied = match self.copy {
            Supply::Absent => None,
            Supply::Identical => Some(rank.clone()),
            Supply::TrailingLf => Some(cat(&[&rank, &"\n"])),
            Supply::Different => Some(rank_json(&sha256(patch.as_bytes()), &self.base_rank())),
        };
        out.push(("rank.json", rank));
        if let Some(bytes) = supplied {
            out.push(("supplied.json", bytes));
        }
        out.push(("policy.toml", policy_toml(self)));
        out
    }

    /// The ranking before this row's signal change (only N03 supplies it).
    fn base_rank(&self) -> Vec<Ranked> {
        self.rank
            .iter()
            .map(|r| Ranked {
                signals: r
                    .signals
                    .iter()
                    .copied()
                    .filter(|s| *s != "secret_like")
                    .collect(),
                ..r.clone()
            })
            .collect()
    }

    /// The literal exit tuple for the expected verdict (handwritten registry rows).
    fn exit(&self) -> String {
        let (code, name, action) = match self.verdict {
            "pass" => (0, "pass", "proceed"),
            "hold" => (8, "policy_hold", "review_evidence"),
            "fail" => (7, "policy_fail", "change_input"),
            _ => return "null".to_owned(),
        };
        cat(&[
            &"{\"code\":",
            &code,
            &",\"name\":",
            &(json_str(name)),
            &",\"action\":",
            &(json_str(action)),
            &"}",
        ])
    }

    fn summary(&self) -> String {
        if self.stage != Stage::Judged {
            return "null".to_owned();
        }
        cat(&[
            &"{\"files\":",
            &(self.rank.len()),
            &",\"rules\":",
            &(self.rules.len()),
            &",\"passed\":",
            &(self.count("pass")),
            &",\"waived\":",
            &(self.count("waived")),
            &",\"held\":",
            &(self.count("hold")),
            &",\"failed\":",
            &(self.count("fail")),
            &"}",
        ])
    }

    fn count(&self, outcome: &str) -> usize {
        self.results.iter().filter(|r| r.outcome == outcome).count()
    }

    /// The manifest entry: identity, input digests and every expected field, as one JSON object.
    pub(super) fn manifest_entry(&self) -> String {
        let stage = match self.stage {
            Stage::Judged => "judged",
            Stage::PolicyRefused => "policy_refused",
            Stage::AdmissionRefused => "admission_refused",
        };
        let digests = json_list(&self.files(), |(name, bytes)| {
            cat(&[
                &"{\"file\":",
                &(json_str(name)),
                &",\"sha256\":",
                &(json_str(&sha256(bytes.as_bytes()))),
                &"}",
            ])
        });
        let results = json_list(&self.results, |r| {
            let violations = json_list(&r.violations, |v| {
                cat(&[
                    &"{\"path\":",
                    &(opt_str(v.path)),
                    &",\"signal\":",
                    &(opt_str(v.signal)),
                    &",\"waived_by\":",
                    &(opt_num(v.waived_by)),
                    &",\"evidence\":",
                    &(json_list(&v.evidence, evidence_json)),
                    &",\"message_contains\":",
                    &(opt_str(v.message_contains)),
                    &"}",
                ])
            });
            cat(&[
                &"{\"id\":",
                &(json_str(r.id)),
                &",\"outcome\":",
                &(json_str(r.outcome)),
                &",\"violations\":",
                &violations,
                &"}",
            ])
        });
        let notes = json_list(&self.notes, |n| {
            cat(&[
                &"{\"code\":",
                &(json_str(n.code)),
                &",\"path\":",
                &(opt_str(n.path)),
                &",\"detail_contains\":",
                &(opt_str(n.detail_contains)),
                &"}",
            ])
        });
        let nums = |v: &[usize]| json_list(v, ToString::to_string);
        let (provenance, ranked_by) = if self.supplied_origin {
            ("supplied", "null".to_owned())
        } else {
            ("produced", json_str(&producer(false)))
        };
        let copy = match self.copy {
            Supply::Absent => "absent",
            Supply::Identical => "identical",
            Supply::TrailingLf => "trailing_lf",
            Supply::Different => "different",
        };
        let coverage = self
            .coverage
            .map_or_else(|| "null".to_owned(), |c| c.to_string());
        let mut assertions: Vec<String> = [
            "setup",
            "coverage",
            "overall",
            "rules",
            "notes",
            "summary",
            "waivers",
            "provenance",
        ]
        .iter()
        .filter(|a| self.stage == Stage::Judged || **a == "setup")
        .map(|a| cat(&[&"Z3.", &self.id, &".", &a]))
        .collect();
        assertions.extend(
            self.named
                .iter()
                .map(|n| cat(&[&"Z3.", &self.id, &".", &n])),
        );
        cat(&[
            &"{\"case_id\":",
            &(json_str(self.id)),
            &",\"base_case_id\":",
            &(json_str(self.base)),
            &",\"intentional_changes\":",
            &(json_str(self.changes)),
            &",\"source_kind\":\"synthetic-core\",\"setup_stage\":",
            &(json_str(stage)),
            &",\"today\":",
            &(json_str(self.today)),
            &",\"origin\":",
            &(json_str(if self.supplied_origin {
                "supplied"
            } else {
                "produced"
            })),
            &",\"supplied_copy\":",
            &(json_str(copy)),
            &",\"facts\":",
            &(self.refused.map_or_else(
                || json_str("git"),
                |why| json_str(&cat(&[&"refused:", &why])),
            )),
            &",\"false_claim\":",
            &self.false_claim,
            &",\"admit_producer\":",
            &(json_str(&producer(self.admit_b))),
            &",\"input_digests\":",
            &digests,
            &",\"expected_coverage\":",
            &coverage,
            &",\"expected_verdict\":",
            &(if self.stage == Stage::Judged {
                json_str(self.verdict)
            } else {
                "null".to_owned()
            }),
            &",\"expected_exit\":",
            &(self.exit()),
            &",\"expected_summary\":",
            &(self.summary()),
            &",\"expected_rule_results\":",
            &results,
            &",\"expected_notes\":",
            &notes,
            &",\"expected_expired_waivers\":",
            &(nums(&self.expired)),
            &",\"expected_unused_waivers\":",
            &(nums(&self.unused)),
            &",\"expected_provenance\":{\"provenance\":",
            &(json_str(provenance)),
            &",\"ranked_by_sha256\":",
            &ranked_by,
            &",\"version\":",
            &(json_str(TOOL_VERSION)),
            &"},\"expected_facts_refusal\":",
            &(opt_str(self.refusal_kind)),
            &",\"required_assertions\":",
            &(json_list(&assertions, |a| json_str(a))),
            &"}",
        ])
    }
}

/// The whole manifest: one entry per case, in ID order, one per line.
pub(super) fn manifest() -> String {
    let entries: Vec<String> = cases().iter().map(Case::manifest_entry).collect();
    cat(&[
        &"{\"schema\":\"forge-warden.z3-manifest.v2\",\"design\":{\"DESIGN.md\":\"232c63d2\",\"CASES.md\":\"9e2497f1\",\"SCORE-FAMILY.md\":\"77463d72\"},\"implementation_base\":",
        &(json_str(IMPLEMENTATION_BASE)),
        &",\"source_identity\":",
        &(json_list(&SOURCE_IDENTITY, |(path, sha)| {
            cat(&[
                &"{\"path\":",
                &(json_str(path)),
                &",\"sha256\":",
                &(json_str(sha)),
                &"}",
            ])
        })),
        &",\"producer_a\":",
        &(json_str(&producer(false))),
        &",\"producer_b\":",
        &(json_str(&producer(true))),
        &",\"claimed_version\":",
        &(json_str(CLAIMED_VERSION)),
        &",\"cases\":[\n",
        &(entries.join(",\n")),
        &"\n]}\n",
    ])
}

// ---------------------------------------------------------------- generated-law model

/// Every Git shape the bounded law generator draws: (shape, old mode, new mode). Modes are
/// 100644, 100755 and 120000 in supported transitions only.
pub(super) const LAW_SHAPES: [(Shape, &str, &str); 14] = [
    (Shape::Edit, "100644", "100644"),
    (Shape::Edit, "100755", "100755"),
    (Shape::Edit, "100644", "100755"),
    (Shape::Edit, "100755", "100644"),
    (Shape::ModeOnly, "100644", "100755"),
    (Shape::ModeOnly, "100755", "100644"),
    (Shape::Add, "", "100644"),
    (Shape::Add, "", "100755"),
    (Shape::Del, "100644", ""),
    (Shape::Del, "100755", ""),
    (Shape::BinEdit, "100644", "100644"),
    (Shape::BinAdd, "", "100644"),
    (Shape::BinDel, "100644", ""),
    (Shape::LinkAdd, "", "120000"),
];

/// DESIGN.md's handwritten facts/status table: the status a ranking must give a Git shape and
/// the complete change-kind signals Git's facts require of it. Executable is gained only on an
/// addition at 100755 or a modification into 100755; a deletion never gains it.
pub(super) fn ranked_for(g: &GitFile) -> (&'static str, Vec<&'static str>) {
    let gains_exec = g.new == "100755" && g.old != "100755";
    let exec: &[&'static str] = if gains_exec { &["executable_bit"] } else { &[] };
    let (status, kind): (&'static str, &[&'static str]) = match g.shape {
        Shape::Edit => ("modified", &[]),
        Shape::ModeOnly if g.old == g.new => ("modified", &[]),
        Shape::ModeOnly => ("mode_changed", &[]),
        Shape::Add => ("added", &["new_file"]),
        Shape::Del => ("deleted", &["deleted_file"]),
        Shape::BinEdit => ("binary_changed", &["binary_change"]),
        Shape::BinAdd => ("binary_changed", &["binary_change", "new_file"]),
        Shape::BinDel => ("binary_changed", &["binary_change", "deleted_file"]),
        Shape::LinkAdd => ("added", &["new_file", "symlink_change"]),
    };
    (status, kind.iter().chain(exec).copied().collect())
}

// ---------------------------------------------------------------- records

fn git(path: &'static str, shape: Shape, old: &'static str, new: &'static str) -> GitFile {
    GitFile {
        path,
        shape,
        old,
        new,
    }
}

fn edit(path: &'static str) -> GitFile {
    git(path, Shape::Edit, "100644", "100644")
}

fn ranked(
    path: &'static str,
    status: &'static str,
    score: u32,
    signals: &[&'static str],
) -> Ranked {
    Ranked {
        path,
        status,
        score,
        signals: signals.to_vec(),
        evidence: Vec::new(),
    }
}

fn rule(id: &'static str, check: Check) -> Rule {
    Rule { id, check }
}

fn waiver(rule: &'static str, paths: &'static str, expires: &'static str) -> Waiver {
    Waiver {
        rule,
        paths,
        expires,
    }
}

fn pass(id: &'static str) -> RuleExp {
    RuleExp {
        id,
        outcome: "pass",
        violations: Vec::new(),
    }
}

fn hit(id: &'static str, outcome: &'static str, v: ViolExp) -> RuleExp {
    RuleExp {
        id,
        outcome,
        violations: std::vec![v],
    }
}

fn at(path: &'static str) -> ViolExp {
    ViolExp {
        path: Some(path),
        signal: None,
        waived_by: None,
        evidence: Vec::new(),
        message_contains: None,
    }
}

fn signal(path: &'static str, s: &'static str) -> ViolExp {
    ViolExp {
        signal: Some(s),
        ..at(path)
    }
}

fn aggregate(message: &'static str) -> ViolExp {
    ViolExp {
        path: None,
        message_contains: Some(message),
        ..at("")
    }
}

fn note(code: &'static str, path: Option<&'static str>) -> NoteExp {
    NoteExp {
        code,
        path,
        detail_contains: None,
    }
}

fn ev(signal: &'static str, case: &str) -> Ev {
    Ev {
        signal,
        address: address(case),
        side: "new",
        line: 1,
    }
}

/// P01: one ordinary text edit of `src/a.rs`, score 0, no signals; one fail-severity
/// `forbid_signal` rule `secret` over all paths; produced by allowlisted producer A; date D.
fn p01() -> Case {
    Case {
        id: "P01",
        base: "",
        changes: "common ordinary edit",
        today: D,
        git: std::vec![edit("src/a.rs")],
        raw: None,
        refused: None,
        rank: std::vec![ranked("src/a.rs", "modified", 0, &[])],
        rules: std::vec![rule("secret", Check::Forbid("secret_like"))],
        waivers: Vec::new(),
        supplied_origin: false,
        copy: Supply::Absent,
        false_claim: false,
        admit_b: false,
        stage: Stage::Judged,
        coverage: Some(true),
        verdict: "pass",
        results: std::vec![pass("secret")],
        notes: Vec::new(),
        expired: Vec::new(),
        unused: Vec::new(),
        named: Vec::new(),
        refusal_kind: None,
    }
}

/// `base` with a new identity.
fn from(base: Case, id: &'static str, changes: &'static str) -> Case {
    Case {
        base: base.id,
        id,
        changes,
        named: Vec::new(),
        ..base
    }
}

/// A one-file shape positive: Git `g`, ranked status and signals.
fn shape(
    id: &'static str,
    changes: &'static str,
    g: GitFile,
    status: &'static str,
    signals: &[&'static str],
) -> Case {
    Case {
        git: std::vec![g],
        rank: std::vec![ranked("src/a.rs", status, 0, signals)],
        ..from(p01(), id, changes)
    }
}

fn p03() -> Case {
    shape(
        "P03",
        "text addition 100644",
        git("src/a.rs", Shape::Add, "", "100644"),
        "added",
        &["new_file"],
    )
}

fn p04() -> Case {
    shape(
        "P04",
        "text deletion 100644",
        git("src/a.rs", Shape::Del, "100644", ""),
        "deleted",
        &["deleted_file"],
    )
}

fn p11() -> Case {
    Case {
        git: std::vec![edit("src/a.rs"), edit("docs/b.rs")],
        rank: std::vec![
            ranked("src/a.rs", "modified", 0, &[]),
            ranked("docs/b.rs", "modified", 0, &[]),
        ],
        rules: std::vec![rule("count", Check::MaxFiles(2))],
        results: std::vec![pass("count")],
        ..from(
            p01(),
            "P11",
            "add docs/b.rs edit; rule max_files count limit 2",
        )
    }
}

fn scored(id: &'static str, changes: &'static str, score: u32, limit: u32) -> Case {
    Case {
        rank: std::vec![ranked("src/a.rs", "modified", score, &[])],
        rules: std::vec![rule("file-score", Check::FileScore(limit))],
        results: std::vec![pass("file-score")],
        ..from(p01(), id, changes)
    }
}

fn p12() -> Case {
    scored(
        "P12",
        "score 10; max_score file-score per file limit 10",
        10,
        10,
    )
}

fn p13() -> Case {
    Case {
        base: "P12",
        ..scored("P13", "score 0, limit 0", 0, 0)
    }
}

fn p14() -> Case {
    Case {
        git: std::vec![edit("src/a.rs"), edit("docs/b.rs")],
        rank: std::vec![
            ranked("src/a.rs", "modified", 6, &[]),
            ranked("docs/b.rs", "modified", 4, &[]),
        ],
        rules: std::vec![rule("sum", Check::SumScore(10))],
        results: std::vec![pass("sum")],
        ..from(
            p01(),
            "P14",
            "src/a.rs 6 + docs/b.rs 4; max_score sum per change limit 10",
        )
    }
}

fn p15() -> Case {
    Case {
        base: "P12",
        ..scored("P15", "score and limit u32::MAX", MAX, MAX)
    }
}

fn p16() -> Case {
    Case {
        base: "P12",
        waivers: std::vec![waiver("file-score", "src/**", D)],
        results: std::vec![hit(
            "file-score",
            "waived",
            ViolExp {
                waived_by: Some(0),
                ..at("src/a.rs")
            },
        )],
        named: std::vec!["expiry_equality"],
        ..scored(
            "P16",
            "score 11; waiver 0 for file-score on src/** expires D",
            11,
            10,
        )
    }
}

fn p17() -> Case {
    Case {
        git: std::vec![
            edit("src/a.rs"),
            git("tests/a.rs", Shape::Add, "", "100644"),
        ],
        rank: std::vec![
            ranked("src/a.rs", "modified", 0, &[]),
            ranked("tests/a.rs", "added", 0, &["new_file"]),
        ],
        rules: std::vec![rule("tests", Check::Companion)],
        results: std::vec![pass("tests")],
        ..from(
            p01(),
            "P17",
            "add tests/a.rs text addition; require_companion tests",
        )
    }
}

fn p19() -> Case {
    let mut file = ranked("src/a.rs", "modified", 0, &["secret_like"]);
    file.evidence = std::vec![ev("secret_like", "P19")];
    Case {
        rank: std::vec![file],
        waivers: std::vec![waiver("secret", "src/**", D)],
        results: std::vec![hit(
            "secret",
            "waived",
            ViolExp {
                signal: Some("secret_like"),
                waived_by: Some(0),
                evidence: std::vec![ev("secret_like", "P19")],
                ..at("src/a.rs")
            },
        )],
        ..from(
            p01(),
            "P19",
            "secret_like with one citation; waiver 0 for secret on src/** expires D",
        )
    }
}

fn n01() -> Case {
    Case {
        supplied_origin: true,
        verdict: "hold",
        notes: std::vec![note("rank_unverified", None)],
        named: std::vec!["advisory_no_pass"],
        ..from(p01(), "N01", "origin supplied, advisory judge")
    }
}

fn n10() -> Case {
    Case {
        refused: Some("Z3 synthetic refusal"),
        coverage: Some(false),
        verdict: "hold",
        notes: std::vec![NoteExp {
            detail_contains: Some("Z3 synthetic refusal"),
            ..note("patch_unverifiable", None)
        }],
        ..from(
            p01(),
            "N10",
            "facts unavailable: GitRefused(\"Z3 synthetic refusal\")",
        )
    }
}

fn n14_policy(c: Case) -> Case {
    Case {
        rules: std::vec![rule("delete", Check::Forbid("deleted_file"))],
        ..c
    }
}

fn n14() -> Case {
    let c = Case {
        git: std::vec![git("src/a.rs", Shape::BinDel, "100644", "")],
        rank: std::vec![ranked("src/a.rs", "binary_changed", 0, &["binary_change"])],
        coverage: Some(false),
        verdict: "fail",
        results: std::vec![hit("delete", "fail", signal("src/a.rs", "deleted_file"))],
        notes: std::vec![note("status_mismatch", Some("src/a.rs"))],
        named: std::vec!["delete_signal"],
        ..from(
            p01(),
            "N14",
            "binary deletion ranked binary_changed/[binary_change] without deleted_file; forbid_signal delete",
        )
    };
    n14_policy(c)
}

fn raw_case(
    id: &'static str,
    changes: &'static str,
    patch: &'static str,
    detail: &'static str,
    kind: &'static str,
) -> Case {
    Case {
        raw: Some((patch, "1\t1\tsrc/a.rs\0", "")),
        refusal_kind: Some(kind),
        coverage: Some(false),
        verdict: "hold",
        notes: std::vec![NoteExp {
            detail_contains: Some(detail),
            ..note("patch_unverifiable", None)
        }],
        named: std::vec!["refusal_kind"],
        ..from(p01(), id, changes)
    }
}

/// Every frozen case, P01-P20 then N01-N28, with N14's complete-signal companion after N14.
pub(super) fn cases() -> Vec<Case> {
    let mut out = std::vec![p01()];
    out.push(Case {
        copy: Supply::Identical,
        ..from(p01(), "P02", "plus byte-identical supplied copy")
    });
    out.push(p03());
    out.push(p04());
    out.push(shape(
        "P05",
        "mode-only 100644->100755",
        git("src/a.rs", Shape::ModeOnly, "100644", "100755"),
        "mode_changed",
        &["executable_bit"],
    ));
    out.push(shape(
        "P06",
        "mode-only 100755->100644",
        git("src/a.rs", Shape::ModeOnly, "100755", "100644"),
        "mode_changed",
        &[],
    ));
    out.push(shape(
        "P07",
        "content edit plus 100644->100755",
        git("src/a.rs", Shape::Edit, "100644", "100755"),
        "modified",
        &["executable_bit"],
    ));
    out.push(shape(
        "P08",
        "binary modification",
        git("src/a.rs", Shape::BinEdit, "100644", "100644"),
        "binary_changed",
        &["binary_change"],
    ));
    out.push(shape(
        "P09",
        "binary addition 100644",
        git("src/a.rs", Shape::BinAdd, "", "100644"),
        "binary_changed",
        &["binary_change", "new_file"],
    ));
    out.push(shape(
        "P10",
        "symlink addition 120000",
        git("src/a.rs", Shape::LinkAdd, "", "120000"),
        "added",
        &["new_file", "symlink_change"],
    ));
    out.push(p11());
    out.push(p12());
    out.push(p13());
    out.push(p14());
    out.push(p15());
    out.push(p16());
    out.push(p17());
    out.push(Case {
        git: std::vec![git("src/a.rs", Shape::Del, "100644", "")],
        rank: std::vec![ranked("src/a.rs", "deleted", 0, &["deleted_file"])],
        rules: std::vec![rule("tests", Check::Companion)],
        results: std::vec![pass("tests")],
        ..from(
            p04(),
            "P18",
            "require_companion tests; deleted trigger, no companion",
        )
    });
    out.push(p19());
    out.push(Case {
        false_claim: true,
        named: std::vec!["identity_from_ranking"],
        ..from(
            p01(),
            "P20",
            "seals claim supplied provenance, producer B and a claimed version",
        )
    });

    out.push(n01());
    out.push(Case {
        rank: std::vec![ranked("src/a.rs", "modified", 0, &["secret_like"])],
        verdict: "fail",
        results: std::vec![hit("secret", "fail", signal("src/a.rs", "secret_like"))],
        ..from(n01(), "N02", "plus secret_like")
    });
    out.push(Case {
        rank: std::vec![ranked("src/a.rs", "modified", 0, &["secret_like"])],
        copy: Supply::Different,
        verdict: "fail",
        results: std::vec![hit("secret", "fail", signal("src/a.rs", "secret_like"))],
        notes: std::vec![note("rank_mismatch", None)],
        named: std::vec!["fail_dominates_note"],
        ..from(p01(), "N03", "secret_like plus a different supplied copy")
    });
    out.push(Case {
        rank: std::vec![ranked("src/a.rs", "modified", 0, &[])],
        coverage: Some(false),
        verdict: "hold",
        notes: std::vec![note("rank_incomplete", Some("docs/b.rs"))],
        named: std::vec!["missing_note"],
        ..from(p11(), "N04", "docs/b.rs removed from the ranking only")
    });
    out.push(Case {
        rank: std::vec![
            ranked("src/a.rs", "modified", 0, &[]),
            ranked("docs/b.rs", "modified", 0, &[]),
        ],
        coverage: Some(false),
        verdict: "hold",
        notes: std::vec![note("rank_extraneous", Some("docs/b.rs"))],
        named: std::vec!["extraneous_note"],
        ..from(p01(), "N05", "docs/b.rs added to the ranking only")
    });
    out.push(Case {
        rank: std::vec![ranked("src/a.rs", "modified", 0, &["new_file"])],
        coverage: Some(false),
        verdict: "hold",
        notes: std::vec![note("status_mismatch", Some("src/a.rs"))],
        named: std::vec!["status_note"],
        ..from(p03(), "N06", "ranked status added->modified, new_file kept")
    });
    out.push(Case {
        rank: std::vec![ranked("src/c.rs", "modified", 0, &[])],
        coverage: Some(false),
        verdict: "hold",
        notes: std::vec![
            note("rank_extraneous", Some("src/c.rs")),
            note("rank_incomplete", Some("src/a.rs")),
        ],
        ..from(p01(), "N07", "ranked path renamed src/a.rs->src/c.rs only")
    });
    out.push(Case {
        rank: std::vec![
            ranked("src/a.rs", "modified", 0, &[]),
            ranked("src/a.rs", "modified", 0, &[]),
        ],
        verdict: "hold",
        notes: std::vec![note("duplicate_paths", Some("src/a.rs"))],
        ..from(p01(), "N08", "src/a.rs ranked twice, identically")
    });
    out.push(Case {
        rank: Vec::new(),
        coverage: Some(false),
        verdict: "hold",
        notes: std::vec![
            note("empty_ranking", None),
            note("rank_incomplete", Some("src/a.rs")),
        ],
        ..from(p01(), "N09", "empty ranking, Git path kept")
    });
    out.push(n10());
    out.push(Case {
        rank: std::vec![ranked("src/a.rs", "modified", 0, &["secret_like"])],
        verdict: "fail",
        results: std::vec![hit("secret", "fail", signal("src/a.rs", "secret_like"))],
        named: std::vec!["fail_dominates_note"],
        ..from(n10(), "N11", "plus secret_like")
    });
    out.push(Case {
        rank: std::vec![ranked("src/a.rs", "modified", 0, &["future_z3"])],
        verdict: "hold",
        notes: std::vec![note("unknown_signal", Some("src/a.rs"))],
        ..from(p01(), "N12", "add signal future_z3")
    });
    out.push(Case {
        rank: std::vec![ranked("src/a.rs", "future_z3", 0, &[])],
        coverage: Some(false),
        verdict: "hold",
        notes: std::vec![
            note("status_mismatch", Some("src/a.rs")),
            note("unknown_status", Some("src/a.rs")),
        ],
        ..from(p01(), "N13", "status modified->future_z3")
    });
    out.push(n14());
    out.push(Case {
        rank: std::vec![ranked(
            "src/a.rs",
            "binary_changed",
            0,
            &["binary_change", "deleted_file"],
        )],
        coverage: Some(true),
        notes: Vec::new(),
        named: Vec::new(),
        id: "N14-complete",
        base: "N14",
        changes: "N14 subcase: the fully signalled binary deletion (FAIL, no note)",
        ..n14()
    });
    let mut n15_file = ranked(
        "src/a.rs",
        "binary_changed",
        0,
        &["binary_change", "secret_like"],
    );
    n15_file.evidence = std::vec![ev("secret_like", "N15")];
    out.push(Case {
        rank: std::vec![n15_file],
        rules: std::vec![
            rule("delete", Check::Forbid("deleted_file")),
            rule("secret", Check::Forbid("secret_like")),
        ],
        results: std::vec![
            hit("delete", "fail", signal("src/a.rs", "deleted_file")),
            hit(
                "secret",
                "fail",
                ViolExp {
                    evidence: std::vec![ev("secret_like", "N15")],
                    ..signal("src/a.rs", "secret_like")
                },
            ),
        ],
        named: std::vec!["secret_union"],
        ..from(
            n14(),
            "N15",
            "plus producer-only secret_like with one citation; rules delete then secret",
        )
    });
    out.push(Case {
        copy: Supply::TrailingLf,
        verdict: "hold",
        notes: std::vec![note("rank_mismatch", None)],
        base: "P02",
        ..from(p01(), "N16", "P02 supplied bytes plus one trailing LF")
    });
    out.push(Case {
        git: std::vec![
            edit("src/a.rs"),
            git("tests/a.rs", Shape::Del, "100644", ""),
        ],
        rank: std::vec![
            ranked("src/a.rs", "modified", 0, &[]),
            ranked("tests/a.rs", "deleted", 0, &["deleted_file"]),
        ],
        verdict: "fail",
        results: std::vec![hit("tests", "fail", at("src/a.rs"))],
        named: std::vec!["deleted_companion"],
        ..from(
            p17(),
            "N17",
            "tests/a.rs a text deletion, not a live addition",
        )
    });
    out.push(Case {
        git: std::vec![
            edit("src/a.rs"),
            git("tests/a.rs", Shape::BinEdit, "100644", "100644"),
        ],
        rank: std::vec![
            ranked("src/a.rs", "modified", 0, &[]),
            ranked("tests/a.rs", "binary_changed", 0, &["binary_change"]),
        ],
        verdict: "fail",
        results: std::vec![hit("tests", "fail", at("src/a.rs"))],
        ..from(p17(), "N18", "tests/a.rs a binary modification")
    });
    out.push(Case {
        today: D_PLUS_1,
        verdict: "fail",
        results: std::vec![hit("file-score", "fail", at("src/a.rs"))],
        expired: std::vec![0],
        named: std::vec!["expiry_off_by_one"],
        ..from(p16(), "N19", "date D->D+1, waiver still expires D")
    });
    out.push(raw_case(
        "N20",
        "facts refusal SymlinkTargetEdit from an existing-target triple",
        "diff --git a/src/a.rs b/src/a.rs\nindex 1111111..2222222 120000\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1 +1 @@\n-old\n\\ No newline at end of file\n+new\n\\ No newline at end of file\n",
        "symlink's target",
        "SymlinkTargetEdit",
    ));
    out.push(raw_case(
        "N21",
        "unsupported 160000 header triple",
        "diff --git a/src/a.rs b/src/a.rs\nindex 1111111..2222222 160000\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1 +1 @@\n-Subproject commit 1111111111111111111111111111111111111111\n+Subproject commit 2222222222222222222222222222222222222222\n",
        "unsupported file mode 160000",
        "UnsupportedMode(\"160000\")",
    ));
    out.push(Case {
        rank: std::vec![ranked("src/a.rs", "modified", 1, &[])],
        verdict: "fail",
        results: std::vec![hit(
            "file-score",
            "fail",
            ViolExp {
                message_contains: Some("risk score 1 exceeds the per-file limit 0"),
                ..at("src/a.rs")
            },
        )],
        ..from(p13(), "N22", "score 0->1, per-file limit 0")
    });
    out.push(Case {
        git: std::vec![edit("src/a.rs"), edit("docs/b.rs")],
        rank: std::vec![
            ranked("src/a.rs", "modified", MAX, &[]),
            ranked("docs/b.rs", "modified", 1, &[]),
        ],
        rules: std::vec![rule("sum", Check::SumScore(MAX))],
        verdict: "fail",
        results: std::vec![hit(
            "sum",
            "fail",
            aggregate("total risk score 4294967296 exceeds the change limit 4294967295"),
        )],
        named: std::vec!["sum_width"],
        ..from(
            p15(),
            "N23",
            "add docs/b.rs score 1; change-sum rule limit u32::MAX",
        )
    });
    out.push(Case {
        rank: std::vec![ranked("src/a.rs", "modified", 11, &[])],
        verdict: "fail",
        results: std::vec![hit(
            "file-score",
            "fail",
            ViolExp {
                message_contains: Some("risk score 11 exceeds the per-file limit 10"),
                ..at("src/a.rs")
            },
        )],
        named: std::vec!["strict_greater"],
        ..from(p12(), "N24", "score 10->11")
    });
    out.push(Case {
        rank: std::vec![
            ranked("src/a.rs", "modified", 6, &[]),
            ranked("docs/b.rs", "modified", 5, &[]),
        ],
        verdict: "fail",
        results: std::vec![hit(
            "sum",
            "fail",
            aggregate("total risk score 11 exceeds the change limit 10"),
        )],
        ..from(p14(), "N25", "docs/b.rs score 4->5")
    });
    out.push(Case {
        waivers: std::vec![waiver("secret", "docs/**", D_MINUS_1)],
        verdict: "fail",
        expired: std::vec![0],
        ..from(
            p01(),
            "N26",
            "unused waiver 0 for secret on docs/** expired D-1",
        )
    });
    out.push(Case {
        rules: Vec::new(),
        stage: Stage::PolicyRefused,
        coverage: None,
        verdict: "none",
        results: Vec::new(),
        ..from(
            p01(),
            "N27",
            "policy with no rules (setup refusal, not a verdict)",
        )
    });
    out.push(Case {
        admit_b: true,
        stage: Stage::AdmissionRefused,
        coverage: None,
        verdict: "none",
        results: Vec::new(),
        ..from(
            p01(),
            "N28",
            "admission asked for producer B, absent from the allowlist",
        )
    });
    out
}
