# Stackstead agent setup v1

Use this guide to add Stackstead to an existing Git repository with a Docker
Compose development setup. Make the smallest changes needed and stop before
committing.

## Guardrails

- Preserve unrelated work and existing developer commands.
- Do not print or copy secrets.
- Do not start or destroy an environment during setup.
- Ask before changing authentication, shared data, or application behavior.

## Setup

1. Confirm the repository root is clean enough to distinguish your changes and
   that `stackstead launch --help` and `docker compose version` succeed. If the
   command is unavailable, point the user to the [installation guide](install.md).
2. Run `stackstead init` if `stackstead.yaml` does not exist, or
   `stackstead init --compose-file <repository-relative-path>` for nested Compose
   files. Then run `stackstead compose plan`.
3. If the plan requests only unambiguous fixed-port rewrites, run
   `stackstead compose apply --yes`. Propose any broader application or Compose
   changes before writing them.
4. Add or update the repository's `AGENTS.md`, `CLAUDE.md`, or equivalent with
   the policy below.
5. Run `stackstead doctor`, inspect `git diff`, and report the changes and any
   remaining project-specific work. Do not commit.

## Repository policy

```md
## Stackstead

For tasks that need services, ports, URLs, databases, migrations, or runtime
tests, work in a Stackstead—not the canonical checkout—and use Stackstead lifecycle
commands instead of bare Docker Compose.

If `$STACKSTEAD_CONTEXT` is set, read it, stay in `$STACKSTEAD_WORKTREE`, and use
only the ports, URLs, and database it provides. Otherwise, create a new environment
with `stackstead --json create <name>`, capture its full `stackstead_id`, run
`stackstead up <full-id>`, then enter it with
`stackstead run <full-id> -- <agent-or-command>`. Reuse an environment only when the user
or manager supplies its exact full ID.
```

After the user reviews and commits the setup, the first environment can start with:

```sh
stackstead launch <name> -- <agent-or-command>
```
