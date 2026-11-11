#!/usr/bin/env python3
"""WP-RUNNER-UP: which jobs may run on the self-hosted runner-up pool, and for which events.

The pool is chosen in exactly one place: the hosted `route` job of each routed workflow. Its bash
script compares event and ref with exact, case-sensitive string tests (GitHub expression `==` ignores
case, so a protected `MAIN` would match `refs/heads/main` in an expression: Bastion B-RUNNER-01). This
test runs that exact script with bash over a truth table: every pull-request shape, merge_group,
follow-on and comment events, unknown events, other refs, protected case variants and an unprotected
main. Routed jobs take its output, falling back to hosted if it is empty or the route job failed.
Every routed job must provision the guest first, right after checkout (Bastion B-RUNNER-02).
WP-RUNNER-UP v8: the routed jobs are the sharded baseline's plan and shards; each has one exact
header. The shards also wait on the plan and run only after it succeeded. The aggregate check
(full-workspace) stays hosted.
RUNNER-ROUTING.1: the route job is pinned line for line (ROUTE_JOB_LINES), so it reads exactly
event, ref and protected; the truth table also holds under a hostile ambient environment (every
documented GITHUB_*/RUNNER_* variable, every workflow env key and every other name the script
mentions). v3: every workflow file is listed and of a pinned shape (top level, workflow env, plain
job keys, exactly one runs-on per job). v5: every workflow file is pinned whole by sha256
(WORKFLOW_SHA256); any workflow edit edits the pin in the same reviewed diff.

This is routing only: anyone who can push a branch can also edit `runs-on` on it, so the trust
boundary is repository write access plus runner-up's fork guard (docs/runner-up.md).
"""
import hashlib
import os
import re
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.dont_write_bytecode = True

ROOT = Path(__file__).resolve().parents[2]
POOL = "runner-up-kinoite"
HOSTED = "ubuntu-latest"
ROUTE = "${{ needs.route.outputs.runner || 'ubuntu-latest' }}"


def header(needs, condition):
    return "    needs: " + needs + "\n    if: ${{ " + condition + " }}\n    runs-on: " + ROUTE + "\n"


# The only jobs that may route to the pool, each with its exact routed header. Every other job is
# hosted, unconditionally.
ROUTED = {
    ".github/workflows/mutation-baseline.yml": {
        "mutation-plan": header("route", "!cancelled()"),
        "mutation-shards": header("[route, mutation-plan]",
                                  "!cancelled() && needs.mutation-plan.result == 'success'"),
    },
}
# Workflows with no route job at all: every job is hosted, literally (fw-verify's hosted_parity
# models only the reviewed `ubuntu-latest` gate shape, so ci.yml is not routed: WP-RUNNER-UP v7).
HOSTED_ONLY = (".github/workflows/ci.yml",)
GUEST_BOOTSTRAP = ("      - name: Prepare a self-hosted runner-up guest (hosted images ship these tools)\n"
                   "        if: runner.environment == 'self-hosted'\n"
                   "        run: bash scripts/guest_bootstrap.sh\n")
TOKEN = re.compile(r"\s*(\(|\)|&&|\|\||==|'[^']*'|[A-Za-z_][A-Za-z0-9_.]*)")


def evaluate(expression, context):
    """Evaluate a `${{ }}` expression of literals, context paths, (), ==, && and ||."""
    body = expression.strip()
    if not (body.startswith("${{") and body.endswith("}}")):
        raise ValueError("not an expression: " + expression)
    body = body[3:-2]
    tokens, at = [], 0
    while body[at:].strip():
        match = TOKEN.match(body, at)
        if not match:
            raise ValueError("unsupported syntax at: " + body[at:])
        tokens.append(match.group(1))
        at = match.end()
    position = 0

    def peek():
        return tokens[position] if position < len(tokens) else None

    def take():
        nonlocal position
        position += 1
        return tokens[position - 1]

    def primary():
        token = take()
        if token == "(":
            value = either()
            if take() != ")":
                raise ValueError("unbalanced parentheses")
            return value
        if token.startswith("'"):
            return token[1:-1]
        if token in ("true", "false"):
            return token == "true"
        if token not in context:
            raise ValueError("unknown context value: " + token)
        return context[token]

    def equality():
        left = primary()
        while peek() == "==":
            take()
            right = primary()
            if isinstance(left, str) and isinstance(right, str):
                left = left.lower() == right.lower()
            else:
                left = left == right
        return left

    def both():
        left = equality()
        while peek() == "&&":
            take()
            right = equality()
            left = right if left else left
        return left

    def either():
        left = both()
        while peek() == "||":
            take()
            right = both()
            left = left if left else right
        return left

    value = either()
    if position != len(tokens):
        raise ValueError("trailing tokens")
    return value


def job_bodies(text):
    """{job id: body text} for the jobs section of a workflow (two-space job indentation)."""
    if "\njobs:\n" not in text:
        raise ValueError("no jobs section")
    section = text.split("\njobs:\n", 1)[1]
    parts = re.split(r"^  ([A-Za-z0-9_-]+):\n", section, flags=re.M)
    return {parts[i]: parts[i + 1] for i in range(1, len(parts) - 1, 2)}


def steps_of(body):
    return re.split(r"(?=^      - )", body.split("    steps:\n", 1)[1], flags=re.M)[1:]


def route_script(text):
    """The `run: |` block of the route job, dedented."""
    body = job_bodies(text)["route"]
    block = body.split("        run: |\n", 1)[1]
    lines = []
    for line in block.splitlines():
        if line and not line.startswith("          "):
            break
        lines.append(line[10:])
    return "\n".join(lines).rstrip() + "\n"


def route(text, event, ref, protected, ambient=None):
    """Run the workflow's own route script with bash; return the runner it chose.

    `ambient` adds the variables a runner would also have set (RUNNER-ROUTING.1). The script's own
    inputs and PATH always override them."""
    with tempfile.TemporaryDirectory() as directory:
        output = Path(directory) / "output"
        output.write_text("")
        env = dict(ambient or {})
        env.update({"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "GITHUB_OUTPUT": str(output),
                    "EVENT": event, "REF": ref, "PROTECTED": protected})
        result = subprocess.run(["bash", "-c", route_script(text)], env=env, capture_output=True,
                                text=True, stdin=subprocess.DEVNULL, timeout=30, check=False,
                                cwd=directory)
        if result.returncode != 0:
            raise ValueError("route script failed: " + result.stderr)
        lines = output.read_text().splitlines()
        if len(lines) != 1 or not lines[0].startswith("runner="):
            raise ValueError("route script must write exactly one runner= line: " + repr(lines))
        return lines[0][len("runner="):]


# RUNNER-ROUTING.1 (Bastion's findings, PLANTS-FENCES-route 6378221d and RUNNER-ROUTING.1 R1): the
# truth table feeds the script only EVENT, REF and PROTECTED, so a condition on anything else the
# runner sets is invisible to it, and an open list of forbidden forms cannot close that (bash
# arithmetic, grep of /proc/self/environ, awk ENVIRON, python3 and input reassignment all read or
# rewrite inputs without a `$` expansion). So the list is closed. (1) Static: every non-comment line
# of the route job is exactly the reviewed text below, and the workflow's top level and env are
# exactly the reviewed ones, so no `defaults:` shell template or BASH_ENV can wrap it. Changing routing therefore means editing this constant in the same
# reviewed diff; the workflow's own shape is pinned too (TOP_LEVEL, below). (2) Dynamic, kept as a second check on whatever the constant says: the truth table
# must route identically under a hostile ambient environment.
ROUTE_JOB_LINES = (
    "    timeout-minutes: 5",
    "    name: route (runner-up for merged main only)",
    "    runs-on: ubuntu-latest",
    "    outputs:",
    "      runner: ${{ steps.pick.outputs.runner }}",
    "    steps:",
    "      - id: pick",
    "        env:",
    "          EVENT: ${{ github.event_name }}",
    "          REF: ${{ github.ref }}",
    "          PROTECTED: ${{ github.ref_protected }}",
    "        run: |",
    "          runner=ubuntu-latest",
    "          case \"$EVENT\" in",
    "            push|schedule|workflow_dispatch)",
    "              if [ \"$REF\" = refs/heads/main ] && [ \"$PROTECTED\" = true ]; then",
    "                runner=" + POOL,
    "              fi",
    "              ;;",
    "          esac",
    "          printf 'runner=%s\\n' \"$runner\" >> \"$GITHUB_OUTPUT\"",
    "          printf 'route: event=%s ref=%s protected=%s -> %s\\n' \"$EVENT\" \"$REF\" \"$PROTECTED\" \"$runner\"",
)
# The default variables GitHub documents for every runner, plus CI.
DOCUMENTED_AMBIENT = (
    "CI", "GITHUB_ACTION", "GITHUB_ACTION_PATH", "GITHUB_ACTION_REPOSITORY", "GITHUB_ACTIONS",
    "GITHUB_ACTOR", "GITHUB_ACTOR_ID", "GITHUB_API_URL", "GITHUB_BASE_REF", "GITHUB_ENV",
    "GITHUB_EVENT_NAME", "GITHUB_EVENT_PATH", "GITHUB_GRAPHQL_URL", "GITHUB_HEAD_REF", "GITHUB_JOB",
    "GITHUB_PATH", "GITHUB_REF", "GITHUB_REF_NAME", "GITHUB_REF_PROTECTED", "GITHUB_REF_TYPE",
    "GITHUB_REPOSITORY", "GITHUB_REPOSITORY_ID", "GITHUB_REPOSITORY_OWNER",
    "GITHUB_REPOSITORY_OWNER_ID", "GITHUB_RETENTION_DAYS", "GITHUB_RUN_ATTEMPT", "GITHUB_RUN_ID",
    "GITHUB_RUN_NUMBER", "GITHUB_SERVER_URL", "GITHUB_SHA", "GITHUB_STEP_SUMMARY",
    "GITHUB_TRIGGERING_ACTOR", "GITHUB_WORKFLOW", "GITHUB_WORKFLOW_REF", "GITHUB_WORKFLOW_SHA",
    "GITHUB_WORKSPACE", "RUNNER_ARCH", "RUNNER_DEBUG", "RUNNER_ENVIRONMENT", "RUNNER_NAME",
    "RUNNER_OS", "RUNNER_TEMP", "RUNNER_TOOL_CACHE",
)
# Never set from outside: they change how bash itself starts or parses.
SHELL_SPECIAL = re.compile(r"(BASH.*|ENV|IFS|SHELLOPTS|PS[0-9]|CDPATH|GLOBIGNORE|POSIXLY_CORRECT|"
                           r"LD_.*|HOME|SHELL|TERM|LANG|LC_.*|TZ|PATH|GITHUB_OUTPUT|EVENT|REF|"
                           r"PROTECTED)\Z")
# Strings, booleans, large and small numbers, and an actor name.
HOSTILE_VALUES = (POOL, "true", "refs/heads/main", "workflow_dispatch", "1", "2", "999999",
                  "dependabot[bot]")


def workflow_env_names(text):
    """The keys of the workflow-level `env:` mapping (runtime variables of every job)."""
    if "\nenv:\n" not in text:
        return []
    block = text.split("\nenv:\n", 1)[1]
    names = []
    for line in block.splitlines():
        if not line.startswith("  "):
            break
        match = re.match(r"  ([A-Za-z_][A-Za-z0-9_]*):", line)
        if match:
            names.append(match.group(1))
    return names


def route_input_problems(text, path):
    """Why the route job, or the workflow around it, is not exactly the reviewed one."""
    body = job_bodies(text)["route"]
    lines = tuple(line for line in body.splitlines()
                  if line.strip() and not line.lstrip().startswith("#"))
    problems = []
    if lines != ROUTE_JOB_LINES:
        problems.append("route: every non-comment line must be ROUTE_JOB_LINES (edit it in review)")
    # No `defaults:` shell template or BASH_ENV may wrap it: the top level is pinned (v3, Keel F1).
    return problems + workflow_shape_problems(path, text)


def hostile_ambient(text, value):
    """Every documented runner variable, every workflow env key and every other name the route
    script mentions, all set to `value`."""
    names = set(DOCUMENTED_AMBIENT) | set(workflow_env_names(text))
    names |= set(re.findall(r"[A-Za-z_][A-Za-z0-9_]*", route_script(text)))
    return {name: value for name in sorted(names) if not SHELL_SPECIAL.match(name)}


def ambient_routing_problems(text):
    """Rows of the truth table that route differently under a hostile ambient environment."""
    problems = []
    for value in HOSTILE_VALUES:
        ambient = hostile_ambient(text, value)
        for (event, ref, protected), expected in TABLE:
            try:
                got = route(text, event, ref, protected, ambient)
            except ValueError as error:
                got = "error: " + str(error)
            if got != expected:
                problems.append((value, event, ref, protected, got))
                break
    return problems


# Edits that make routing depend on an ambient input. The static layer must refuse every one; the
# flag says whether the dynamic layer must also refuse it on its own.
GUARD = '[ "$REF" = refs/heads/main ] && [ "$PROTECTED" = true ]'
INIT = "runner=ubuntu-latest\n"
REWRITE = " && { EVENT=push; REF=refs/heads/main; PROTECTED=true; }\n"


def after_init(condition):
    return (INIT, INIT + "          " + condition + REWRITE)


AMBIENT_EDITS = {
    "head ref condition (Bastion)": (
        "if " + GUARD + "; then", 'if { ' + GUARD + '; } || [ -n "${GITHUB_HEAD_REF:-}" ]; then',
        True),
    "runner override default (Bastion)": (
        INIT, "runner=${RUNNER_OVERRIDE:-ubuntu-latest}\n", True),
    "dispatch from any branch": (
        "if " + GUARD + "; then",
        "if { " + GUARD + '; } || [ "$GITHUB_EVENT_NAME" = workflow_dispatch ]; then', True),
    "printenv, no dollar": (
        "if " + GUARD + "; then",
        "if { " + GUARD + "; } || printenv GITHUB_HEAD_REF >/dev/null; then", True),
    "[[ -v ]], no dollar": (
        "if " + GUARD + "; then", "if { " + GUARD + "; } || [[ -v GITHUB_HEAD_REF ]]; then", True),
    "no initial runner": (INIT, "", True),
    "arithmetic (Bastion R1)": after_init("(( GITHUB_RUN_ATTEMPT > 1 ))") + (True,),
    "[[ -gt ]] (Bastion R1)": after_init("[[ GITHUB_RUN_NUMBER -gt 5 ]]") + (True,),
    "let (Bastion R1)": after_init('let "GITHUB_RUN_ATTEMPT > 1"') + (True,),
    "grep /proc/self/environ (Bastion R1)": after_init(
        "grep -qa 'GITHUB_HEAD_REF=dependabot' /proc/self/environ") + (True,),
    "awk ENVIRON (Bastion R1)": after_init(
        "awk 'BEGIN{exit ENVIRON[\"GITHUB_ACTOR\"]!=\"dependabot[bot]\"}'") + (True,),
    "python3 os.environ (Bastion R1)": after_init(
        "python3 -c 'import os,sys; sys.exit(os.environ.get(\"GITHUB_HEAD_REF\") is None)'") + (True,),
    "a new step input": (
        "          PROTECTED: ${{ github.ref_protected }}\n",
        "          PROTECTED: ${{ github.ref_protected }}\n          HEAD: ${{ github.head_ref }}\n",
        False),
    "REF from the head ref": (
        "          REF: ${{ github.ref }}\n", "          REF: ${{ github.head_ref }}\n", False),
    "PROTECTED a constant": (
        "          PROTECTED: ${{ github.ref_protected }}\n", "          PROTECTED: ${{ true }}\n",
        False),
    "job-level env": (
        "    runs-on: ubuntu-latest\n    outputs:\n",
        "    runs-on: ubuntu-latest\n    env:\n      X: ${{ github.head_ref }}\n    outputs:\n",
        False),
    "job-level shell": (
        "    runs-on: ubuntu-latest\n    outputs:\n",
        "    runs-on: ubuntu-latest\n    defaults:\n      run:\n        shell: sh\n    outputs:\n",
        False),
    "workflow-level defaults": (
        "\njobs:\n", "\ndefaults:\n  run:\n    shell: sh\n\njobs:\n", False),
    "event file read": (
        "if " + GUARD + "; then",
        "if { " + GUARD + '; } || [ -n "$(cat "$GITHUB_EVENT_PATH" 2>/dev/null)" ]; then', False),
}


def routed_job_problems(text, routed):
    """Why the routed jobs of a workflow are not admissible (empty when they are)."""
    boot = GUEST_BOOTSTRAP
    problems = []
    for job, body in job_bodies(text).items():
        if job == "route":
            if "    runs-on: ubuntu-latest\n" not in body:
                problems.append("route must run hosted")
            continue
        if job not in routed:
            if "    runs-on: ubuntu-latest\n" not in body or "needs.route" in body:
                problems.append(job + ": an unrouted job must run on ubuntu-latest")
            continue
        if routed[job] not in body:
            problems.append(job + ": routed job header must be exactly needs/if/runs-on of the route")
        steps = steps_of(body)
        if len(steps) < 2 or not steps[0].startswith("      - uses: actions/checkout@"):
            problems.append(job + ": the first step must be the checkout")
        elif steps[1] != boot:
            problems.append(job + ": the guest bootstrap must come right after checkout, guarded")
        if body.count("scripts/guest_bootstrap.sh") != 1:
            problems.append(job + ": exactly one guest bootstrap")
    return problems


# RUNNER-ROUTING.1 v3 (Keel's second attack, ATTACK-e4975c3e F1-F4): a rule that looks for a
# forbidden spelling is open, because YAML reads `"defaults":`, `defaults :` and `defaults:` as one
# key. So the shape of every workflow is pinned instead. (F1) A routed workflow's top-level lines,
# and its workflow env, are exactly the reviewed ones, so no `defaults:` shell template and no
# BASH_ENV can wrap the pinned route script. (F2/F3) In every workflow, each job key is a plain
# `  name:` line, each job-level key is a plain `    key:` line, and a job has exactly one
# `runs-on:`. (F4) Every workflow file is listed in ROUTED or HOSTED_ONLY. v4: every listed
# workflow's top level is pinned, and a job's first field sits at indent 4.
TOP_LEVEL = {
    ".github/workflows/mutation-baseline.yml": (
        "name: mutation-baseline", "on:", "permissions:", "concurrency:", "env:", "jobs:"),
    # v4 (Bastion R1 on 1022927f): every listed workflow's top level is pinned, and `jobs:` is last.
    ".github/workflows/ci.yml": ("name: ci", "on:", "permissions:", "concurrency:", "env:", "jobs:"),
}
WORKFLOW_ENV = {
    ".github/workflows/mutation-baseline.yml": ('  CARGO_INCREMENTAL: "0"', "  CARGO_TERM_COLOR: always"),
}
JOB_KEY = re.compile(r"  [A-Za-z0-9_-]+:\Z")
JOB_FIELD = re.compile(r"    [a-z][a-z-]*:(?: |\Z)")


def content_lines(text):
    return [line for line in text.splitlines() if line.strip() and not line.lstrip().startswith("#")]


def workflow_shape_problems(path, text):
    """Why a workflow file is not of the reviewed shape (empty when it is)."""
    problems = []
    lines = content_lines(text)
    top = tuple(line for line in lines if not line.startswith(" "))
    if top != TOP_LEVEL.get(path):
        problems.append(path + ": top-level keys must be exactly " + repr(TOP_LEVEL.get(path)))
    if path in WORKFLOW_ENV:
        env = []
        if "\nenv:\n" in text:
            for line in text.split("\nenv:\n", 1)[1].splitlines():
                if line.strip() and not line.startswith(" "):
                    break
                if line.strip() and not line.lstrip().startswith("#"):
                    env.append(line)
        if tuple(env) != WORKFLOW_ENV[path]:
            problems.append(path + ": workflow env must be exactly " + repr(WORKFLOW_ENV[path]))
    if "\njobs:\n" not in text:
        return problems + [path + ": no jobs section"]
    jobs = []
    for line in content_lines(text.split("\njobs:\n", 1)[1]):
        if not line.startswith(" "):
            break  # the pinned top level makes `jobs:` last; this keeps a fold out regardless
        jobs.append(line)
    previous = 0
    for line in jobs:
        indent = len(line) - len(line.lstrip(" "))
        if previous == 2 and indent != 4:
            problems.append(path + ": a job's first field must be at indent 4: " + line.strip())
        previous = indent
        if indent in (1, 3, 5) or "\t" in line[:indent + 1]:
            problems.append(path + ": odd indentation: " + line.strip())
        elif indent == 2 and not JOB_KEY.match(line):
            problems.append(path + ": a job key must be a plain `  name:` line: " + line.strip())
        elif indent == 4 and not JOB_FIELD.match(line):
            problems.append(path + ": a job field must be a plain `    key:` line: " + line.strip())
    keys = [line[2:-1] for line in jobs if JOB_KEY.match(line)]
    if len(keys) != len(set(keys)):
        problems.append(path + ": a job id occurs twice (Keel H1)")
    for job, body in job_bodies(text).items():
        runs = [line for line in body.splitlines() if line.startswith("    runs-on:")]
        if len(runs) != 1:
            problems.append(path + ": " + job + ": exactly one runs-on")
        elif job not in ROUTED.get(path, {}) and runs[0] != "    runs-on: ubuntu-latest":
            problems.append(path + ": " + job + ": an unrouted job runs on the literal ubuntu-latest")
    return problems


def listed_workflow_problems(names):
    """Workflow files that are in neither ROUTED nor HOSTED_ONLY."""
    listed = set(ROUTED) | set(HOSTED_ONLY)
    return sorted(name for name in names if name not in listed)


# RUNNER-ROUTING.1 v5 (Bastion's design note after v2-v4 each fell to a YAML spelling the line
# checks read differently; Keel H1 on f14d9169, a repeated job id): line-level checks of YAML stay
# open, and this repository's Python is standard-library only, so there is no YAML parser to
# compare with. The control is therefore closed by content: every workflow file is pinned whole.
# Editing a workflow means editing its pin here, in the same reviewed diff, and the truth table,
# the hostile environment and the shape checks below then run on the new text. The shape checks
# are kept as diagnostics that explain what a reviewer should look at, not as the control.
WORKFLOW_SHA256 = {
    ".github/workflows/ci.yml": "be47194f4f4b53665ae66080c8c42f5410a48126dbe055108da8276d758f0acd",
    ".github/workflows/mutation-baseline.yml": "9d34b3fd261b45dbd340245da6a023c32e7d09e825a1199c6823a1e614f8d9d0",
}


DECOY = "        run: |\n          echo x\n          #    runs-on: ubuntu-latest\n"
NEW_JOB = "  extra:\n    timeout-minutes: 5\n    runs-on: {runs}\n    steps:\n      - id: x\n" + DECOY
SHELL_TEMPLATE = "  run:\n    shell: env EVENT=push REF=refs/heads/main PROTECTED=true bash {0}\n"
BASELINE = ".github/workflows/mutation-baseline.yml"
CI = ".github/workflows/ci.yml"
SHAPE_EDITS = {
    "K7 quoted defaults key (Keel)": (BASELINE, "\njobs:\n", '\n"defaults":\n' + SHELL_TEMPLATE + "\njobs:\n"),
    "K8 spaced defaults key (Keel)": (BASELINE, "\njobs:\n", "\ndefaults :\n" + SHELL_TEMPLATE + "\njobs:\n"),
    "plain defaults key": (BASELINE, "\njobs:\n", "\ndefaults:\n" + SHELL_TEMPLATE + "\njobs:\n"),
    "K9 BASH_ENV in workflow env (Bastion N1)": (
        BASELINE, "  CARGO_TERM_COLOR: always\n", "  CARGO_TERM_COLOR: always\n  BASH_ENV: /tmp/x\n"),
    "quoted BASH_ENV in workflow env": (
        BASELINE, "  CARGO_TERM_COLOR: always\n", '  CARGO_TERM_COLOR: always\n  "BASH_ENV": /tmp/x\n'),
    "K1 self-hosted job with a decoy line, ci.yml (Keel)": (
        CI, "\njobs:\n", "\njobs:\n" + NEW_JOB.format(runs="self-hosted")),
    "K2 the same in the baseline (Keel)": (
        BASELINE, "\njobs:\n", "\njobs:\n" + NEW_JOB.format(runs="self-hosted")),
    "K3 index-syntax route output (Keel)": (
        BASELINE, "\njobs:\n", "\njobs:\n" + NEW_JOB.format(runs="${{ needs['route'].outputs.runner }}")),
    "K4 pool name built by format() (Keel)": (
        CI, "\njobs:\n", "\njobs:\n" + NEW_JOB.format(runs="${{ format('runner-up-{0}', 'kinoite') }}")),
    "K11 toJSON route output (Keel)": (
        BASELINE, "\njobs:\n",
        "\njobs:\n" + NEW_JOB.format(runs="${{ fromJSON(toJSON(needs)).route.outputs.runner }}")),
    "K10 job key with a trailing comment (Keel)": (
        CI, "\njobs:\n", "\njobs:\n  extra: # x\n    runs-on: self-hosted\n    steps:\n      - run: echo x\n"),
    "quoted runs-on key": (
        CI, "\njobs:\n", '\njobs:\n  extra:\n    runs-on: ubuntu-latest\n    "runs-on": self-hosted\n    steps:\n'
                         "      - run: echo x\n"),
    "flow-mapping job": (CI, "\njobs:\n", "\njobs:\n  extra:\n    {runs-on: self-hosted}\n"),
    "six-space job body, pool by format(), decoy in a trailing run-name (Bastion R1 v3)": (
        CI, None,
        "  nightly-more:\n      timeout-minutes: 5\n      runs-on: ${{ format('runner-up-{0}', 'kinoite') }}\n"
        "      steps:\n        - run: echo x\nrun-name: |\n    runs-on: ubuntu-latest\n"),
    "trailing top-level key in ci.yml": (CI, None, "run-name: x\n"),
    "H1 repeated job id behind a quoted multi-line name (Keel)": (
        CI, "\njobs:\n", '\njobs:\n  sneak:\n    name: "a\n    runs-on: ubuntu-latest\n  vocabulary:\n'
                         '    env: "\n    runs-on: self-hosted\n    steps:\n      - run: echo x\n'),
    "three-space job body": (
        CI, "\njobs:\n", "\njobs:\n  extra:\n   runs-on: self-hosted\n   steps:\n     - run: echo x\n"),
}
# FENCE-MUTATION (ledger Q36): edits that trip exactly one rule each, so that weakening that rule
# alone is seen (a mutation pass found these rules were only ever hit together with another one).
ISOLATED_SHAPE_EDITS = {
    "a job's first field deeper than 4": (
        "\njobs:\n  extra:\n      timeout-minutes: 5\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo x\n",
        "a job's first field must be at indent 4"),
    "odd indentation": (
        "\njobs:\n  extra:\n    runs-on: ubuntu-latest\n    steps:\n     - run: echo x\n", "odd indentation"),
    "a tab in the indentation": (
        "\njobs:\n  extra:\n    runs-on: ubuntu-latest\n    steps:\n    \t- run: echo x\n", "odd indentation"),
    "no runs-on": (
        "\njobs:\n  extra:\n    timeout-minutes: 5\n    steps:\n      - run: echo x\n", "exactly one runs-on"),
    # Keel R1 (FENCE-MUTATION verdict 574d355e): only the start anchor of match() refuses these.
    "a quoted field that ends like a plain one": (
        '\njobs:\n  extra:\n    runs-on: ubuntu-latest\n    "runs-on": self-hosted #    x: y\n    steps:\n'
        "      - run: echo x\n", "a job field must be a plain"),
    "a job key that ends like a plain one": (
        "\njobs:\n  sneak: #  x:\n    runs-on: self-hosted\n    steps:\n      - run: echo x\n",
        "a job key must be a plain"),
}

def case(event, ref="refs/heads/main", protected="true"):
    return (event, ref, protected)


TABLE = [
    (case("push"), POOL),
    (case("schedule"), POOL),
    (case("workflow_dispatch"), POOL),
    (case("push", protected="false"), HOSTED),
    (case("workflow_dispatch", protected="false"), HOSTED),
    (case("workflow_dispatch", protected=""), HOSTED),
    # B-RUNNER-01: protected case variants are other branches, never main.
    (case("workflow_dispatch", "refs/heads/MAIN"), HOSTED),
    (case("push", "refs/heads/MAIN"), HOSTED),
    (case("schedule", "refs/heads/Main"), HOSTED),
    (case("workflow_dispatch", "refs/heads/main-next"), HOSTED),
    (case("push", "refs/heads/main "), HOSTED),
    (case("push", "refs/heads/feature"), HOSTED),
    (case("push", "refs/tags/main"), HOSTED),
    (case("workflow_dispatch", "refs/heads/wp/RUNNER-UP"), HOSTED),
    (case("pull_request", "refs/pull/3/merge"), HOSTED),
    (case("pull_request"), HOSTED),
    (case("pull_request_target"), HOSTED),
    (case("merge_group", "refs/heads/gh-readonly-queue/main/pr-3-abc"), HOSTED),
    (case("merge_group"), HOSTED),
    (case("workflow_run"), HOSTED),
    (case("issue_comment"), HOSTED),
    (case("repository_dispatch"), HOSTED),
    (case("PUSH"), HOSTED),
    (case("a_future_event"), HOSTED),
]


class Routing(unittest.TestCase):
    def workflows(self):
        return {path: (ROOT / path).read_text() for path in ROUTED}

    def test_every_job_is_hosted_or_routed_through_the_route_job(self):
        for path, text in self.workflows().items():
            with self.subTest(path=path):
                self.assertTrue(set(ROUTED[path]) <= set(job_bodies(text)), path)
                self.assertEqual(routed_job_problems(text, ROUTED[path]), [])

    def test_unrouted_workflows_are_hosted_and_have_no_route_job(self):
        for path in HOSTED_ONLY:
            text = (ROOT / path).read_text()
            with self.subTest(path=path):
                bodies = job_bodies(text)
                self.assertNotIn("route", bodies)
                for job, body in bodies.items():
                    self.assertIn("    runs-on: ubuntu-latest\n", body, job)
                    self.assertNotIn("needs.route", body, job)
                    self.assertNotIn("guest_bootstrap", body, job)

    def test_the_route_job_is_identical_in_every_routed_workflow(self):
        bodies = {job_bodies(text)["route"] for text in self.workflows().values()}
        self.assertEqual(len(bodies), 1)

    def test_only_the_route_script_names_the_pool(self):
        for path in sorted((ROOT / ".github/workflows").glob("*.y*ml")):
            relative = path.relative_to(ROOT).as_posix()
            text = path.read_text()
            expected = 1 if relative in ROUTED else 0
            self.assertEqual(text.count(POOL), expected, relative)
            if expected:
                self.assertIn(POOL, route_script(text), relative)

    def test_only_merged_protected_main_code_routes_to_the_pool(self):
        for text in self.workflows().values():
            for (event, ref, protected), expected in TABLE:
                with self.subTest(event=event, ref=ref, protected=protected):
                    self.assertEqual(route(text, event, ref, protected), expected)

    def test_the_route_job_is_exactly_the_reviewed_job(self):
        for path, text in self.workflows().items():
            with self.subTest(path=path):
                self.assertEqual(route_input_problems(text, path), [])

    def test_routing_ignores_a_hostile_ambient_environment(self):
        for path, text in self.workflows().items():
            with self.subTest(path=path):
                ambient = hostile_ambient(text, POOL)
                self.assertIn("GITHUB_HEAD_REF", ambient)
                for name in workflow_env_names(text):
                    self.assertIn(name, ambient)
                self.assertEqual(ambient_routing_problems(text), [])

    def test_each_ambient_input_edit_is_refused_by_its_layer(self):
        for path, text in self.workflows().items():
            for name, (old, new, dynamic) in AMBIENT_EDITS.items():
                with self.subTest(path=path, edit=name):
                    self.assertEqual(text.count(old), 1, "anchor must be unique")
                    edited = text.replace(old, new, 1)
                    self.assertTrue(route_input_problems(edited, path), "static layer")
                    if dynamic:
                        self.assertTrue(ambient_routing_problems(edited), "dynamic layer")

    def test_every_workflow_file_is_listed_and_of_the_reviewed_shape(self):
        names = sorted(path.relative_to(ROOT).as_posix()
                       for path in (ROOT / ".github/workflows").glob("*.y*ml"))
        self.assertEqual(listed_workflow_problems(names), [])
        for name in names:
            with self.subTest(path=name):
                self.assertEqual(workflow_shape_problems(name, (ROOT / name).read_text()), [])

    def test_each_shape_rule_refuses_alone(self):
        text = (ROOT / CI).read_text()
        for name, (new, message) in ISOLATED_SHAPE_EDITS.items():
            with self.subTest(edit=name):
                problems = workflow_shape_problems(CI, text.replace("\njobs:\n", new, 1))
                self.assertEqual(len(problems), 1, problems)
                self.assertIn(message, problems[0])
        problems = workflow_shape_problems(CI, text.replace("\njobs:\n", "\njobz:\n", 1))
        self.assertTrue(problems and any("no jobs section" in p for p in problems), problems)

    def test_the_unrouted_and_route_job_rules_refuse_alone(self):
        text = (ROOT / BASELINE).read_text()
        hosted = "    runs-on: ubuntu-latest\n    outputs:\n      runner: ${{ steps.pick"
        self.assertEqual(text.count(hosted), 1)
        problems = routed_job_problems(text.replace(hosted, hosted.replace("ubuntu-latest", "self-hosted"), 1),
                                       ROUTED[BASELINE])
        self.assertEqual(problems, ["route must run hosted"])
        aggregate = "    needs: [mutation-plan, mutation-shards]\n    if: always()\n    runs-on: ubuntu-latest\n"
        self.assertEqual(text.count(aggregate), 1)
        reads = aggregate + "    env:\n      X: ${{ needs.route.outputs.runner }}\n"
        self.assertEqual(routed_job_problems(text.replace(aggregate, reads, 1), ROUTED[BASELINE]),
                         ["full-workspace: an unrouted job must run on ubuntu-latest"])

    def test_the_route_runner_refuses_a_script_that_writes_other_than_one_runner_line(self):
        text = (ROOT / BASELINE).read_text()
        line = "printf 'runner=%s\\n' \"$runner\" >> \"$GITHUB_OUTPUT\""
        self.assertEqual(text.count(line), 1)
        for name, edited in {"twice": text.replace(line, line + "\n          " + line, 1),
                             "misnamed": text.replace(line, line.replace("runner=%s", "runnr=%s"), 1)}.items():
            with self.subTest(case=name), self.assertRaises(ValueError):
                route(edited, "push", "refs/heads/main", "true")

    def test_only_the_exact_shell_special_names_are_kept_out_of_the_hostile_environment(self):
        # Keel R2: without the end anchor, names that only extend a special one would be left out.
        for name in ("EVENT", "REF", "PATH", "GITHUB_OUTPUT", "BASH_ENV", "ENV", "LC_ALL"):
            self.assertTrue(SHELL_SPECIAL.match(name), name)
        for name in ("EVENT_NAME", "REFX", "PATHS", "GITHUB_OUTPUT2", "ENVIRONMENT", "PROTECTED_X"):
            self.assertFalse(SHELL_SPECIAL.match(name), name)

    def test_workflow_env_names_reads_exactly_the_workflow_env(self):
        self.assertEqual(workflow_env_names((ROOT / BASELINE).read_text()),
                         ["CARGO_INCREMENTAL", "CARGO_TERM_COLOR"])
        self.assertEqual(workflow_env_names("name: x\njobs:\n"), [])

    def test_every_workflow_file_is_pinned_whole(self):
        names = sorted(path.relative_to(ROOT).as_posix()
                       for path in (ROOT / ".github/workflows").iterdir())
        self.assertEqual(names, sorted(WORKFLOW_SHA256), "every workflow file has a pin")
        for name in names:
            with self.subTest(path=name):
                digest = hashlib.sha256((ROOT / name).read_bytes()).hexdigest()
                self.assertEqual(digest, WORKFLOW_SHA256[name],
                                 "a workflow changed: review it, then update WORKFLOW_SHA256")

    def test_each_shape_edit_is_refused(self):
        self.assertTrue(listed_workflow_problems([".github/workflows/new.yml"]))
        for name, (path, old, new) in SHAPE_EDITS.items():
            with self.subTest(edit=name):
                text = (ROOT / path).read_text()
                if old is None:  # appended at the end of the file
                    edited = text + new
                else:
                    self.assertEqual(text.count(old), 1, "anchor must be unique")
                    edited = text.replace(old, new, 1)
                self.assertTrue(workflow_shape_problems(path, edited), name)

    def test_a_routed_job_falls_back_to_hosted_without_a_route_output(self):
        self.assertEqual(evaluate(ROUTE, {"needs.route.outputs.runner": ""}), HOSTED)
        self.assertEqual(evaluate(ROUTE, {"needs.route.outputs.runner": POOL}), POOL)
        self.assertEqual(evaluate(ROUTE, {"needs.route.outputs.runner": HOSTED}), HOSTED)

    def test_the_evaluator_has_github_semantics(self):
        self.assertEqual(evaluate("${{ 'A' == 'a' }}", {}), True)
        self.assertEqual(evaluate("${{ false && 'x' || 'y' }}", {}), "y")
        self.assertEqual(evaluate("${{ true && 'x' || 'y' }}", {}), "x")
        with self.assertRaises(ValueError):
            evaluate("${{ startsWith(github.ref, 'refs/') }}", {"github.ref": "r"})
        with self.assertRaises(ValueError):
            evaluate("${{ github.unknown }}", {})
        # FENCE-MUTATION: paths the mutation pass found unexercised.
        with self.assertRaises(ValueError):
            evaluate("${{ 'a'", {})
        with self.assertRaises(ValueError):  # ends right, starts wrong: the start check alone
            evaluate("{{ 'a' }}", {})
        self.assertEqual(evaluate("${{ ('a' == 'A') }}", {}), True)
        self.assertEqual(evaluate("${{ true == true }}", {}), True)
        self.assertEqual(evaluate("${{ true == 'true' }}", {}), False)

    def test_every_routed_job_refuses_a_moved_dropped_or_unguarded_bootstrap(self):
        boot = GUEST_BOOTSTRAP
        guard = "        if: runner.environment == 'self-hosted'\n"
        for path, text in self.workflows().items():
            bodies = job_bodies(text)
            for job in ROUTED[path]:
                body = bodies[job]
                compiler = re.search(r"      - run: rustup toolchain install [^\n]*\n", body)[0]
                without = body.replace(boot, "", 1)
                head = ROUTED[path][job]
                condition = re.search(r"    if: [^\n]*\n", head)[0]
                bad = {
                    "dropped": without,
                    "after the compiler": without.replace(compiler, compiler + boot, 1),
                    "unguarded": body.replace(boot, boot.replace(guard, ""), 1),
                    "header without the fallback": body.replace(head, head.replace(" || 'ubuntu-latest'", ""), 1),
                    "header without its condition": body.replace(head, head.replace(condition, ""), 1),
                    "header without !cancelled()": body.replace(head, head.replace("!cancelled()", "success()"), 1),
                    "header hosted": body.replace(head, head.replace(ROUTE, HOSTED), 1),
                    "checkout not first": body.replace("      - uses: actions/checkout@", "      - uses: actions/cache@", 1),
                    "two bootstraps": body.replace(boot, boot + boot, 1),
                }
                if "needs.mutation-plan.result" in head:
                    bad["shards without the plan-success condition"] = body.replace(
                        head, head.replace(" && needs.mutation-plan.result == 'success'", ""), 1)
                    bad["shards not waiting on the plan"] = body.replace(
                        head, head.replace("[route, mutation-plan]", "route"), 1)
                for name, mutated in bad.items():
                    with self.subTest(path=path, job=job, mutation=name):
                        self.assertNotEqual(mutated, body)
                        self.assertTrue(
                            routed_job_problems(text.replace(body, mutated, 1), ROUTED[path]))

    def test_the_aggregate_check_stays_hosted_and_needs_plan_and_shards(self):
        body = job_bodies((ROOT / ".github/workflows/mutation-baseline.yml").read_text())["full-workspace"]
        self.assertIn("    needs: [mutation-plan, mutation-shards]\n    if: always()\n"
                      "    runs-on: ubuntu-latest\n", body)
        self.assertNotIn("guest_bootstrap", body)

    def test_routed_workflows_hold_no_secrets_and_least_privilege(self):
        for path, text in self.workflows().items():
            self.assertIn("\npermissions:\n  contents: read\n", text, path)
            for forbidden in ("secrets.", "id-token", "pull_request_target", "workflow_run"):
                self.assertNotIn(forbidden, text, (path, forbidden))
            for job in ROUTED[path]:
                body = job_bodies(text)[job]
                self.assertNotIn("permissions:", body, (path, job))
                self.assertIn("persist-credentials: false", body, (path, job))


if __name__ == "__main__":
    unittest.main()
