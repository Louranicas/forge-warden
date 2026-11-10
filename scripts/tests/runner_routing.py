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

This is routing only: anyone who can push a branch can also edit `runs-on` on it, so the trust
boundary is repository write access plus runner-up's fork guard (docs/runner-up.md).
"""
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


def route(text, event, ref, protected):
    """Run the workflow's own route script with bash; return the runner it chose."""
    with tempfile.TemporaryDirectory() as directory:
        output = Path(directory) / "output"
        output.write_text("")
        env = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "GITHUB_OUTPUT": str(output),
               "EVENT": event, "REF": ref, "PROTECTED": protected}
        result = subprocess.run(["bash", "-c", route_script(text)], env=env, capture_output=True,
                                text=True, stdin=subprocess.DEVNULL, timeout=30, check=False)
        if result.returncode != 0:
            raise ValueError("route script failed: " + result.stderr)
        lines = output.read_text().splitlines()
        if len(lines) != 1 or not lines[0].startswith("runner="):
            raise ValueError("route script must write exactly one runner= line: " + repr(lines))
        return lines[0][len("runner="):]


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
                }
                if "needs.mutation-plan.result" in head:
                    bad["shards without the plan-success condition"] = body.replace(
                        head, head.replace(" && needs.mutation-plan.result == 'success'", ""), 1)
                    bad["shards not waiting on the plan"] = body.replace(
                        head, head.replace("[route, mutation-plan]", "route"), 1)
                for name, mutated in bad.items():
                    with self.subTest(path=path, job=job, mutation=name):
                        self.assertNotEqual(mutated, body)
                        self.assertNotEqual(
                            routed_job_problems(text.replace(body, mutated, 1), ROUTED[path]), [])

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
