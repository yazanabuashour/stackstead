# Why Stackstead?

Git worktrees isolate source code. They do not, by themselves, isolate the
application that code runs against.

Two agents can have separate branches and still collide through a fixed host
port, the same Compose project, a shared database or volume, a copied URL, or a
cleanup command derived from an ambiguous branch name. Manually assigning each
resource can work for one or two sessions, but it leaves identity spread across
shell history, environment variables, Compose flags, and operator memory.

Stackstead binds those pieces to one durable full ID:

| Concern | Worktrees plus manual Compose | Stackstead |
| --- | --- | --- |
| Source | Separate checkout | Separate or externally owned checkout |
| Runtime | Caller selects a Compose project | Project is derived from the durable ID |
| Ports and URLs | Caller allocates and communicates them | Generated, recorded, and passed to the agent |
| Database and volumes | Easy to share accidentally | Bound to the owned Compose project |
| Agent context | Prompt or shell convention | Generated human and JSON contracts |
| Recovery | Operator reconstructs the target | Exact-ID inspect, logs, up, and repair |
| Teardown | Caller must prove what is safe to remove | Ownership is revalidated and cleanup fails closed |

Stackstead reuses the repository's reviewed Docker Compose topology. It is a
runtime substrate, not an agent scheduler, editor, terminal multiplexer, or
second application definition. Agents and managers keep their existing user
experience while Stackstead owns the runtime contract.

## When Stackstead is a good fit

- Two or more local coding agents need the same application services.
- The repository already has a Docker Compose development setup.
- Port, database, volume, URL, or teardown collisions are slowing parallel work.
- Environments need to survive agent sessions for inspection or recovery.

## When it is not

- A task needs only source files and no local services.
- The repository does not use Docker Compose.
- The desired boundary is a hostile-code sandbox, secret manager, remote
  scheduler, or multi-user authorization system.
- The application depends on intentionally shared host networking, bind mounts,
  external volumes, or services outside the configured runtime.

See the complete [security boundary](../SECURITY.md#security-boundary) and
[Docker Compose guide](compose.md) before adopting Stackstead for sensitive
local data.
