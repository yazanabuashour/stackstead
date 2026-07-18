# Stackstead early adopter program

Stackstead is looking for developers and small teams already running multiple
coding agents against repositories with Docker Compose. The goal is to validate
onboarding and lifecycle behavior on real applications before a broad launch.

## Good candidates

- You run two or more Codex, Claude Code, Cursor, or other coding-agent sessions
  against the same repository.
- Your application already has a local Docker Compose stack.
- You have encountered port, service, database, volume, URL, or cleanup
  collisions between parallel tasks.
- You use macOS or glibc Linux and are comfortable evaluating pre-1.0 CLI
  software.

Stackstead is not currently recruiting Windows-only, Kubernetes-only,
remote-scheduler, or hostile-code-sandbox use cases.

## What participation looks like

1. Share a non-secret summary of the repository's Compose topology and current
   collision problem.
2. Install the latest checksummed release and let a coding agent follow the
   [setup guide](agent-setup.md).
3. Review and commit the proposed `stackstead.yaml`, Compose edits, and agent
   policy before starting an environment.
4. Run two or more isolated tasks, then exercise inspect, stop/resume, and exact
   teardown.
5. Report where maintainer intervention, manual Docker work, or unclear
   diagnostics were required.

Never attach `.stackstead/.env`, application secrets, private source, or private
database contents. A repository URL is optional; a topology description is
enough to begin.

[Open an early-adopter report](https://github.com/yazanabuashour/stackstead/issues/new?template=early-adopter.yml)
or start a GitHub Discussion. Public case studies are opt-in and reviewed with
the participant before publication.

## Success measures

Before a broad launch, the project is looking for evidence that:

- several outside users can complete setup on unrelated repositories;
- both macOS and Linux users can operate the released binary;
- repeated exact teardown leaves no owned runtime, volume, worktree, or state
  residue;
- most onboarding succeeds without maintainer shell intervention; and
- failures can be understood through `doctor`, `inspect`, and documented
  recovery commands without guessed cleanup targets.

## Copyable invitation

> Worktrees isolate code, not the application behind it. Stackstead gives each
> coding agent its own Compose project, ports, database, URLs, and exact teardown
> while reusing the repository's real stack. It is pre-1.0 on macOS and glibc
> Linux, and we are looking for a small group of Docker Compose projects already
> running parallel coding agents to validate onboarding and lifecycle reliability.
