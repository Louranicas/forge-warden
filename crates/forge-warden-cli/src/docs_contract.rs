//! Executable front-door examples are parsed by the actual clap grammar, without running them.
use super::Cli;
use clap::Parser as _;
use forge_warden_core::defaults::NamedDefault as _;

fn document(path: &str) -> String {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    String::from_utf8(super::doors::read_file(&root.join(path)).unwrap()).unwrap()
}

// A deliberately limited shell-example lexer: quoting, escaping, operators and comments. Shell
// expansions remain literal argument values; nothing is evaluated. Unclosed quotes/escapes and
// YAML block commands are refused. This does not validate general shell program semantics.
#[derive(Debug, PartialEq)]
enum Token {
    Word(String),
    Separator(char),
}

fn words(source: &str) -> Result<Vec<Token>, String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut active = false;
    let mut quote = None;
    let mut escaped = false;
    for c in source.chars() {
        if escaped {
            active = true;
            word.push(c);
            escaped = false;
        } else if c == '\\' && quote != Some('\'') {
            active = true;
            escaped = true;
        } else if Some(c) == quote {
            quote = None;
        } else if quote.is_some() {
            word.push(c);
        } else if c == '\'' || c == '"' {
            active = true;
            quote = Some(c);
        } else if c == '#' && !active {
            break;
        } else if c.is_whitespace() || "|;&<>".contains(c) {
            if active {
                out.push(Token::Word(std::mem::take(&mut word)));
                active = false;
            }
            if !c.is_whitespace() {
                out.push(Token::Separator(c));
            }
        } else {
            active = true;
            word.push(c);
        }
    }
    if quote.is_some() || escaped {
        return Err("unclosed quote or escape in executable documentation".to_owned());
    }
    if active {
        out.push(Token::Word(word));
    }
    Ok(out)
}

fn shell_lines(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut continued = String::new();
    for line in source.lines() {
        if let Some(part) = line.strip_suffix('\\') {
            continued.push_str(part);
            continued.push(' ');
        } else {
            continued.push_str(line);
            out.push(std::mem::take(&mut continued));
        }
    }
    assert!(continued.is_empty(), "unterminated shell continuation");
    out
}

fn block_examples(language: &str, body: &str) -> Vec<String> {
    match language {
        "bash" | "sh" | "shell" | "console" => shell_lines(body),
        "text" | "toml" | "json" => Vec::new(),
        "yaml" => {
            let mut out = Vec::new();
            for line in body.lines() {
                let trimmed = line.trim_start().trim_start_matches("- ");
                if let Some(run) = trimmed.strip_prefix("run:") {
                    assert!(
                        !run.trim_start()
                            .starts_with(['\'', '"', '!', '&', '*', '{', '[']),
                        "unsupported YAML run scalar"
                    );
                    assert!(
                        !run.trim_start().starts_with(['|', '>']),
                        "extend the parser before adding YAML block commands"
                    );
                    out.push(run.to_owned());
                } else {
                    assert!(
                        trimmed.is_empty()
                            || trimmed.starts_with('#')
                            || (trimmed.starts_with("env: {") && trimmed.ends_with('}')),
                        "unsupported YAML example line: {line}"
                    );
                }
            }
            out
        }
        _ => {
            panic!("unsupported command-bearing fence language {language:?}; label data explicitly")
        }
    }
}

// The admitted Markdown subset: repeated list/quote containers, then fence or indented code.
// Keep indentation evidence before trimming each container, including after a quote marker.
fn container_content(mut line: &str) -> (&str, bool) {
    let mut indented = false;
    loop {
        let columns =
            line.chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .fold(0_usize, |column, c| {
                    column.saturating_add(if c == '\t' {
                        4_usize.saturating_sub(column % 4)
                    } else {
                        1
                    })
                });
        indented |= columns >= 4;
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix('>') {
            line = rest.strip_prefix(' ').or_display(rest);
            continue;
        }
        if let Some(rest) = ['-', '*', '+']
            .iter()
            .find_map(|marker| trimmed.strip_prefix(*marker))
            .filter(|rest| rest.is_empty() || rest.starts_with([' ', '\t']))
        {
            line = rest.strip_prefix(' ').or_display(rest);
            continue;
        }
        let digits = trimmed.bytes().take_while(u8::is_ascii_digit).count();
        if (1..=9).contains(&digits) {
            let after = trimmed.get(digits..).unwrap();
            if let Some(rest) = after
                .strip_prefix('.')
                .or_else(|| after.strip_prefix(')'))
                .filter(|rest| rest.is_empty() || rest.starts_with([' ', '\t']))
            {
                line = rest.strip_prefix(' ').or_display(rest);
                continue;
            }
        }
        return (trimmed, indented);
    }
}

// Only matched, equal-length backtick runs delimit inline code. Unmatched ticks are literal:
// they must not hide a raw HTML code opener. This deliberately scans per line and may refuse
// unsupported multiline inline spans rather than silently exempting subsequent HTML.
fn without_inline_code(mut line: &str) -> String {
    // Escaped backticks need a larger inline grammar; conservatively exempt nothing.
    if line.contains(r"\`") {
        return line.to_owned();
    }
    let mut out = String::new();
    while let Some(start) = line.find('`') {
        out.push_str(line.get(..start).unwrap());
        let after = line.get(start..).unwrap();
        let width = after.bytes().take_while(|b| *b == b'`').count();
        let rest = after.get(width..).unwrap();
        let mut cursor = rest;
        let mut closing = None;
        while let Some(next) = cursor.find('`') {
            let run = cursor.get(next..).unwrap();
            let length = run.bytes().take_while(|b| *b == b'`').count();
            if length == width {
                closing = Some(run.get(length..).unwrap());
                break;
            }
            cursor = run.get(length..).unwrap();
        }
        if let Some(tail) = closing {
            line = tail;
        } else {
            out.push_str(after);
            return out;
        }
    }
    out.push_str(line);
    out
}

fn raw_code_tag(line: &str) -> bool {
    let plain = without_inline_code(line).to_ascii_lowercase();
    ["pre", "xmp", "listing", "code", "samp", "kbd"]
        .iter()
        .any(|name| {
            let opening = format!("<{name}");
            plain.match_indices(&opening).any(|(start, _)| {
                plain
                    .get(start.saturating_add(opening.len())..)
                    .unwrap()
                    .chars()
                    .next()
                    .is_none_or(|c| c.is_whitespace() || c == '>' || c == '/')
            })
        })
}

fn fence_start(line: &str) -> Option<(char, usize, &str)> {
    let (trimmed, _) = container_content(line);
    let marker = trimmed.chars().next()?;
    if !['`', '~'].contains(&marker) {
        return None;
    }
    let length = trimmed.chars().take_while(|c| *c == marker).count();
    if length < 3 {
        return None;
    }
    Some((marker, length, trimmed.get(length..)?.trim()))
}

fn examples(readme: &str, justfile: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut fence: Option<(char, usize, &str)> = None;
    let mut body = String::new();
    let mut indented_body = String::new();
    for line in readme.lines() {
        let (content, indented) = container_content(line);
        if fence.is_none() {
            assert!(
                !raw_code_tag(line),
                "raw HTML code requires an explicit language fence: {line}"
            );
            if !indented && !indented_body.is_empty() {
                out.extend(shell_lines(&indented_body));
                indented_body.clear();
            }
        }
        if let Some((marker, length, info)) = fence_start(line) {
            if let Some((opened, opening_length, language)) = fence {
                if marker == opened && length >= opening_length && info.is_empty() {
                    out.extend(block_examples(language, &body));
                    body.clear();
                    fence = None;
                    continue;
                }
            } else {
                fence = Some((marker, length, info));
                continue;
            }
        }
        if fence.is_some() {
            body.push_str(content);
            body.push('\n');
        } else if indented {
            indented_body.push_str(content);
            indented_body.push('\n');
        }
    }
    out.extend(shell_lines(&indented_body));
    assert!(fence.is_none(), "unclosed documentation block");
    let recipes = justfile
        .lines()
        .filter(|line| line.starts_with(' ') || line.starts_with('\t'))
        .collect::<Vec<_>>()
        .join("\n");
    out.extend(shell_lines(&recipes));
    out
}

fn commands(readme: &str, justfile: &str) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    for line in examples(readme, justfile) {
        let tokens = words(&line).unwrap();
        for (start, item) in tokens.iter().enumerate() {
            let Token::Word(token) = item else { continue };
            if token != "forge-warden" && token != "--bin=forge-warden" {
                continue;
            }
            // cargo --bin names a target, not an executable. Only the args after `--` reach clap.
            let argv = if token == "--bin=forge-warden"
                || (start > 0 && tokens[start.saturating_sub(1)] == Token::Word("--bin".to_owned()))
            {
                let Some(separator) = tokens
                    .iter()
                    .enumerate()
                    .skip(start.saturating_add(1))
                    .find(|(_, value)| **value == Token::Word("--".to_owned()))
                    .map(|(index, _)| index)
                else {
                    panic!("documented cargo invocation needs the clap argument separator: {line}");
                };
                tokens.iter().skip(separator.saturating_add(1))
            } else {
                tokens.iter().skip(start.saturating_add(1))
            };
            let mut command = vec!["forge-warden".to_owned()];
            command.extend(
                argv.take_while(|arg| !matches!(arg, Token::Separator(_)))
                    .filter_map(|arg| match arg {
                        Token::Word(value) => Some(value.clone()),
                        Token::Separator(_) => None,
                    }),
            );
            out.push(command);
        }
    }
    out
}

#[test]
fn executable_documentation_uses_the_actual_cli_grammar() {
    for row in document("docs/command-sources.tsv").lines() {
        let (path, expected) = row.split_once('\t').unwrap();
        let expected: usize = expected.parse().unwrap();
        let text = document(path);
        let found = if path == "justfile" {
            commands("", &text)
        } else {
            commands(&text, "")
        };
        assert_eq!(found.len(), expected, "command inventory changed in {path}");
        for argv in found {
            assert!(
                Cli::try_parse_from(&argv).is_ok(),
                "{path}: invalid documented command: {argv:?}"
            );
        }
    }
}

#[test]
fn command_discovery_preserves_quotes_pipes_and_continuations_and_rejects_bad_cli() {
    let readme = "```bash\nforge-warden check --policy 'a b' \\\n --patch p --ranker \"$(command -v ranker)\" > out # comment\n```\n```text\nforge-warden made-up output\n```\n";
    let argv = commands(
        readme,
        "    producer | cargo run --bin forge-warden -- schema exit-codes",
    );
    assert_eq!(argv.len(), 2);
    for args in &argv {
        assert!(Cli::try_parse_from(args).is_ok(), "{args:?}");
    }
    for text in [
        "forge-warden check --policy p --rank r",
        "forge-warden schema verdict-v99",
        "forge-warden lint p --invented",
    ] {
        for args in commands(&format!("```bash\n{text}\n```"), "") {
            assert!(
                Cli::try_parse_from(&args).is_err(),
                "bad command admitted: {args:?}"
            );
        }
    }
    assert!(words("forge-warden lint 'unclosed").is_err());
    let quoted = commands("```bash\nforge-warden lint '|' --invented\n```", "");
    assert_eq!(quoted.len(), 1);
    assert!(Cli::try_parse_from(&quoted[0]).is_err());
    assert_eq!(words("''").unwrap(), vec![Token::Word(String::new())]);
}

#[test]
fn indented_fences_tabs_and_nonstandard_languages_cannot_hide_commands() {
    for fence in ["```", "~~~"] {
        let text = format!("  {fence}bash\n  forge-warden lint p --invented\n  {fence}\n");
        let found = commands(&text, "\tforge-warden init --invented");
        assert_eq!(found.len(), 2);
        for argv in found {
            assert!(Cli::try_parse_from(&argv).is_err());
        }
    }
    let escaped = "```zsh\nforge\\-warden init --invented\n```\n";
    assert!(std::panic::catch_unwind(|| commands(escaped, "")).is_err());
    for language in ["zsh", "fish", "bash title=x", ""] {
        let text = format!("```{language}\nforge-warden init --invented\n```\n");
        assert!(
            std::panic::catch_unwind(|| commands(&text, "")).is_err(),
            "unrecognized command language admitted: {language}"
        );
    }
    for style in ["|", "|-", "|+", ">", ">-", ">+"] {
        let text = format!("```yaml\n- run: {style}\n    forge-warden init --invented\n```\n");
        assert!(std::panic::catch_unwind(|| commands(&text, "")).is_err());
    }
}

#[test]
fn rendered_code_cannot_escape_admission_or_refusal() {
    for tag in [
        "pre",
        "PRE class=x",
        "xmp",
        "listing",
        "code",
        "samp",
        "kbd",
    ] {
        for prefix in [
            "",
            "<div>",
            "Run ",
            "> ",
            "- ",
            "* ",
            "+ ",
            "1. ",
            "2) ",
            "> - ",
            "- > ",
            "1. > + 2. > ",
        ] {
            let source = format!(
                "{prefix}<{tag}>
forge-warden init --invented
"
            );
            assert!(
                std::panic::catch_unwind(|| commands(&source, "")).is_err(),
                "raw code escaped: {source:?}"
            );
        }
    }
    for prefix in [
        "",
        "> ",
        ">> ",
        "- ",
        "* ",
        "+ ",
        "1. ",
        "2) ",
        "> - ",
        "- > ",
        "1. > + 2. > ",
    ] {
        for fence in ["```", "~~~"] {
            let source = format!(
                "{prefix}{fence}sh
{prefix}forge-warden init --invented
{prefix}{fence}
"
            );
            let found = commands(&source, "");
            assert_eq!(found.len(), 1, "container fence skipped: {source:?}");
            assert!(Cli::try_parse_from(&found[0]).is_err());
            let valid = format!(
                "{prefix}{fence}sh
{prefix}forge-warden check --policy p \\
{prefix}--patch p --ranker r
{prefix}{fence}
"
            );
            let accepted = commands(&valid, "");
            assert_eq!(accepted.len(), 1);
            assert!(Cli::try_parse_from(&accepted[0]).is_ok());
        }
        let indented = format!(
            "{prefix}    forge-warden check --policy p \\
{prefix}    --patch p --ranker r --invented
"
        );
        let found = commands(&indented, "");
        assert_eq!(found.len(), 1, "indented code skipped: {indented:?}");
        assert!(Cli::try_parse_from(&found[0]).is_err());
        let valid_indented = indented.replace(" --invented", "");
        let accepted_indented = commands(&valid_indented, "");
        assert_eq!(accepted_indented.len(), 1);
        assert!(Cli::try_parse_from(&accepted_indented[0]).is_ok());
    }
}

#[test]
fn raw_tag_boundaries_inline_spans_and_explicit_data_are_distinct() {
    for source in [
        "Use `<pre>`, ``<xmp>``, and `<listing>` in prose.",
        "<preview> <prelude> <codebase> <xmpfoo> <listingx>",
        "forge-warden init --invented",
        "```text
<pre>forge-warden init --invented</pre>
```",
        "```sh
printf '<pre>'
```",
    ] {
        assert!(commands(source, "").is_empty(), "{source:?}");
    }
    for source in [
        "unmatched `<pre>",
        r"escaped \`<pre>`",
        "``one ` tick`` <pre>",
        "<PrE	class=x>",
        "<pre/>",
        "<pre",
        "> `safe` Run <pre>bad</pre>",
    ] {
        assert!(
            std::panic::catch_unwind(|| commands(source, "")).is_err(),
            "{source:?}"
        );
    }
}

#[test]
fn mixed_tabs_are_indented_code_in_every_container() {
    for prefix in ["", "> ", "> - ", "1. > "] {
        for indent in ["    ", "	", " 	", "  	", "   	"] {
            let source = format!(
                "{prefix}{indent}forge-warden init --invented
"
            );
            let found = commands(&source, "");
            assert_eq!(found.len(), 1, "{source:?}");
            assert!(Cli::try_parse_from(&found[0]).is_err());
        }
    }
}
