# Claude Managed Agents: an independent reviewer for every pull request

`reviewer.agent.json` defines a server-hosted agent that reviews a forge-warden pull request in
a managed sandbox: it runs the fast gate, ranks the change with deep-diff-forge, judges it with
the **base branch's** `warden.toml`, and reviews the riskiest hunks against
[`CLAUDE.md`](../../CLAUDE.md). It is the same procedure as `.claude/agents/reviewer.md`, run
somewhere the author's session cannot influence.

Managed Agents is a **beta** API. The calls below follow Anthropic's documentation as of
2026-11-05 (sources at the end). Where the docs did not show a detail, this file says so rather
than guessing; check the linked pages before automating.

## 1. Create the agent (once)

```bash
curl https://api.anthropic.com/v1/agents \
  -H 'Content-Type: application/json' \
  -H 'anthropic-version: 2026-10-21' \
  -H 'anthropic-beta: managed-agents-2026-04-01' \
  -H "Authorization: Bearer $ANTHROPIC_API_KEY" \
  -d @agents/managed/reviewer.agent.json
```

Keep the returned agent `id`.

## 2. An environment

The documentation shows environments defined in YAML (`config.type: cloud`,
`networking.type: limited`) and applied with the `ant` CLI. The reviewer needs network access to
crates.io and github.com to build forge-warden and install deep-diff-forge. The REST request for
creating an environment was **not** shown in the pages consulted; use the `ant` CLI or the
environment docs.

## 3. Start a review session on a pull request

```python
import anthropic

client = anthropic.Anthropic()
session = client.beta.sessions.create(
    agent=AGENT_ID,
    environment_id=ENVIRONMENT_ID,
    resources=[{
        "type": "github_repository",
        "url": "https://github.com/Louranicas/forge-warden",   # no .git suffix
        "mount_path": "/workspace/forge-warden",
        "authorization_token": GITHUB_TOKEN,                   # read-only is enough
        "checkout": {"type": "branch", "name": PR_BRANCH},
    }],
)
```

Then send the session a message such as "Review this branch against origin/main." How messages
are sent and events streamed is in the sessions documentation linked below.

Give the token **read** access only: the agent's instructions forbid pushing, and a token that
cannot push makes that a guarantee rather than a request.

## Why an independent agent

The author's session (human or agent) has seen its own reasoning and tends to grade its own work
generously. A reviewer started cold, from the bytes and the contract, does not. Its report is
advice to a human: nothing here merges anything.

## Sources

- https://platform.claude.com/docs/en/managed-agents/quickstart
- https://platform.claude.com/docs/en/api/beta/agents/create
- https://platform.claude.com/docs/en/managed-agents/sessions
- https://platform.claude.com/docs/en/managed-agents/github
