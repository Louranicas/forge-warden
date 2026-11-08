//! Executable front-door examples are parsed by the actual clap grammar, without running them.
use super::Cli;
use clap::Parser as _;
use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};

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

// This is a policy on parser-classified HTML, not a Markdown/inline-span recognizer.
// No raw HTML code opener is allowlisted; ordinary inline code is Event::Code instead.
fn raw_code_tag(html: &str) -> bool {
    let plain = html.to_ascii_lowercase();
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

fn examples(markdown: &str, justfile: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut language = None;
    let mut body = String::new();
    for event in Parser::new(markdown) {
        if let Event::Start(Tag::CodeBlock(kind)) = event {
            assert!(language.is_none(), "nested code-block event");
            language = Some(match kind {
                CodeBlockKind::Indented => "sh".to_owned(),
                CodeBlockKind::Fenced(info) => info.into_string(),
            });
        } else if let Event::End(TagEnd::CodeBlock) = event {
            out.extend(block_examples(&language.take().unwrap(), &body));
            body.clear();
        } else if let Event::Text(text) = event {
            if language.is_some() {
                body.push_str(&text);
            }
        } else if let Event::Html(html) | Event::InlineHtml(html) = event {
            assert!(
                !raw_code_tag(&html),
                "raw HTML code requires an explicit language fence: {html}"
            );
        }
    }
    assert!(language.is_none(), "unclosed parser code-block event");
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

    for (prefix, continuation) in [
        ("", ""),
        ("> ", "> "),
        (">> ", ">> "),
        ("- ", "  "),
        ("* ", "  "),
        ("+ ", "  "),
        ("1. ", "   "),
        ("2) ", "   "),
        ("> - ", ">   "),
        ("- > ", "  > "),
        ("1. > + 2. > ", "   >      > "),
    ] {
        for fence in ["```", "~~~"] {
            let source = format!(
                "{prefix}{fence}sh\n{continuation}forge-warden init --invented\n{continuation}{fence}\n"
            );
            let found = commands(&source, "");
            assert_eq!(found.len(), 1, "container fence skipped: {source:?}");
            assert!(Cli::try_parse_from(&found[0]).is_err());
            let valid = format!(
                "{prefix}{fence}sh\n{continuation}forge-warden check --policy p \\\n{continuation}--patch p --ranker r\n{continuation}{fence}\n"
            );
            let accepted = commands(&valid, "");
            assert_eq!(accepted.len(), 1);
            assert!(Cli::try_parse_from(&accepted[0]).is_ok());
        }
        let indented = format!(
            "{prefix}intro\n{continuation}\n{continuation}    forge-warden check --policy p \\\n{continuation}    --patch p --ranker r --invented\n"
        );
        let found = commands(&indented, "");
        assert_eq!(found.len(), 1, "{indented:?}");
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
        "<div></div>\n\nUse `<pre>` in prose.",
        "</div>\n\n```text\n<pre>recorded output</pre>\n```",
        "<div>\n\n```text\n<pre>recorded output</pre>\n```",
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
    for (prefix, continuation) in [("", ""), ("> ", "> "), ("> - ", ">   "), ("1. > ", "   > ")] {
        for indent in ["    ", "    \t", " \t\t", "  \t\t", "   \t\t"] {
            let source = format!(
                "{prefix}intro\n{continuation}\n{continuation}{indent}forge-warden init --invented\n"
            );
            let found = commands(&source, "");
            assert_eq!(found.len(), 1, "{source:?}");
            assert!(Cli::try_parse_from(&found[0]).is_err());
        }
    }
}

#[test]
fn raw_html_context_cannot_hide_code_tags_behind_backticks() {
    for source in [
        "<div>`<pre>forge-warden init --invented</pre>`</div>",
        "<custom-element>\n`<pre>forge-warden init --invented</pre>`\n</custom-element>",
        "<!-- ` --> <pre>forge-warden init --invented</pre> `",
    ] {
        assert!(
            std::panic::catch_unwind(|| commands(source, "")).is_err(),
            "{source:?}"
        );
    }
    assert_eq!(
        commands("Use `<div>` and `<pre>` as inline examples.", "").len(),
        0
    );
}

#[test]
fn xv_bastion_closing_html_context() {
    let mut escaped = Vec::new();
    for closing in ["</div>", "</table>", "</section>", "</custom-element>"] {
        let source = format!("{closing}\n`<pre>forge-warden init --invented</pre>`\n");
        let result = std::panic::catch_unwind(|| commands(&source, ""));
        if result.is_ok() {
            escaped.push(format!("{source:?}: {result:?}"));
        }
    }
    assert!(
        escaped.is_empty(),
        "CLOSING HTML ESCAPES: {}",
        escaped.join("; ")
    );
}
#[test]
fn xv_bastion_raw_html_fence_exemption() {
    let mut escaped = Vec::new();
    for tag in ["div", "table", "section"] {
        let source =
            format!("<{tag}>\n```text\n<pre>forge-warden init --invented</pre>\n```\n</{tag}>\n");
        let result = std::panic::catch_unwind(|| commands(&source, ""));
        if result.is_ok() {
            escaped.push(format!("{source:?}: {result:?}"));
        }
    }
    assert!(
        escaped.is_empty(),
        "RAW HTML FENCE ESCAPES: {}",
        escaped.join("; ")
    );
}

#[test]
fn keel_original_reproducers_are_asserted_controls() {
    for source in [
        "<div><pre>\nforge-warden init --invented\n</pre></div>\n",
        "> <pre>\n> forge-warden init --invented\n> </pre>\n",
        "- <pre>\n  forge-warden init --invented\n  </pre>\n",
        "<code>\nforge-warden init --invented\n</code>\n",
        "<xmp>\nforge-warden init --invented\n</xmp>\n",
        "<listing>\nforge-warden init --invented\n</listing>\n",
        "Run <pre>\nforge-warden init --invented\n</pre>\n",
        "\u{a0}<pre>\nforge-warden init --invented\n</pre>\n",
    ] {
        assert!(
            std::panic::catch_unwind(|| commands(source, "")).is_err(),
            "{source:?}"
        );
    }
    for source in [
        "> ```bash\n> forge-warden init --invented\n> ```\n",
        ">     forge-warden init --invented\n",
        "1. Run:\n\n   ```bash\n   forge-warden init --invented\n   ```\n",
        "- a\n  - b\n\n      ```bash\n      forge-warden init --invented\n      ```\n",
        "```bash\nforge-warden init --invented\n```\n",
    ] {
        let found = commands(source, "");
        assert_eq!(found.len(), 1, "{source:?}");
        assert!(Cli::try_parse_from(&found[0]).is_err());
    }
    for prose in [
        "forge-warden init --invented\n",
        "> intro\n> \n> \tforge-warden init --invented\n",
    ] {
        assert!(commands(prose, "").is_empty(), "{prose:?}");
    }
}
