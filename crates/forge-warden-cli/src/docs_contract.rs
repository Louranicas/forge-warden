//! Executable front-door examples are parsed by the actual clap grammar, without running them.
use super::Cli;
use clap::Parser as _;

const README: &str = include_str!("../../../README.md");
const JUSTFILE: &str = include_str!("../../../justfile");

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

fn examples(readme: &str, justfile: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut fence = None;
    let mut continued = String::new();
    for line in readme.lines() {
        if let Some(language) = line.strip_prefix("```") {
            assert!(continued.is_empty(), "unterminated shell continuation");
            fence = if fence.is_some() {
                None
            } else {
                Some(language)
            };
            continue;
        }
        let Some(language) = fence else { continue };
        let executable = matches!(language, "bash" | "sh" | "shell" | "console");
        let yaml_run = language == "yaml" && line.trim_start().starts_with("- run:");
        if executable || yaml_run {
            assert!(
                !yaml_run || !line.trim_end().ends_with(['|', '>']),
                "extend the example parser before adding YAML block commands"
            );
            let line = if yaml_run {
                line.trim_start().strip_prefix("- run:").unwrap()
            } else {
                line
            };
            if let Some(part) = line.strip_suffix('\\') {
                continued.push_str(part);
                continued.push(' ');
            } else {
                continued.push_str(line);
                out.push(std::mem::take(&mut continued));
            }
        }
    }
    assert!(
        fence.is_none() && continued.is_empty(),
        "unclosed documentation block"
    );
    out.extend(
        justfile
            .lines()
            .filter(|line| line.starts_with("    "))
            .map(str::to_owned),
    );
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
    let commands = commands(README, JUSTFILE);
    assert!(
        commands.len() >= 6,
        "executable example discovery lost commands"
    );
    for argv in commands {
        assert!(
            Cli::try_parse_from(&argv).is_ok(),
            "invalid documented command: {argv:?}"
        );
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
