#!/usr/bin/env python3
"""Every command a tracked script can call is one the gate image has (gap G9, Mission 7).

Train 14's first gate went red because `scripts/tests/dogfood.sh` called `cmp`: every lane's
toolbox has diffutils, the offline gate image does not. This check reads the repository's own
scripts and names each command they call that is not in DECLARED below.

What is read, and how:
- every tracked shell file (`*.sh`, or a first line naming sh or bash), with a linear token
  reader: quotes, `$(...)`, backquotes, `<(...)`, here-documents (a body that starts with a
  sh/bash shebang, or is fed to bash/sh, is read as shell; one with a python shebang or fed to
  python3 is read as Python; any other body is data), and the command position after `;`, `|`,
  `&&`, `||`, `&`, `(`, `{`, `!`, the reserved words and the prefixes `env`, `exec`, `command`,
  `nice`, `timeout`;
- every tracked Python file and Python here-document: `subprocess`/`os` process calls whose
  argument list starts with a string literal.

Fail closed: a construct this reader does not handle (`case` inside a substitution, an
unterminated quote or here-document, an `eval`, a dynamic command word that is not in
DYNAMIC) is a problem, not a skip. Stated limits: a Python call whose argument list does not
start with a literal (a variable, `sys.executable`, a helper's list) is not resolved; a data
here-document that a later line runs is not read; a command reached only through a
non-literal path is pinned by DYNAMIC, not resolved.

usage: python3 -I scripts/script_commands.py     (exit 1 names each undeclared call)
"""
import ast
from pathlib import Path
import re
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
KEYWORDS = frozenset('if then else elif fi do done while until for in case esac select function time ! { }'.split())
OPENERS = frozenset('if then else elif do while until ! { time'.split())
CLOSERS = frozenset('fi done esac }'.split())
PREFIXES = frozenset({'env', 'exec', 'command', 'nice', 'timeout', 'sudo'})
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
PY_CALLS = frozenset({'run', 'call', 'check_call', 'check_output', 'Popen', 'system', 'popen',
                      'execv', 'execvp', 'execvpe', 'execve', 'execl', 'execlp', 'spawnv', 'spawnvp'})


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
    """(line, word) for each command word of the shell text, nested substitutions included."""
    calls = []

    def sink(tokens):
        calls.extend(command_words(tokens, calls))

    lexer = Lexer(text, line, sink)
    sink(lexer.tokens())
    return calls


def command_words(tokens, calls):
    found, functions = [], set()
    expect, mode, prefix, owner, script = True, None, None, None, None
    for index, token in enumerate(tokens):
        kind = token[0]
        if kind == 'heredoc':
            body, start, consumer = token[1], token[2], owner
            first = body.split('\n', 1)[0]
            if first.startswith('#!'):
                consumer = 'python3' if 'python' in first else ('bash' if re.search(r'\b(ba)?sh\b', first) else None)
            if consumer in ('bash', 'sh'):
                calls.extend(shell_calls(body, start))
            elif consumer in ('python3', 'python'):
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
            if op == ')' and index > 0 and tokens[index - 1][:2] == ('op', '(') and found:
                functions.add(found.pop()[1])
                continue
            expect, prefix, script = op != ')', None, None
            if mode == 'for' and op in (';', '\n'):
                mode = 'for-do'
            continue
        word = token[1]
        if mode == 'array':
            continue
        if mode and mode[0] == 'target':
            mode = mode[1]
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
            if script == 'next':
                # The text of `trap '...'` and `bash -c '...'` runs as shell.
                if word.literal:
                    calls.extend(shell_calls(word.value, word.line))
                else:
                    found.append((word.line, word.raw))
                script = None
            elif script in ('bash', 'sh') and word.literal and re.fullmatch(r'-[a-z]*c[a-z]*', word.value):
                script = 'next'
            continue
        if prefix in PREFIXES and word.literal and word.value.startswith('-'):
            if (prefix, word.value) in (('env', '-u'), ('nice', '-n')):
                mode = ('target', mode)
            if prefix == 'command' and word.value in ('-v', '-V'):
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
        if word.literal and value in KEYWORDS:
            if value in ('for', 'select'):
                mode = 'for'
            elif value == 'case':
                mode = 'case-subject'
            expect = value in OPENERS
            continue
        if word.literal and value == '[[':
            mode = '[['
            continue
        if word.literal and value == 'eval':
            raise Unsupported(f'line {word.line}: eval')
        found.append((word.line, value if word.literal else word.raw))
        # /usr/bin/env runs its operand as env does; an absolute path acts as its file name.
        owner = value.rsplit('/', 1)[-1] if word.literal else None
        prefix = owner if owner in PREFIXES else None
        script = 'next' if owner == 'trap' else owner
        expect = prefix is not None
    return found + [(0, '()' + name) for name in functions]


def python_calls(text, line=1):
    """(line, command) for each process call whose argument list starts with a string literal."""
    found = []
    for node in ast.walk(ast.parse(text)):
        if not isinstance(node, ast.Call):
            continue
        func = node.func
        name = func.attr if isinstance(func, ast.Attribute) else getattr(func, 'id', None)
        if name not in PY_CALLS:
            continue
        first = node.args[0] if node.args else next((k.value for k in node.keywords if k.arg == 'args'), None)
        if isinstance(first, (ast.List, ast.Tuple)) and first.elts:
            first = first.elts[0]
        if isinstance(first, ast.Constant) and isinstance(first.value, str):
            found.append((line + node.lineno - 1, first.value.split()[0] if first.value.strip() else ''))
    return found


def tracked(root):
    names = subprocess.run(['git', '-C', str(root), 'ls-files', '-z'], check=True,
                           stdout=subprocess.PIPE, stdin=subprocess.DEVNULL).stdout.decode().split('\0')
    return [name for name in names if name]


def is_shell(root, name):
    if name.endswith('.sh'):
        return True
    path = root / name
    if path.is_symlink() or not path.is_file():
        return False
    with path.open('rb') as handle:
        first = handle.readline(200)
    return bool(re.match(rb'#!\s*\S*(?:/|env\s+)(?:ba)?sh\b', first))


def problems(root):
    found, files = [], tracked(root)
    for name in files:
        try:
            if is_shell(root, name):
                calls = shell_calls((root / name).read_text(encoding='utf-8'))
            elif name.endswith('.py'):
                calls = python_calls((root / name).read_text(encoding='utf-8'))
            else:
                continue
        except (Unsupported, SyntaxError) as error:
            found.append(f'{name}: not read: {error}')
            continue
        functions = {word[2:] for _, word in calls if word.startswith('()')}
        for line, word in calls:
            if word.startswith('()') or word in functions or word in BUILTINS:
                continue
            if word in DECLARED:
                if word in ONLY_IN and name not in ONLY_IN[word]:
                    found.append(f'{name}:{line}: calls `{word}`, which only '
                                 f'{", ".join(sorted(ONLY_IN[word]))} may call')
                continue
            if word in DYNAMIC.get(name, ()):
                continue
            if '/' in word and not word.startswith(('$', '"', "'")):
                if word.startswith('/') and Path(word).name in IMAGE:
                    continue
                if not word.startswith('/') and word.lstrip('./') in files:
                    continue
            found.append(f'{name}:{line}: calls `{word}`, which the gate image does not declare '
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
