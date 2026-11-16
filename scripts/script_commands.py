#!/usr/bin/env python3
"""Every command a tracked script can call is one the gate image has (gap G9, Mission 7).

Train 14's first gate went red because `scripts/tests/dogfood.sh` called `cmp`: every lane's
toolbox has diffutils, the offline gate image does not. This check reads the repository's own
scripts and names each command they call that is not in DECLARED below.

What is read, and how:
- every tracked shell file (`*.sh`, `*.bash`, or a `#!` line naming sh, bash or dash, also
  through `env` and `env -S`), with a linear token reader: quotes, `$(...)`, backquotes,
  `<(...)`, here-documents (a body that starts with a sh/bash shebang, or is fed to bash/sh, is
  read as shell; one with a python shebang or fed to python3 is read as Python; any other body
  is data), and the command position after `;`, `|`, `&&`, `||`, `&`, `(`, `{`, `!`, the
  reserved words (`coproc` too), the prefixes `env`, `exec`, `command`, `nice`, `nohup`,
  `timeout`, `xargs`, `sudo` (their options and option arguments skipped), find's `-exec`,
  `-execdir`, `-ok` and `-okdir`, the command after rustup's `run` and its toolchain, the
  program options of tar (`-I`, `--use-compress-program`, `--to-command`,
  `--checkpoint-action=exec=`, `--info-script`, `--new-volume-script`) and sort
  (`--compress-program`), and the text of `bash -c`, `sh -c`, `trap`, `env -S` and
  `python3 -c`;
- every tracked Python file (`*.py`, or a python `#!` line) and Python here-document:
  `subprocess`, `os`, `asyncio` and `pty` process calls (also imported under another name),
  each branch of a conditional argument list, and `shell=` on unless it is literally false.
  A string run by a shell (`shell=True`, `os.system`, `os.popen`, `getoutput`,
  `create_subprocess_shell`) is read as shell text; an argument list is read as the command
  line it makes, so the prefixes above apply to it; a non-literal argument stands for a
  dynamic word.
- the interpreter a `#!` line names is itself a call, on line 1.

Fail closed: a construct this reader does not handle (`case` inside a substitution, an
unterminated quote or here-document, an `eval`, `shopt -s expand_aliases`, a dynamic command
word that is not in DYNAMIC, a tracked file whose `#!` line names another interpreter) is a
problem, not a skip. A word run through `command`, `exec`, `env` or another prefix is never
excused by a shell function of the same name.

A tracked file that a call runs or sources by a literal path (`./FILE`, `bash FILE`,
`python3 FILE`, `. FILE`) is read in the language it runs in, with or without an extension or a
`#!` line; the functions of a sourced file count for the file that sources it.

Stated limits (each needs a deliberate author): a Python call whose program is not a literal
(a variable, `sys.executable`, an argument list built by a call other than `'...'.split()` and
`shlex.split('...')`, a process function bound to another name by assignment) is not resolved;
a script run or sourced through a non-literal path is not read; a data here-document that a
later line runs is not read; a command reached only through a non-literal path is pinned by
DYNAMIC, not resolved; commands run by another program's own language or configuration are not
read (`awk` `system()`, sed's `e`, Git's pager, editor and alias settings); a shell function
excuses a call of its name anywhere in the file, also a call before the definition, which bash
would run as the real command.

usage: python3 -I scripts/script_commands.py     (exit 1 names each undeclared call)
"""
import ast
from pathlib import Path
import posixpath
import re
import shlex
import subprocess
import sys

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[1]

# The commands any script may call: each one is in the offline gate image, measured with
# `compgen -c` in localhost/ddf-council-gate:20261026c (image id 8f4ea620d021..., 2026-11-15).
# That image has no cmp, diff, jq, xxd or file. Adding a name here needs the same measurement.
IMAGE = frozenset("""
    bash sh env cat cp mv rm ln ls mkdir rmdir chmod touch mktemp dirname basename realpath readlink
    cut head tail sort uniq tr wc tee sha256sum date sleep stat seq id uname nproc
    sed grep awk tar gzip find xargs curl git python3 cargo rustc rustup rustfmt ssh-keygen
    timeout nice nohup
""".split())
# Commands only the named files may call, and why the gate never needs them there.
ONLY_IN = {
    # ci_local runs these steps only after `command -v` finds the tool, and names them as not
    # run otherwise (tool-versions.toml [local_gate] hosted_only). Not in the gate image.
    'deep-diff-forge': frozenset({'scripts/ci_local.sh'}),
    'zizmor': frozenset({'scripts/ci_local.sh'}),
    # The self-hosted runner guest's bootstrap (WP-RUNNER-UP) installs a toolchain on a fresh
    # Ubuntu guest; the gate never runs it. apt-get is not in the gate image.
    'apt-get': frozenset({'scripts/guest_bootstrap.sh'}),
    'sudo': frozenset({'scripts/guest_bootstrap.sh'}),
    'cc': frozenset({'scripts/guest_bootstrap.sh'}),
}
DECLARED = IMAGE | frozenset(ONLY_IN)

BUILTINS = frozenset("""
    : . [ [[ ]] alias bg bind break builtin caller cd command compgen complete continue declare
    dirs disown echo enable eval exec exit export false fc fg getopts hash help history jobs kill
    let local logout mapfile popd printf pushd pwd read readarray readonly return set shift shopt
    source suspend test times trap true type typeset ulimit umask unalias unset wait
""".split()) - {'eval'}
KEYWORDS = frozenset('if then else elif fi do done while until for in case esac select function time coproc ! { }'.split())
OPENERS = frozenset('if then else elif do while until ! { time coproc'.split())
CLOSERS = frozenset('fi done esac }'.split())
# Commands that run their operand. Each maps to the options that take a separate argument
# (`exec -a NAME`, `env -C DIR`, `timeout -s KILL`): that argument is not the command.
PREFIX_OPTIONS = {
    'env': frozenset({'-u', '--unset', '-C', '--chdir'}),
    'exec': frozenset({'-a'}),
    'command': frozenset(),
    'nice': frozenset({'-n', '--adjustment'}),
    'nohup': frozenset(),
    'timeout': frozenset({'-s', '--signal', '-k', '--kill-after'}),
    'xargs': frozenset({'-a', '--arg-file', '-d', '--delimiter', '-E', '-I', '-L', '--max-lines',
                        '-n', '--max-args', '-P', '--max-procs', '-s', '--max-chars'}),
    'sudo': frozenset({'-u', '-g', '-p', '-C', '-D', '-h', '-r', '-t', '-T', '-U'}),
}
PREFIXES = frozenset(PREFIX_OPTIONS)
FIND_EXEC = frozenset({'-exec', '-execdir', '-ok', '-okdir'})
# Options whose value is a command line the program runs (`tar --to-command=...`).
PROGRAM_OPTIONS = {
    'tar': frozenset({'-I', '--use-compress-program', '--to-command', '--info-script',
                      '--new-volume-script', '-F'}),
    'sort': frozenset({'--compress-program'}),
}
# Shell options that take a separate argument (`bash -o pipefail FILE`).
SHELL_OPTION_ARGS = frozenset({'-o', '+o', '-O', '+O', '--rcfile', '--init-file'})
SHELLS = frozenset({'sh', 'bash', 'dash'})
PYTHONS = re.compile(r'python(?:3(?:\.[0-9]+)?)?')
# A non-literal Python argument stands for this dynamic word in the command line it makes.
PY_DYNAMIC = '"$python_expression"'
OPERATORS = (';;&', ';;', ';&', '&&', '||', '|&', '&', '|', ';', '(', ')', '\n')
REDIRECTS = ('<<<', '<<-', '<<', '&>>', '&>', '>>', '>&', '<&', '<>', '>|', '>', '<')
ASSIGNMENT = re.compile(r'[A-Za-z_][A-Za-z0-9_]*(?:\[[^]]*\])?\+?=')
# `env NAME=value` and `env "$name=value"`: a name, literal or one parameter, then `=`.
ENV_ASSIGNMENT = re.compile(r'["\']?(?:[A-Za-z_][A-Za-z0-9_]*|\$\{?[A-Za-z_][A-Za-z0-9_]*\}?)=')
# Non-literal command words, pinned by file and by the exact text as written. Each runs a file
# this repository builds, fetches or tracks, or a function's own arguments.
DYNAMIC = {
    # The versioned hooks run the repository's own gate scripts (read here as tracked files).
    '.githooks/pre-commit': {'"$(git rev-parse --show-toplevel)/scripts/ci_fast.sh"'},
    '.githooks/pre-push': {'"$(git rev-parse --show-toplevel)/scripts/ci_local.sh"'},
    # The dogfood driver runs the ranker it fetched (sha256-pinned) and the judge it built.
    'scripts/dogfood.sh': {'"$out/ranker/bin/deep-diff-forge"', '"$out/target/release/forge-warden"'},
    # The guest bootstrap runs the rustup-init it downloaded (sha256-pinned).
    'scripts/guest_bootstrap.sh': {'"$work/rustup-init"'},
    # gitenv and made run their own arguments; every caller passes a declared command (bash, env),
    # but those arguments are not read as commands here (a stated limit).
    'scripts/tests/dogfood.sh': {'"$@"'},
}
# Non-literal command words allowed exactly once in the named file (a second one is named).
DYNAMIC_ONCE = {
    # The routing test runs the `run:` block of the workflow's route job with `bash -c`; that
    # text lives in .github/workflows/ (it calls only `[` and printf; Bastion, M7-G9 round 2).
    'scripts/tests/runner_routing.py': PY_DYNAMIC,
}
# Markers in a reader's output for a file a call runs or sources: `bash FILE`, `python3 FILE`,
# `. FILE`. problems() reads that file in that language (or names it as not read).
RUNS = {'(sh)': 'shell', '(py)': 'python', '(src)': 'shell'}
# Python process calls, by where the command is: (index of the program, index where its
# arguments start, whether those arguments are one list or spread out). Shell-text calls read
# their first argument as shell text.
PY_SHELL_TEXT = frozenset({'system', 'popen', 'getoutput', 'getstatusoutput', 'create_subprocess_shell'})
PY_ARGV = {
    # subprocess: one list (or a string, which is the program; with shell=True, shell text).
    'run': None, 'call': None, 'check_call': None, 'check_output': None, 'Popen': None,
    # os.exec*/posix_spawn*: program, then an argv list whose first item is only argv[0].
    'execv': (0, 1, 'list'), 'execve': (0, 1, 'list'), 'execvp': (0, 1, 'list'),
    'execvpe': (0, 1, 'list'), 'posix_spawn': (0, 1, 'list'), 'posix_spawnp': (0, 1, 'list'),
    'execl': (0, 1, 'spread'), 'execle': (0, 1, 'spread'), 'execlp': (0, 1, 'spread'),
    'execlpe': (0, 1, 'spread'),
    # os.spawn*: mode first, then the program.
    'spawnv': (1, 2, 'list'), 'spawnve': (1, 2, 'list'), 'spawnvp': (1, 2, 'list'),
    'spawnvpe': (1, 2, 'list'), 'spawnl': (1, 2, 'spread'), 'spawnle': (1, 2, 'spread'),
    'spawnlp': (1, 2, 'spread'), 'spawnlpe': (1, 2, 'spread'),
    # asyncio.create_subprocess_exec(program, *args): no argv[0] to skip.
    'create_subprocess_exec': (0, 1, 'program-then-args'),
}


class Unsupported(ValueError):
    pass


class Word:
    def __init__(self, line, raw, value, literal):
        self.line, self.raw, self.value, self.literal = line, raw, value, literal


class Lexer:
    """One pass over shell text: tokens, here-document bodies and nested substitution calls."""

    def __init__(self, text, line, sink):
        self.text, self.i, self.line, self.sink = text, 0, line, sink

    def error(self, why):
        raise Unsupported(f'line {self.line}: {why}')

    def tokens(self, closer=None):
        out, pending, depth = [], [], 0
        text = self.text
        while self.i < len(text):
            c = text[self.i]
            if c in ' \t':
                self.i += 1
            elif c == '\\' and text[self.i + 1:self.i + 2] == '\n':
                self.i += 2
                self.line += 1
            elif c == '#':
                while self.i < len(text) and text[self.i] != '\n':
                    self.i += 1
            elif c == '\n':
                out.append(('op', '\n', self.line))
                self.i += 1
                self.line += 1
                for heredoc in pending:
                    out.append(self.heredoc_body(*heredoc))
                pending = []
            elif c in '<>' and text[self.i + 1:self.i + 2] == '(':
                self.i += 2
                self.substitution()
                out.append(('word', Word(self.line, '<(...)', None, False)))
            elif text.startswith(REDIRECTS, self.i) and not (c == '&' and text.startswith('&&', self.i)):
                op = next(r for r in REDIRECTS if text.startswith(r, self.i))
                self.i += len(op)
                out.append(('redirect', op, self.line))
                if op in ('<<', '<<-'):
                    while text[self.i:self.i + 1] in (' ', '\t'):
                        self.i += 1
                    word = self.word()
                    out.append(('word', word))
                    pending.append((word, op == '<<-', len(out)))
            elif text.startswith(OPERATORS, self.i):
                op = next(o for o in OPERATORS if text.startswith(o, self.i))
                if closer and op == ')' and depth == 0:
                    self.i += 1
                    if pending:
                        self.error('here-document inside a substitution')
                    return out
                depth += {'(': 1, ')': -1}.get(op, 0)
                self.i += len(op)
                out.append(('op', op, self.line))
            else:
                out.append(('word', self.word()))
        if closer:
            self.error('unterminated substitution')
        if pending:
            self.error('unterminated here-document')
        return out

    def heredoc_body(self, word, strip_tabs, index):
        if not word.literal and word.value is None:
            self.error('here-document delimiter is not literal')
        delimiter, start, lines = word.value, self.line, []
        while True:
            if self.i >= len(self.text):
                self.error(f'here-document {delimiter!r} is not terminated')
            end = self.text.find('\n', self.i)
            end = len(self.text) if end < 0 else end
            raw = self.text[self.i:end]
            self.i, self.line = end + 1, self.line + 1
            if (raw.lstrip('\t') if strip_tabs else raw) == delimiter:
                break
            lines.append(raw)
        body = '\n'.join(lines) + '\n'
        quoted = any(q in word.raw for q in '\'"\\')
        if not quoted and ('$(' in body or '`' in body):
            Lexer(body, start, self.sink).double_quoted(stop=None)
        return ('heredoc', body, start, index)

    def substitution(self):
        start = self.line
        nested = Lexer(self.text, self.line, self.sink)
        nested.i = self.i
        tokens = nested.tokens(closer=')')
        if any(t[0] == 'word' and t[1].value == 'case' for t in tokens):
            raise Unsupported(f'line {start}: `case` inside a substitution')
        self.sink(tokens)
        self.i, self.line = nested.i, nested.line

    def backquote(self):
        start, self.i = self.line, self.i + 1
        end = self.text.find('`', self.i)
        while end > 0 and self.text[end - 1] == '\\':
            end = self.text.find('`', end + 1)
        if end < 0:
            self.error('unterminated backquote')
        inner = self.text[self.i:end]
        self.sink(Lexer(inner, start, self.sink).tokens())
        self.line += inner.count('\n')
        self.i = end + 1

    def dollar(self):
        """At `$`: returns literal text when the expansion is literal ($'...'), else None."""
        text, i = self.text, self.i
        if text.startswith('$((', i):
            depth, j = 0, i + 1
            while j < len(text):
                depth += {'(': 1, ')': -1}.get(text[j], 0)
                if depth == 0:
                    break
                j += 1
            self.line += text[i:j].count('\n')
            self.i = j + 1
            return None
        if text.startswith('$(', i):
            self.i += 2
            self.substitution()
            return None
        if text.startswith('${', i):
            depth, j = 0, i + 1
            while j < len(text):
                depth += {'{': 1, '}': -1}.get(text[j], 0)
                if depth == 0:
                    break
                j += 1
            if j >= len(text):
                self.error('unterminated ${')
            inner = text[i + 2:j]
            if '$(' in inner or '`' in inner:
                Lexer(inner, self.line, self.sink).double_quoted(stop=None)
            self.i = j + 1
            return None
        if text.startswith("$'", i):
            j = i + 2
            while j < len(text) and text[j] != "'":
                j += 2 if text[j] == '\\' else 1
            self.i = j + 1
            return text[i + 2:j]
        match = re.compile(r'\$(?:[A-Za-z_][A-Za-z0-9_]*|[0-9@*#?$!-])').match(text, i)
        self.i = match.end() if match else i + 1
        return None if match else '$'

    def double_quoted(self, stop='"'):
        """Read to the closing quote (or the end); returns the text when it holds no expansion."""
        text, chunk, literal = self.text, [], True
        while self.i < len(text):
            c = text[self.i]
            if stop and c == stop:
                self.i += 1
                return ''.join(chunk) if literal else None
            if c == '\\':
                chunk.append(text[self.i + 1:self.i + 2])
                self.line += text[self.i + 1:self.i + 2] == '\n'
                self.i += 2
            elif c == '$':
                value = self.dollar()
                literal = literal and value is not None
                chunk.append(value or '')
            elif c == '`':
                self.backquote()
                literal = False
            else:
                chunk.append(c)
                self.line += c == '\n'
                self.i += 1
        if stop:
            self.error('unterminated double quote')
        return None

    def word(self):
        text, start, line = self.text, self.i, self.line
        value, literal = [], True
        while self.i < len(text):
            c = text[self.i]
            if c in ' \t\n;&|()<>':
                break
            if c == '\\':
                if text[self.i + 1:self.i + 2] == '\n':
                    self.i += 2
                    self.line += 1
                    continue
                value.append(text[self.i + 1:self.i + 2])
                self.i += 2
            elif c == "'":
                end = text.find("'", self.i + 1)
                if end < 0:
                    self.error('unterminated single quote')
                value.append(text[self.i + 1:end])
                self.line += text[self.i:end].count('\n')
                self.i = end + 1
            elif c == '"':
                self.i += 1
                part = self.double_quoted()
                literal = literal and part is not None
                value.append(part or '')
            elif c == '$':
                part = self.dollar()
                literal = literal and part is not None
                value.append(part or '')
            elif c == '`':
                self.backquote()
                literal = False
            else:
                value.append(c)
                self.i += 1
        if self.i == start:
            self.error(f'cannot read {text[self.i]!r}')
        return Word(line, text[start:self.i], ''.join(value) if literal else None, literal)


def shell_calls(text, line=1):
    """(line, word, external) for each command word of the shell text, nested substitutions
    included. external: the word is run by a prefix or by find, so no shell function can be it."""
    calls = []

    def sink(tokens):
        calls.extend(command_words(tokens, calls))

    lexer = Lexer(text, line, sink)
    sink(lexer.tokens())
    return calls


def external(calls):
    """The same calls, none of which a shell function can answer (a prefix or Python runs them)."""
    return [(line, word, True) for line, word, _ in calls if not word.startswith('()')]


def shebang(line):
    """(kind, program) for a `#!` line: kind is 'shell', 'python' or 'other'."""
    words = line[2:].split()
    program, rest = (words[0].rsplit('/', 1)[-1], words[1:]) if words else ('', [])
    if program == 'env':
        i = 0
        while i < len(rest):
            word = rest[i]
            if word in ('-S', '--split-string'):
                i += 1
            elif word.startswith('-S'):
                rest[i] = word[2:]
                break
            elif word in PREFIX_OPTIONS['env']:
                i += 2
            elif word.startswith('-') or '=' in word:
                i += 1
            else:
                break
        program = rest[i].rsplit('/', 1)[-1] if i < len(rest) else ''
    kind = 'shell' if program in SHELLS else 'python' if PYTHONS.fullmatch(program) else 'other'
    return kind, program


def command_words(tokens, calls):
    found, functions = [], set()
    expect, mode, prefix, owner, script = True, None, None, None, None
    finding = by_find = skip_parens = after_coproc = False
    for index, token in enumerate(tokens):
        kind = token[0]
        if kind == 'heredoc':
            body, start, consumer = token[1], token[2], owner
            first = body.split('\n', 1)[0]
            if first.startswith('#!'):
                consumer = {'shell': 'bash', 'python': 'python3'}.get(shebang(first)[0])
            if consumer in SHELLS:
                calls.extend(shell_calls(body, start))
            elif consumer and PYTHONS.fullmatch(consumer):
                calls.extend(python_calls(body, start))
            continue
        if kind == 'redirect':
            mode = ('target', mode)
            continue
        if kind == 'op':
            op = token[1]
            if mode == 'array':
                if op == ')':
                    mode = None
                continue
            if mode == 'case-subject':
                continue
            if mode == 'case-pattern':
                if op == ')':
                    mode, expect = None, True
                continue
            if op in (';;', ';&', ';;&'):
                mode, expect = 'case-pattern', False
                continue
            if skip_parens and op in ('(', ')'):
                continue
            skip_parens = False
            if op == ')' and index > 0 and tokens[index - 1][:2] == ('op', '(') and found:
                functions.add(found.pop()[1])
                continue
            expect, prefix, script, finding, by_find = op != ')', None, None, False, False
            if mode == 'for' and op in (';', '\n'):
                mode = 'for-do'
            continue
        word = token[1]
        skip_parens = False
        if mode == 'array':
            continue
        if mode and mode[0] == 'target':
            mode = mode[1]
            continue
        if mode == 'function-name':
            # `function NAME`, with or without `()`: NAME is defined here, not called.
            functions.add(word.value if word.literal else word.raw)
            mode, expect, skip_parens = None, True, True
            continue
        if mode in ('for', 'for-do'):
            if word.value == 'do':
                mode, expect = None, True
            continue
        if mode == 'case-subject':
            if word.value == 'in':
                mode = 'case-pattern'
            continue
        if mode == 'case-pattern':
            if word.value == 'esac':
                mode, expect = None, False
            continue
        if mode == '[[':
            if word.value == ']]':
                mode, expect = None, False
            continue
        if not expect:
            if finding and word.literal and word.value in FIND_EXEC:
                # find runs the word after -exec/-execdir/-ok/-okdir as a command.
                expect, prefix, script, by_find = True, None, None, True
            elif script in ('next', 'next-python'):
                # The text of `trap '...'`, `bash -c '...'`, `env -S '...'` runs as shell;
                # the text of `python3 -c '...'` runs as Python.
                if not word.literal:
                    found.append((word.line, word.raw, True))
                elif script == 'next':
                    calls.extend(shell_calls(word.value, word.line))
                else:
                    calls.extend(python_calls(word.value, word.line))
                script = None
            elif isinstance(script, tuple) and script[0] == 'program-option':
                if word.literal:
                    calls.extend(external(shell_calls(word.value, word.line)))
                else:
                    found.append((word.line, word.raw, True))
                script = script[1]
            elif isinstance(script, tuple):
                script = script[1]
            elif script in SHELLS and word.literal:
                if re.fullmatch(r'-[a-z]*c[a-z]*', word.value):
                    script = 'next'
                elif word.value in SHELL_OPTION_ARGS:
                    script = ('option-argument', script)
                elif word.value == '-s':
                    script = None  # the script is standard input (a here-document, read above)
                elif not word.value.startswith(('-', '+')):
                    found.append((word.line, '(sh)' + word.value, True))
                    script = None
            elif script and PYTHONS.fullmatch(script) and word.literal:
                if re.fullmatch(r'-[A-Za-z]*c', word.value):
                    script = 'next-python'
                elif word.value in ('-W', '-X'):
                    script = ('option-argument', script)
                elif word.value == '-m':
                    script = None
                elif not word.value.startswith('-'):
                    found.append((word.line, '(py)' + word.value, True))
                    script = None
            elif script in ('.', 'source') and word.literal:
                found.append((word.line, '(src)' + word.value, True))
                script = None
            elif script == 'rustup' and word.literal and word.value == 'run':
                script = 'rustup-toolchain'
            elif script == 'rustup-toolchain':
                # rustup's `run` subcommand: the word after the toolchain is a command.
                expect, prefix, script, by_find = True, None, None, True
            elif script in PROGRAM_OPTIONS and word.literal:
                option, equals, program = word.value.partition('=')
                if option in PROGRAM_OPTIONS[script]:
                    if equals:
                        calls.extend(external(shell_calls(program, word.line)))
                    else:
                        script = ('program-option', script)
                elif script == 'tar' and word.value.startswith('--checkpoint-action=exec='):
                    calls.extend(external(shell_calls(word.value.split('=', 2)[2], word.line)))
            elif script == 'shopt' and word.literal and word.value == 'expand_aliases':
                raise Unsupported(f'line {word.line}: `shopt expand_aliases` (aliases are not read)')
            continue
        if prefix in PREFIXES and word.literal and word.value.startswith('-'):
            if word.value in PREFIX_OPTIONS[prefix]:
                mode = ('target', mode)
            elif prefix == 'env' and word.value in ('-S', '--split-string'):
                expect, script = False, 'next'
            elif prefix == 'env' and word.value.startswith(('-S', '--split-string=')):
                text = word.value[2:] if word.value.startswith('-S') else word.value.split('=', 1)[1]
                calls.extend(external(shell_calls(text, word.line)))
                expect = False
            elif prefix == 'command' and word.value in ('-v', '-V'):
                expect, prefix = False, 'probe'
            continue
        if prefix == 'env' and ENV_ASSIGNMENT.match(word.raw):
            continue
        if prefix == 'timeout' and word.literal and re.fullmatch(r'[0-9.]+[smhd]?', word.value):
            continue
        if ASSIGNMENT.match(word.raw) and prefix is None:
            if word.raw.endswith('=') and tokens[index + 1:index + 2] and tokens[index + 1][:2] == ('op', '('):
                mode = 'array'
            continue
        value = word.value
        if after_coproc:
            after_coproc = False
            nxt = tokens[index + 1:index + 2]
            if nxt and nxt[0][0] == 'word' and nxt[0][1].value == '{':
                continue  # `coproc NAME { ...; }`: NAME names the coprocess
        if word.literal and value in KEYWORDS:
            if value in ('for', 'select'):
                mode = 'for'
            elif value == 'case':
                mode = 'case-subject'
            elif value == 'function':
                mode = 'function-name'
            after_coproc = value == 'coproc'
            expect = value in OPENERS
            continue
        if word.literal and value == '[[':
            mode = '[['
            continue
        if word.literal and value == 'eval':
            raise Unsupported(f'line {word.line}: eval')
        found.append((word.line, value if word.literal else word.raw, prefix is not None or by_find))
        # /usr/bin/env runs its operand as env does; an absolute path acts as its file name.
        owner = value.rsplit('/', 1)[-1] if word.literal else None
        prefix = owner if owner in PREFIXES else None
        script = 'next' if owner == 'trap' else owner
        finding = finding or owner == 'find'
        by_find = False
        expect = prefix is not None
    return found + [(0, '()' + name, False) for name in functions]


def branches(node):
    """Each value a conditional expression can take (`a if x else b`)."""
    if isinstance(node, ast.IfExp):
        return branches(node.body) + branches(node.orelse)
    return [node]


def py_items(node):
    """The items of a literal argument list (`[...]`, `(...)`, `[...] + x`, `'a b'.split()`,
    `shlex.split('a b')`), else None. An item that is not there literally (`*rest`, the `x` of
    `+ x`) is None."""
    if isinstance(node, (ast.List, ast.Tuple)):
        return [None if isinstance(item, ast.Starred) else item for item in node.elts]
    if isinstance(node, ast.BinOp) and isinstance(node.op, ast.Add):
        left = py_items(node.left)
        if left is not None:
            right = py_items(node.right)
            return left + (right if right is not None else [None])
    if isinstance(node, ast.Call):
        func = node.func
        name = func.attr if isinstance(func, ast.Attribute) else getattr(func, 'id', None)
        owner = func.value if isinstance(func, ast.Attribute) else None
        if (name == 'split' and isinstance(owner, ast.Constant) and isinstance(owner.value, str)
                and not node.args and not node.keywords):
            return [ast.Constant(part) for part in owner.value.split()]
        if (name == 'split' and (owner is None or getattr(owner, 'id', None) == 'shlex') and node.args
                and isinstance(node.args[0], ast.Constant) and isinstance(node.args[0].value, str)):
            try:
                return [ast.Constant(part) for part in shlex.split(node.args[0].value)]
            except ValueError:
                return None
    return None


def py_text(node):
    """A literal string, or an f-string with each expression standing as a dynamic word."""
    if isinstance(node, ast.Constant) and isinstance(node.value, str):
        return node.value
    if isinstance(node, ast.JoinedStr):
        return ''.join(part.value if isinstance(part, ast.Constant) else PY_DYNAMIC for part in node.values)
    return None


def py_word(node):
    if isinstance(node, ast.Constant) and isinstance(node.value, str):
        return shlex.quote(node.value)
    return PY_DYNAMIC


def argv_text(items):
    """The command line an argument list makes, or None when its program is not a literal."""
    head = items[0] if items else None
    if not (isinstance(head, ast.Constant) and isinstance(head.value, str)):
        return None  # stated limit: the program is not a literal
    return ' '.join(py_word(item) for item in items)


def python_calls(text, line=1):
    """(line, command, True) for each process call whose program is a string literal: the call is
    turned into the shell text or command line it runs, which shell_calls reads."""
    tree = ast.parse(text)
    imported = {}  # local name -> (module, name), for `from subprocess import run as sh`
    for node in ast.walk(tree):
        if isinstance(node, ast.ImportFrom) and node.module in ('subprocess', 'os', 'asyncio', 'pty'):
            for alias in node.names:
                imported[alias.asname or alias.name] = (node.module, alias.name)
    found = []
    for node in ast.walk(tree):
        if not isinstance(node, ast.Call):
            continue
        func = node.func
        if isinstance(func, ast.Attribute):
            module, name = getattr(func.value, 'id', None), func.attr
        else:
            module, name = imported.get(getattr(func, 'id', None), (None, getattr(func, 'id', None)))
        args = list(node.args)
        keywords = {item.arg: item.value for item in node.keywords if item.arg}
        texts = []
        if name in PY_SHELL_TEXT:
            first = args[0] if args else keywords.get('cmd', keywords.get('command'))
            texts = [py_text(branch) for branch in branches(first)] if first is not None else []
        elif name == 'spawn' and module == 'pty':
            argv = args[0] if args else keywords.get('argv')
            texts = [argv_text(py_items(branch) or [branch]) for branch in branches(argv)] if argv is not None else []
        elif name in PY_ARGV:
            spec = PY_ARGV[name]
            if spec is None:
                first = args[0] if args else keywords.get('args')
                shell = keywords.get('shell')
                # shell= is on unless it is literally false (shell=1 and shell=flag run a shell).
                shell_on = shell is not None and not (isinstance(shell, ast.Constant) and not shell.value)
                for branch in branches(first) if first is not None else []:
                    listed = py_items(branch)
                    if shell_on:
                        texts.append(py_text(branch if listed is None else (listed[0] if listed else None)))
                    else:
                        texts.append(argv_text(listed if listed is not None else [branch]))
            else:
                program, start, shape = spec
                if len(args) <= program:
                    continue
                if shape == 'list':
                    lists = branches(args[start]) if len(args) > start else [None]
                    for listed in lists:
                        rest = [] if listed is None else py_items(listed)
                        rest = [None] if rest is None else rest[1:]
                        texts.append(argv_text([args[program]] + rest))
                elif shape == 'spread':
                    texts.append(argv_text([args[program]] + args[start + 1:]))
                else:
                    texts.append(argv_text([args[program]] + args[start:]))
        for shell_text in texts:
            if shell_text is not None:
                found.extend(external(shell_calls(shell_text, line + node.lineno - 1)))
    return found


def tracked(root):
    names = subprocess.run(['git', '-C', str(root), 'ls-files', '-z'], check=True,
                           stdout=subprocess.PIPE, stdin=subprocess.DEVNULL).stdout.decode().split('\0')
    return [name for name in names if name]


def classify(root, name):
    """(kind, `#!` line as (kind, program) or None): kind 'shell', 'python', 'other' or None.
    A `#!` line naming sh, bash, dash or python wins over the extension (a `.sh` file with a
    python `#!` runs as Python); otherwise `.sh`/`.bash` is shell and `.py` is Python."""
    path, bang = root / name, None
    if not path.is_symlink() and path.is_file():
        with path.open('rb') as handle:
            first = handle.readline(400).decode('utf-8', 'replace').rstrip('\n')
        if re.match(r'#!\s*/', first):
            bang = shebang(first)
    if bang and bang[0] in ('shell', 'python'):
        return bang[0], bang
    if name.endswith(('.sh', '.bash')):
        return 'shell', bang
    if name.endswith('.py'):
        return 'python', bang
    return (bang[0] if bang else None), bang


STANDARD_BIN = ('/usr/bin/', '/bin/', '/usr/sbin/', '/sbin/', '/usr/local/bin/')


def repo_path(word):
    """A literal relative path as a repository path (`./x` is `x`); None outside the repository."""
    if word.startswith(('/', '$', '"', "'")):
        return None
    path = posixpath.normpath(word)
    return None if path == '..' or path.startswith('../') else path


def problems(root):
    """Every file is read in the language it runs in: by its own kind, and also in the language
    of each call that runs or sources it by a literal path (`bash FILE`, `python3 FILE`,
    `. FILE`, `./FILE`). A tracked file a call runs, but that cannot be read, is named."""
    found, files = [], tracked(root)
    tracked_files = set(files)
    kinds = {name: classify(root, name) for name in files}
    readings, order = {}, []

    def want(name, lang):
        if (name, lang) not in readings:
            readings[(name, lang)] = None
            order.append((name, lang))

    for name in files:
        kind, bang = kinds[name]
        if kind in ('shell', 'python'):
            want(name, kind)
        elif kind == 'other':
            found.append(f'{name}: not read: line 1: its `#!` line names {bang[1]!r}, '
                         'which this reader does not read')
    position = 0
    while position < len(order):
        name, lang = order[position]
        position += 1
        label = name if kinds[name][0] == lang else f'{name} (run as {lang})'
        try:
            text = (root / name).read_text(encoding='utf-8')
            calls = shell_calls(text) if lang == 'shell' else python_calls(text)
        except (Unsupported, SyntaxError, UnicodeDecodeError) as error:
            readings[(name, lang)] = label
            found.append(f'{label}: not read: {error}')
            continue
        readings[(name, lang)] = calls
        for _, word, _ in calls:
            marker = next((m for m in RUNS if word.startswith(m)), None)
            if marker:
                target = repo_path(word[len(marker):])
                if target in tracked_files:
                    want(target, RUNS[marker])
            elif '/' in word:
                target = repo_path(word)
                if target in tracked_files and kinds[target][0] != 'other':
                    # Run as a program: its `#!` decides; without one, the shell runs it as a script.
                    want(target, kinds[target][0] if kinds[target][0] in ('shell', 'python') else 'shell')

    def functions_of(key, seen):
        calls = readings.get(key)
        if not isinstance(calls, list):
            return set()
        names = {word[2:] for _, word, _ in calls if word.startswith('()')}
        for _, word, _ in calls:
            if word.startswith('(src)'):
                target = repo_path(word[5:])
                if (target, 'shell') not in seen:
                    seen.add((target, 'shell'))
                    names |= functions_of((target, 'shell'), seen)
        return names

    for name, lang in order:
        calls = readings[(name, lang)]
        if not isinstance(calls, list):
            continue
        kind, bang = kinds[name]
        label = name if kind == lang else f'{name} (run as {lang})'
        if bang and kind == lang:
            calls = [(1, bang[1], True)] + calls
        functions = functions_of((name, lang), {(name, lang)})
        once = 0
        for line, word, by_prefix in calls:
            if word.startswith('()') or any(word.startswith(m) for m in RUNS):
                continue
            if word in BUILTINS or (word in functions and not by_prefix):
                continue
            if word in DECLARED:
                if word in ONLY_IN and name not in ONLY_IN[word]:
                    found.append(f'{label}:{line}: calls `{word}`, which only '
                                 f'{", ".join(sorted(ONLY_IN[word]))} may call')
                continue
            if word in DYNAMIC.get(name, ()):
                continue
            if DYNAMIC_ONCE.get(name) == word:
                once += 1
                if once == 1:
                    continue
            if '/' in word and not word.startswith(('$', '"', "'")):
                if word.startswith(STANDARD_BIN) and word.rsplit('/', 1)[-1] in IMAGE:
                    continue
                if repo_path(word) in tracked_files:
                    continue  # read above in the language it runs in (or named as not read)
            found.append(f'{label}:{line}: calls `{word}`, which the gate image does not declare '
                         '(scripts/script_commands.py)')
    return found


def main():
    found = problems(ROOT)
    for line in found:
        print('script commands: ' + line, file=sys.stderr)
    if found:
        return 1
    print('script commands: every command the tracked scripts call is declared for the gate image')
    return 0


if __name__ == '__main__':
    sys.exit(main())
