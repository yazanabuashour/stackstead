# Agent integration

Create, start, and enter a new environment in one command:

```sh
stackstead launch feature-a -- claude
```

`launch` composes `create`, `up`, and `run` for a new environment. It preserves the
environment if startup fails and returns the child command's exit code. It does not
reuse an existing environment.

Run a command inside an existing environment by its full ID:

```sh
stackstead run <full-id> -- <agent-command> [agent-arguments...]
```

The `--` ends Stackstead option parsing. Everything after it is passed directly
to the child process without a shell, so spaces and agent-specific flags retain
their argument boundaries. The child runs from the manifest-owned source
checkout with inherited terminal input and output.

The examples use a readable slug for onboarding. Automation should capture the
full `stackstead_id` from `stackstead --json create` or `adopt` and use that full ID
for `run` and every destructive command. Inside the wrapper, the authoritative
value is `$STACKSTEAD_ID`; do not resolve the slug again.

`run` and `launch` reject `--json`: stdout and stderr belong directly to the
child and cannot also be a stable Stackstead JSON document. A shared run lease
prevents `destroy` from removing source or runtime state until the child exits,
while ordinary lifecycle commands remain available to the agent. On Unix the
child inherits that lease, so killing only the Stackstead wrapper cannot silently
unblock teardown beneath a still-running agent.

## Runtime contract

The wrapper injects the generated `.stackstead/.env` values, then pins its
non-secret runtime identity. Most agents need only the first three values;
the rest support scripts and integrations.

| Variable | Meaning |
| --- | --- |
| `STACKSTEAD_ID` | Core: durable stackstead identity |
| `STACKSTEAD_WORKTREE` | Core: exact source checkout and child working directory |
| `STACKSTEAD_CONTEXT` | Core: human/agent-readable contract and project rules |
| `STACKSTEAD_PROJECT` | Stackstead project name |
| `STACKSTEAD_MANIFEST` | Machine-readable runtime contract |
| `STACKSTEAD_ENV_FILE` | Generated environment file; do not print or retain its contents |
| `COMPOSE_PROJECT_NAME` | Manifest-owned identity using Docker Compose's standard variable |

Read these values; do not invent or override them. `STACKSTEAD_ID` is also
written to the generated Compose environment, while the wrapper pins it for
agents, hooks, and other child commands. Stackstead does not print the generated
environment or child arguments. The child still inherits the launching user's
host environment and permissions; this is runtime isolation, not a hostile-code
sandbox.

## Repository instructions

Generated context cannot help an agent that starts normally in the canonical
checkout and does not yet know the project expects Stackstead. Add the copyable
policy from the [agent setup guide](agent-setup.md#repository-policy) to
`AGENTS.md`, `CLAUDE.md`, or the equivalent repository instruction file.
Stackstead recommends this after human-readable `init` output. `doctor` reads
recognized root instruction files to check the policy marker, but Stackstead
never creates or edits those human-owned files.

The layers have separate responsibilities:

| Layer | Responsibility |
| --- | --- |
| Repository instructions | Decide when this project requires Stackstead and how to enter an environment. |
| `stackstead run` | Pin the checkout, environment, identity, and run lease mechanically. |
| Generated context | Supply the exact identity, resources, rules, and commands for one environment. |
