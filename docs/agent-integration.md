# Agent integration

Create, start, and enter a new environment in one command:

```sh
stackstead launch feature-a -- claude
```

`launch` composes `create`, `up`, and `run` for a new environment. It preserves the
environment if startup fails and returns the child command's exit code. It does not
reuse an existing environment.

When a launcher already runs beneath a generated worktree, resolve and use its
validated full ID:

```sh
id="$(stackstead current)"
stackstead run "$id" -- true
```

`current` validates identity without reading generated environment or probing
Docker. Use `stackstead --json current` when you also need the validated source
ownership, repository, worktree, and pointer binding. See the [current-worktree
contract](agent-contract.md#current-worktree-identity) for validation details.

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
command's exit code. `run` starts the child from the source checkout recorded
in the manifest. `exec` targets the manifest-owned Compose project and exact service
after verifying that the service is configured, owned, and running.

The examples use a readable slug for onboarding. Automation should validate the
`StacksteadChange` version 1 response and expected `created` or `adopted` action,
then read `.stackstead.stackstead_id`. Use that full ID for `run`, `exec`, and
every destructive command. `stackstead --json current` instead returns
`.stackstead_id` at the top level. See the [CLI JSON contract](agent-contract.md#cli-json).
Inside a `run` wrapper, the authoritative value is `$STACKSTEAD_ID`; do not resolve
the slug again.

`run`, `exec`, and `launch` reject `--json`: stdout and stderr belong directly to
the child and cannot also be a stable Stackstead JSON document. `run` and `exec`
hold a shared run lease, so lifecycle mutation waits for the active command.
`exec` keeps the Compose client in the foreground and hands the lease into that
process. On Linux and macOS, `run` uses a private supervisor that retains the
lease and cleans the host child's original process group after normal completion
or wrapper interruption. It observes exit without reaping the leader until all
group signals have finished. Normal execution has no timed polling loop.

Linux also adopts and reaps descendants that detach into another session. macOS
does not reap grandchildren or guarantee cleanup of descendants that escape the
original group. The supervisor never follows a child into an unrelated group.

If the direct child cannot be terminated, the supervisor reports the failure and
retains its lease until that child exits. Other descendant-cleanup failures return
an error and can release the lease with unresolved work. Check those failures
before proceeding with teardown. Supervised commands require the default
`SIGCHLD` disposition without `SA_NOCLDWAIT`; incompatible child-reaping policies
are rejected before target execution.

## Read generated context

Before runtime-sensitive work, read the context and inspect the environment:

```sh
stackstead context <full-id> --print
stackstead inspect <full-id>
```

The context defaults to `.stackstead/AGENT_CONTEXT.md` in the generated worktree.
It supplies the exact identity, resources, generated-file paths, database
endpoint, repository rules, and full-ID commands. Credentials stay in the
generated environment, not the context. Add project-specific constraints through
[`agent.rules`](config.md#agent).

- Stay in the named source checkout and use only its ports, URLs, and environment.
- Do not fall back to shared development data. Document intentional external
  resources or shared host paths in `agent.rules`; they are [not isolated](compose.md#volume-isolation).
- Inspect status and logs before changing startup code; check database
  reachability before migrations.
- Use Stackstead lifecycle commands for stop, recovery, and exact-ID teardown.
  `up` and `repair` may rerun configured work; they are not read-only.

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
| `STACKSTEAD_COMPOSE_PROJECT` | The same Compose identity for host scripts |

Read these values; do not invent or override them. The host wrapper pins
`STACKSTEAD_ID` even when it is absent from `env.generate`. To use it in Compose
interpolation, include it in `env.generate`, as `init` does. Container environment
still belongs to the Compose configuration. Stackstead does not print the
generated environment or child arguments.

`stackstead env <full-id>` reports the generated file's location and redacted
keys; `--print` remains redacted unless you explicitly add `--show-secrets`.
Do not print, copy, or retain `.stackstead/.env` in logs or durable knowledge.
Use `stackstead repair <full-id>` rather than editing generated files. See the
[JSON reference](agent-contract.md#consume-responses-safely) for redaction and
secret-output behavior.

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
