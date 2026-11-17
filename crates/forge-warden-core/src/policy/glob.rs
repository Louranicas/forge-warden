//! The policy's glob matcher (EFFECT-WALL step 2b): a port of globset 0.4.21 with the options the
//! policy used (`literal_separator(true)`, `backslash_escape(true)`, case-sensitive, no empty
//! alternates, unclosed classes refused), so that the core needs no `std` (globset has no `no_std`
//! mode). The parser is globset's, rule for rule, and refuses the same patterns with the same words;
//! matching follows the regex globset built (`(?-u)`, `.` matching every byte, anchored at both
//! ends), byte by byte: `?` and `*` take bytes other than `/`, a class is a set of bytes, a negated
//! class can take `/`. EFFECT-WALL-1's pins (`policy/tests.rs`) are the oracle.
//!
//! Matching runs a Thompson automaton over the path's bytes: linear in the path's length times the
//! pattern's size, with no backtracking, so a hostile path cannot make it slow.

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
        match self.pop_token()? {
            Some(Token::RecursivePrefix) => self.push_token(Token::RecursivePrefix),
            Some(Token::RecursiveSuffix) => self.push_token(Token::RecursiveSuffix),
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

/// globset's `add_to_last_range`: the range ends at `end`, which may not come before its start.
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

    fn insert_range(&mut self, low: u8, high: u8) {
        for byte in low..=high {
            self.insert(byte);
        }
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

/// A compiled glob: its automaton, entered at `start`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Glob {
    states: Vec<State>,
    start: usize,
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
    Ok(Glob {
        states: builder.states,
        start,
    })
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
        if low > high {
            return Err(format!("invalid range; '{start}' > '{end}'"));
        }
        for byte in before {
            set.insert(*byte);
        }
        set.insert_range(low, high);
        for byte in after {
            set.insert(*byte);
        }
    }
    Ok(set)
}

impl Glob {
    /// Whether the whole of `path` matches.
    pub(super) fn matches(&self, path: &str) -> bool {
        let mut current = Vec::new();
        let mut seen = vec![false; self.states.len()];
        self.close(self.start, &mut current, &mut seen);
        for &byte in path.as_bytes() {
            if current.is_empty() {
                return false;
            }
            let mut next = Vec::new();
            seen.fill(false);
            for &index in &current {
                if let Some(State::Byte(set, after)) = self.states.get(index)
                    && set.contains(byte)
                {
                    self.close(*after, &mut next, &mut seen);
                }
            }
            current = next;
        }
        current
            .iter()
            .any(|&index| matches!(self.states.get(index), Some(State::Match)))
    }

    /// Add `from` and every state reachable from it without taking a byte.
    fn close(&self, from: usize, out: &mut Vec<usize>, seen: &mut [bool]) {
        let mut stack = vec![from];
        while let Some(index) = stack.pop() {
            let Some(mark) = seen.get_mut(index) else {
                continue;
            };
            if *mark {
                continue;
            }
            *mark = true;
            match self.states.get(index) {
                Some(State::Split(a, b)) => {
                    stack.push(*b);
                    stack.push(*a);
                }
                Some(State::Byte(..) | State::Match) => out.push(index),
                None => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::compile;
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

    /// Patterns and paths over the characters every rule of the parser and matcher turns on.
    const PATTERN_PIECES: &str = r"(\*|\*\*|\?|\[|\]|\{|\}|,|\\|/|!|\^|-|a|b|é|\.){1,12}";
    const PATH_PIECES: &str = r"(a|b|/|é|\.|\n|-|\]|\{|,|\*){0,12}";

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(4096))]

        // The differential oracle (EFFECT-WALL step 2b, R4): the same patterns refused with the same
        // words, and for every accepted pattern the same answer on every path.
        #[test]
        fn the_matcher_agrees_with_globset(
            pattern in PATTERN_PIECES,
            paths in proptest::collection::vec(PATH_PIECES, 1..8),
        ) {
            let ours = compile(&pattern);
            match (globset(&pattern), &ours) {
                (Err(theirs), Err(mine)) => prop_assert_eq!(&theirs, mine, "{:?}", pattern),
                (Ok(set), Ok(glob)) => {
                    for path in &paths {
                        prop_assert_eq!(set.is_match(path.as_str()), glob.matches(path), "{:?} on {:?}", pattern, path);
                    }
                }
                (theirs, mine) => prop_assert!(false, "{:?}: globset {:?}, ours {:?}", pattern, theirs.err(), mine.as_ref().err()),
            }
        }
    }

    #[test]
    fn globset_and_the_matcher_agree_on_the_pinned_edges() {
        for (pattern, path) in [
            ("a/*/b", "a//b"),
            ("src/*", "src/"),
            ("*/", "a/"),
            ("src/**", "src"),
            ("**/a", "a"),
            ("**", "\n/x"),
            ("[!a]", "/"),
            ("[é]", "é"),
            ("?", "é"),
            ("{a,}", ""),
            ("{,}", ""),
            ("a{,{,}}", "a"),
        ] {
            let set = globset(pattern).unwrap();
            assert_eq!(
                set.is_match(path),
                compile(pattern).unwrap().matches(path),
                "{pattern:?} on {path:?}"
            );
        }
    }
}
