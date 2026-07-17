# Agent integration

Create, start, and enter a new environment in one command:

```sh
stackstead launch feature-a -- claude
```

`launch` composes `create`, `up`, and `run` for a new environment. It preserves the
environment if startup fails and returns the child command's exit code. It does not
reuse an existing environment.

Run a host command from an existing environment's worktree by its full ID:

```sh
stackstead run <full-id> -- <agent-command> [agent-arguments...]
```

Run a command inside one of its configured, running Compose services:

```sh
stackstead exec <full-id> <service> -- <command> [arguments...]
```

The `--` ends Stackstead option parsing. Everything after it is passed directly
without a shell, so spaces and command-specific flags retain their argument
boundaries. Both commands inherit terminal input and output and return the child
command's exit code. `run` starts the child from the manifest-owned source
checkout. `exec` targets the manifest-owned Compose project and exact service
after verifying that the service is configured, owned, and running.

The examples use a readable slug for onboarding. Automation should capture the
full `stackstead_id` from `stackstead --json create` or `adopt` and use that full ID
for `run`, `exec`, and every destructive command. Inside a `run` wrapper, the
authoritative value is `$STACKSTEAD_ID`; do not resolve the slug again.

`run`, `exec`, and `launch` reject `--json`: stdout and stderr belong directly to
the child and cannot also be a stable Stackstead JSON document. `run` and `exec`
hold a shared run lease, so lifecycle mutation waits for the active command.
`exec` keeps the Compose client in the foreground and hands the lease into that
process. On Unix the `run` wrapper instead uses a small supervisor that retains
the lease and owns the host child's exact process group. If the wrapper is
interrupted, the supervisor terminates and reaps that group before releasing the
lease. Linux also uses child-subreaper support to clean descendants that detach
into a new session. macOS has no equivalent portable subreaper API: process-group
cleanup is exact, but cleanup of a child that deliberately calls `setsid` is best
effort.

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
environment or child arguments.

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
| `stackstead run` | Pin host execution to the checkout, environment, identity, and run lease. |
| `stackstead exec` | Pin service execution to the Compose project, service, runtime ownership, and run lease. |
| Generated context | Supply the exact identity, resources, rules, and commands for one environment. |
