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
  program options of tar (`-I`, `-F`, `--use-compress-program`, `--to-command`,
  `--checkpoint-action exec=`, `--info-script`, `--new-volume-script`) and sort
  (`--compress-program`), and the text of `bash -c`, `sh -c`, `trap`, `env -S` and
  `python3 -c`. The options of a shell, Python, `.`, tar and sort are read as their own
  parsers read them: option groups (`bash -euo pipefail FILE`, `python3 -uW ignore FILE`,
  `tar -cIxz`), attached arguments, `--`, and long options by any unambiguous prefix
  (`--to-comm=`); tar's traditional dash-less first word (`tar cIf ...`) is not read;
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
`python3 FILE`, `. FILE`, `bash < FILE`, `< FILE bash`, `0< FILE sh`, `bash <>FILE`,
`python3 - < FILE`, `bash -- < FILE`) is read in the language it runs in; standard input on the
simple command itself (`<`, `0<`, `<>`, `0<>`, or a here-string `<<< TEXT`, whose TEXT is read)
counts as the script only when the shell or Python got no script file and no `-c`. It is
read with or without an extension or a `#!` line; the functions of a sourced file count for the
file that sources it. A file another file runs by a literal path may not be one that ONLY_IN lets
call commands the gate image lacks (it is named).

Stated limits (each needs a deliberate author): a Python call whose program is not a literal
(a variable, `sys.executable`, an argument list built by a call other than `'...'.split()` and
`shlex.split('...')`, a process function bound to another name by assignment) is not resolved;
a script run or sourced through a non-literal path, a pipe (`cat FILE | bash`) or a process
substitution (`bash <(...)`, `. <(...)`) is not read, nor standard input the shell or Python
inherits from elsewhere: a redirect on a group, subshell or loop (`{ bash; } < FILE`,
`( bash ) < FILE`, `done < FILE`), an earlier `exec < FILE`, a duplicated descriptor
(`exec 3< FILE` then `bash <&3`), or a Python `stdin=` argument; a data here-document that a later line
runs is not read; a command reached only through a non-literal path is pinned by
DYNAMIC, not resolved; commands run by another program's own language or configuration are not
read: a program in IMAGE told to run a command by an argument, an option, its configuration or
an environment variable beyond the spellings above (awk `system()`, `print |` and `getline`;
sed's `e`; `git bisect run`, `git rebase -x`, `git submodule foreach`, `--upload-pack`, Git's
pager, editor and alias settings, `GIT_SSH_COMMAND`; `cargo --config` runners, `RUSTC_WRAPPER`,
`rustc -C linker=`; `BASH_ENV`); a shell function
excuses a call of its name anywhere in the file, also a call before the definition, which bash
would run as the real command.

usage: python3 -I scripts/script_commands.py     (exit 1 names each undeclared call)
"""
import ast
import os
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
# od: measured 2026-11-16 by Keel in the same image (8f4ea620d021): compgen -c lists it, /usr/bin/od,
# GNU coreutils 9.10 (scripts/tests/release.sh reads gzip header bytes with it; Bastion's LOW).
IMAGE = frozenset("""
    bash sh env cat cp mv rm ln ls mkdir rmdir chmod touch mktemp dirname basename realpath readlink
    cut head tail sort uniq tr wc tee sha256sum date sleep stat seq id uname nproc od
    sed grep awk tar gzip find xargs curl git python3 cargo rustc rustup rustfmt ssh-keygen
    timeout nice nohup
""".split())
# Commands only the named files may call, and why the gate never needs them there.
ONLY_IN = {
    # ci_local runs these steps only after `command -v` finds the tool, and names them as not
    # run otherwise (tool-versions.toml [local_gate] hosted_only). Not in the gate image.
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
    'env': frozenset({'-a', '--argv0', '-u', '--unset', '-C', '--chdir'}),
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
# Long options whose value is a command line the program runs (`tar --to-command=...`), read
# by any unambiguous prefix as GNU getopt does; tar's short options read as a getopt cluster.
PROGRAM_LONG = {'tar': ('use-compress-program', 'to-command', 'info-script', 'new-volume-script'),
                'sort': ('compress-program',)}
TAR_SHORT_ARGUMENT = frozenset('bCfFgHIKLNTVX')
TAR_SHORT_PROGRAM = frozenset('IF')
PYTHON_LONG_ARGUMENT = frozenset({'--check-hash-based-pycs'})
SHELLS = frozenset({'sh', 'bash', 'dash'})
PYTHONS = re.compile(r'python(?:3(?:\.[0-9]+)?)?')
# A non-literal Python argument stands for this dynamic word in the command line it makes.
PY_DYNAMIC = '"$python_expression"'
OPERATORS = (';;&', ';;', ';&', '&&', '||', '|&', '&', '|', ';', '(', ')', '\n')
REDIRECTS = ('<<<', '<<-', '<<', '&>>', '&>', '>>', '>&', '<&', '<>', '>|', '>', '<')
FD_REDIRECT = re.compile(r'([0-9]+)(?=<|>)')
ASSIGNMENT = re.compile(r'[A-Za-z_][A-Za-z0-9_]*(?:\[[^]]*\])?\+?=')
# `env NAME=value` and `env "$name=value"`: a name, literal or one parameter, then `=`.
ENV_ASSIGNMENT = re.compile(r'["\']?(?:[A-Za-z_][A-Za-z0-9_]*|\$\{?[A-Za-z_][A-Za-z0-9_]*\}?)=')
# Non-literal command words, pinned by file and by the exact text as written. Each runs a file
# this repository builds, fetches or tracks, or a function's own arguments.
DYNAMIC = {
    # The versioned hooks run the repository's own gate scripts (read here as tracked files).
    '.githooks/pre-commit': {'"$root/scripts/ci_fast.sh"'},
    '.githooks/pre-push': {'"$root/scripts/ci_local.sh"'},
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
        self.argument = value


class Lexer:
    """One pass over shell text: tokens, here-document bodies and nested substitution calls."""

    def __init__(self, text, line, sink):
        self.text, self.i, self.line, self.sink = text, 0, line, sink

    @staticmethod
    def call(line, word, external, tokens=None, start=0, directory=None):
        return line, word, external

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
            elif FD_REDIRECT.match(text, self.i) or (
                    text.startswith(REDIRECTS, self.i) and not (c == '&' and text.startswith('&&', self.i))):
                # `0< FILE`, `2>/dev/null`: the descriptor number is part of the redirect, not a word.
                fd = FD_REDIRECT.match(text, self.i)
                if fd:
                    self.i = fd.end(1)
                op = next(r for r in REDIRECTS if text.startswith(r, self.i))
                self.i += len(op)
                out.append(('redirect', op, self.line, fd.group(1) if fd else None))
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
        if not quoted:
            body = self.unquoted_heredoc(body, start)
        return ('heredoc', body, start, index)

    def unquoted_heredoc(self, body, start):
        if '$(' in body or '`' in body:
            type(self)(body, start, self.sink).double_quoted(stop=None)
        return body

    def substitution(self):
        start = self.line
        nested = type(self)(self.text, self.line, self.sink)
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
        self.sink(type(self)(inner, start, self.sink).tokens())
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
                type(self)(inner, self.line, self.sink).double_quoted(stop=None)
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
                following = text[self.i + 1:self.i + 2]
                if following in ('$', '`', '\\', '\n') or (stop and following == stop):
                    if following != '\n':
                        chunk.append(following)
                    self.line += following == '\n'
                    self.i += 2
                else:
                    chunk.append(c)
                    self.i += 1
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
        return ''.join(chunk) if literal else None

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


class WorkflowLexer(Lexer):
    workspace = ROOT
    unknown = '\0dynamic'
    variable = re.compile(r'\$(?:GITHUB_WORKSPACE(?![A-Za-z0-9_])|\{GITHUB_WORKSPACE\})')

    @classmethod
    def actions_text(cls, text):
        return re.sub(r'\$\{\{\s*github\.workspace\s*\}\}', lambda _: str(cls.workspace), text)

    @classmethod
    def location(cls, directory, value):
        if not value or cls.unknown in value or '${{' in value:
            raise Unsupported('cannot resolve gate command directory before Just installation')
        path = Path(os.path.normpath(directory / value))
        if not path.is_relative_to(cls.workspace):
            raise Unsupported('gate command directory is outside the workspace before Just installation')
        return path

    def dollar(self):
        match = self.variable.match(self.text, self.i)
        if match:
            self.i = match.end()
            return str(self.workspace)
        value = super().dollar()
        return self.unknown if value is None else value

    def word(self):
        word = super().word()
        if word.value and self.unknown in word.value:
            word.value, word.literal = None, False
        return word

    def unquoted_heredoc(self, body, start):
        source = type(self)(body, start, self.sink).double_quoted(stop=None)
        if source is None or self.unknown in source:
            raise Unsupported('cannot resolve heredoc expansion before Just installation')
        return source

    @staticmethod
    def call(line, word, external, tokens=None, start=0, directory=None):
        if tokens is None:
            return line, word, external, None, directory
        arguments = []
        while start < len(tokens):
            token = tokens[start]
            if token[0] == 'redirect':
                start += 2
            elif token[0] == 'word':
                arguments.append(token[1].argument)
                start += 1
            else:
                break
        return line, word, external, arguments, directory


def shell_calls(text, line=1, lexer=Lexer, directory=None):
    """(line, word, external) for each command word of the shell text, nested substitutions
    included. external: the word is run by a prefix or by find, so no shell function can be it."""
    calls = []

    def sink(tokens):
        calls.extend(command_words(tokens, calls, lexer, directory))

    sink(lexer(text, line, sink).tokens())
    return calls


def external(calls):
    """The same calls, none of which a shell function can answer (a prefix or Python runs them)."""
    return [(line, word, True, *args) for line, word, _, *args in calls if not word.startswith('()')]


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


OPERAND_STATES = SHELLS | {'.', 'source'} | set(PROGRAM_LONG)


def long_option(value, names):
    """The one name of `names` that `--value` spells (GNU getopt takes any unambiguous prefix)."""
    option = value[2:].split('=', 1)[0]
    hits = [name for name in names if len(option) >= 2 and name.startswith(option)]
    return hits[0] if len(hits) == 1 else None


def operand_word(state, value, raw):
    """One word after a program that runs a file or a command line it is given: a shell, Python,
    `.`/`source`, tar or sort, read as that program's own option parser reads it (option groups,
    attached arguments, `--`, long-option prefixes). value is None for a non-literal word.
    Returns (next state, events); an event is (kind, text): 'shell' or 'python' text to read,
    'dynamic' (a non-literal command line, named), or a RUNS marker with the file it runs."""
    if isinstance(state, tuple):
        kind = state[0]
        if kind == 'option-argument':
            return (state[1] if state[2] <= 1 else ('option-argument', state[1], state[2] - 1)), []
        if kind == 'file-next':
            return None, ([(state[1], value)] if value is not None else [])
        if value is None:
            return state[1], [('dynamic', raw)]
        if kind == 'program-option':
            return state[1], [('shell', value)]
        return state[1], ([('shell', value[5:])] if value.startswith('exec=') else [])  # checkpoint
    if value is None:
        return (None if state in SHELLS or state in ('.', 'source') or PYTHONS.fullmatch(state) else state), []
    if state in SHELLS:
        if value == '--':
            return ('file-next', '(sh)'), []
        if value in ('--rcfile', '--init-file'):
            return ('option-argument', state, 1), []
        if re.fullmatch(r'[-+][A-Za-z]+', value):
            letters = value[1:]
            count = sum(letter in 'oO' for letter in letters)  # each -o/-O takes the next word
            if value[0] == '-' and 'c' in letters:
                return (('option-argument', 'next', count) if count else 'next'), []
            if value[0] == '-' and 's' in letters:
                return 'stdin-sh', []  # the script is standard input; later words are arguments
            return (('option-argument', state, count) if count else state), []
        if value.startswith(('-', '+')):
            return state, []
        return None, [('(sh)', value)]
    if state in ('.', 'source'):
        return (state, []) if value == '--' else (None, [('(src)', value)])
    if state in PROGRAM_LONG:
        if value.startswith('--'):
            option, equals, argument = value[2:].partition('=')
            if long_option(value, PROGRAM_LONG[state]):
                return (state, [('shell', argument)]) if equals else (('program-option', state), [])
            if state == 'tar' and option.startswith('checkpoint-') and 'checkpoint-action'.startswith(option):
                if equals:
                    return state, ([('shell', argument[5:])] if argument.startswith('exec=') else [])
                return ('checkpoint-option', state), []
            return state, []
        if state == 'tar' and re.fullmatch(r'-[A-Za-z].*', value):
            letters = value[1:]
            for index, letter in enumerate(letters):
                if letter in TAR_SHORT_ARGUMENT:
                    rest = letters[index + 1:]
                    if letter in TAR_SHORT_PROGRAM:
                        return (state, [('shell', rest)]) if rest else (('program-option', state), [])
                    return (state if rest else ('option-argument', state, 1)), []
        return state, []
    # Python.
    if value == '--':
        return ('file-next', '(py)'), []
    if value == '-':
        return 'stdin-py', []  # the script is standard input; later words are arguments
    if value in PYTHON_LONG_ARGUMENT:
        return ('option-argument', state, 1), []
    if value.startswith('--'):
        return state, []
    if value.startswith('-'):
        letters = value[1:]
        for index, letter in enumerate(letters):
            if letter in 'cmWX':
                rest = letters[index + 1:]
                if letter == 'c':
                    return (None, [('python', rest)]) if rest else ('next-python', [])
                if letter == 'm':
                    return None, []
                return (state if rest else ('option-argument', state, 1)), []
        return state, []
    return None, [('(py)', value)]


def stdin_runner(script):
    """The RUNS marker when the command's standard input is its script: a shell or Python that got
    no script file and no `-c` (also after `-s`, `-` or a `--` with no file after it, or with an
    option argument pending)."""
    if isinstance(script, tuple) and script[0] == 'file-next' and script[1] in ('(sh)', '(py)'):
        return script[1]  # `bash -- < FILE`, `python3 -- < FILE`: no file came after `--`
    base = script[1] if isinstance(script, tuple) and script[0] == 'option-argument' else script
    if base in SHELLS or base == 'stdin-sh':
        return '(sh)'
    if base == 'stdin-py' or (isinstance(base, str) and PYTHONS.fullmatch(base)):
        return '(py)'
    return None


def flush_stdin(script, stdin_words, found, calls, lexer, directory):
    """At the end of a simple command: `bash < FILE`, `< FILE bash`, `0< FILE sh` run FILE, and
    `bash <<< 'TEXT'` runs TEXT. Returns the emptied list."""
    runner = stdin_runner(script) if stdin_words else None
    for operator, word in stdin_words if runner else []:
        if operator == '<<<' and word.literal:
            text = word.value
            read = shell_calls if runner == '(sh)' else python_calls
            calls.extend(read(text, word.line, lexer, directory))
        elif directory is not None and not word.literal:
            raise Unsupported('cannot resolve gate script input before Just installation')
        elif operator == '<<<':
            found.append(lexer.call(word.line, word.raw, True, directory=directory))  # a dynamic script text is named
        elif word.literal:
            found.append(lexer.call(word.line, runner + word.value, True, directory=directory))
    return []


def command_words(tokens, calls, lexer, directory):
    initial_directory = heredoc_directory = directory
    found, functions = [], set()
    expect, mode, prefix, owner, script = True, None, None, None, None
    finding = by_find = skip_parens = after_coproc = False
    stdin_words = []  # (operator, word) of `<`/`<>`/`<<<` in the current simple command
    for index, token in enumerate(tokens):
        kind = token[0]
        if kind == 'heredoc':
            body, start, consumer = token[1], token[2], owner
            first = body.split('\n', 1)[0]
            if first.startswith('#!'):
                consumer = {'shell': 'bash', 'python': 'python3'}.get(shebang(first)[0])
            if consumer in SHELLS:
                calls.extend(shell_calls(body, start, lexer, heredoc_directory))
            elif consumer and PYTHONS.fullmatch(consumer):
                calls.extend(python_calls(body, start, lexer, heredoc_directory))
            continue
        if kind == 'redirect':
            # Standard input (`<`, `0<`, `<>`, `0<>`, `<<<`), before or after the command word: decided at the
            # end of the simple command, when it is known whether a shell or Python got a script.
            stdin = token[1] in ('<', '<>', '<<<') and (len(token) < 4 or token[3] in (None, '0'))
            mode = ('stdin-target', mode, token[1]) if stdin else ('target', mode)
            continue
        if kind == 'op':
            op = token[1]
            heredoc_directory = directory
            stdin_words = flush_stdin(script, stdin_words, found, calls, lexer, directory)
            directory = initial_directory
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
        if mode == 'env-directory':
            directory = lexer.location(directory, word.value)
            mode = None
            continue
        if mode and mode[0] in ('target', 'stdin-target'):
            if mode[0] == 'stdin-target':
                stdin_words.append((mode[2], word))
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
                    if directory is not None:
                        raise Unsupported('cannot resolve inline gate command before Just installation')
                    found.append(lexer.call(word.line, word.raw, True, directory=directory))
                elif script == 'next':
                    calls.extend(shell_calls(word.value, word.line, lexer, directory))
                else:
                    calls.extend(python_calls(word.value, word.line, lexer, directory))
                script = None
            elif script == 'rustup' and word.literal and word.value == 'run':
                script = 'rustup-toolchain'
            elif script == 'rustup-toolchain':
                # rustup's `run` subcommand: the word after the toolchain is a command.
                expect, prefix, script, by_find = True, None, None, True
            elif isinstance(script, tuple) or script in OPERAND_STATES or (script and PYTHONS.fullmatch(script)):
                if directory is not None and not word.literal and (
                        script in SHELLS or script in ('.', 'source') or
                        isinstance(script, str) and PYTHONS.fullmatch(script) or
                        isinstance(script, tuple) and script[0] == 'file-next'):
                    raise Unsupported('cannot resolve gate script operand before Just installation')
                script, events = operand_word(script, word.value if word.literal else None, word.raw)
                for event, text in events:
                    if event == 'shell':
                        calls.extend(external(shell_calls(text, word.line, lexer, directory)))
                    elif event == 'python':
                        calls.extend(python_calls(text, word.line, lexer, directory))
                    elif event == 'dynamic':
                        if directory is not None:
                            raise Unsupported('cannot resolve inline gate command before Just installation')
                        found.append(lexer.call(word.line, text, True, directory=directory))
                    else:
                        found.append(lexer.call(word.line, event + text, True, tokens, index + 1, directory))
            elif script == 'shopt' and word.literal and word.value == 'expand_aliases':
                raise Unsupported(f'line {word.line}: `shopt expand_aliases` (aliases are not read)')
            continue
        if prefix in PREFIXES and word.literal and word.value.startswith('-'):
            option, equals, argument = word.value.partition('=')
            if (directory is not None and prefix == 'env' and (word.value == '-C' or
                    (option.startswith('--') and len(option) > 2 and '--chdir'.startswith(option)))):
                if equals:
                    directory = lexer.location(directory, argument)
                else:
                    mode = 'env-directory'
            elif (directory is not None and prefix == 'env' and (re.match(r'^-[iv0]*S', option)
                  or option.startswith('--') and len(option) > 2 and '--split-string'.startswith(option))):
                raise Unsupported('cannot resolve split env options before Just installation')
            elif word.value in PREFIX_OPTIONS[prefix]:
                mode = ('target', mode)
            elif prefix == 'env' and word.value in ('-S', '--split-string'):
                expect, script = False, 'next'
            elif prefix == 'env' and word.value.startswith(('-S', '--split-string=')):
                text = word.value[2:] if word.value.startswith('-S') else word.value.split('=', 1)[1]
                calls.extend(external(shell_calls(text, word.line, lexer, directory)))
                expect = False
            elif prefix == 'command' and word.value in ('-v', '-V'):
                expect, prefix = False, 'probe'
            elif not word.value.startswith('--'):
                # A cluster (`-0rn 1`, `-iu X`): the first letter that takes an argument takes the
                # rest of the cluster, or the next word when it ends the cluster.
                for offset, letter in enumerate(word.value[1:], 2):
                    if '-' + letter in PREFIX_OPTIONS[prefix]:
                        if directory is not None and prefix == 'env' and letter == 'C':
                            if offset == len(word.value):
                                mode = 'env-directory'
                            else:
                                directory = lexer.location(directory, word.value[offset:])
                        elif offset == len(word.value):
                            mode = ('target', mode)
                        break
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
        if directory is not None and (not word.literal or value in ('cd', 'pushd', 'popd')):
            raise Unsupported('cannot resolve gate command location before Just installation')
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
        found.append(lexer.call(word.line, value if word.literal else word.raw, prefix is not None or by_find,
                                tokens, index + 1, directory))
        # /usr/bin/env runs its operand as env does; an absolute path acts as its file name.
        owner = value.rsplit('/', 1)[-1] if word.literal else None
        prefix = owner if owner in PREFIXES else None
        script = 'next' if owner == 'trap' else owner
        finding = finding or owner == 'find'
        by_find = False
        expect = prefix is not None
    flush_stdin(script, stdin_words, found, calls, lexer, directory)
    return found + [lexer.call(0, '()' + name, False, directory=directory) for name in functions]


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


def python_calls(text, line=1, lexer=Lexer, directory=None):
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
        if directory is not None and name in ('chdir', 'fchdir'):
            raise Unsupported('cannot resolve Python directory changes before Just installation')
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
        cwd = directory
        if directory is not None and texts and keywords.get('cwd') is not None:
            value = keywords['cwd']
            if not isinstance(value, ast.Constant) or (value.value is not None and not isinstance(value.value, str)):
                raise Unsupported('cannot resolve Python process directory before Just installation')
            if value.value is not None:
                cwd = lexer.location(directory, value.value)
        for shell_text in texts:
            if shell_text is not None:
                found.extend(external(shell_calls(shell_text, line + node.lineno - 1, lexer, cwd)))
            elif directory is not None:
                raise Unsupported('cannot resolve Python process command before Just installation')
    return found


# `.git` is never a tracked path at any depth; build output is skipped only at the root, since
# Git can track `x/target/f`.
ROOT_SKIPPED_DIRS = frozenset({'target', 'mutants.out', 'mutants.out.old'})
# Git's own list of repository-local variables (`git rev-parse --local-env-vars`, Git 2.56.1):
# inherited, they would make the Git calls below answer for another repository or index.
GIT_LOCAL_ENV = frozenset({
    'GIT_ALTERNATE_OBJECT_DIRECTORIES', 'GIT_CONFIG', 'GIT_CONFIG_PARAMETERS', 'GIT_CONFIG_COUNT',
    'GIT_OBJECT_DIRECTORY', 'GIT_DIR', 'GIT_WORK_TREE', 'GIT_IMPLICIT_WORK_TREE', 'GIT_GRAFT_FILE',
    'GIT_INDEX_FILE', 'GIT_NO_REPLACE_OBJECTS', 'GIT_REPLACE_REF_BASE', 'GIT_PREFIX',
    'GIT_SHALLOW_FILE', 'GIT_COMMON_DIR'})


def tracked(root):
    """Git's tracked files when `root` is the top of a Git work tree; otherwise (a source export,
    such as the hosted-parity export phase, which holds exactly the tracked files) every file
    below `root`, a symlink as a file: a tree without Git is read, never skipped."""
    env = {key: value for key, value in os.environ.items()
           if key not in GIT_LOCAL_ENV and not key.startswith('GIT_CONFIG_KEY_')
           and not key.startswith('GIT_CONFIG_VALUE_')}
    try:
        top = subprocess.run(['git', '-C', str(root), 'rev-parse', '--show-toplevel'], env=env,
                             stdin=subprocess.DEVNULL, capture_output=True, check=False)
    except OSError:
        top = None
    if top is not None and top.returncode == 0 and Path(top.stdout.decode().strip()).resolve() == Path(root).resolve():
        names = subprocess.run(['git', '-C', str(root), 'ls-files', '-z'], check=True, env=env,
                               stdout=subprocess.PIPE, stdin=subprocess.DEVNULL).stdout.decode().split('\0')
        return [name for name in names if name]
    found = []
    for directory, subdirectories, files in os.walk(root):
        base = Path(directory).relative_to(root)
        links = [d for d in subdirectories if (Path(directory) / d).is_symlink()]
        subdirectories[:] = sorted(d for d in subdirectories if d != '.git' and d not in links
                                   and not (base == Path('.') and d in ROOT_SKIPPED_DIRS))
        found.extend((base / name).as_posix() for name in files + links)
    return sorted(found)


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
        for line, word, _ in calls:
            marker = next((m for m in RUNS if word.startswith(m)), None)
            target = repo_path(word[len(marker):] if marker else word) if (marker or '/' in word) else None
            if target not in tracked_files:
                continue
            if marker:
                want(target, RUNS[marker])
            elif kinds[target][0] != 'other':
                # Run as a program: its `#!` decides; without one, the shell runs it as a script.
                want(target, kinds[target][0] if kinds[target][0] in ('shell', 'python') else 'shell')
            privileged = sorted(command for command, owners in ONLY_IN.items() if target in owners)
            if privileged and target != name:
                found.append(f'{label}:{line}: runs `{target}`, which alone may call '
                             f'{", ".join(privileged)} (not in the gate image)')

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
