//! The policy's glob matcher (EFFECT-WALL step 2b): a port of globset 0.4.21 with the options the
//! policy used (`literal_separator(true)`, `backslash_escape(true)`, case-sensitive, no empty
//! alternates, unclosed classes refused), so that the core needs no `std` (globset has no `no_std`
//! mode). The parser is globset's, rule for rule, and refuses the same patterns with the same words;
//! matching follows the regex globset built (`(?-u)`, `.` matching every byte, anchored at both
//! ends), byte by byte: `?` and `*` take bytes other than `/`, a class is a set of bytes, a negated
//! class can take `/`. EFFECT-WALL-1's pins (`policy/tests.rs`) are the oracle.
//!
//! Matching runs a Thompson automaton over the path's bytes, as sets of states: linear in the path's
//! length times the pattern's size, with no backtracking; a path of 256 bytes or more goes through a
//! lazily built DFA. Measured on the starter's binary-text rule (three globs, about 168 alternatives),
//! release build: 0.56 us for a 43-byte path, 19 us for a 4 KiB dot-dense path and 30 us for a 4 KiB
//! pseudo-random one (globset: 0.07 us, 5 us, 5.6 us), so a 64 MiB patch of 4 KiB paths costs that rule
//! about half a second (EFFECT-WALL step 2, Bastion's SHOULD 1; the numbers are in
//! `docs/POLICY_REFERENCE.md` "Size limits").

use alloc::collections::BTreeMap;
use alloc::{borrow::ToOwned, format, string::String, vec, vec::Vec};

/// One parsed element of a glob, as globset tokenizes it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Literal(char),
    /// `?`
    Any,
    /// `*`
    ZeroOrMore,
    /// `**/` at the start (or `**` alone).
    RecursivePrefix,
    /// `/**` at the end.
    RecursiveSuffix,
    /// `/**/` in the middle.
    RecursiveZeroOrMore,
    Class {
        negated: bool,
        ranges: Vec<(char, char)>,
    },
    Alternates(Vec<Vec<Token>>),
}

/// globset's parser (glob.rs `Parser`), with `literal_separator` and `backslash_escape` on and
/// `allow_unclosed_class` off.
struct Parser<'a> {
    chars: core::iter::Peekable<core::str::Chars<'a>>,
    alternates_stack: Vec<usize>,
    branches: Vec<Vec<Token>>,
    prev: Option<char>,
    cur: Option<char>,
}

/// globset's messages, word for word (`ErrorKind::description` and its `Display`).
const UNCLOSED_CLASS: &str = "unclosed character class; missing ']'";
const UNOPENED_ALTERNATES: &str =
    "unopened alternate group; missing '{' (maybe escape '}' with '[}]'?)";
const UNCLOSED_ALTERNATES: &str =
    "unclosed alternate group; missing '}' (maybe escape '{' with '[{]'?)";
const DANGLING_ESCAPE: &str = "dangling '\\'";

const fn is_separator(c: char) -> bool {
    c == '/'
}

impl Parser<'_> {
    fn bump(&mut self) -> Option<char> {
        self.prev = self.cur;
        self.cur = self.chars.next();
        self.cur
    }

    fn peek(&mut self) -> Option<char> {
        self.chars.peek().copied()
    }

    fn parse(&mut self) -> Result<(), String> {
        while let Some(c) = self.bump() {
            match c {
                '?' => self.push_token(Token::Any)?,
                '*' => self.parse_star()?,
                '[' => self.parse_class()?,
                '{' => {
                    self.alternates_stack.push(self.branches.len());
                    self.branches.push(Vec::new());
                }
                '}' => self.pop_alternate()?,
                ',' => {
                    if self.alternates_stack.is_empty() {
                        self.push_token(Token::Literal(','))?;
                    } else {
                        self.branches.push(Vec::new());
                    }
                }
                '\\' => match self.bump() {
                    None => return Err(DANGLING_ESCAPE.to_owned()),
                    Some(escaped) => self.push_token(Token::Literal(escaped))?,
                },
                other => self.push_token(Token::Literal(other))?,
            }
        }
        Ok(())
    }

    fn pop_alternate(&mut self) -> Result<(), String> {
        let Some(start) = self.alternates_stack.pop() else {
            return Err(UNOPENED_ALTERNATES.to_owned());
        };
        let alternates = Token::Alternates(self.branches.split_off(start));
        self.push_token(alternates)
    }

    fn push_token(&mut self, token: Token) -> Result<(), String> {
        match self.branches.last_mut() {
            Some(branch) => {
                branch.push(token);
                Ok(())
            }
            None => Err(UNOPENED_ALTERNATES.to_owned()),
        }
    }

    fn pop_token(&mut self) -> Result<Option<Token>, String> {
        match self.branches.last_mut() {
            Some(branch) => Ok(branch.pop()),
            None => Err(UNOPENED_ALTERNATES.to_owned()),
        }
    }

    fn have_tokens(&self) -> Result<bool, String> {
        match self.branches.last() {
            Some(branch) => Ok(!branch.is_empty()),
            None => Err(UNOPENED_ALTERNATES.to_owned()),
        }
    }

    /// Two stars that are not a recursive form are two `*`, exactly as globset pushes them.
    fn push_two_stars(&mut self) -> Result<(), String> {
        self.push_token(Token::ZeroOrMore)?;
        self.push_token(Token::ZeroOrMore)
    }

    fn parse_star(&mut self) -> Result<(), String> {
        let prev = self.prev;
        if self.peek() != Some('*') {
            return self.push_token(Token::ZeroOrMore);
        }
        self.bump();
        if !self.have_tokens()? {
            if self.peek().is_some_and(|c| !is_separator(c)) {
                return self.push_two_stars();
            }
            self.push_token(Token::RecursivePrefix)?;
            self.bump();
            return Ok(());
        }
        if !prev.is_some_and(is_separator)
            && (self.branches.len() <= 1 || (prev != Some(',') && prev != Some('{')))
        {
            return self.push_two_stars();
        }
        let is_suffix = match self.peek() {
            None => {
                self.bump();
                true
            }
            Some(',' | '}') if self.branches.len() >= 2 => true,
            Some(c) if is_separator(c) => {
                self.bump();
                false
            }
            Some(_) => return self.push_two_stars(),
        };
        // globset also keeps a popped `RecursiveSuffix`; that arm is left out because no token can
        // follow one in its list: a suffix is pushed only at the end of the pattern, or just before
        // the `,` or `}` that ends its branch, so no later `**` pops it.
        match self.pop_token()? {
            Some(Token::RecursivePrefix) => self.push_token(Token::RecursivePrefix),
            _ if is_suffix => self.push_token(Token::RecursiveSuffix),
            _ => self.push_token(Token::RecursiveZeroOrMore),
        }
    }

    fn parse_class(&mut self) -> Result<(), String> {
        let mut ranges: Vec<(char, char)> = Vec::new();
        let negated = matches!(self.peek(), Some('!' | '^'));
        if negated {
            self.bump();
        }
        let mut first = true;
        let mut in_range = false;
        loop {
            let Some(c) = self.bump() else {
                return Err(UNCLOSED_CLASS.to_owned());
            };
            match c {
                ']' if first => ranges.push((']', ']')),
                ']' => break,
                '-' if first => ranges.push(('-', '-')),
                '-' if in_range => {
                    extend_last_range(&mut ranges, '-')?;
                    in_range = false;
                }
                '-' => in_range = true,
                other => {
                    if in_range {
                        extend_last_range(&mut ranges, other)?;
                    } else {
                        ranges.push((other, other));
                    }
                    in_range = false;
                }
            }
            first = false;
        }
        if in_range {
            ranges.push(('-', '-'));
        }
        self.push_token(Token::Class { negated, ranges })
    }
}

/// globset's range extension: the range ends at `end`, which may not come before its start.
fn extend_last_range(ranges: &mut [(char, char)], end: char) -> Result<(), String> {
    let Some(last) = ranges.last_mut() else {
        return Err(UNCLOSED_CLASS.to_owned());
    };
    last.1 = end;
    if last.1 < last.0 {
        return Err(format!("invalid range; '{}' > '{}'", last.0, last.1));
    }
    Ok(())
}

/// A set of bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Bytes {
    low: u128,
    high: u128,
}

impl Bytes {
    const NONE: Self = Self { low: 0, high: 0 };
    const ALL: Self = Self {
        low: u128::MAX,
        high: u128::MAX,
    };

    fn one(byte: u8) -> Self {
        let mut set = Self::NONE;
        set.insert(byte);
        set
    }

    fn insert(&mut self, byte: u8) {
        let bit = 1_u128.wrapping_shl(u32::from(byte & 0x7f));
        if byte & 0x80 == 0 {
            self.low |= bit;
        } else {
            self.high |= bit;
        }
    }

    /// Insert `low..=high`; `false`, inserting nothing, when the range is inverted (`low > high`).
    #[must_use]
    fn insert_range(&mut self, low: u8, high: u8) -> bool {
        if low > high {
            return false;
        }
        for byte in low..=high {
            self.insert(byte);
        }
        true
    }

    fn contains(self, byte: u8) -> bool {
        let bit = 1_u128.wrapping_shl(u32::from(byte & 0x7f));
        let half = if byte & 0x80 == 0 {
            self.low
        } else {
            self.high
        };
        half & bit != 0
    }

    const fn complement(self) -> Self {
        Self {
            low: !self.low,
            high: !self.high,
        }
    }

    /// Every byte but `/`: what `?` and `*` take under `literal_separator`.
    fn not_slash() -> Self {
        let mut set = Self::ALL;
        let slash = Self::one(b'/');
        set.low &= !slash.low;
        set
    }
}

/// One automaton state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Take one byte in the set, then go on.
    Byte(Bytes, usize),
    /// Go on along either branch without taking a byte.
    Split(usize, usize),
    Match,
}

/// A compiled glob: its automaton as sets of states (bitsets of `words` words), precomputed so that
/// one byte costs a few word operations plus the closures of the states that take it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Glob {
    words: usize,
    /// The states reached before any byte.
    start: Vec<u64>,
    /// For each byte value, the states that take it.
    takes: Vec<Vec<u64>>,
    /// For each state that takes a byte, the states reached right after it (its successor's closure).
    after: Vec<Vec<usize>>,
    /// The accepting states.
    accepting: Vec<u64>,
}

/// Parse `pattern` and build its automaton, or say why the pattern is refused.
pub(super) fn compile(pattern: &str) -> Result<Glob, String> {
    let mut parser = Parser {
        chars: pattern.chars().peekable(),
        alternates_stack: Vec::new(),
        branches: vec![Vec::new()],
        prev: None,
        cur: None,
    };
    parser.parse()?;
    if parser.branches.len() > 1 {
        return Err(UNCLOSED_ALTERNATES.to_owned());
    }
    let tokens = parser
        .branches
        .pop()
        .ok_or_else(|| UNOPENED_ALTERNATES.to_owned())?;
    let mut builder = Builder {
        states: vec![State::Match],
    };
    let start = if tokens == [Token::RecursivePrefix] {
        // globset's special case: `**` alone matches everything.
        builder.repeat(Bytes::ALL, 0)
    } else {
        builder.sequence(&tokens, 0)?
    };
    Ok(Glob::new(&builder.states, start))
}

/// Builds an automaton back to front: each piece is compiled with the state that follows it.
struct Builder {
    states: Vec<State>,
}

impl Builder {
    fn push(&mut self, state: State) -> usize {
        let index = self.states.len();
        self.states.push(state);
        index
    }

    /// `set*` then `next`.
    fn repeat(&mut self, set: Bytes, next: usize) -> usize {
        let split = self.states.len();
        let byte = split.saturating_add(1);
        self.push(State::Split(byte, next));
        self.push(State::Byte(set, split));
        split
    }

    fn sequence(&mut self, tokens: &[Token], next: usize) -> Result<usize, String> {
        let mut start = next;
        for token in tokens.iter().rev() {
            start = self.token(token, start)?;
        }
        Ok(start)
    }

    fn literal(&mut self, text: &[u8], next: usize) -> usize {
        text.iter().rev().fold(next, |after, byte| {
            self.push(State::Byte(Bytes::one(*byte), after))
        })
    }

    fn token(&mut self, token: &Token, next: usize) -> Result<usize, String> {
        Ok(match token {
            Token::Literal(c) => {
                let mut buffer = [0_u8; 4];
                self.literal(c.encode_utf8(&mut buffer).as_bytes(), next)
            }
            Token::Any => self.push(State::Byte(Bytes::not_slash(), next)),
            Token::ZeroOrMore => self.repeat(Bytes::not_slash(), next),
            // `(?:/?|.*/)`
            Token::RecursivePrefix => {
                let slash = self.push(State::Byte(Bytes::one(b'/'), next));
                let slash_after_any = self.push(State::Byte(Bytes::one(b'/'), next));
                let any = self.repeat(Bytes::ALL, slash_after_any);
                let either = self.push(State::Split(slash, any));
                self.push(State::Split(next, either))
            }
            // `/.*`
            Token::RecursiveSuffix => {
                let any = self.repeat(Bytes::ALL, next);
                self.push(State::Byte(Bytes::one(b'/'), any))
            }
            // `(?:/|/.*/)`
            Token::RecursiveZeroOrMore => {
                let one = self.push(State::Byte(Bytes::one(b'/'), next));
                let closing = self.push(State::Byte(Bytes::one(b'/'), next));
                let any = self.repeat(Bytes::ALL, closing);
                let opening = self.push(State::Byte(Bytes::one(b'/'), any));
                self.push(State::Split(one, opening))
            }
            Token::Class { negated, ranges } => {
                let set = class_bytes(ranges)?;
                let set = if *negated { set.complement() } else { set };
                self.push(State::Byte(set, next))
            }
            Token::Alternates(branches) => {
                let mut starts = Vec::new();
                for branch in branches.iter().filter(|branch| !produces_nothing(branch)) {
                    starts.push(self.sequence(branch, next)?);
                }
                // Every branch empty: globset writes no group at all, so nothing is required here.
                let Some((&last, rest)) = starts.split_last() else {
                    return Ok(next);
                };
                rest.iter().rev().fold(last, |others, &branch| {
                    self.push(State::Split(branch, others))
                })
            }
        })
    }
}

/// Whether `tokens` write nothing into globset's regex (only alternates whose branches write
/// nothing): such a branch is dropped, as globset drops an empty alternate.
fn produces_nothing(tokens: &[Token]) -> bool {
    tokens.iter().all(|token| match token {
        Token::Alternates(branches) => branches.iter().all(|branch| produces_nothing(branch)),
        Token::Literal(_)
        | Token::Any
        | Token::ZeroOrMore
        | Token::RecursivePrefix
        | Token::RecursiveSuffix
        | Token::RecursiveZeroOrMore
        | Token::Class { .. } => false,
    })
}

/// The byte set globset's class means. It writes each range's ends as escaped UTF-8 bytes into a
/// byte-level regex class (`[a-\xc3\xa9]`), where a `-` makes a range of the bytes on either side
/// of it, so the class is read here the same way.
fn class_bytes(ranges: &[(char, char)]) -> Result<Bytes, String> {
    let mut set = Bytes::NONE;
    for &(start, end) in ranges {
        let mut first = [0_u8; 4];
        let first = start.encode_utf8(&mut first).as_bytes();
        if start == end {
            for byte in first {
                set.insert(*byte);
            }
            continue;
        }
        let mut second = [0_u8; 4];
        let second = end.encode_utf8(&mut second).as_bytes();
        let (Some((&low, before)), Some((&high, after))) =
            (first.split_last(), second.split_first())
        else {
            return Err(format!("invalid range; '{start}' > '{end}'"));
        };
        for byte in before {
            set.insert(*byte);
        }
        // Fail-closed, though the parser already refused `start > end` (then the first end's last byte
        // is always at or below the second end's first byte): `insert_range` refuses an inverted range,
        // and its own test pins that comparison.
        if !set.insert_range(low, high) {
            return Err(format!("invalid range; '{start}' > '{end}'"));
        }
        for byte in after {
            set.insert(*byte);
        }
    }
    Ok(set)
}

/// Set bit `index` of a state set.
fn set_bit(set: &mut [u64], index: usize) {
    let (Some(word), Ok(shift)) = (set.get_mut(index / 64), u32::try_from(index & 63)) else {
        return;
    };
    *word |= 1_u64.wrapping_shl(shift);
}

/// `from` and every state reachable from it without taking a byte, keeping the states that take a
/// byte or accept (a Split only passes on).
fn closure(states: &[State], from: usize) -> Vec<usize> {
    let mut seen = vec![false; states.len()];
    let mut out = Vec::new();
    let mut stack = vec![from];
    while let Some(index) = stack.pop() {
        let Some(mark) = seen.get_mut(index) else {
            continue;
        };
        if *mark {
            continue;
        }
        *mark = true;
        match states.get(index) {
            Some(State::Split(a, b)) => {
                stack.push(*b);
                stack.push(*a);
            }
            Some(State::Byte(..) | State::Match) => out.push(index),
            None => {}
        }
    }
    out
}

impl Glob {
    fn new(states: &[State], start: usize) -> Self {
        let words = states.len().div_ceil(64);
        let mut start_set = vec![0; words];
        for index in closure(states, start) {
            set_bit(&mut start_set, index);
        }
        let mut takes = vec![vec![0; words]; 256];
        let mut after = vec![Vec::new(); states.len()];
        let mut accepting = vec![0; words];
        for (index, state) in states.iter().enumerate() {
            match state {
                State::Byte(set, next) => {
                    for (byte, members) in (0..=u8::MAX).zip(takes.iter_mut()) {
                        if set.contains(byte) {
                            set_bit(members, index);
                        }
                    }
                    if let Some(slot) = after.get_mut(index) {
                        *slot = closure(states, *next);
                    }
                }
                State::Match => set_bit(&mut accepting, index),
                State::Split(..) => {}
            }
        }
        Self {
            words,
            start: start_set,
            takes,
            after,
            accepting,
        }
    }

    /// Whether the whole of `path` matches: linear in the path's length times the pattern's size.
    /// A path of `CACHED_FROM` bytes or more is matched through a lazily built DFA (each state set
    /// met is numbered, and its transitions are kept), so a long, repetitive path costs a table
    /// lookup per byte; a shorter one steps the state sets directly.
    pub(super) fn matches(&self, path: &str) -> bool {
        if uses_the_dfa(path.len()) {
            self.matches_cached(path.as_bytes())
        } else {
            self.matches_from(self.start.clone(), path.as_bytes())
        }
    }

    /// Step the state set `current` over `bytes`, then whether it accepts.
    fn matches_from(&self, mut current: Vec<u64>, bytes: &[u8]) -> bool {
        let mut next = vec![0_u64; self.words];
        for &byte in bytes {
            if !self.step(&current, byte, &mut next) {
                return false;
            }
            core::mem::swap(&mut current, &mut next);
        }
        self.accepts(&current)
    }

    /// The states reached from `current` by `byte`, into `next`; whether there are any.
    fn step(&self, current: &[u64], byte: u8, next: &mut [u64]) -> bool {
        next.fill(0);
        let Some(members) = self.takes.get(usize::from(byte)) else {
            return false;
        };
        let mut any = false;
        for (word_index, (word, member)) in current.iter().zip(members).enumerate() {
            let mut taking = word & member;
            // Bounded by the bits set, so no edit of the clearing step can loop forever.
            for _ in 0..taking.count_ones() {
                let bit = taking.trailing_zeros();
                taking &= taking.wrapping_sub(1);
                let Ok(bit) = usize::try_from(bit) else {
                    continue;
                };
                let state = word_index.saturating_mul(64).saturating_add(bit);
                if let Some(reached) = self.after.get(state) {
                    for &target in reached {
                        set_bit(next, target);
                        any = true;
                    }
                }
            }
        }
        any
    }

    fn accepts(&self, set: &[u64]) -> bool {
        set.iter()
            .zip(&self.accepting)
            .any(|(word, accepting)| word & accepting != 0)
    }

    /// The lazy DFA: state sets numbered as they are met, 256 transitions per number (`UNKNOWN` until
    /// computed). At `CACHED_SETS` numbered sets it stops numbering and
    /// steps the rest of the path directly, so a path built to meet many sets costs no more memory.
    fn matches_cached(&self, bytes: &[u8]) -> bool {
        self.matches_cached_with(bytes, CACHED_SETS).0
    }

    /// [`Glob::matches_cached`] with a cap of `cap` numbered sets (tests use small caps), and how
    /// many state steps it computed (the rest were table lookups).
    fn matches_cached_with(&self, bytes: &[u8], cap: usize) -> (bool, usize) {
        let mut sets: Vec<Vec<u64>> = vec![self.start.clone()];
        let mut numbers: BTreeMap<Vec<u64>, u32> = BTreeMap::new();
        numbers.insert(self.start.clone(), 0);
        let mut table: Vec<u32> = vec![UNKNOWN; 256];
        let mut current: usize = 0;
        let mut next = vec![0_u64; self.words];
        let mut steps: usize = 0;
        for (position, &byte) in bytes.iter().enumerate() {
            let slot = current
                .saturating_mul(256)
                .saturating_add(usize::from(byte));
            if let Some(&known) = table.get(slot)
                && known != UNKNOWN
            {
                let Ok(known) = usize::try_from(known) else {
                    return (false, steps);
                };
                current = known;
                continue;
            }
            let Some(set) = sets.get(current) else {
                return (false, steps);
            };
            steps = steps.saturating_add(1);
            if !self.step(set, byte, &mut next) {
                // Nothing reached: no later byte can match, so the transition is not kept.
                return (false, steps);
            }
            let number = if let Some(&known) = numbers.get(&next) {
                known
            } else {
                let (Ok(number), true) = (u32::try_from(sets.len()), sets.len() < cap) else {
                    // The cache is full: finish by stepping from this set.
                    let Some(rest) = bytes.get(position.saturating_add(1)..) else {
                        return (self.accepts(&next), steps);
                    };
                    return (self.matches_from(next, rest), steps);
                };
                sets.push(next.clone());
                numbers.insert(next.clone(), number);
                table.extend(core::iter::repeat_n(UNKNOWN, 256));
                number
            };
            if let Some(entry) = table.get_mut(slot) {
                *entry = number;
            }
            let Ok(number) = usize::try_from(number) else {
                return (false, steps);
            };
            current = number;
        }
        (
            sets.get(current).is_some_and(|set| self.accepts(set)),
            steps,
        )
    }
}

/// Paths of at least this many bytes are matched through the lazy DFA.
const CACHED_FROM: usize = 256;

/// Whether a path of `len` bytes is matched through the lazy DFA (`CACHED_FROM` bytes or more).
const fn uses_the_dfa(len: usize) -> bool {
    len >= CACHED_FROM
}
/// The lazy DFA numbers at most this many state sets per path.
const CACHED_SETS: usize = 1024;
/// A transition not computed yet.
const UNKNOWN: u32 = u32::MAX;

#[cfg(test)]
mod tests {
    use super::{Bytes, CACHED_FROM, compile, uses_the_dfa};
    use alloc::{string::String, string::ToString};
    use proptest::prelude::*;

    /// globset 0.4.21 as the policy built it until EFFECT-WALL step 2b: the parse error's words, or
    /// a one-glob set (built the way `paths::compile` built it).
    fn globset(pattern: &str) -> Result<globset::GlobSet, String> {
        let glob = globset::GlobBuilder::new(pattern)
            .literal_separator(true)
            .backslash_escape(true)
            .build()
            .map_err(|e| e.kind().to_string())?;
        let mut builder = globset::GlobSetBuilder::new();
        builder.add(glob);
        builder.build().map_err(|e| e.to_string())
    }

    /// Patterns over every character the parser turns on, plus whole pieces that random characters
    /// seldom form: ranges (also across non-ASCII ends), negations of each spelling, a leading `]`
    /// or `-`, a trailing `-`, and `**` beside commas, braces and slashes inside alternates (the
    /// shapes mutation testing found unexercised by single characters alone).
    const PATTERN_PIECES: &str = concat!(
        r"(\*|\*\*|\?|\[|\]|\{|\}|,|\\|/|!|\^|-|a|b|c|é|\.",
        r"|\[a-c\]|\[!a-c\]|\[\^b\]|\[!é\]|\[\]a\]|\[-a\]|\[a-\]|\[é-ë\]|\[a-é\]|\[!a-é\]|\[b-a\]",
        r"|\{a,b\}|\{,a\}|\{a,\}|\{\*\*,a\}|\{a,\*\*\}|\{a/\*\*,b\}|\{\*\*/a,b\}|\{a,\{b,c\}\}|\*\*/|/\*\*|/\*\*/){1,10}"
    );
    const PATH_PIECES: &str = r"(a|b|c|d|/|é|ê|ë|ÿ|\.|\n|-|\]|\{|,|\*){0,12}";

    /// The differential oracle (EFFECT-WALL step 2b, R4): the same patterns refused with the same
    /// words, and for every accepted pattern the same answer on every path. 8000 cases; the runner is
    /// configured in code, not in an attribute (Q37: attributes hold literals only).
    #[test]
    fn the_matcher_agrees_with_globset() {
        let mut runner = proptest::test_runner::TestRunner::new(ProptestConfig::with_cases(8000));
        let cases = (PATTERN_PIECES, proptest::collection::vec(PATH_PIECES, 1..8));
        let outcome = runner.run(&cases, |(pattern, paths)| {
            let ours = compile(&pattern);
            match (globset(&pattern), &ours) {
                (Err(theirs), Err(mine)) => prop_assert_eq!(&theirs, mine, "{:?}", pattern),
                (Ok(set), Ok(glob)) => {
                    for path in &paths {
                        prop_assert_eq!(
                            set.is_match(path.as_str()),
                            glob.matches(path),
                            "{:?} on {:?}",
                            pattern,
                            path
                        );
                    }
                }
                (theirs, mine) => prop_assert!(
                    false,
                    "{:?}: globset {:?}, ours {:?}",
                    pattern,
                    theirs.err(),
                    mine.as_ref().err()
                ),
            }
            Ok(())
        });
        if let Err(failure) = outcome {
            panic!("{failure}");
        }
    }

    /// The lazy DFA (paths of `CACHED_FROM` bytes or more) against globset, and against plain
    /// stepping and the DFA with tiny caps (so the cache-full fallback runs), on long paths made by
    /// repeating generated pieces.
    #[test]
    fn the_cached_matcher_agrees_on_long_paths() {
        let mut runner = proptest::test_runner::TestRunner::new(ProptestConfig::with_cases(2000));
        let cases = (PATTERN_PIECES, proptest::collection::vec(PATH_PIECES, 1..4));
        let outcome = runner.run(&cases, |(pattern, pieces)| {
            let (Ok(set), Ok(glob)) = (globset(&pattern), compile(&pattern)) else {
                return Ok(());
            };
            for piece in &pieces {
                let mut path = String::new();
                while path.len() < CACHED_FROM.saturating_add(44) {
                    path.push_str(piece);
                    path.push('a');
                }
                let want = set.is_match(path.as_str());
                let bytes = path.as_bytes();
                prop_assert_eq!(want, glob.matches(&path), "{:?} on {:?}", pattern, path);
                prop_assert_eq!(
                    want,
                    glob.matches_from(glob.start.clone(), bytes),
                    "stepping"
                );
                prop_assert_eq!(want, glob.matches_cached_with(bytes, 1).0, "cap 1");
                prop_assert_eq!(want, glob.matches_cached_with(bytes, 3).0, "cap 3");
            }
            Ok(())
        });
        if let Err(failure) = outcome {
            panic!("{failure}");
        }
    }

    #[test]
    fn paths_of_cached_from_bytes_or_more_go_through_the_dfa() {
        assert!(!uses_the_dfa(0));
        assert!(!uses_the_dfa(CACHED_FROM.saturating_sub(1)));
        assert!(uses_the_dfa(CACHED_FROM));
        assert!(uses_the_dfa(CACHED_FROM.saturating_add(1)));
    }

    #[test]
    fn an_inverted_byte_range_is_refused_and_inserts_nothing() {
        let mut set = Bytes::NONE;
        assert!(set.insert_range(b'a', b'a'));
        assert!(set.contains(b'a'));
        assert!(set.insert_range(0x80, 0xbf));
        assert!(set.contains(0x80) && set.contains(0xbf) && !set.contains(0xc0));
        let mut inverted = Bytes::NONE;
        assert!(!inverted.insert_range(b'b', b'a'));
        assert!(!inverted.contains(b'a') && !inverted.contains(b'b'));
    }

    /// The DFA keeps its transitions: on a long, repetitive path it computes a handful of state steps
    /// and looks the rest up; with a cap of one numbered set it stops numbering after the first step.
    /// (Which mode `matches` picks changes the cost only; `uses_the_dfa`'s own test pins the
    /// threshold, Train B12r.)
    #[test]
    fn the_dfa_reuses_its_transitions() {
        let glob = compile("**/*.{rs,toml}").unwrap();
        let path = "ab/".repeat(333) + "x.rs";
        let (matched, steps) = glob.matches_cached_with(path.as_bytes(), super::CACHED_SETS);
        assert!(matched);
        assert!(
            steps > 1 && steps < 16,
            "{steps} steps computed for {} bytes (more than one: the DFA numbers its sets; few: it reuses them)",
            path.len()
        );
        let (capped_match, capped_steps) = glob.matches_cached_with(path.as_bytes(), 1);
        assert!(capped_match);
        assert_eq!(
            capped_steps, 1,
            "a cap of one numbered set stops numbering after the first step"
        );
    }

    #[test]
    fn globset_and_the_matcher_agree_on_the_pinned_edges() {
        // Every hard pattern against every hard path, each answer taken from globset itself: the
        // shapes in-diff mutation testing showed random generation reaching too rarely (`/**,` outside
        // an alternate group, `**` beside commas and braces inside one, recursive forms that follow
        // each other, classes with ranges and negations over non-ASCII ends).
        // An escaped `,` or `{` inside an alternate is the one way `**` meets a previous character of
        // `,` or `{` in a branch that already has tokens (Train B12r: the `&&` of parse_star's first
        // test was thought equivalent; `{a\,**}` tells the two apart).
        let patterns = [
            "{a\\,**}",
            "{a\\{**}",
            "{a\\,**,b}",
            "{b,a\\,**}",
            "a\\,**",
            "a/*/b",
            "src/*",
            "*/",
            "src/**",
            "**/a",
            "**",
            "[!a]",
            "[é]",
            "?",
            "{a,}",
            "{,}",
            "a{,{,}}",
            "a/**,b",
            "a/**}",
            "{a/**,b}",
            "{a,**}",
            "{**,a}",
            "{a,**/b}",
            "{**/a,b}",
            "**/**",
            "a/**/**",
            "a/**/**/b",
            "**/**/a",
            "{a/**}",
            "{a/**/b,c}",
            "{,**}",
            "{**}",
            "a**",
            "**a",
            "a/**b",
            "[a-c]",
            "[!a-c]",
            "[^b]",
            "[]a]",
            "[-a]",
            "[a-]",
            "[é-ë]",
            "[a-é]",
            "[!a-é]",
            "[!é]",
            "*,*",
            "a,b",
            "\\*",
            "\\{a}",
            "a,**",
            "{ab**}",
            "{a,b**}",
            "{ab**,c}",
            "[+--]",
            "[!+--]",
            "[a-a]",
        ];
        let paths = [
            "", "a", "b", "c", "d", "/", "a/", "a//b", "a/b", "a/x/b", "a/x/y,b", "a/**,b",
            "a/b/c", "b/a", "src", "src/", "src/x", "src/x/y", "x/a", "x/y/a", "\n/x", "é", "ê",
            "ë", "ÿ", "]", "-", "a,b", "{a}", "*", "**", "ab", "ba", "aa", ",", "+", "a,x",
            "a,x/y", "axy", "a/xy", "bxy", "b/x", "a,", "a,/x", "a{", "a{x", "a{x/y", "a{/x",
        ];
        for pattern in patterns {
            let ours = compile(pattern);
            match globset(pattern) {
                Err(theirs) => assert_eq!(Err(theirs), ours.map(|_| ()), "{pattern:?}"),
                Ok(set) => {
                    let glob = ours.unwrap();
                    for path in paths {
                        assert_eq!(
                            set.is_match(path),
                            glob.matches(path),
                            "{pattern:?} on {path:?}"
                        );
                    }
                }
            }
        }
    }
}
