# Docker Compose

Stackstead runs the installed Docker Compose command. It parses conventional Compose service/port declarations for onboarding and fixed-port safety; it does not rewrite arbitrary Compose models, Dockerfiles, or application configuration.

## Discover and review

From a repository containing `compose.yaml`, `compose.yml`, `docker-compose.yaml`, or `docker-compose.yml`:

```sh
stackstead init
stackstead compose plan
stackstead compose plan --json
```

The plan identifies service and container ports, reuses an existing Compose variable such as `APP_PORT` when one is already present, proposes a generated variable only for fixed mappings, and infers loopback URLs only for conventional HTTP service/port combinations. `init` uses the same plan to generate `stackstead.yaml`. Unsupported, ambiguous, container-only, ranged, colliding-variable, or non-sequence port declarations are hard safety errors rather than warnings.

For a tracked Compose file nested below the repository top level, select it explicitly:

```sh
stackstead init --compose-file infra/docker/postgres/docker-compose.yml
stackstead compose plan --compose-file infra/docker/postgres/docker-compose.yml
stackstead compose apply --compose-file infra/docker/postgres/docker-compose.yml --yes
```

After `stackstead.yaml` exists with exactly one configured Compose file, `compose
plan` and `compose apply` use that path automatically. Stackstead reports tracked
nested candidates when root discovery fails; it never guesses among multiple
nested candidates.

## Project identity

The Compose project name is always `project.name` + `"-"` + the full stackstead ID. Each manifest records that name. Lifecycle commands independently verify it against the manifest's project and ID, and reject another owner's claim to the same name.

All Compose commands run from the generated source worktree and include the recorded project, generated env file, and configured files. Conceptually:

```sh
docker compose \
  -p <compose-project> \
  --env-file <worktree>/.stackstead/.env \
  -f docker-compose.yml \
  -f <worktree>/.stackstead/compose-ownership.yaml \
  up -d
```

Before launching Docker, Stackstead removes every generated application key from the
subprocess's inherited environment, never applies generated process-control
keys, and pins `COMPOSE_PROJECT_NAME` from the
manifest. Compose then reads the generated values through the explicit
`--env-file`, so a same-named caller variable cannot override an allocated port.
Tracked config cannot define process-, Docker-, or Compose-control keys, so it
cannot redirect executable lookup, the daemon, files, or project identity.

Stop uses `stop`; destroy stops the verified runtime and uses `down -v --remove-orphans --rmi local`. If subsequent Stackstead-owned source removal fails, a pinned helper image restores ownership and permissions under only that exact worktree before one retry. The helper is skipped for external or normally removable source and never mounts a peer worktree. Compose removal deletes only Compose-local build images without a custom image tag while preserving pulled and explicitly tagged images. Logs use the same identity and file list with `logs --tail=<n>`. Stackstead does not rediscover a project by container labels or directory name when a manifest exists.

The final generated override labels every direct service and managed network or
volume with the manifest's cryptographically random runtime token. Stackstead also
holds a labeled claim volume for the Compose namespace. Before start, stop, or
destroy, it enumerates both project-labeled resources and the conventional or
explicit resource names from the reviewed Compose model, then refuses any
missing or mismatched token. A never-started stackstead with no claim and no
resources remains a safe no-op; resources without the matching claim fail
closed. Top-level `include`, service-level `extends`, anonymous volumes, and
undeclared named volumes are rejected because Stackstead cannot verify ownership
of the full resource set they introduce.

## Readiness evidence

Declare required services and roles in
[`runtime.readiness`](config.md#runtimereadiness). Stackstead resolves selected
services and replica counts from the installed Compose command's normalized
model, using the manifest project, file order, generated environment, and
ownership override. Counts follow `scale`, then `deploy.replicas`, then Compose's
default of one instance. Conflicting or invalid counts fail resolution.

Startup captures the raw `COMPOSE_PROFILES` value. Later readiness captures reuse
that value, including its absence, rather than the inspecting caller's profile
selection. Other interpolation inputs must reproduce the startup model; drift
or unavailable evidence prevents `ready`.

The manifest stores a SHA-256 fingerprint of the full normalized model with
canonical object ordering, plus Compose's native configuration hash for each
required service. Native hashes alone omit fields such as build settings,
replica counts, dependency edges, and profiles. Neither fingerprint attests
application source, build-context contents, data, remote image freshness, or job
business inputs. Stackstead does not retain or print the normalized model,
container environment, or health logs as readiness evidence.

Readiness uses ownership-verified container metadata, immutable IDs, current
native hashes, and distinct positive instance ordinals. Ordinals need not be
dense; instances numbered 2 and 3 can satisfy a two-instance requirement. Known
one-off containers do not count. Duplicate or unprovable instance identities,
stale expectations, and missing health or ownership evidence cannot yield
`ready`. Check the requirement issues alongside raw service rows; an exit code
zero remains `exited (0)` and counts as success only for an explicit job.

`ps` and `inspect` are snapshots assembled from separate observations, not atomic
snapshots or real-time guarantees. They share the readiness evaluator. Optional
failures remain visible in service rows but do not alone block the required set.
See the [inspection and list fields](agent-contract.md#inspection-version-4-and-list-version-2).

## Host ports must be variables

For parallel stacksteads, the Compose file must consume the deterministic host port generated by Stackstead:

```yaml
services:
  web:
    image: nginx:alpine
    ports:
      - "127.0.0.1:${WEB_PORT}:80"
```

Pair it with config:

```yaml
resources:
  ports:
    strategy: deterministic
    base: 39000
    stride: 20
    expose:
      web:
        container: 80
        url: "http://127.0.0.1:{{ ports.web }}"

env:
  file: ".stackstead/.env"
  generate:
    WEB_PORT: "{{ ports.web }}"
```

Do not use fixed host mappings:

```yaml
# These prevent safe parallel stacksteads.
ports:
  - "3000:3000"
  - "127.0.0.1:3000:3000"
```

A container-only entry such as `ports: ["80"]` is also rejected: Docker would choose a random host port while the Stackstead manifest advertises a deterministic one. Write `127.0.0.1:${WEB_PORT}:80` explicitly. Stackstead models TCP loopback endpoints, so UDP, concrete non-loopback, and unspecified bindings such as `${WEB_PORT}:80`, `0.0.0.0`, or `::` are rejected. `compose apply` can convert an unqualified fixed mapping such as `3000:80` to the loopback generated form.

`stackstead doctor`, `create`/`adopt`, and `up` structurally validate every `services.*.ports` entry. Every published port must consume the exact `env.generate` key whose value is `{{ ports.<name> }}`, and the container port must match the durable manifest. Fixed, missing, unsupported, inline, aliased, ranged, disconnected, or duplicate-variable mappings fail before Docker runs. To apply only the straightforward line edits:

```sh
stackstead compose plan
stackstead compose apply --yes
git diff -- docker-compose.yml # use the path reported by the plan
```

`apply` requires explicit confirmation and atomically rewrites unambiguous fixed short mappings such as `3000:80`, host-IP forms such as `127.0.0.1:3000:80`, and fixed long-form `published` values. It plans and rewrites from one content snapshot, then refuses to replace a file that changed before persistence. It preserves the tracked file's permissions and refuses duplicate host-port matches rather than guessing. It does not edit container-only mappings, variable mappings, arbitrary extensions, networks, volumes, application configuration, or multiple-file overlays.

## Multiple files

Files are passed to Compose in declared order:

```yaml
runtime:
  provider: docker-compose
  files:
    - compose.yml
    - compose.development.yml
```

Paths are relative to the generated source worktree. Every file must exist during config validation. Override files may omit `services`; they contribute zero port declarations. Keep stackstead-specific substitutions in env variables rather than modifying tracked Compose files at runtime.

Stackstead does not support top-level Compose `include` or service-level `extends`,
including either directive introduced through YAML anchors or merge keys. These
forms can add published ports outside Stackstead's structural safety checks, so
validation fails before Docker runs. List each file explicitly in
`runtime.files` instead, keep service declarations direct, and use ordinary
Compose override mappings for environment-specific changes.

## Host and service commands

Use `run` for a host command rooted in the exact source worktree and `exec` for a
command inside a configured, running Compose service:

```sh
stackstead run <full-id> -- npm test
stackstead exec <full-id> api -- npm test
```

`exec` uses the manifest's Compose project, env file, configured files, and
generated ownership override. It verifies the runtime token and service state,
passes the command arguments directly to Compose, returns the command's exit
code, and holds the shared run lease for the Compose exec process.

Keep Compose service commands in the foreground. Framework modes that detach or
daemonize must be disabled in the project's checked-in configuration; detached
processes are outside Compose lifecycle and teardown coverage.

When a container bind-mounts the generated source checkout, run it as the host
UID/GID or keep dependency output in managed volumes. Root-owned ignored files
inside the checkout can prevent Git from removing the worktree. If Git has
already unregistered a partially removed worktree, Stackstead's retry path removes
only the manifest-owned remainder after ownership is corrected.

## Volume isolation

Compose normally prefixes managed volume names with the project name. Because every stackstead has a different project name, a declaration such as this produces separate database state:

```yaml
services:
  postgres:
    volumes:
      - postgres-data:/var/lib/postgresql/data

volumes:
  postgres-data: {}
```

External networks and volumes, bind mounts that point to shared host data, host networking, and services outside the Compose project remain outside this isolation guarantee. Managed networks and volumes must use Compose's project-scoped names; Stackstead rejects their custom `name` values. It also rejects interpolated resource names and cross-file network/volume redeclarations because they prevent safe ownership checks against the effective Compose model. Stackstead does not rewrite these constructs. Avoid external state when branch-local isolation is required.

## Service names and URLs

The keys under `resources.ports.expose` are Stackstead service names. Their `container` ports document the mapping, while optional URL templates power `stackstead inspect` and `stackstead open`. URL templates must target loopback HTTP(S) hosts (`localhost`, `127.0.0.0/8`, or `::1`); Stackstead will not launch a configured remote page in the browser.

`stackstead open feature-a web --print` prints a URL without opening it. Interactive launch additionally holds the stackstead run lease, verifies the host-wide port lease and Docker runtime token, requires the mapped Compose service to be running, and matches its exact published container/host endpoint before invoking the browser. A service with only a port is reported as `127.0.0.1:<port>` and is not treated as HTTP.

## Logs and troubleshooting

```sh
stackstead logs feature-a --tail 200
stackstead logs feature-a --service web --follow
stackstead inspect feature-a
stackstead doctor
```

Stackstead reports captured command failures with the executable and arguments while redacting obvious secret values. It is not an observability system and does not retain a separate copy of all container logs.
