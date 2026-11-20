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
  each branch of a conditional argument list, and both shell modes when the option is dynamic.
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

Stated limits (each needs a deliberate author): a script run or sourced through a non-literal path, a pipe (`cat FILE | bash`) or a process
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
import _imp
import ast
import builtins
import hashlib
from collections import namedtuple
from contextvars import ContextVar
from importlib import import_module
import os
from pathlib import Path
import posixpath
import re
import shlex
import subprocess
import sys
import unittest

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
DECLARED = IMAGE | frozenset(ONLY_IN) | {'just'}

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
PY_UNKNOWN = '"${\0python_argument}"'
PY_DYNAMIC = '"$python_expression"'
PY_SPREAD = '"${\0python_spread}"'
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
PY_SHELL_TEXT = frozenset({
    'os.system', 'os.popen', 'subprocess.getoutput', 'subprocess.getstatusoutput', 'asyncio.create_subprocess_shell',
})
PY_ARGV = {
    # subprocess: one list (or a string, which is the program; with shell=True, shell text).
    'subprocess.run': None, 'subprocess.call': None, 'subprocess.check_call': None,
    'subprocess.check_output': None, 'subprocess.Popen': None,
    # os.exec*/posix_spawn*: program, then an argv list whose first item is only argv[0].
    'os.execv': (0, 'list', None, None), 'os.execve': (0, 'list', 'path', 'argv'),
    'os.execvp': (0, 'list', 'file', 'args'), 'os.execvpe': (0, 'list', 'file', 'args'),
    'os.posix_spawn': (0, 'list', None, None), 'os.posix_spawnp': (0, 'list', None, None),
    'os.execl': (0, 'spread', None, None), 'os.execle': (0, 'spread', None, None),
    'os.execlp': (0, 'spread', None, None), 'os.execlpe': (0, 'spread', None, None),
    # os.spawn*: mode first, then the program.
    'os.spawnv': (1, 'list', 'file', 'args'), 'os.spawnve': (1, 'list', 'file', 'args'),
    'os.spawnvp': (1, 'list', 'file', 'args'), 'os.spawnvpe': (1, 'list', 'file', 'args'),
    'os.spawnl': (1, 'spread', None, None), 'os.spawnle': (1, 'spread', None, None),
    'os.spawnlp': (1, 'spread', None, None), 'os.spawnlpe': (1, 'spread', None, None),
}


class Unsupported(ValueError):
    location = None
    lineno = None


class Word:
    def __init__(self, line, raw, value, literal, lexer):
        self.line, self.raw, self.value, self.literal = line, raw, value, literal
        self.argument = value
        self.lexer = lexer


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
                out.append(('word', Word(self.line, '<(...)', None, False, type(self))))
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
        body_lexer = type(self)
        if not quoted:
            body, body_lexer = self.unquoted_heredoc(body, start)
        return ('heredoc', body, start, index, body_lexer)

    def unquoted_heredoc(self, body, start):
        if '$(' in body or '`' in body:
            type(self)(body, start, self.sink).double_quoted(stop=None)
        return body, type(self)

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
        return Word(line, text[start:self.i], ''.join(value) if literal else None, literal, type(self))


class WorkflowLexer(Lexer):
    workspace = ROOT
    unknown = '\0dynamic'
    actions_workspace = re.compile(r'\$\{\{\s*github\.workspace\s*\}\}')
    expanded_workspace = False
    variable = re.compile(r'\$(?:GITHUB_WORKSPACE(?![A-Za-z0-9_])|\{GITHUB_WORKSPACE\})')

    @classmethod
    def path(cls, directory, value):
        if not value or cls.unknown in value:
            raise Unsupported('cannot resolve gate command path before Just installation')
        match = cls.actions_workspace.match(value)
        if not match and cls.expanded_workspace:
            match = cls.variable.match(value)
        if match:
            suffix = value[match.end():]
            if suffix and not suffix.startswith('/'):
                raise Unsupported('cannot resolve workspace path before Just installation')
            directory, value = cls.workspace, suffix.lstrip('/')
        if '${{' in value:
            raise Unsupported('cannot resolve gate command path before Just installation')
        return Path(os.path.normpath(directory / value))

    @classmethod
    def location(cls, directory, value):
        path = cls.path(directory, value)
        if not path.is_relative_to(cls.workspace):
            raise Unsupported('gate command directory is outside the workspace before Just installation')
        return path

    def dollar(self):
        match = self.actions_workspace.match(self.text, self.i) or self.variable.match(self.text, self.i)
        if match:
            self.i = match.end()
            if match.re is self.variable:
                self.has_workspace = True
            return match.group()
        value = super().dollar()
        return self.unknown if value is None else value

    def word(self):
        self.has_workspace = False
        word = super().word()
        word.lexer = ExpandedWorkflowLexer if self.has_workspace else type(self)
        if word.value and self.unknown in word.value:
            word.value, word.literal = None, False
        return word

    def unquoted_heredoc(self, body, start):
        reader = type(self)(body, start, self.sink)
        source = reader.double_quoted(stop=None)
        if source is None or self.unknown in source:
            raise Unsupported('cannot resolve heredoc expansion before Just installation')
        return source, ExpandedWorkflowLexer if getattr(reader, 'has_workspace', False) else type(self)

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
                if token[1].raw == PY_SPREAD:
                    return line, word, external, None, directory
                arguments.append(token[1].argument)
                start += 1
            else:
                break
        return line, word, external, arguments, directory


class ExpandedWorkflowLexer(WorkflowLexer):
    expanded_workspace = True


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


def operand_word(state, value, raw, workflow=False):
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
                    if workflow:
                        return (None, [('(module)', rest)]) if rest else (('file-next', '(module)'), [])
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
            calls.extend(read(text, word.line, word.lexer, directory))
        elif directory is not None and not word.literal:
            raise Unsupported('cannot resolve gate script input before Just installation')
        elif operator == '<<<':
            found.append(lexer.call(word.line, word.raw, True, directory=directory))  # a dynamic script text is named
        elif word.literal:
            path = str(word.lexer.path(directory, word.value)) if directory is not None else word.value
            found.append(lexer.call(word.line, runner + path, True, directory=directory))
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
            body_lexer = token[4]
            first = body.split('\n', 1)[0]
            if first.startswith('#!'):
                consumer = {'shell': 'bash', 'python': 'python3'}.get(shebang(first)[0])
            if consumer in SHELLS:
                calls.extend(shell_calls(body, start, body_lexer, heredoc_directory))
            elif consumer and PYTHONS.fullmatch(consumer):
                calls.extend(python_calls(body, start, body_lexer, heredoc_directory))
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
        if word.raw in (PY_SPREAD, PY_UNKNOWN) and (expect or finding or isinstance(script, tuple) or
                script in OPERAND_STATES or script in ('next', 'next-python', 'rustup', 'rustup-toolchain') or
                script and PYTHONS.fullmatch(script)):
            if word.raw == PY_UNKNOWN:
                raise Unsupported('cannot resolve wrapped Python process argument')
            raise Unsupported('cannot resolve starred Python process arguments')
        skip_parens = False
        if mode == 'array':
            continue
        if mode == 'env-directory':
            directory = word.lexer.location(directory, word.value)
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
                    calls.extend(shell_calls(word.value, word.line, word.lexer, directory))
                else:
                    calls.extend(python_calls(word.value, word.line, word.lexer, directory))
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
                script, events = operand_word(script, word.value if word.literal else None, word.raw, directory is not None)
                for event, text in events:
                    if event == 'shell':
                        calls.extend(external(shell_calls(text, word.line, word.lexer, directory)))
                    elif event == 'python':
                        calls.extend(python_calls(text, word.line, word.lexer, directory))
                    elif event == 'dynamic':
                        if directory is not None:
                            raise Unsupported('cannot resolve inline gate command before Just installation')
                        found.append(lexer.call(word.line, text, True, directory=directory))
                    else:
                        if directory is not None and event in RUNS:
                            text = str(word.lexer.path(directory, text))
                        found.append(lexer.call(word.line, event + text, True, tokens, index + 1, directory))
            elif script == 'shopt' and word.literal and word.value == 'expand_aliases':
                raise Unsupported(f'line {word.line}: `shopt expand_aliases` (aliases are not read)')
            continue
        if prefix in PREFIXES and word.literal and word.value.startswith('-'):
            option, equals, argument = word.value.partition('=')
            if (directory is not None and prefix == 'env' and (word.value == '-C' or
                    (option.startswith('--') and len(option) > 2 and '--chdir'.startswith(option)))):
                if equals:
                    directory = word.lexer.location(directory, argument)
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
                calls.extend(external(shell_calls(text, word.line, word.lexer, directory)))
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
                                directory = word.lexer.location(directory, word.value[offset:])
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
        command = value if word.literal else word.raw
        if directory is not None and '/' in command:
            command = str(word.lexer.path(directory, command))
        found.append(lexer.call(word.line, command, prefix is not None or by_find,
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


def expanded_items(items):
    result = []
    for item in items:
        if isinstance(item, ast.Starred):
            expanded = py_items(item.value)
            result.extend([item] if expanded is None else expanded)
        else:
            result.append(item)
    return result


def py_items(node):
    if isinstance(node, (ast.List, ast.Tuple)):
        return expanded_items(node.elts)
    if isinstance(node, ast.BinOp) and isinstance(node.op, ast.Add):
        left = py_items(node.left)
        if left is not None:
            right = py_items(node.right)
            if right is None and not left:
                return None
            return left + (right if right is not None else [ast.Starred(value=node.right)])
    return None


def py_text(node):
    if isinstance(node, ast.Constant) and isinstance(node.value, str):
        return node.value
    return None


def py_word(node, strict):
    if isinstance(node, ast.Starred):
        return PY_SPREAD
    if isinstance(node, ast.Constant) and isinstance(node.value, str):
        return shlex.quote(node.value)
    return PY_UNKNOWN if strict else PY_DYNAMIC


def argv_text(items, strict=False):
    """Resolve the effective program before serializing its arguments."""
    head = items[0] if items else None
    if isinstance(head, ast.Starred):
        raise Unsupported('cannot resolve starred Python process arguments')
    if isinstance(head, FixtureExecutable):
        return head
    if not (isinstance(head, ast.Constant) and isinstance(head.value, str) and head.value):
        raise Unsupported('cannot resolve Python process executable')
    return ' '.join(py_word(item, strict) for item in items)


class PythonHelper:
    def __init__(self, node, frames, decorator, source):
        self.node = node
        self.decorator = decorator
        self.source = source
        self.frames = frames
        self.called = False
        self.summaries = {}
        self.managed_body = None


class PythonClass:
    def __init__(self, node, bases, skipped):
        custom = bool(node.keywords)
        possible = set().union(*bases)
        self.test_case = any(base == 'unittest.TestCase' or isinstance(base, PythonClass)
                             and base.test_case is True for base in possible)
        self.skipped = skipped or any(isinstance(base, PythonClass) and base.skipped for base in possible)
        self.lineage = (self,)
        self.base = 'builtins.object'
        for choices in bases:
            if len(choices) != 1:
                custom = True
                continue
            base, = choices
            if isinstance(base, PythonClass):
                if any(ancestor in self.lineage for ancestor in base.lineage):
                    custom = True
                if self.base != 'builtins.object' and any(
                        self.base != 'unittest.TestCase' or hasattr(unittest.TestCase, name)
                        for cls in base.lineage for name in cls.members if isinstance(name, str)):
                    custom = True
                self.lineage += base.lineage
                base = base.base
            if base != 'builtins.object':
                if self.base != 'builtins.object':
                    custom = True
                self.base = base
        if custom:
            self.base = UNKNOWN_CALLABLE
        if not self.test_case and self.base not in ('builtins.object', 'builtins.str', 'builtins.ValueError',
                                                    'builtins.OSError', 'ast.expr'):
            self.test_case = None
        self.members = {}


class PythonInstance:
    def __init__(self, owner):
        self.owner = owner


class FixtureDescriptor(ast.expr):
    def __init__(self, paths):
        self.paths = paths


class ChangedTail(ast.Starred):
    pass


class FixtureExecutable(ast.expr):
    def __init__(self, descriptor, named=False):
        self.descriptor = descriptor
        self.named = named


class VerifiedCommand(str):
    pass


class LookupExecutable(ast.expr):
    def __init__(self, program):
        self.program = program


class CapturedArgument(ast.expr):
    def __init__(self, owners):
        self.owners = frozenset(owners)


BoundCall = namedtuple('BoundCall', ('target', 'receiver', 'args', 'keywords'), defaults=(None, (), ()))


UNKNOWN_CALLABLE = object()
UNKNOWN_PROVIDER = object()
MISSING_FIELD = object()
MISSING_NAME = object()
EXCEPTION = object()
SKIPPED_HANDLER = object()
STANDARD_CONTEXT = object()
UNRESOLVED_CONTEXT = object()
LIST_MUTATIONS = frozenset({'append', 'extend', 'insert', 'pop', 'remove', 'clear', 'sort', 'reverse',
                            '__iadd__', '__imul__', '__setitem__', '__delitem__'})
FILE_CONTEXT = object()
FILESYSTEM_PATH = object()
PRIVATE_FIXTURE_HOME = object()
NONSUPPRESSING_CONTEXT = object()
GENERATOR_DECORATORS = ('contextlib.contextmanager', 'contextlib.asynccontextmanager')
GeneratorManager = namedtuple('GeneratorManager', 'helper call receiver')
PATH_CONSTRUCTORS = {'pathlib.Path', 'pathlib.PosixPath', 'pathlib.WindowsPath',
                     'pathlib.PurePath', 'pathlib.PurePosixPath', 'pathlib.PureWindowsPath'}
PROVIDER_FIELDS = {'sys.executable', 'shutil.which', 'sys.modules', 'tempfile.TemporaryDirectory',
                   'unittest.mock.patch.TEST_PREFIX'} | PATH_CONSTRUCTORS
PROVIDER_MODULES = {name.partition('.')[0] for name in PROVIDER_FIELDS}
FIXTURE_OPTIONS_SHA256 = 'ea05fe7e2ebd01d1a8c8c46c854bc8f250e8bbf08189e0b83a39d7b8be04acf9'
FixtureInput = namedtuple('FixtureInput', 'source option')
FIXTURE_VERIFIER_SHA256 = '738ecf280222d99f9d5042d41f37129fc7cb924ca2481fafe605f9c93918447a'


class PatchedProvider:
    def __init__(self, changes):
        self.changes = changes


class LocalModule(namedtuple('LocalModule', 'path')):
    def member(self, name, imports, state):
        stored = state.get((self, name), {MISSING_FIELD})
        values = stored - {MISSING_FIELD}
        if MISSING_FIELD in stored:
            values = values | imports.loaded[self.path].get(name, set())
        return values


ImportedMember = namedtuple('ImportedMember', 'module name source')
SequenceState = namedtuple('SequenceState', 'elements head revised', defaults=(False,))


class SequenceBinding:
    def __init__(self, origin, items):
        self.origin = origin
        self.items = items
        exact = all(not isinstance(item, ast.Starred) for item in items)
        elements = tuple(item.owners for item in items) if exact else None
        head = items[0].owners if items and not isinstance(items[0], ast.Starred) else None
        self.initial = SequenceState(elements, head)

    def states(self, names):
        values = names.get((self, 'contents'), {self.initial})
        return {self.initial if value is MISSING_FIELD else value for value in values}


IMPORT_SOURCE = ContextVar('script_import_source', default=None)


class SuiteImports:
    def __init__(self, directory):
        self.root, origin, self.files, self.reviewed = IMPORT_SOURCE.get() or (ROOT, ROOT, None, {})
        self.directory = origin if directory is None else self.root / directory.relative_to(WorkflowLexer.workspace)
        self.loaded = {}
        self.stack = []
        self.class_paths = {}
        self.fixture_verifier = None
        self.fixture_manager = None
        self.fixture_options = None

    def register_fixture(self, body, filename, source):
        if filename == 'scripts/golden_output.py':
            definitions = [node for node in body if isinstance(node, ast.FunctionDef)]
            for name, field in (('verified_fixture', 'fixture_manager'), ('verify_artifact', 'fixture_verifier'),
                                ('parse_options', 'fixture_options')):
                matches = [node for node in definitions if node.name == name]
                if len(matches) != 1:
                    continue
                definition, = matches
                if field != 'fixture_manager':
                    lines = (source.encode() if isinstance(source, str) else source).splitlines(keepends=True)
                    operation = b''.join(lines[definition.lineno - 1:definition.end_lineno])
                    digest = FIXTURE_VERIFIER_SHA256 if field == 'fixture_verifier' else FIXTURE_OPTIONS_SHA256
                    if definition.decorator_list or hashlib.sha256(operation).hexdigest() != digest:
                        raise Unsupported('reviewed fixture verification operation changed')
                setattr(self, field, definition)

    def load(self, path, module, ordering, targets, names):
        if path in self.loaded:
            return self.loaded[path]
        relative = path.relative_to(self.root).as_posix()
        if (relative not in self.files or path.resolve() != path or not path.is_file()
                or path in self.stack or len(self.stack) >= 12):
            raise Unsupported('cannot resolve unsafe or recursive unittest import ' + module)
        self.stack.append(path)
        namespace = {key: values for key, values in names.items() if isinstance(key, tuple)}
        namespace['__name__'] = frozenset({ast.Constant(module)})
        try:
            data = path.read_bytes()
            tree = ast.parse(data, filename=relative)
            self.register_fixture(tree.body, relative, data)
            yield from python_callees(tree.body, namespace, ordering, targets, imports=self, source=path)
        finally:
            self.stack.pop()
        self.loaded[path] = namespace
        names.update((key, values) for key, values in namespace.items() if isinstance(key, tuple))
        for method in self.reviewed.get(relative, ()):
            for cls in namespace.get(method.partition('.')[0], ()):
                if isinstance(cls, PythonClass):
                    self.class_paths[cls] = WorkflowLexer.workspace / relative
        return namespace

    def has_local_root(self, module):
        root = module.partition('.')[0]
        stem = self.directory / root
        if not root or _imp.is_builtin(root) or _imp.is_frozen(root):
            return False
        return (stem.with_suffix('.py').is_file() or (stem / '__init__.py').is_file() or
                root not in sys.stdlib_module_names and stem.is_dir())

    def module(self, module, source, ordering, targets, names, *, bind_root=False):
        absolute = module.lstrip('.')
        levels = len(module) - len(absolute)
        parent = self.directory
        if levels:
            if source is None:
                raise Unsupported('cannot resolve relative unittest import')
            parent = source.parent
            for _ in range(levels - 1):
                parent = parent.parent
        parts = absolute.split('.') if absolute else []
        stem = parent.joinpath(*parts)
        if not stem.is_relative_to(self.root) or len(parts) > 12:
            raise Unsupported('cannot resolve unittest import inside the checkout')
        if self.files is None:
            self.files = set(tracked(self.root))
        chain = []
        for index, part in enumerate(parts[:-1], 1):
            parent = parent / part
            initializer = parent / '__init__.py'
            chain.append(initializer)
            if initializer not in self.stack:
                yield from self.load(initializer, '.'.join(parts[:index]), ordering, targets, names)
        paths = [stem / '__init__.py']
        if absolute:
            paths.append(stem.with_suffix('.py'))
        candidates = [path for path in paths if path.relative_to(self.root).as_posix() in self.files]
        if len(candidates) != 1:
            raise Unsupported('cannot resolve unittest import ' + module)
        leaf, = candidates
        yield from self.load(leaf, module, ordering, targets, names)
        chain.append(leaf)
        for package, child, part in zip(chain, chain[1:], parts[1:]):
            if package in self.loaded:
                self.loaded[package][part] = frozenset({LocalModule(child)})
        return LocalModule(chain[0] if bind_root else leaf)

    def resolve(self, symbol, ordering, targets, names, chain=()):
        if symbol in chain or len(chain) >= 12:
            raise Unsupported('cannot resolve recursive unittest re-export ' + symbol.name)
        reference = yield from self.module(symbol.module, symbol.source, ordering, targets, names)
        values = reference.member(symbol.name, self, names)
        if not values and reference.path.name == '__init__.py':
            child = reference.path.parent / symbol.name
            if child.with_suffix('.py').is_file() or (child / '__init__.py').is_file():
                parent = symbol.module if symbol.module.endswith('.') else symbol.module + '.'
                submodule = yield from self.module(parent + symbol.name, symbol.source, ordering, targets, names)
                return {submodule}
        if not values or any(value in (None, MISSING_NAME, UNKNOWN_CALLABLE, UNRESOLVED_CONTEXT)
                             or isinstance(value, BoundCall) for value in values):
            raise Unsupported('cannot resolve imported unittest binding ' + symbol.name)
        resolved = set()
        for value in values:
            if isinstance(value, ImportedMember):
                resolved.update((yield from self.resolve(value, ordering, targets, names, (*chain, symbol))))
            else:
                resolved.add(value)
        return resolved


def generator_phases(body):
    sites = [node for stmt in body for node in ast.walk(stmt) if isinstance(node, (ast.Yield, ast.YieldFrom))]
    if len(sites) != 1 or isinstance(sites[0], ast.YieldFrom):
        raise Unsupported('cannot resolve Python context manager yield')

    def split(statements):
        for index, stmt in enumerate(statements):
            if sites[0] not in ast.walk(stmt):
                continue
            before, after = statements[:index], statements[index + 1:]
            if any(isinstance(child, (ast.Return, ast.Break, ast.Continue)) for prior in before for child in ast.walk(prior)):
                raise Unsupported('cannot establish Python context manager entry')
            if isinstance(stmt, ast.Expr) and stmt.value is sites[0]:
                return [*before, ast.copy_location(ast.Return(value=stmt.value.value), stmt)], after, []
            if isinstance(stmt, ast.Try) and not stmt.handlers and stmt.finalbody:
                start, finish, failure = split(stmt.body)
                normal = ast.Try(body=finish, handlers=[], orelse=[], finalbody=stmt.finalbody) if finish else None
                if any(isinstance(child, (ast.Return, ast.Break, ast.Continue))
                       for cleanup in stmt.finalbody for child in ast.walk(cleanup)):
                    raise Unsupported('cannot resolve control transfer in generator cleanup')
                return before + start, ([normal] if normal else stmt.finalbody) + after, failure + stmt.finalbody
            if isinstance(stmt, (ast.With, ast.AsyncWith)):
                start, finish, failure = split(stmt.body)
                span = type(stmt)(items=stmt.items, body=start)
                span._generator_span = True
                return [*before, span], finish + after, failure
            break
        raise Unsupported('cannot resolve conditional Python context manager yield')

    return split(body)


def process_value(call, targets):
    if len(targets) != 1:
        return call
    target, = targets
    if target == 'str.split' and isinstance(call.func, ast.Attribute) and not call.args and not call.keywords:
        owner = call.func.value
        if isinstance(owner, ast.Constant) and isinstance(owner.value, str):
            return ast.List(elts=[ast.Constant(part) for part in owner.value.split()])
    if target == 'shlex.split' and len(call.args) == 1 and not call.keywords:
        argument, = call.args
        if isinstance(argument, ast.Constant) and isinstance(argument.value, str):
            try:
                return ast.List(elts=[ast.Constant(part) for part in shlex.split(argument.value)])
            except ValueError:
                return call
    if isinstance(target, PythonHelper):
        helper = target.node
        params = helper.args.posonlyargs + helper.args.args
        if (isinstance(helper, ast.FunctionDef) and not helper.decorator_list and len(helper.body) == 1
                and isinstance(helper.body[0], ast.Return) and not call.keywords
                and len(params) == len(call.args) and not helper.args.kwonlyargs
                and all(isinstance(arg, ast.Constant) and isinstance(arg.value, str) for arg in call.args)):
            values = {param.arg: arg for param, arg in zip(params, call.args)}
            result = helper.body[0].value
            items = result.elts if isinstance(result, (ast.List, ast.Tuple)) else [result]
            items = [values.get(item.id) if isinstance(item, ast.Name) else item for item in items]
            if all(isinstance(item, ast.Constant) and isinstance(item.value, str) for item in items):
                return ast.List(elts=items) if isinstance(result, (ast.List, ast.Tuple)) else items[0]
    return call


class PythonScope:
    def __init__(self):
        self.captured = set()


def loop_inputs(node, body):
    if hasattr(node, '_command_inputs'):
        return node._command_inputs
    inputs = set()
    assigned = set()
    statements = [node.test, *body] if isinstance(node, ast.While) else body
    for statement in statements:
        inputs.update(child.id for child in ast.walk(statement) if isinstance(child, ast.Name)
                      and isinstance(child.ctx, ast.Load) and child.id not in assigned)
        if isinstance(statement, ast.Assign):
            assigned.update(target.id for target in statement.targets if isinstance(target, ast.Name))
        elif isinstance(statement, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
            assigned.add(statement.name)
        elif isinstance(statement, ast.Name) and isinstance(statement.ctx, ast.Store):
            assigned.add(statement.id)
    node._command_inputs = inputs
    return inputs


def local_bindings(body):
    def scan(node):
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
            yield node.name
            return
        if isinstance(node, ast.Name) and isinstance(node.ctx, (ast.Store, ast.Del)):
            yield node.id
        if isinstance(node, ast.alias):
            yield node.asname or node.name.partition('.')[0]
        if isinstance(node, ast.ExceptHandler) and node.name:
            yield node.name
        for child in ast.iter_child_nodes(node):
            yield from scan(child)
    if body and not hasattr(body[0], '_local_bindings'):
        body[0]._local_bindings = {name for node in body for name in scan(node)}
    return body[0]._local_bindings if body else set()


def literal_elements(node):
    if isinstance(node, (ast.List, ast.Tuple)):
        items = expanded_items(node.elts)
        if not any(isinstance(item, ast.Starred) for item in items):
            return items
    if isinstance(node, (ast.Set, ast.Dict)):
        keys = node.elts if isinstance(node, ast.Set) else node.keys
        if all(isinstance(key, ast.Constant) for key in keys):
            distinct = dict.fromkeys(key.value for key in keys)
            if isinstance(node, ast.Dict) or len(distinct) <= 1:
                return [ast.Constant(value) for value in distinct]
    if isinstance(node, ast.Constant) and isinstance(node.value, (str, bytes)):
        return [ast.Constant(value) for value in node.value]
    return None


def loop_values(values, state, instances):
    def shape(value, path):
        if isinstance(value, ast.Constant):
            return ast.Constant, type(value.value), value.value
        if isinstance(value, CapturedArgument):
            return CapturedArgument, frozenset(shape(item, path) for item in value.owners)
        if isinstance(value, SequenceBinding):
            if value in path:
                return value.origin
            return value.origin, frozenset(shape(item, path | {value}) for item in value.states(state))
        if isinstance(value, SequenceState):
            items = value.elements
            return (None if items is None else tuple(frozenset(shape(item, path) for item in choices) for choices in items),
                    value.revised)
        if isinstance(value, (ast.Tuple, ast.List)):
            return type(value), tuple(shape(item, path) for item in value.elts)
        if value not in instances or not isinstance(value, PythonInstance) or value in path:
            return value
        fields = frozenset((key[1], frozenset(shape(item, path | {value}) for item in items))
                           for key, items in state.items() if isinstance(key, tuple) and key[0] is value)
        aliases = frozenset(key for key, items in state.items() if isinstance(key, str) and value in items)
        return value.owner, fields, aliases

    return len(values) > 1, frozenset(shape(value, set()) for value in values)


def python_callees(body, bindings, ordering, targets, enclosing=(), active=None, class_body=False,
                   require_command=False, reachable=True, exceptional_fields=None, testcase_super_init=False, allocations=None,
                   protocol_returns=None, imports=None, source=None):
    if active is None:
        active = {}
    if allocations is None:
        allocations = []
    scope = PythonScope()
    functions = {}
    results = {}
    transfers = []

    def frames(names):
        return enclosing if class_body else (*enclosing, (scope, names))

    def read_helper(helper, names, call=None, receiver=None, strict=False, interrupted=None, protocol=False,
                    phase=None, suspended=None):
        function = helper.node
        live = dict(frames(names))
        parents = tuple((key, live.get(key, frame)) for key, frame in helper.frames)
        local = fields(names)
        for parameter in ast.iter_child_nodes(function.args):
            if isinstance(parameter, ast.arg):
                local[parameter.arg] = frozenset({parameter})
        positional = function.args.posonlyargs + function.args.args
        if receiver is not None and positional:
            local[positional[0].arg] = frozenset({receiver})
            positional = positional[1:]
        if call is not None and not suspended:
            def bind_argument(parameter, value):
                local[parameter.arg] = frozenset(parameter if target is None else target
                                                 for target in qualified(value, names))
                if not (ordering or strict or receiver is not None):
                    return
                literal = argument(value, names)
                if isinstance(literal, ast.Constant) and isinstance(literal.value, str):
                    local[parameter.arg] = frozenset({literal})

            for parameter, value in zip(positional, expanded_items(call.args)):
                if isinstance(value, ast.Starred):
                    break
                bind_argument(parameter, value)
            named = {arg.arg: arg for arg in function.args.args + function.args.kwonlyargs}
            for keyword in call.keywords:
                if keyword.arg in named:
                    bind_argument(named[keyword.arg], keyword.value)
        if suspended:
            local.update(suspended)
        elif function is imports.fixture_manager:
            inputs = [next(iter(local[arg.arg])) for arg in positional if len(local[arg.arg]) == 1]
            if (len(inputs) != 3 or not all(isinstance(value, FixtureInput) for value in inputs)
                    or [value.option for value in inputs] != ['binary', 'receipt', 'subject']
                    or len({value.source for value in inputs}) != 1):
                raise Unsupported('cannot resolve verified fixture inputs')
            local[PRIVATE_FIXTURE_HOME] = frozenset({object()})
        helper.called = True
        if function in active:
            if (ordering or strict) and active[function] != local:
                raise Unsupported('cannot resolve recursive Python callable before Just installation')
            return {UNKNOWN_CALLABLE}
        entry = local.copy()
        context = (call, receiver, strict, reachable, interrupted is not None, tuple(active), protocol)
        environment = [entry, *(frame for _, frame in parents), *active.values()]
        if call is not None and helper.managed_body is not None:
            if not helper.managed_body:
                raise Unsupported('cannot resolve decorated context manager lifecycle')
            phase = helper.managed_body
            protocol = True
        if phase is None and helper.summaries.get(context) == environment:
            return {UNKNOWN_CALLABLE}
        first_allocation = len(allocations)
        active[function] = entry
        try:
            exceptions = {} if interrupted is not None else None
            base_init = False
            if isinstance(receiver, PythonInstance) and receiver.owner.base == 'unittest.TestCase' and function.name == '__init__':
                initializers = [cls.members['__init__'] for cls in receiver.owner.lineage if '__init__' in cls.members]
                base_init = bool(initializers and initializers[-1] == {helper})
            returns_value = any(isinstance(child, ast.Return) for statement in function.body for child in ast.walk(statement))
            protocol_values = set() if protocol or returns_value else None
            returned = yield from python_callees(function.body if phase is None else phase, local, ordering, targets, parents, active,
                                                require_command=strict, reachable=reachable and call is not None,
                                                exceptional_fields=exceptions, testcase_super_init=base_init, allocations=allocations,
                                                protocol_returns=protocol_values, imports=imports, source=helper.source)
            if suspended is not None and not suspended:
                suspended.update((key, values) for key, values in local.items() if isinstance(key, str))
            updates = fields(local)
            raised = fields(exceptions or {})
            if returned == {UNKNOWN_CALLABLE}:
                fresh = set(allocations[first_allocation:])
                visible = {key: value for key, value in updates.items() if key[0] not in fresh}
                failures = {key: value for key, value in raised.items() if key[0] not in fresh}
                if visible == fields(entry) and all(value <= entry.get(key, frozenset()) for key, value in failures.items()):
                    updates, raised = visible, failures
                    if not raised and phase is None:
                        helper.summaries[context] = [frame.copy() for frame in environment]
            if interrupted is not None:
                interrupted.update(join((interrupted, raised)))
            if call is not None:
                names.update(updates)
        finally:
            del active[function]
        if helper.decorator is UNKNOWN_CALLABLE or not protocol and not isinstance(function, ast.FunctionDef):
            return {UNKNOWN_CALLABLE}
        if not protocol:
            return {BoundCall(value) if isinstance(value, str) else value for value in returned}
        return returned

    def fields(names):
        return {key: values for key, values in names.items() if isinstance(key, tuple)}

    def member(owner, name, names):
        cls = owner.owner if isinstance(owner, PythonInstance) else owner
        stored = names.get((owner, name), {MISSING_FIELD})
        bound = set(stored) - {MISSING_FIELD}
        if MISSING_FIELD not in stored:
            return bound
        for ancestor in cls.lineage:
            stored = names.get((ancestor, name), {MISSING_FIELD})
            bound.update(stored - {MISSING_FIELD})
            if MISSING_FIELD not in stored:
                return bound
            if name in ancestor.members:
                values = ancestor.members[name]
                break
        else:
            if cls.base == 'unittest.TestCase' and hasattr(unittest.TestCase, name):
                if name in ('subTest', 'assertRaises', 'assertRaisesRegex', 'assertWarns', 'assertWarnsRegex'):
                    bound.add('unittest.TestCase.' + name)
                return bound
            if isinstance(cls.base, ImportedMember) and (cls.base.module, cls.base.name, name) == ('yaml', 'SafeDumper', 'add_representer'):
                return bound
            if cls.base not in ('builtins.object', 'builtins.str', 'builtins.ValueError', 'builtins.OSError'):
                bound.add(BoundCall(UNKNOWN_CALLABLE))
            return bound
        for value in values:
            if isinstance(value, PythonHelper):
                if value.decorator is UNKNOWN_CALLABLE and value.managed_body is None:
                    value = UNKNOWN_CALLABLE
                elif value.decorator == 'builtins.classmethod':
                    value = BoundCall(value, cls)
                elif isinstance(owner, PythonInstance) and (value.decorator is None or value.decorator in GENERATOR_DECORATORS):
                    value = BoundCall(value, owner)
            bound.add(value)
        return bound

    def qualified(node, names):
        if isinstance(node, CapturedArgument):
            return node.owners
        if isinstance(node, (ast.Constant, FixtureDescriptor, FixtureExecutable, LookupExecutable)):
            return frozenset({node})
        if isinstance(node, ast.JoinedStr):
            parts = []
            for field in node.values:
                if isinstance(field, ast.Constant):
                    parts.append(field.value)
                    continue
                owners = qualified(field.value, names)
                if len(owners) != 1 or field.conversion != -1 or field.format_spec is not None:
                    return frozenset({None})
                owner, = owners
                if isinstance(owner, ast.Constant) and isinstance(owner.value, str):
                    parts.append(owner.value)
                elif len(node.values) == 1 and isinstance(owner, FixtureExecutable):
                    return owners
                elif (len(node.values) == 2 and parts == ['/proc/self/fd/']
                      and isinstance(owner, FixtureDescriptor)):
                    return frozenset({FixtureExecutable(owner)})
                else:
                    return frozenset({None})
            return frozenset({ast.Constant(''.join(parts))})
        if isinstance(node, (ast.Call, ast.List, ast.Tuple)):
            return results.get(node, frozenset({UNKNOWN_CALLABLE}))
        if isinstance(node, ast.BinOp) and isinstance(node.op, ast.Div) and qualified(node.left, names) == {FILESYSTEM_PATH}:
            parts = qualified(node.right, names)
            if parts and all(part is FILESYSTEM_PATH or isinstance(part, ast.Constant) and isinstance(part.value, str)
                             for part in parts):
                return frozenset({FILESYSTEM_PATH})
        if isinstance(node, ast.Name):
            if node.id in names:
                return names[node.id]
            for parent_scope, parent in reversed(enclosing):
                parent_scope.captured.add(node.id)
                if node.id in parent:
                    return parent[node.id]
            return frozenset({'builtins.' + node.id if node.id in vars(builtins) else None})
        if isinstance(node, ast.Attribute):
            if isinstance(node.value, ast.Constant) and isinstance(node.value.value, str) and node.attr == 'split':
                return frozenset({'str.split'})
            values = set()
            for owner in qualified(node.value, names):
                if isinstance(owner, (PythonClass, PythonInstance)):
                    values.update(member(owner, node.attr, names))
                elif owner is UNKNOWN_PROVIDER:
                    values.add(UNKNOWN_PROVIDER)
                elif isinstance(owner, PatchedProvider) and node.attr in ('start', 'stop'):
                    values.add(BoundCall(node.attr, owner))
                elif isinstance(owner, SequenceBinding) and isinstance(owner.origin, ast.List) and node.attr in LIST_MUTATIONS:
                    values.add(BoundCall('builtins.list.' + node.attr, owner))
                elif owner == 'unittest.TestCase.super()' and node.attr == '__init__':
                    values.add('unittest.TestCase.__init__')
                elif owner in ('builtins.super()', 'unittest.TestCase.super()'):
                    values.add(BoundCall(UNKNOWN_CALLABLE))
                elif isinstance(owner, LocalModule):
                    members = owner.member(node.attr, imports, names) or {UNKNOWN_CALLABLE}
                    values.update(BoundCall(UNKNOWN_CALLABLE) if member in (None, UNKNOWN_CALLABLE, UNRESOLVED_CONTEXT)
                                  else member for member in members)
                elif isinstance(owner, ImportedMember):
                    values.add(ImportedMember(owner.module + '.' + owner.name, node.attr, owner.source))
                elif isinstance(owner, str) and owner.partition('.')[0] not in sys.stdlib_module_names:
                    values.add(ImportedMember(owner, node.attr, source))
                else:
                    name = owner + '.' + node.attr if isinstance(owner, str) else None
                    values.update(names.get((name, 'provider'), {name}))
            return frozenset(values)
        if isinstance(node, ast.Subscript):
            container = qualified(node.value, names)
            key = argument(node.slice, names)
            if isinstance(key, ast.Constant) and isinstance(key.value, str):
                if len(container) == 1:
                    for field in provider_fields(container, key, names, dictionary=True):
                        values = names.get((field, 'provider'), frozenset({field}))
                        if container == {'sys.modules'} and values != {key.value}:
                            return frozenset({UNKNOWN_PROVIDER})
                        return values
        if isinstance(node, ast.BoolOp):
            return frozenset().union(*(qualified(value, names) for value in node.values))
        if isinstance(node, ast.NamedExpr):
            return qualified(node.target, names)
        if isinstance(node, ast.IfExp):
            return qualified(node.body, names) | qualified(node.orelse, names)
        return frozenset({None})

    def join(states, *, paths=False):
        merged = {}
        for state in states:
            for key, values in state.items():
                merged[key] = merged.get(key, frozenset()) | values
        for key in merged:
            if (isinstance(key, tuple) or paths and isinstance(key, str)) and any(key not in state for state in states):
                merged[key] |= {MISSING_FIELD if isinstance(key, tuple) else MISSING_NAME}
        return merged

    def suite(statements, names, interrupted):
        for child in statements:
            if (yield from visit(child, names, interrupted)) is False:
                return False
        return True

    def alternatives(paths, names, interrupted):
        outcomes = []
        for path in paths:
            branch = names.copy()
            if (yield from suite(path, branch, interrupted)):
                outcomes.append(branch)
        if outcomes:
            names.clear()
            names.update(join(outcomes, paths=True))
        return bool(outcomes)

    def argument(value, names):
        if isinstance(value, (ast.Name, ast.Attribute)):
            owners = qualified(value, names)
            if len(owners) == 1:
                owner, = owners
                if isinstance(owner, (ast.Constant, ast.Tuple, ast.List)):
                    return owner
        if isinstance(value, (ast.List, ast.Tuple)):
            items = [argument(item, names) for item in value.elts]
            if items != value.elts:
                return type(value)(elts=items, ctx=ast.Load())
        return value

    def process_argument(value, names, seen=frozenset()):
        owners = qualified(value, names)
        if isinstance(value, (ast.List, ast.Tuple)) and all(isinstance(item, ast.Constant) for item in value.elts):
            return value
        if any(isinstance(owner, SequenceBinding) for owner in owners):
            choices = []
            for owner in owners:
                if not isinstance(owner, SequenceBinding) or owner in seen:
                    return CapturedArgument({UNKNOWN_CALLABLE})
                for state in owner.states(names):
                    if state.revised:
                        if not state.head:
                            return CapturedArgument({UNKNOWN_CALLABLE})
                        items = [CapturedArgument(state.head), ChangedTail(value=CapturedArgument({UNKNOWN_CALLABLE}))]
                    else:
                        items = owner.items
                    items = [process_argument(item, names, seen | {owner}) for item in items]
                    choices.append(type(owner.origin)(elts=items, ctx=ast.Load()))
            result = choices.pop()
            for choice in choices:
                result = ast.IfExp(test=CapturedArgument({UNKNOWN_CALLABLE}), body=choice, orelse=result)
            return result
        if len(owners) == 1:
            owner, = owners
            if owner == 'sys.executable':
                return LookupExecutable('python3')
            if isinstance(owner, LookupExecutable):
                return owner
            if isinstance(owner, (FixtureDescriptor, FixtureExecutable)):
                descriptor = owner.descriptor if isinstance(owner, FixtureExecutable) else owner
                if names.get((descriptor, 'verified')) != {True}:
                    raise Unsupported('cannot resolve closed fixture executable')
                return owner
            if isinstance(owner, (ast.List, ast.Tuple)):
                return type(owner)(elts=[process_argument(item, names, seen) for item in owner.elts], ctx=ast.Load())
            if isinstance(value, (CapturedArgument, ast.JoinedStr)) and isinstance(owner, ast.Constant):
                return owner
        if isinstance(value, ast.Starred):
            item = process_argument(value.value, names, seen)
            return value if item is value.value else ast.Starred(value=item, ctx=ast.Load())
        literal = argument(value, names)
        if isinstance(literal, (ast.List, ast.Tuple)):
            items = [process_argument(item, names, seen) for item in literal.elts]
            if items != literal.elts:
                return type(literal)(elts=items, ctx=ast.Load())
        return literal

    def captured(value, names):
        if isinstance(value, CapturedArgument):
            return value
        owners = qualified(value, names)
        if any(isinstance(owner, SequenceBinding) for owner in owners):
            return CapturedArgument(owners)
        literal = argument(value, names)
        if isinstance(literal, ast.Constant):
            return literal
        if isinstance(literal, (ast.List, ast.Tuple)):
            return type(literal)(elts=[captured(item, names) for item in literal.elts], ctx=ast.Load())
        if isinstance(literal, ast.Starred):
            return ast.Starred(value=captured(literal.value, names), ctx=ast.Load())
        return CapturedArgument(qualified(value, names) or {UNKNOWN_CALLABLE})

    def sequence_value(node, names):
        owners = qualified(node, names)
        if len(owners) == 1:
            owner, = owners
            if isinstance(owner, SequenceBinding):
                states = owner.states(names)
                if len(states) == 1:
                    state, = states
                    if state.elements is not None:
                        return type(owner.origin)(elts=[CapturedArgument(items) for items in state.elements])
            if isinstance(owner, (ast.Tuple, ast.List)):
                return owner
        return node

    def bind_target(target, value, names):
        if isinstance(target, (ast.Tuple, ast.List)):
            owners = qualified(value, names)
            if len(owners) > 1 or any(isinstance(owner, SequenceBinding) and len(owner.states(names)) > 1 for owner in owners):
                outcomes = []
                for owner in owners:
                    for state in owner.states(names) if isinstance(owner, SequenceBinding) else (None,):
                        branch = names.copy()
                        if state is not None:
                            branch[(owner, 'contents')] = frozenset({state})
                        if bind_target(target, CapturedArgument({owner}), branch) is not False:
                            outcomes.append(branch)
                if outcomes:
                    names.update(join(outcomes, paths=True))
                return bool(outcomes)
            value = sequence_value(value, names)
            items = py_items(value)
            if items is None:
                items = [CapturedArgument({UNKNOWN_CALLABLE})] * len(target.elts)
            elif any(isinstance(t, ast.Starred) for t in target.elts):
                index = next(i for i, t in enumerate(target.elts) if isinstance(t, ast.Starred))
                end = len(items) - (len(target.elts) - index - 1)
                rest = ast.List(elts=items[index:end])
                binding = SequenceBinding(rest, tuple(CapturedArgument(qualified(item, names)) for item in rest.elts))
                allocations.append(binding)
                items = [*items[:index], CapturedArgument({binding}), *items[end:]]
            if len(items) != len(target.elts):
                return False
            for part, item in zip(target.elts, items):
                if bind_target(part.value if isinstance(part, ast.Starred) else part, item, names) is False:
                    return False
            return
        owners = {value} if isinstance(value, (ast.List, ast.Tuple)) else qualified(value, names)
        values = frozenset(BoundCall(UNKNOWN_CALLABLE) if owner in (None, UNKNOWN_CALLABLE, UNRESOLVED_CONTEXT, MISSING_NAME)
                           or isinstance(owner, ast.arg) else owner for owner in owners)
        if isinstance(target, ast.Name):
            names[target.id] = values
        elif isinstance(target, ast.Attribute):
            receivers = qualified(target.value, names)
            for receiver in receivers:
                if isinstance(receiver, (PythonClass, PythonInstance)):
                    key = receiver, target.attr
                    names[key] = values if len(receivers) == 1 else values | names.get(key, {MISSING_FIELD})
        else:
            raise Unsupported('cannot resolve Python binding target')

    def resolved_handler(handler):
        target = handler.target if isinstance(handler, BoundCall) else handler
        if isinstance(target, PythonHelper) and target.decorator is not UNKNOWN_CALLABLE:
            return
        if target in ('unittest.TestCase.__init__', 'unittest.TestCase.setUp', 'unittest.TestCase.tearDown',
                      'unittest.TestCase.setUpClass', 'unittest.TestCase.tearDownClass'):
            return
        raise Unsupported('cannot resolve Python lifecycle or test handler')

    def callable_test(values):
        if not values:
            raise Unsupported('cannot resolve unittest test member')
        noncallable = (ast.Constant, ast.List, ast.Tuple, ast.Set, ast.Dict, SequenceBinding)
        if all(isinstance(value, noncallable) or isinstance(value, PythonInstance) and
               value.owner.base == 'builtins.object' and all('__call__' not in cls.members for cls in value.owner.lineage)
               for value in values):
            return False
        for value in values:
            if value is not SKIPPED_HANDLER:
                resolved_handler(value)
        return True

    def list_change(owner, method, args, names):
        if not isinstance(owner, SequenceBinding) or not isinstance(owner.origin, ast.List):
            return
        if len(args) < 2 and method in ('insert', '__setitem__'):
            raise Unsupported('cannot resolve list mutation arguments')

        def literal(node):
            try:
                return ast.literal_eval(argument(node, names))
            except (ValueError, TypeError, SyntaxError):
                return UNKNOWN_CALLABLE

        index = literal(args[0]) if args else -1
        if args and isinstance(args[0], ast.Slice):
            bounds = [literal(node) if node else None for node in (args[0].lower, args[0].upper, args[0].step)]
            index = slice(*bounds) if all(value is None or type(value) is int for value in bounds) else None
        if type(index) not in (int, slice):
            index = None
        states = set()
        returned = set()
        for state in owner.states(names):
            elements = list(state.elements) if state.elements is not None else None
            head = state.head
            after_head = (isinstance(index, int) and index > 0 or isinstance(index, slice)
                          and index.start is not None and index.start > 0 and index.step in (None, 1))
            preserves_head = (method in ('append', 'extend', '__iadd__') or
                              after_head and method in ('__setitem__', '__delitem__', 'insert', 'pop') or
                              method == '__imul__' and isinstance(index, int) and index > 0)
            try:
                if method == 'clear' or method == '__imul__' and index == 0:
                    elements = []
                elif elements is None:
                    pass
                elif method == 'append' and args:
                    elements.append(qualified(args[0], names))
                elif method in ('extend', '__iadd__') or method == '__setitem__' and isinstance(index, slice):
                    more = literal_elements(sequence_value(args[-1], names)) if args else None
                    if more is None:
                        if isinstance(index, slice):
                            selected = range(*index.indices(len(elements)))
                            preserves_head = 0 not in selected and selected.start != 0
                        elements = None
                    else:
                        elements[index if method == '__setitem__' else slice(len(elements), None)] = [
                            qualified(item, names) for item in more]
                elif method == '__setitem__' and isinstance(index, int):
                    elements[index] = qualified(args[1], names)
                elif method == 'insert' and isinstance(index, int):
                    elements.insert(index, qualified(args[1], names))
                elif method == '__delitem__' and index is not None:
                    del elements[index]
                elif method == 'pop' and isinstance(index, int):
                    returned.update(elements.pop(index))
                elif method == '__imul__' and isinstance(index, int) and len(elements) * index <= 1024:
                    elements *= max(0, index)
                elif method == 'reverse':
                    elements.reverse()
                elif method == 'remove' and args:
                    value = literal(args[0])
                    for position, choices in enumerate(elements):
                        if value is UNKNOWN_CALLABLE or not all(isinstance(item, ast.Constant) for item in choices):
                            elements = None
                            break
                        matches = {item.value == value for item in choices}
                        if matches == {True}:
                            del elements[position]
                            break
                        if True in matches:
                            elements = None
                            break
                        preserves_head = True
                else:
                    elements = None
            except (IndexError, ValueError, OverflowError):
                elements, head = None, None
            if method == 'pop' and elements is None:
                returned.add(BoundCall(UNKNOWN_CALLABLE))
            if elements is None:
                states.add(SequenceState(None, head if preserves_head else None, True))
            elif tuple(elements) == state.elements:
                states.add(state)
            else:
                states.add(SequenceState(tuple(elements), head if elements and elements[0] == head else None, True))
        names[(owner, 'contents')] = frozenset(states)
        if method in ('__iadd__', '__imul__'):
            return {owner}
        return returned or {ast.Constant(None)}

    def mutate(owners, method, args, names):
        outcomes = []
        returned = set()
        for owner in owners:
            branch = names.copy() if len(owners) > 1 else names
            returned.update(list_change(owner, method, args, branch) or {UNKNOWN_CALLABLE})
            if branch is not names:
                outcomes.append(branch)
        if outcomes:
            names.update(join(outcomes))
        return returned

    def carries_descriptor(value, names, live, seen=frozenset()):
        if isinstance(value, (ast.List, ast.Tuple, ast.Set)):
            return any(carries_descriptor(item, names, live) for item in value.elts)
        if isinstance(value, ast.Dict):
            return any(carries_descriptor(item, names, live) for item in (*value.keys, *value.values))
        if isinstance(value, ast.Starred):
            return carries_descriptor(value.value, names, live)
        owners = qualified(value, names)
        if live.intersection(owners):
            return True
        return any(carries_descriptor(CapturedArgument(items), names, live, seen | {owner})
                   for owner in owners if isinstance(owner, SequenceBinding) and owner not in seen
                   for state in owner.states(names) for items in state.elements or ())

    def disjoint_range(first, limit, names, live):

        def bound(node):
            value = argument(node, names)
            if isinstance(value, ast.Constant) and type(value.value) is int:
                return None, value.value
            if isinstance(node, ast.BinOp) and isinstance(node.op, (ast.Add, ast.Sub)):
                base, offset = bound(node.left)
                delta = argument(node.right, names)
                if offset is not None and isinstance(delta, ast.Constant) and type(delta.value) is int:
                    return base, offset + (delta.value if isinstance(node.op, ast.Add) else -delta.value)
            values = qualified(node, names)
            if len(values) == 1 and isinstance(fd := next(iter(values)), FixtureDescriptor):
                return fd, 0
            return None, None

        low, start = bound(first)
        high, end = bound(limit) if limit is not None else (low, None if start is None else start + 1)
        return (low is high and start is not None and end is not None and start >= end
                or end is not None and end <= 0 and (high is None or live == {high})
                or start is not None and start > 0 and live == {low})

    def readonly_descriptor_call(target, call, names):
        if target in ('os.read', 'os.readv', 'os.pread', 'os.preadv', 'os.fstat', 'os.fstatvfs', 'os.fpathconf',
                      'os.isatty', 'os.ttyname', 'os.device_encoding', 'os.get_inheritable', 'os.get_blocking'):
            return True
        if target not in ('builtins.open', 'io.open', 'os.fdopen', 'io.FileIO'):
            return False
        keywords = {kw.arg: kw.value for kw in call.keywords}
        mode = argument(call.args[1] if len(call.args) > 1 else keywords.get('mode', ast.Constant('r')), names)
        close_index = 2 if target == 'io.FileIO' else 6
        closefd = argument(call.args[close_index] if len(call.args) > close_index else keywords.get('closefd'), names)
        return (None not in keywords and 'opener' not in keywords and len(call.args) <= 7
                and isinstance(mode, ast.Constant) and mode.value in ('r', 'rb', 'br', 'rt', 'tr')
                and isinstance(closefd, ast.Constant) and closefd.value is False)

    def provider_fields(owners, key, names, *, dictionary=False):
        key = argument(key, names)
        fields = set()
        for name in PROVIDER_FIELDS:
            module, _, attr = name.rpartition('.')
            if (module + '.__dict__' if dictionary else module) in owners:
                if not isinstance(key, ast.Constant) or key.value == attr:
                    fields.add(name)
        if dictionary and 'sys.modules' in owners:
            fields.update('sys.modules.' + module for module in PROVIDER_MODULES
                          if not isinstance(key, ast.Constant) or key.value == module)
        return fields

    def record_provider_import(module, names):
        if module in PROVIDER_MODULES:
            field = 'sys.modules.' + module
            known = names.get((field, 'provider'), ())
            if (not names.get(('sys.modules', 'provider'), frozenset()) <= {'sys.modules', MISSING_FIELD} or
                    any(value not in (module, field, MISSING_FIELD) for value in known)):
                return False
            names[(field, 'provider')] = frozenset({module})
        return True

    def mapping_writes(mapping, keywords, attr, names):
        if any(kw.arg in (None, attr) for kw in keywords):
            return True
        if mapping is None:
            return False
        mapping = argument(mapping, names)
        if isinstance(mapping, ast.Dict):
            keys = [argument(key, names) for key in mapping.keys]
            return any(not isinstance(key, ast.Constant) or key.value == attr for key in keys)
        return not (isinstance(mapping, (ast.List, ast.Tuple)) and not mapping.elts)

    def patch_provider(patch, state, enter, *, saved_as):
        for name, changes in patch.changes.items():
            field = name, 'provider'
            saved = patch, (saved_as, name)
            previous = state.get(saved, frozenset())
            if enter:
                if previous:
                    raise Unsupported('cannot resolve repeatedly started executable provider patch')
                default = ast.Constant('test') if name == 'unittest.mock.patch.TEST_PREFIX' else name
                state[saved] = state.get(field, frozenset({default}))
                if changes:
                    state[field] = frozenset({UNKNOWN_CALLABLE})
            elif previous:
                if MISSING_FIELD in previous:
                    raise Unsupported('cannot resolve conditional executable provider patch')
                state[field] = previous
                state[saved] = frozenset()

    def invoke(target, call, names, receiver=None, wrapped=False, interrupted=None):
        if isinstance(target, BoundCall):
            if target.args or target.keywords:
                keywords = {kw.arg: kw for kw in (*target.keywords, *call.keywords)}
                call = ast.copy_location(ast.Call(func=call.func, args=[*target.args, *call.args],
                                                  keywords=list(keywords.values())), call)
            return (yield from invoke(target.target, call, names, target.receiver or receiver,
                                      wrapped or target.receiver is None, interrupted))
        if isinstance(receiver, PatchedProvider) and target in ('start', 'stop'):
            patch_provider(receiver, names, target == 'start', saved_as='started')
            return {UNKNOWN_CALLABLE}
        if target is UNKNOWN_PROVIDER:
            return {UNKNOWN_PROVIDER}
        if isinstance(target, PatchedProvider):
            raise Unsupported('cannot resolve executable provider patch application')
        if isinstance(target, str) and target.startswith('builtins.list.') and target.rsplit('.', 1)[1] in LIST_MUTATIONS:
            if receiver is not None:
                return list_change(receiver, target.rsplit('.', 1)[1], call.args, names)
            if call.args:
                return mutate(qualified(call.args[0], names), target.rsplit('.', 1)[1], call.args[1:], names)
            return {UNKNOWN_CALLABLE}
        if isinstance(target, PythonHelper):
            if target.decorator in GENERATOR_DECORATORS:
                target.called = True
                frozen = ast.copy_location(ast.Call(func=call.func, args=[captured(arg, names) for arg in call.args],
                    keywords=[ast.keyword(arg=kw.arg, value=captured(kw.value, names)) for kw in call.keywords]), call)
                return {GeneratorManager(target, frozen, receiver)}
            returned = yield from read_helper(target, names, call, receiver, strict=wrapped or require_command,
                                              interrupted=interrupted)
            if target.node is imports.fixture_options:
                invocation = object()
                items = [CapturedArgument({UNKNOWN_CALLABLE}), CapturedArgument({UNKNOWN_CALLABLE})]
                items.extend(CapturedArgument({FixtureInput(invocation, key)}) for key in ('binary', 'receipt', 'subject'))
                return {ast.Tuple(elts=items)}
            if target.node is imports.fixture_verifier and imports.fixture_manager in active:
                if len(call.args) != 5 or call.keywords or not isinstance(call.args[1], ast.Name):
                    raise Unsupported('cannot resolve verified fixture arguments')
                descriptors = qualified(call.args[0], names)
                if (len(descriptors) != 1 or not isinstance(fd := next(iter(descriptors)), FixtureDescriptor)
                        or names.get((fd, 'verified')) not in ({True}, {False})):
                    raise Unsupported('cannot resolve verified fixture descriptor')
                inputs = imports.fixture_manager.args.args
                entry = active[imports.fixture_manager]
                if (len(inputs) != 3 or fd.paths != entry[inputs[0].arg] or
                        qualified(call.args[2], names) != entry.get(PRIVATE_FIXTURE_HOME) or
                        qualified(call.args[3], names) != entry[inputs[1].arg] or
                        qualified(call.args[4], names) != entry[inputs[2].arg]):
                    raise Unsupported('cannot resolve verified fixture inputs')
                names[(fd, 'verified')] = frozenset({True})
                names[call.args[1].id] = frozenset({FixtureExecutable(fd, named=True)})
            return {BoundCall(value) if value in (UNKNOWN_CALLABLE, UNRESOLVED_CONTEXT) or isinstance(value, ast.arg) else value
                    for value in returned}
        if isinstance(target, PythonClass):
            instance = PythonInstance(target)
            allocations.append(instance)
            if (target.base not in ('builtins.object', 'builtins.str', 'builtins.ValueError', 'builtins.OSError',
                                    'unittest.TestCase', 'ast.expr') or
                    any('__new__' in cls.members or (cls, '__new__') in names or (cls, '__init__') in names
                        for cls in target.lineage)):
                raise Unsupported('cannot resolve Python constructor')
            if any('__init__' in cls.members for cls in target.lineage):
                initializers = member(instance, '__init__', names)
                if not initializers:
                    raise Unsupported('cannot resolve Python initializer')
                for method in initializers - {SKIPPED_HANDLER}:
                    resolved_handler(method)
                    yield from invoke(method, call, names, interrupted=interrupted)
            return {instance}
        if target == 'builtins.super':
            return {'unittest.TestCase.super()' if testcase_super_init and not call.args and not call.keywords
                    else 'builtins.super()'}
        if target == 'unittest.main' and reachable:
            if call.args or any(kw.arg not in ('exit', 'verbosity', 'failfast', 'buffer', 'catchbreak', 'warnings')
                                for kw in call.keywords):
                raise Unsupported('cannot resolve unittest test selection')
            module = frames(names)[0][1]
            suites = []
            for key in sorted(key for key in module if isinstance(key, str)):
                values = module[key]
                if any(isinstance(value, ImportedMember) for value in values):
                    resolved = set()
                    for value in values:
                        if isinstance(value, ImportedMember):
                            resolved.update((yield from imports.resolve(value, ordering, targets, names)))
                        else:
                            resolved.add(value)
                    values = resolved
                if not any(isinstance(value, PythonClass) for value in values):
                    continue
                if len(values) != 1:
                    raise Unsupported('cannot resolve unittest discovery')
                cls, = values
                if cls.test_case is None or cls.test_case and cls.base != 'unittest.TestCase':
                    raise Unsupported('cannot resolve unittest discovery')
                if not cls.test_case:
                    continue
                if cls in imports.class_paths:
                    yield call, '(suite)' + str(imports.class_paths[cls]), False, source
                    continue
                members = {key for ancestor in cls.lineage for key in ancestor.members if isinstance(key, str)}
                members.update(name for owner, name in fields(names) if owner in cls.lineage)
                methods = {name: member(cls, name, names) for name in members}
                tests = sorted(name for name in methods if name.startswith('test') and callable_test(methods[name]))
                if not tests and 'runTest' in methods:
                    tests = ['runTest']
                cases = []
                for method in tests:
                    constructor = ast.copy_location(ast.Call(func=call.func, args=[ast.Constant(method)], keywords=[]), call)
                    instance, = yield from invoke(cls, constructor, names, interrupted=interrupted)
                    cases.append((method, instance))
                if cases:
                    if suites and suites[-1][0] is cls:
                        suites[-1][2].extend(cases)
                    else:
                        suites.append((cls, methods, cases))
            for cls, methods, cases in suites:
                if cls.skipped or methods.get('setUpClass') == {SKIPPED_HANDLER}:
                    continue

                dispatch_call = ast.copy_location(ast.Call(func=call.func, args=[], keywords=[]), call)

                def dispatch(owner, method):
                    if method in methods or (owner, method) in names:
                        handlers = member(owner, method, names)
                        if not handlers:
                            raise Unsupported('cannot resolve unittest handler')
                        for handler in handlers - {SKIPPED_HANDLER}:
                            resolved_handler(handler)
                            yield from invoke(handler, dispatch_call, names, interrupted=interrupted)

                yield from dispatch(cls, 'setUpClass')
                for method, instance in cases:
                    if methods[method] == {SKIPPED_HANDLER} or methods.get('setUp') == {SKIPPED_HANDLER}:
                        continue
                    for phase in ('setUp', method, 'tearDown'):
                        yield from dispatch(instance, phase)
                yield from dispatch(cls, 'tearDownClass')
        if target == 'functools.partial':
            args = expanded_items(call.args)
            if not args or isinstance(args[0], ast.Starred) or any(kw.arg is None for kw in call.keywords):
                return {BoundCall(UNKNOWN_CALLABLE)}
            origins = qualified(args[0], names) or {None}
            return {BoundCall(origin if origin is not None else UNKNOWN_CALLABLE,
                              args=tuple(captured(arg, names) for arg in args[1:]),
                              keywords=tuple(ast.keyword(arg=kw.arg, value=captured(kw.value, names))
                                             for kw in call.keywords)) for origin in origins}
        if target in (UNKNOWN_CALLABLE, UNRESOLVED_CONTEXT) or isinstance(target, (ast.arg, PythonInstance)):
            if ordering or wrapped or require_command:
                raise Unsupported('cannot resolve Python callable')
        if target == 'shutil.which' and not call.keywords and len(call.args) == 1:
            value = argument(call.args[0], names)
            if isinstance(value, ast.Constant) and value.value == 'just':
                return {LookupExecutable('just')}
        if target == 'builtins.vars' and len(call.args) == 1:
            modules = qualified(call.args[0], names)
            if modules and all(module in PROVIDER_MODULES or module == 'unittest.mock.patch' for module in modules):
                return {module + '.__dict__' for module in modules}
        patching = target in ('unittest.mock.patch', 'unittest.mock.patch.object', 'unittest.mock.patch.dict')
        if patching or target in ('builtins.setattr', 'builtins.delattr'):
            keywords = {kw.arg: kw.value for kw in call.keywords}
            subject = call.args[0] if call.args else keywords.get('in_dict' if target == 'unittest.mock.patch.dict' else 'target')
            second = call.args[1] if len(call.args) > 1 else None
            providers = set()
            if target == 'unittest.mock.patch':
                path = argument(subject, names)
                if isinstance(path, ast.Constant) and path.value in PROVIDER_FIELDS:
                    providers.add(path.value)
            elif target == 'unittest.mock.patch.dict':
                owners = qualified(subject, names)
                mapping = second or keywords.get('values')
                clear = argument(call.args[2] if len(call.args) > 2 else keywords.get('clear', ast.Constant(False)), names)
                path = argument(subject, names)
                if isinstance(path, ast.Constant) and isinstance(path.value, str):
                    owners = {path.value}
                providers = provider_fields(owners, None, names, dictionary=True)
                if providers:
                    return {PatchedProvider({name: not isinstance(clear, ast.Constant) or clear.value or
                                             mapping_writes(mapping, call.keywords, name.rpartition('.')[2], names)
                                             for name in providers})}
            else:
                providers = provider_fields(qualified(subject, names), second or keywords.get('attribute'), names)
            if patching and providers:
                return {PatchedProvider(dict.fromkeys(providers, True))}
            for name in providers:
                names[(name, 'provider')] = frozenset({UNKNOWN_CALLABLE})
        if target == 'unittest.mock.patch.stopall':
            started = {key[0] for key, values in names.items() if isinstance(key, tuple) and
                       isinstance(key[0], PatchedProvider) and key[1][0] == 'started' and values}
            patched_fields = [name for patch in started for name in patch.changes]
            if len(set(patched_fields)) != len(patched_fields):
                raise Unsupported('cannot resolve overlapping executable provider patches')
            for patch in started:
                patch_provider(patch, names, False, saved_as='started')
        if isinstance(target, str) and ('.__dict__.' in target or target.startswith(('builtins.dict.', 'sys.modules.'))
                                        or target in ('operator.setitem', 'operator.delitem')):
            owner, _, method = target.rpartition('.')
            operands = call.args
            if owner in ('builtins.dict', 'operator') and operands:
                dictionary, *operands = operands
                owners = qualified(dictionary, names)
            else:
                owners = {owner}
            for name in provider_fields(owners, None, names, dictionary=True):
                attr = name.rpartition('.')[2]
                if method in ('clear', 'popitem') or (method in ('update', '__ior__') and
                        mapping_writes(operands[0] if operands else None, call.keywords, attr, names)) or (
                        method in ('__setitem__', '__delitem__', 'setitem', 'delitem', 'pop') and operands and
                        name in provider_fields(owners, operands[0], names, dictionary=True)):
                    names[(name, 'provider')] = frozenset({UNKNOWN_CALLABLE})
        if imports.fixture_manager is not None:
            live = {key[0] for key, values in names.items() if isinstance(key, tuple)
                    and isinstance(key[0], FixtureDescriptor) and True in values}
            module, _, operation = target.partition('.') if isinstance(target, str) else ('', '', '')
            if module not in ('os', 'posix'):
                operation = ''
            if operation == 'open' and (live or imports.fixture_manager in active):
                path = call.args[0] if call.args else next((kw.value for kw in call.keywords if kw.arg == 'path'), None)
                fd = FixtureDescriptor(qualified(path, names))
                names[(fd, 'verified')] = frozenset({False})
                return {fd}
            if operation in ('close', 'closerange', 'dup2', 'dup3') or target == 'socket.close':
                keywords = {kw.arg: kw.value for kw in call.keywords}
                replacing = operation in ('dup2', 'dup3')
                index = int(replacing)
                destination = call.args[index] if len(call.args) > index else keywords.get('fd2' if replacing else 'fd')
                limit = None
                if operation == 'closerange':
                    destination = call.args[0] if call.args else keywords.get('fd_low')
                    limit = call.args[1] if len(call.args) > 1 else keywords.get('fd_high', CapturedArgument({None}))
                owners = qualified(destination, names)
                known = (limit is None and len(owners) == 1 and isinstance(fd := next(iter(owners)), FixtureDescriptor)
                         and names.get((fd, 'verified')) in ({True}, {False}))
                if live and not disjoint_range(destination, limit, names, live):
                    if not known or (replacing and fd in live):
                        raise Unsupported('cannot resolve fixture descriptor replacement or close')
                if replacing:
                    src = call.args[0] if call.args else keywords.get('fd')
                    if live.intersection(qualified(src, names)):
                        raise Unsupported('cannot resolve fixture descriptor duplication')
                elif known:
                    names[(fd, 'verified')] = frozenset({None})
                else:
                    for key in list(names):
                        if isinstance(key, tuple) and isinstance(key[0], FixtureDescriptor) and key[0] not in live:
                            names[key] = frozenset({None})
            elif live and not readonly_descriptor_call('os.' + operation if module == 'posix' else target, call, names):
                if target in ('builtins.open', 'io.open', 'io.FileIO', 'os.fdopen'):
                    file = call.args[0] if call.args else next((kw.value for kw in call.keywords
                                                              if kw.arg in ('fd', 'file', 'name')), None)
                    owners = qualified(file, names)
                    path = owners and all(owner is FILESYSTEM_PATH or
                                          isinstance(owner, ast.Constant) and isinstance(owner.value, (str, bytes))
                                          for owner in owners)
                    if not path and not disjoint_range(file, None, names, live):
                        raise Unsupported('cannot resolve fixture descriptor ownership')
                process = target in PY_ARGV or target in ('asyncio.create_subprocess_exec', 'asyncio.subprocess.create_subprocess_exec')
                operands = [*call.args, *(kw.value for kw in call.keywords if not (process and kw.arg == 'pass_fds'))]
                if any(carries_descriptor(operand, names, live) for operand in operands):
                    raise Unsupported('cannot resolve fixture descriptor passed to another call')
        if isinstance(target, ImportedMember):
            return {UNRESOLVED_CONTEXT}
        if isinstance(target, str):
            if target.startswith('asyncio.subprocess.'):
                target = 'asyncio.' + target.removeprefix('asyncio.subprocess.')
            if target in PY_ARGV or target in PY_SHELL_TEXT or target in ('asyncio.create_subprocess_exec', 'pty.spawn'):
                args = [process_argument(value, names) for value in call.args]
                keyword_values = [process_argument(kw.value, names) for kw in call.keywords]
                if args != call.args or keyword_values != [kw.value for kw in call.keywords]:
                    keywords = [ast.keyword(arg=kw.arg, value=value) for kw, value in zip(call.keywords, keyword_values)]
                    call = ast.copy_location(ast.Call(func=call.func, args=args, keywords=keywords), call)
            yield call, target, wrapped or require_command, source
            if target in PATH_CONSTRUCTORS:
                return {FILESYSTEM_PATH}
            if target in ('builtins.open', 'io.open', 'io.FileIO', 'os.fdopen', 'io.StringIO', 'io.BytesIO', 'tempfile.TemporaryFile',
                          'tempfile.NamedTemporaryFile'):
                return {FILE_CONTEXT}
            if target in ('contextlib.closing', 'contextlib.aclosing', 'contextlib.redirect_stdout', 'contextlib.redirect_stderr'):
                if len(call.args) == 1 and target != 'contextlib.aclosing':
                    streams = qualified(call.args[0], names)
                    if streams == {FILE_CONTEXT} or target.startswith('contextlib.redirect_') and streams <= {
                            FILE_CONTEXT, 'sys.stdout', 'sys.stderr', 'sys.__stdout__', 'sys.__stderr__'}:
                        return {FILE_CONTEXT}
                return {UNRESOLVED_CONTEXT}
            if target in ('subprocess.Popen', 'tempfile.TemporaryDirectory', 'os.scandir'):
                return {NONSUPPRESSING_CONTEXT}
            if target in ('contextlib.suppress', 'contextlib.ExitStack',
                          'unittest.mock.patch', 'unittest.mock.patch.dict', 'unittest.mock.patch.object',
                          'unittest.TestCase.subTest', 'unittest.TestCase.assertRaises', 'unittest.TestCase.assertRaisesRegex',
                          'unittest.TestCase.assertWarns', 'unittest.TestCase.assertWarnsRegex'):
                return {STANDARD_CONTEXT}
            if target.partition('.')[0] not in sys.stdlib_module_names:
                return {UNRESOLVED_CONTEXT}
        return {UNKNOWN_CALLABLE}

    def record_exception(interrupted, names):
        interrupted.update(join((interrupted, names), paths=True) if interrupted else names)
        interrupted[EXCEPTION] = frozenset()

    def with_items(node, items, names, interrupted):
        if not items:
            return (yield from suite(node.body, names, interrupted))
        item, *rest = items
        expr = item.context_expr
        yield from visit(expr, names, interrupted)
        managers = qualified(expr, names)
        prefix = '__a' if isinstance(node, ast.AsyncWith) else '__'
        entry = prefix + 'enter__'
        exit_method = prefix + 'exit__'
        suspended = {}

        def lifecycle(method, state, args):
            outcomes = []
            values = set()
            for manager in managers:
                branch = state.copy()
                if getattr(node, '_generator_span', False) and not (
                        isinstance(expr, ast.Call) and qualified(expr.func, names) == {'tempfile.TemporaryDirectory'}):
                    raise Unsupported('cannot resolve context surrounding Python generator yield')
                if isinstance(manager, PatchedProvider):
                    patch_provider(manager, branch, method == entry, saved_as='entered')
                    values.add(UNKNOWN_CALLABLE if method == entry else ast.Constant(False))
                elif isinstance(manager, GeneratorManager):
                    if (manager.helper.decorator == 'contextlib.asynccontextmanager') != isinstance(node, ast.AsyncWith):
                        raise Unsupported('cannot resolve Python generator context protocol')
                    start, finish, failure = generator_phases(manager.helper.node.body)
                    entering = method == entry
                    phase = start if entering else finish if isinstance(args[0], ast.Constant) else failure
                    frame = suspended.setdefault(manager, {})
                    result = yield from read_helper(manager.helper, branch, manager.call, manager.receiver,
                                                    interrupted=interrupted, protocol=True, phase=phase, suspended=frame)
                    values.update(result if entering else {ast.Constant(False)})
                elif isinstance(manager, PythonInstance):
                    handlers = member(manager.owner, method, state)
                    if len(handlers) != 1:
                        raise Unsupported('cannot resolve Python context manager lifecycle')
                    handler, = handlers
                    resolved_handler(handler)
                    receiver = manager
                    if isinstance(handler, BoundCall):
                        handler, receiver = handler.target, handler.receiver
                    if not isinstance(handler, PythonHelper):
                        raise Unsupported('cannot resolve Python context manager method')
                    if handler.decorator == 'builtins.staticmethod':
                        receiver = None
                    call = ast.copy_location(ast.Call(func=expr, args=args, keywords=[]), expr)
                    values.update((yield from read_helper(handler, branch, call, receiver, interrupted=interrupted, protocol=True)))
                else:
                    if isinstance(manager, BoundCall) or manager in (MISSING_NAME, UNRESOLVED_CONTEXT) or (
                            ordering and manager not in (STANDARD_CONTEXT, FILE_CONTEXT, NONSUPPRESSING_CONTEXT)):
                        raise Unsupported('cannot resolve Python context manager')
                    if (method == entry and getattr(node, '_generator_span', False) and active
                            and next(reversed(active)) is imports.fixture_manager):
                        values.update(active[imports.fixture_manager][PRIVATE_FIXTURE_HOME])
                    elif method == exit_method and manager in (FILE_CONTEXT, NONSUPPRESSING_CONTEXT):
                        values.add(ast.Constant(False))
                    else:
                        values.add(FILE_CONTEXT if manager is FILE_CONTEXT and method == entry else UNKNOWN_CALLABLE)
                outcomes.append(branch)
            if outcomes:
                state.update(join(outcomes, paths=True))
            return values

        entered = yield from lifecycle(entry, names, [])
        if interrupted is not None:
            record_exception(interrupted, names)
        exceptions = {}
        if item.optional_vars:
            yield from visit(item.optional_vars, names, exceptions)
            outcomes = []
            for value in entered or {UNKNOWN_CALLABLE}:
                branch = names.copy()
                bind_target(item.optional_vars, value if isinstance(value, (ast.Tuple, ast.List)) else CapturedArgument({value}), branch)
                outcomes.append(branch)
            names.update(join(outcomes, paths=True))
        start = len(transfers)
        completed = yield from with_items(node, rest, names, exceptions)
        normal_args = [ast.Constant(None)] * 3
        for _, state in transfers[start:]:
            yield from lifecycle(exit_method, state, normal_args)
            if interrupted is not None:
                record_exception(interrupted, state)
        if completed:
            yield from lifecycle(exit_method, names, normal_args)
            if interrupted is not None:
                record_exception(interrupted, names)
        if EXCEPTION in exceptions:
            del exceptions[EXCEPTION]
            result = yield from lifecycle(exit_method, exceptions, [CapturedArgument({UNKNOWN_CALLABLE})] * 3)
            if interrupted is not None:
                record_exception(interrupted, exceptions)
            if any(not isinstance(value, ast.Constant) or value.value for value in result):
                names.update(join((names, exceptions), paths=True) if completed else exceptions)
                return True
        return completed

    def visit(node, names, interrupted=None):
        may_raise = (interrupted is not None and isinstance(node, (ast.stmt, ast.expr)) and
                not isinstance(node, (ast.Pass, ast.Constant, ast.Return, ast.Break, ast.Continue, ast.Expr,
                                      ast.Assign, ast.Global, ast.Nonlocal, ast.Try, ast.TryStar)) and
                not (isinstance(node, ast.Name) and (isinstance(node.ctx, ast.Store) or
                     isinstance(node.ctx, ast.Load) and node.id in names and MISSING_NAME not in names[node.id])))
        if may_raise and isinstance(node, ast.Name) and isinstance(node.ctx, ast.Load) and node.id not in names:
            if node.id not in local_bindings(body):
                owners = qualified(node, names)
                may_raise = None in owners or MISSING_NAME in owners
        if may_raise:
            record_exception(interrupted, names)
        try:
            completed = yield from visit_node(node, names, interrupted)
        except Unsupported as error:
            if error.lineno is None:
                error.lineno = getattr(node, 'lineno', None)
            if source is not None and error.location is None and hasattr(node, 'lineno'):
                error.location = source.relative_to(imports.root), node.lineno
            raise
        if may_raise and isinstance(node, ast.expr) and not isinstance(node, (ast.Name, ast.Constant)):
            record_exception(interrupted, names)
        return completed

    def visit_node(node, names, interrupted):
        def bind(key, values):
            names[key] = values
        if isinstance(node, (ast.Attribute, ast.Subscript)) and isinstance(node.ctx, (ast.Store, ast.Del)):
            key = ast.Constant(node.attr) if isinstance(node, ast.Attribute) else node.slice
            for name in provider_fields(qualified(node.value, names), key, names, dictionary=isinstance(node, ast.Subscript)):
                names[(name, 'provider')] = frozenset({UNKNOWN_CALLABLE})
        if isinstance(node, ast.AugAssign) and isinstance(node.op, ast.BitOr):
            for name in provider_fields(qualified(node.target, names), None, names, dictionary=True):
                if mapping_writes(node.value, (), name.rpartition('.')[2], names):
                    names[(name, 'provider')] = frozenset({UNKNOWN_CALLABLE})
        if isinstance(node, ast.AugAssign):
            method = '__iadd__' if isinstance(node.op, ast.Add) else '__imul__' if isinstance(node.op, ast.Mult) else None
            owners = qualified(node.target, names)
            if owners and all(isinstance(owner, SequenceBinding) and isinstance(owner.origin, ast.List) for owner in owners):
                yield from visit(node.value, names, interrupted)
                mutate(owners, method, [node.value], names)
                return
        if isinstance(node, ast.Subscript) and isinstance(node.ctx, ast.Del):
            mutate(qualified(node.value, names), '__delitem__', [node.slice], names)
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
            for child in ast.iter_child_nodes(node):
                if not isinstance(child, ast.stmt):
                    yield from visit(child, names, interrupted)
            skipped = False
            decorators = []
            for expression in node.decorator_list:
                if isinstance(expression, ast.Call):
                    if qualified(expression.func, names) == {'unittest.skip'}:
                        skipped = True
                        continue
                    owners = qualified(expression.func, names)
                    if owners in ({'unittest.skipIf'}, {'unittest.skipUnless'}):
                        condition = (expression.args[0] if expression.args else
                                     next((kw.value for kw in expression.keywords if kw.arg == 'condition'), None))
                        if isinstance(condition, ast.Constant):
                            skipped |= bool(condition.value) == ('unittest.skipIf' in owners)
                        continue
                decorators.append(qualified(expression, names))
            if isinstance(node, ast.ClassDef):
                bases = [qualified(base, names) for base in node.bases]
                cls = PythonClass(node, bases, skipped)
                allocations.append(cls)
                yield from python_callees(node.body, cls.members, ordering, targets, frames(names), active,
                                         class_body=True, require_command=require_command, reachable=reachable, allocations=allocations,
                                         imports=imports, source=source)
                for owners in reversed(decorators):
                    if len(owners) == 1 and isinstance(patch := next(iter(owners)), PatchedProvider):
                        prefixes = names.get(('unittest.mock.patch.TEST_PREFIX', 'provider'), {ast.Constant('test')})
                        possible = tuple(p.value for p in prefixes if isinstance(p, ast.Constant) and isinstance(p.value, str))
                        inherited = {name: values for ancestor in reversed(cls.lineage)
                                     for name, values in ancestor.members.items()
                                     if isinstance(name, str) and (len(possible) != len(prefixes) or name.startswith(possible))}
                        for name, values in inherited.items():
                            wrapped = set()
                            for value in values:
                                if isinstance(value, PythonHelper):
                                    helper = PythonHelper(value.node, value.frames, value.decorator, value.source)
                                    context = ast.withitem(context_expr=CapturedArgument({patch}), optional_vars=None)
                                    body = value.node.body if value.managed_body is None else value.managed_body
                                    helper.managed_body = [ast.With(items=[context], body=body)] if body else []
                                    wrapped.add(helper)
                                else:
                                    wrapped.add(value)
                            cls.members[name] = frozenset(wrapped)
                        continue
                    application = ast.copy_location(ast.Call(func=node, args=[CapturedArgument({cls})], keywords=[]), node)
                    for owner in owners or {UNKNOWN_CALLABLE}:
                        if isinstance(owner, PythonHelper):
                            returned = yield from invoke(owner, application, names, interrupted=interrupted)
                            if returned == {cls}:
                                continue
                        cls.base = UNKNOWN_CALLABLE
                        cls.test_case = None
                bind(node.name, frozenset({cls}))
            else:
                if skipped:
                    bind(node.name, frozenset({SKIPPED_HANDLER}))
                    return
                decorator = None
                if decorators:
                    if len(decorators) == 1 and decorators[0] in ({'builtins.classmethod'}, {'builtins.staticmethod'},
                                                                  {'contextlib.contextmanager'}, {'contextlib.asynccontextmanager'}):
                        decorator, = decorators[0]
                    else:
                        decorator = UNKNOWN_CALLABLE
                key = node, decorator
                managers = [owner for group in decorators for owner in group if isinstance(owner, (GeneratorManager, PatchedProvider))]
                if managers:
                    key = node, tuple(map(frozenset, decorators))
                if key not in functions:
                    helper = PythonHelper(node, frames(bindings), decorator, source)
                    if managers:
                        helper.managed_body = []
                        if len(managers) == len(decorators) and all(len(group) == 1 for group in decorators):
                            helper.decorator = None
                            body = node.body
                            for manager in reversed(managers):
                                asynchronous = isinstance(manager, GeneratorManager) and manager.helper.decorator == 'contextlib.asynccontextmanager'
                                if isinstance(manager, GeneratorManager) and asynchronous != isinstance(node, ast.AsyncFunctionDef):
                                    break
                                context = ast.withitem(context_expr=CapturedArgument({manager}), optional_vars=None)
                                body = [(ast.AsyncWith if asynchronous else ast.With)(items=[context], body=body)]
                            else:
                                helper.managed_body = body
                    functions[key] = helper
                helper = functions[key]
                bind(node.name, frozenset({helper}))
            return
        if isinstance(node, (ast.If, ast.IfExp)):
            yield from visit(node.test, names, interrupted)
            test = node.test
            if (source is not None and isinstance(test, ast.Compare) and isinstance(test.left, ast.Name)
                    and test.left.id == '__name__' and len(test.ops) == 1 and isinstance(test.ops[0], ast.Eq)
                    and isinstance(test.comparators[0], ast.Constant) and test.comparators[0].value == '__main__'):
                return (yield from suite(node.orelse if isinstance(node, ast.If) else [node.orelse], names, interrupted))
            if isinstance(test, ast.Compare) and len(test.ops) == 1 and len(test.comparators) == 1:
                left, right = qualified(test.left, names), qualified(test.comparators[0], names)
                if len(left) == len(right) == 1:
                    lhs, rhs = next(iter(left)), next(iter(right))
                    if isinstance(test.ops[0], (ast.Is, ast.IsNot)) and isinstance(lhs, str) and isinstance(rhs, str):
                        objects = []
                        for name in (lhs, rhs):
                            module, _, attr = name.rpartition('.')
                            if module in sys.stdlib_module_names and module in sys.modules:
                                objects.append(vars(sys.modules[module]).get(attr, MISSING_FIELD))
                        if len(objects) == 2 and all(obj is not MISSING_FIELD for obj in objects):
                            test = ast.Constant((objects[0] is objects[1]) != isinstance(test.ops[0], ast.IsNot))
                    elif (isinstance(test.ops[0], (ast.Eq, ast.NotEq)) and
                          isinstance(lhs, ast.Constant) and isinstance(rhs, ast.Constant)):
                        test = ast.Constant((lhs.value == rhs.value) != isinstance(test.ops[0], ast.NotEq))
            if isinstance(test, ast.Constant):
                arm = node.body if test.value else node.orelse
                return (yield from suite(arm if isinstance(node, ast.If) else [arm], names, interrupted))
            paths = (node.body, node.orelse) if isinstance(node, ast.If) else ([node.body], [node.orelse])
            return (yield from alternatives(paths, names, interrupted))
        if isinstance(node, ast.BoolOp):
            yield from visit(node.values[0], names, interrupted)
            outcomes = [names.copy()]
            for value in node.values[1:]:
                yield from visit(value, names, interrupted)
                outcomes.append(names.copy())
            names.update(join(outcomes, paths=True))
            return
        if isinstance(node, (ast.With, ast.AsyncWith)):
            return (yield from with_items(node, node.items, names, interrupted))
        if isinstance(node, ast.match_case):
            yield from visit(node.pattern, names, interrupted)
            if node.guard:
                yield from visit(node.guard, names, interrupted)
            return (yield from suite(node.body, names, interrupted))
        if isinstance(node, ast.Match):
            yield from visit(node.subject, names, interrupted)
            paths = [[case] for case in node.cases]
            last = node.cases[-1]
            if last.guard or not isinstance(last.pattern, ast.MatchAs) or last.pattern.pattern is not None:
                paths.append([])
            return (yield from alternatives(paths, names, interrupted))
        if isinstance(node, (ast.For, ast.AsyncFor, ast.While)):
            if isinstance(node, ast.While):
                yield from visit(node.test, names, interrupted)
                body = node.body
            else:
                yield from visit(node.iter, names, interrupted)
                body = [node.target, *node.body]
            inputs = loop_inputs(node, body)
            broken = []
            iterable = sequence_value(node.iter, names) if isinstance(node, ast.For) else None
            owners = qualified(node.iter, names) if isinstance(node, ast.For) else ()
            owner = next(iter(owners)) if len(owners) == 1 else None
            if len(owners) > 1 and all(isinstance(value, SequenceBinding) for value in owners):
                paths = [[ast.copy_location(ast.For(target=node.target, iter=CapturedArgument({value}),
                                                   body=node.body, orelse=node.orelse), node)] for value in owners]
                return (yield from alternatives(paths, names, interrupted))
            elements = literal_elements(iterable)
            candidates = []
            if elements is not None:
                elements = [captured(element, names) for element in elements]
            elif isinstance(node, ast.For) and isinstance(node.iter, (ast.Set, ast.Dict)):
                items = node.iter.keys if isinstance(node.iter, ast.Dict) else node.iter.elts
                candidates = [captured(item, names) for item in items]
            sequence = owner if isinstance(owner, SequenceBinding) else None
            initial_length = len(elements or ())
            if sequence:
                initial_length = max(len(state.elements or ()) for state in sequence.states(names))
            loop_allocations = set()
            finished = []
            exhausted = True
            for iteration in range(initial_length + 9):
                if sequence:
                    states = sequence.states(names)
                    ended = {state for state in states if state.elements is not None and iteration >= len(state.elements)}
                    if ended:
                        done = names.copy()
                        done[(sequence, 'contents')] = frozenset(ended)
                        finished.append(done)
                    pending = states - ended
                    if not pending:
                        exhausted = False
                        break
                    names[(sequence, 'contents')] = frozenset(pending)
                    if all(state.elements is not None for state in pending):
                        current = frozenset().union(*(state.elements[iteration] for state in pending))
                        item = CapturedArgument(current)
                        exact = True
                    else:
                        exact = False
                        elements = None
                        candidates = [CapturedArgument({UNKNOWN_CALLABLE})]
                else:
                    exact = elements is not None
                    if exact:
                        if iteration >= len(elements):
                            break
                        item = elements[iteration]
                if iteration == initial_length + 8:
                    raise Unsupported(f'line {node.lineno}: cannot resolve changing Python loop bindings')
                first_allocation = len(allocations)
                start = len(transfers)
                branch = names.copy()
                if exact:
                    yield from visit(node.target, branch, interrupted)
                    bind_target(node.target, item, branch)
                elif candidates:
                    yield from visit(node.target, branch, interrupted)
                    choices = []
                    for candidate in candidates:
                        choice = branch.copy()
                        bind_target(node.target, candidate, choice)
                        choices.append(choice)
                    branch.update(join(choices, paths=True))
                if (isinstance(node, ast.For) and not exact and not candidates and
                        (isinstance(node.target, ast.Name) or any(isinstance(value, SequenceBinding)
                         for value in qualified(node.iter, names)))):
                    bind_target(node.target, CapturedArgument({UNKNOWN_CALLABLE}), branch)
                    completed = yield from suite(node.body, branch, interrupted)
                else:
                    completed = yield from suite(node.body if exact or candidates else body, branch, interrupted)
                loop_exits = transfers[start:]
                transfers[start:] = [transfer for transfer in loop_exits if transfer[0] is ast.Return]
                back_edges = [state for kind, state in loop_exits if kind is ast.Continue]
                broken.extend(state for kind, state in loop_exits if kind is ast.Break)
                if completed:
                    back_edges.append(branch)
                if exact:
                    if not back_edges:
                        exhausted = False
                        break
                    names.clear()
                    names.update(join(back_edges, paths=True))
                    continue
                next_entry = join([names, *back_edges], paths=True)
                for key in next_entry:
                    if (isinstance(key, tuple) and isinstance(key[0], SequenceBinding) and key[1] == 'contents'
                            and next_entry[key] != names.get(key) and len(key[0].states(next_entry)) > 1
                            and len({len(state.elements) if state.elements is not None else None
                                     for state in key[0].states(next_entry)}) > 1):
                        heads = {state.head for state in key[0].states(next_entry)}
                        head = next(iter(heads)) if len(heads) == 1 else None
                        next_entry[key] = frozenset({SequenceState(None, head, True)})
                fresh = set(allocations[first_allocation:])
                loop_allocations.update(fresh)
                changed = any(values != names.get(key) and
                              (key in inputs or key in scope.captured or isinstance(key, tuple) and key[0] not in fresh) and
                              (loop_values(values, next_entry, loop_allocations) !=
                               loop_values(names.get(key, ()), names, loop_allocations))
                              for key, values in next_entry.items())
                names.update(next_entry)
                if not changed:
                    break
                if isinstance(node, ast.While):
                    yield from visit(node.test, names, interrupted)
            if exhausted:
                finished.append(names.copy())
            if finished:
                names.clear()
                names.update(join(finished, paths=True))
                if (yield from suite(node.orelse, names, interrupted)):
                    broken.append(names.copy())
            if broken:
                names.clear()
                names.update(join(broken, paths=True))
            return bool(broken)
        if isinstance(node, (ast.Try, ast.TryStar)):
            start = len(transfers)
            exits = names.copy()
            success = names.copy()
            normal = []
            completed = yield from suite(node.body, success, exits)
            handler_entry = exits.copy()
            if completed and (yield from suite(node.orelse, success, exits)):
                normal.append(success)
            for handler in node.handlers:
                handled = handler_entry.copy()
                if handler.type:
                    yield from visit(handler.type, handled, exits)
                handled.pop(handler.name, None)
                if (yield from suite(handler.body, handled, exits)):
                    handled.pop(handler.name, None)
                    normal.append(handled)
                if isinstance(node, ast.TryStar):
                    handler_entry = join((handler_entry, exits))
            pending = transfers[start:]
            del transfers[start:]
            for kind, state in pending:
                if (yield from suite(node.finalbody, state, interrupted)):
                    transfers.append((kind, state))
            if node.finalbody or interrupted is not None:
                if (yield from suite(node.finalbody, exits, interrupted)) and interrupted is not None:
                    interrupted.update(join((interrupted, exits)))
            names.clear()
            names.update(join(normal, paths=True) if normal else exits)
            if normal:
                return (yield from suite(node.finalbody, names, interrupted))
            return False
        if isinstance(node, ast.Import):
            for alias in node.names:
                root = alias.name.split('.')[0]
                if imports.has_local_root(root):
                    value = yield from imports.module(alias.name, source, ordering, targets, names, bind_root=not alias.asname)
                elif record_provider_import(root, names):
                    value = alias.name if alias.asname else root
                else:
                    value = UNKNOWN_PROVIDER
                bind(alias.asname or root, frozenset({value}))
        elif isinstance(node, ast.ImportFrom):
            module = '.' * node.level + (node.module or '')
            local = node.level or imports.has_local_root(module)
            available = local or record_provider_import(module.partition('.')[0], names)
            for alias in node.names:
                if alias.name == '*' and local:
                    reference = yield from imports.module(module, source, ordering, targets, names)
                    namespace = imports.loaded[reference.path]
                    exported = [name for name in namespace if isinstance(name, str) and not name.startswith('_')]
                    if '__all__' in namespace:
                        exports = sequence_value(CapturedArgument(namespace['__all__']), names)
                        exported = []
                        items = literal_elements(exports)
                        if items is None:
                            raise Unsupported('cannot resolve local wildcard exports')
                        for item in items:
                            values = qualified(item, names)
                            if len(values) != 1 or not isinstance(value := next(iter(values)), ast.Constant) or value.value not in namespace:
                                raise Unsupported('cannot resolve local wildcard exports')
                            exported.append(value.value)
                    for name in exported:
                        bind(name, reference.member(name, imports, names))
                elif alias.name == '*' and module in ('subprocess', 'asyncio', 'asyncio.subprocess', 'os', 'pty', 'shlex', 'functools'):
                    for exported in import_module(module).__all__:
                        bind(exported, frozenset({module + '.' + exported if available else UNKNOWN_PROVIDER}))
                else:
                    if local or module.partition('.')[0] not in sys.stdlib_module_names:
                        symbol = ImportedMember(module, alias.name, source)
                        values = (yield from imports.resolve(symbol, ordering, targets, names)) if (source is not None or local) else {symbol}
                    elif available:
                        name = module + '.' + alias.name
                        values = names.get((name, 'provider'), {name})
                    else:
                        values = {UNKNOWN_PROVIDER}
                    bind(alias.asname or alias.name, frozenset(values))
        elif isinstance(node, (ast.Assign, ast.AnnAssign, ast.NamedExpr)):
            if isinstance(node, ast.AnnAssign):
                yield from visit(node.annotation, names, interrupted)
                if node.value is None:
                    return
            yield from visit(node.value, names, interrupted)
            values = qualified(node.value, names)
            if class_body and isinstance(node.value, (ast.Set, ast.Dict)):
                values = frozenset({node.value})
            for target in node.targets if isinstance(node, ast.Assign) else [node.target]:
                yield from visit(target, names, interrupted)
                if isinstance(target, ast.Name):
                    bind(target.id, values)
                elif isinstance(target, (ast.Tuple, ast.List)):
                    if bind_target(target, CapturedArgument(values), names) is False:
                        return False
                elif isinstance(target, ast.Subscript):
                    mutate(qualified(target.value, names), '__setitem__', [target.slice, node.value], names)
                elif isinstance(target, ast.Attribute):
                    owners = qualified(target.value, names)
                    if owners == {'unittest.mock.patch'} and target.attr == 'TEST_PREFIX':
                        bind(('unittest.mock.patch.TEST_PREFIX', 'provider'), values)
                    for owner in owners:
                        if isinstance(owner, (PythonInstance, PythonClass, LocalModule)):
                            prior = names.get((owner, target.attr), {MISSING_FIELD}) if len(owners) > 1 else ()
                            bind((owner, target.attr), frozenset(prior) | values)
            return
        elif isinstance(node, ast.Name) and isinstance(node.ctx, (ast.Store, ast.Del)):
            names.pop(node.id, None)
        elif isinstance(node, ast.Attribute) and isinstance(node.ctx, (ast.Store, ast.Del)):
            targets.setdefault(node, set()).update(qualified(node, names))
            if isinstance(node.ctx, ast.Del):
                for owner in qualified(node.value, names):
                    if isinstance(owner, LocalModule):
                        names[(owner, node.attr)] = frozenset({UNKNOWN_CALLABLE})
        elif isinstance(node, (ast.List, ast.Tuple)) and isinstance(node.ctx, ast.Load):
            items = []
            for item in expanded_items(node.elts):
                yield from visit(item, names, interrupted)
                items.append(ast.Starred(value=captured(item.value, names)) if isinstance(item, ast.Starred)
                             else CapturedArgument(qualified(item, names)))
            owner = SequenceBinding(node, tuple(items))
            allocations.append(owner)
            results[node] = frozenset({owner})
            return
        elif isinstance(node, ast.Call):
            yield from visit(node.func, names, interrupted)
            callees = qualified(node.func, names)
            targets.setdefault(node, set()).update(callees or {None})
            for operand in (*node.args, *node.keywords):
                yield from visit(operand, names, interrupted)
            returned = set()
            outcomes = []
            for name in callees:
                branch = names.copy() if len(callees) > 1 else names
                returned.update((yield from invoke(name, node, branch, interrupted=interrupted)))
                if branch is not names:
                    outcomes.append(fields(branch))
            names.update(join(outcomes))
            results[node] = frozenset(returned or {UNKNOWN_CALLABLE})
            return
        for child in ast.iter_child_nodes(node):
            yield from visit(child, names, interrupted)

        if isinstance(node, (ast.Return, ast.Break, ast.Continue)):
            if protocol_returns is not None and isinstance(node, ast.Return):
                result = captured(node.value, names) if node.value is not None else ast.Constant(None)
                protocol_returns.update(qualified(result, names) if isinstance(result, CapturedArgument) else {result})
            transfers.append((type(node), names.copy()))
            return False
        if isinstance(node, ast.Raise):
            return False

    completed = yield from suite(body, bindings, exceptional_fields)
    returned = [fields(state) for kind, state in transfers if kind is ast.Return]
    if returned:
        if completed:
            returned.append(fields(bindings))
        bindings.update(join(returned))
    for helper in functions.values():
        if not class_body and not ordering and source is None and not helper.called and helper.decorator not in GENERATOR_DECORATORS:
            yield from read_helper(helper, bindings)
    if protocol_returns is not None:
        if completed:
            protocol_returns.add(ast.Constant(None))
        return protocol_returns
    if len(body) == 1 and isinstance(body[0], ast.Return):
        return frozenset(BoundCall(value) if isinstance(value, str) else
                         UNKNOWN_CALLABLE if value is None else value
                         for value in qualified(body[0].value, bindings)) or {UNKNOWN_CALLABLE}
    return {UNKNOWN_CALLABLE}


def python_calls(text, line=1, lexer=Lexer, directory=None, *, filename=None):
    """Read process calls as their effective executable and command line or shell payload."""
    tree = ast.parse(text)
    found = []
    targets = {}
    imports = SuiteImports(directory)
    imports.register_fixture(tree.body, filename, text)
    calls = dict.fromkeys(python_callees(tree.body, {}, directory is not None, targets, imports=imports))

    rebound = set().union(*(owners for node, owners in targets.items() if isinstance(node, ast.Attribute)))

    def process_arguments(node):
        if isinstance(node, LookupExecutable):
            return ast.Constant(node.program)
        if isinstance(node, ast.Call):
            owners = targets.get(node, set())
            if not rebound & owners:
                replacement = process_value(node, owners)
                if replacement is not node:
                    return replacement
        for field, value in ast.iter_fields(node):
            if isinstance(value, list):
                for index, item in enumerate(value):
                    if isinstance(item, ast.AST):
                        value[index] = process_arguments(item)
            elif isinstance(value, ast.AST):
                setattr(node, field, process_arguments(value))
        return node

    def read_call(node, name, wrapped, start):
        if name.startswith('(suite)'):
            yield lexer.call(start, '(py)' + name.removeprefix('(suite)'), True, directory=directory)
            return
        if directory is not None and name in ('os.chdir', 'os.fchdir'):
            raise Unsupported('cannot resolve Python directory changes before Just installation')
        if name not in PY_SHELL_TEXT and name not in PY_ARGV and name not in ('pty.spawn', 'asyncio.create_subprocess_exec'):
            return
        args = expanded_items([process_arguments(arg) for arg in node.args])
        keywords = {item.arg: process_arguments(item.value) for item in node.keywords}
        if None in keywords:
            raise Unsupported('cannot resolve unpacked Python process keywords')
        subprocess_argv = name in PY_ARGV and PY_ARGV[name] is None
        if subprocess_argv and any(isinstance(arg, ast.Starred) for arg in args[:10]):
            raise Unsupported('cannot resolve starred Python process arguments')
        for position, parameter in enumerate(('args', 'bufsize', 'executable', 'stdin', 'stdout', 'stderr',
                                              'preexec_fn', 'close_fds', 'shell', 'cwd')):
            if subprocess_argv and len(args) > position and parameter in keywords:
                raise Unsupported('duplicate Python subprocess argument: ' + parameter)
        override = keywords.get('executable')
        if subprocess_argv and len(args) > 2:
            override = args[2]
        executables = [None]
        if override is not None and (subprocess_argv or name.startswith('asyncio.')):
            executables = []
            for option in branches(override):
                if isinstance(option, FixtureExecutable) and not option.named:
                    executables.append(option)
                    continue
                if not isinstance(option, ast.Constant) or (option.value is not None and
                        not (isinstance(option.value, str) and option.value)):
                    raise Unsupported('cannot resolve Python process executable')
                executables.append(option if option.value is not None else None)

        def after_argv_zero(items):
            return items if items and isinstance(items[0], ast.Starred) else items[1:]

        strict = wrapped

        def process_argv(items):
            texts = []
            for exe in executables:
                if isinstance(exe, FixtureExecutable) and not (
                        items and isinstance(items[0], FixtureExecutable) and items[0].descriptor is exe.descriptor):
                    raise Unsupported('cannot resolve verified fixture argv zero')
                texts.append(argv_text([exe, *after_argv_zero(items)] if exe is not None else items, strict))
            return texts

        def shell_invocations(payload, rest):
            if any(isinstance(item, ChangedTail) for item in rest):
                raise Unsupported('cannot resolve changed Python shell arguments')
            if isinstance(payload, CapturedArgument):
                raise Unsupported('cannot resolve Python shell payload')
            if isinstance(payload, ast.Starred):
                raise Unsupported('cannot resolve starred Python process arguments')
            invocations = []
            for exe in executables:
                if exe is None:
                    invocations.append(py_text(payload))
                    continue
                if isinstance(exe, FixtureExecutable):
                    raise Unsupported('cannot resolve Python shell interpreter')
                interpreter = posixpath.basename(exe.value)
                if interpreter not in SHELLS and not PYTHONS.fullmatch(interpreter):
                    raise Unsupported('cannot resolve Python shell interpreter')
                if not isinstance(payload, ast.Constant) or not isinstance(payload.value, str):
                    raise Unsupported('cannot resolve interpreter payload')
                invocations.append(argv_text([exe, ast.Constant('-c'), payload, *rest], True))
            return invocations

        texts = []
        if name in PY_SHELL_TEXT:
            first = args[0] if args else keywords.get('cmd', keywords.get('command'))
            if isinstance(first, ast.Starred):
                raise Unsupported('cannot resolve starred Python process arguments')
            if first is not None:
                for branch in branches(first):
                    texts.extend(shell_invocations(branch, ()))
        elif name == 'asyncio.create_subprocess_exec':
            program = args[0] if args else keywords.get('program')
            for branch in branches(program) if program is not None else []:
                texts.extend(process_argv([branch, *args[1:]]))
        elif name == 'pty.spawn':
            argv = args[0] if args else keywords.get('argv')
            texts = [argv_text(py_items(branch) or [branch], wrapped) for branch in branches(argv)] if argv is not None else []
        else:
            spec = PY_ARGV[name]
            if spec is None:
                first = args[0] if args else keywords.get('args')
                shell = args[8] if len(args) > 8 else keywords.get('shell')
                shell_on = False
                if shell is not None:
                    shell_on = bool(shell.value) if isinstance(shell, ast.Constant) else None
                strict = wrapped or shell_on is None
                for branch in branches(first) if first is not None else []:
                    listed = py_items(branch)
                    if shell_on is not False:
                        payload = branch if listed is None else (listed[0] if listed else None)
                        texts.extend(shell_invocations(payload, listed[1:] if listed else ()))
                    if shell_on is not True:
                        if listed is None and py_text(branch) is None and any(executables):
                            listed = [ast.Starred(value=branch)]
                        texts.extend(process_argv(listed if listed is not None else [branch]))

            else:
                index, shape, program_key, argv_key = spec
                if any(isinstance(arg, ast.Starred) for arg in args[:index + 1]):
                    raise Unsupported('cannot resolve starred Python process arguments')
                program = args[index] if len(args) > index else keywords.get(program_key)
                if program is None:
                    return
                if shape == 'list':
                    argv = args[index + 1] if len(args) > index + 1 else keywords.get(argv_key)
                    for listed in branches(argv):
                        vector = py_items(listed)
                        if vector is None:
                            rest = [listed] if isinstance(listed, ast.Starred) else [None]
                        else:
                            rest = after_argv_zero(vector)
                        texts.extend(argv_text([choice, *rest], wrapped) for choice in branches(program))
                else:
                    texts.extend(argv_text([choice, *after_argv_zero(args[index + 1:])], wrapped) for choice in branches(program))
        cwd = directory
        value = args[9] if subprocess_argv and len(args) > 9 else keywords.get('cwd')
        if directory is not None and texts and value is not None:
            if not isinstance(value, ast.Constant) or (value.value is not None and not isinstance(value.value, str)):
                raise Unsupported('cannot resolve Python process directory before Just installation')
            if value.value is not None:
                cwd = lexer.location(directory, value.value)
        for shell_text in dict.fromkeys(texts):
            if isinstance(shell_text, FixtureExecutable):
                inherited = py_items(keywords.get('pass_fds'))
                preexec = args[6] if subprocess_argv and len(args) > 6 else keywords.get('preexec_fn')
                if (shell_text.named or inherited != [shell_text.descriptor]
                        or preexec is not None and not (isinstance(preexec, ast.Constant) and preexec.value is None)):
                    raise Unsupported('cannot resolve inherited fixture executable')
                yield lexer.call(start, VerifiedCommand('forge-warden'), True, directory=cwd)
            elif shell_text is not None:
                yield from external(shell_calls(shell_text, start, lexer, cwd))
            else:
                raise Unsupported('cannot resolve Python process command')

    for node, name, wrapped, source in calls:
        start = node.lineno if source is not None else line + node.lineno - 1
        try:
            for number, *rest in read_call(node, name, wrapped, start):
                if source is not None and isinstance(number, int):
                    number = source.relative_to(imports.root), number
                found.append((number, *rest))
        except Unsupported as error:
            error.lineno = start
            if source is not None and error.location is None:
                error.location = source.relative_to(imports.root), start
            raise
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


def command_location(label, line):
    path, number = line if isinstance(line, tuple) else (label, line)
    return f'{path}:{number}'


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
            origin = (root / name).parent if lang == 'python' else root
            token = IMPORT_SOURCE.set((root, origin, tracked_files, {}))
            try:
                calls = shell_calls(text) if lang == 'shell' else python_calls(text, filename=name)
            finally:
                IMPORT_SOURCE.reset(token)
        except (Unsupported, SyntaxError, UnicodeDecodeError) as error:
            readings[(name, lang)] = label
            location = getattr(error, 'location', None)
            at = command_location(label, location) if location else label
            found.append(f'{at}: not read: {error}')
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
                found.append(f'{command_location(label, line)}: runs `{target}`, which alone may call '
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
            if isinstance(word, VerifiedCommand) or word in BUILTINS or (word in functions and not by_prefix):
                continue
            if word in DECLARED:
                if word in ONLY_IN and name not in ONLY_IN[word]:
                    found.append(f'{command_location(label, line)}: calls `{word}`, which only '
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
            found.append(f'{command_location(label, line)}: calls `{word}`, which the gate image does not declare '
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
