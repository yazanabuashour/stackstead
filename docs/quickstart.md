# Quickstart

Stackstead builds on a repository's Docker Compose development setup to give each
coding agent its own worktree, services, ports, and data. Prerequisites are Git,
Docker with the Compose plugin, and the `stackstead` binary.

## Start an environment

If the repository already contains a reviewed and committed `stackstead.yaml`:

```sh
stackstead launch feature-a -- codex
```

`launch` creates and starts a new environment, prints its full ID, URLs, and startup
timings, then runs the command inside the exact generated checkout. It preserves
the environment for inspection if startup or the child command fails.

Use the printed full ID for later operations:

```sh
stackstead inspect <full-id>
stackstead logs <full-id> --tail 200
stackstead stop <full-id>
stackstead destroy <full-id> --yes
```

## Add Stackstead

### Use your coding agent

Open the [Stackstead agent setup v1 guide](agent-setup-v1.md), then paste this into
a coding agent running from the repository root:

> Set up Stackstead in this repository. Follow the Stackstead agent setup v1 guide,
> reuse the existing Compose setup, make the smallest changes needed, and show
> me the diff before committing.

The [versioned agent guide](agent-setup-v1.md) contains the current commands,
repository policy, and safety boundaries.

### Set it up manually

1. Generate and review the contract:

   ```sh
   stackstead init
   stackstead compose plan
   # If the plan requests unambiguous fixed-port rewrites:
   stackstead compose apply --yes
   git diff
   ```

   For a nested Compose file, pass its repository-relative path with
   `stackstead init --compose-file <path>`.

2. Add the [Stackstead repository policy](agent-setup-v1.md#repository-policy) to
   `AGENTS.md`, `CLAUDE.md`, or the equivalent file used by your coding agents.

3. Review and commit `stackstead.yaml`, the Compose changes, and the repository
   policy together. New environments pin that committed contract.

4. Start the first environment:

   ```sh
   stackstead launch feature-a -- codex
   ```

For configuration details, see [Configuration](config.md). For recovery and
cleanup behavior, see [Lifecycle and cleanup](lifecycle.md).
