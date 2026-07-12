# Configuration

Stackstead reads `stackstead.yaml` at the canonical repository root. The file is a checked-in declaration of how a branch-local source checkout becomes a branch-local runtime; generated values and secrets belong in the stackstead state, not in this file.

Run `stackstead init` to discover the first conventional Compose file, published services, container ports, likely HTTP URLs, Postgres, generated port variables, and HTTP health checks. Review the generated contract and `stackstead compose plan`; discovery is deliberately conservative. Stackstead validates the full configuration and fails on unsupported providers, invalid paths or ports, missing Compose files, bad environment names, and unknown template variables.

## Complete example

```yaml
version: "1"
kind: StacksteadProject

project:
  name: loan-platform

source:
  provider: git-worktree
  base: main

state:
  root: "../.stacksteads"

runtime:
  provider: docker-compose
  files:
    - docker-compose.yml
  project_name_template: "{{ project.name }}-{{ stackstead.id }}"

resources:
  ports:
    strategy: deterministic
    base: 39000
    stride: 50
    expose:
      web:
        container: 3000
        url: "http://127.0.0.1:{{ ports.web }}"
      api:
        container: 4000
        url: "http://127.0.0.1:{{ ports.api }}"
      postgres:
        container: 5432

dependencies:
  provider: command
  install:
    command: ""
    shell: false

database:
  postgres:
    strategy: compose-volume
    service: postgres
    database: app
    user: app
    password: app
    seed:
      command: ""
      shell: false

health:
  timeout_seconds: 60
  interval_millis: 500
  checks:
    - name: web
      url: "{{ urls.web }}"
      expect_status: 200

env:
  file: ".stackstead/.env"
  generate:
    WEB_PORT: "{{ ports.web }}"
    API_PORT: "{{ ports.api }}"
    POSTGRES_PORT: "{{ ports.postgres }}"
    DATABASE_URL: "postgres://app:app@127.0.0.1:{{ ports.postgres }}/app"
    STACKSTEAD_ID: "{{ stackstead.id }}"

agent:
  context_file: ".stackstead/AGENT_CONTEXT.md"
  rules:
    - "Use only the generated ports in this stackstead."
    - "Do not connect to the shared development database."
    - "Run stackstead inspect before debugging service failures."

hooks:
  post_create: []
  pre_up: []
  post_up: []
  pre_destroy: []
```

## Top-level fields

`version` must be `"1"` and `kind` must be `StacksteadProject`. Stackstead is pre-release and does not read or migrate older draft contracts. Both header fields are required. Optional sections receive conservative defaults, but `project.name` is required.

### `project`

`name` is a stable project identity. It participates in the state path and default Compose project name, so use a short filesystem- and Docker-safe name.

### `source`

The only supported provider is `git-worktree`. `base` names the local branch or revision from which new stackstead branches start. Stackstead does not clone or fetch a missing base.

### `state`

`root` is resolved relative to the canonical repository root. The recommended `../.stacksteads` keeps generated worktrees and state beside the repository. A root inside the repository is rejected because repository-controlled directory replacement cannot be made race-free with portable path-based filesystem operations.

### `runtime`

The only provider is `docker-compose`. `files` are resolved in the bound source worktree and passed to Compose in order. `project_name_template` must render the durable identity `{{ project.name }}-{{ stackstead.id }}`; this redundancy lets every lifecycle command reject a corrupted or redirected Compose target.

Stackstead appends a generated `.stackstead/compose-ownership.yaml` as the final Compose file on every invocation: `docker compose -p <project> --env-file <env> -f <configured>... -f .stackstead/compose-ownership.yaml ...`. The override applies the manifest v2 `io.stackstead.runtime-token` label to every direct service and managed network or volume. `include`, `extends`, anonymous volumes, undeclared named volumes, managed resource `name` values, interpolated `container_name` values, cross-file redeclarations of top-level networks or volumes, non-boolean `external` values, and resource shapes that cannot be labeled completely are rejected instead of being started without ownership metadata. External networks and volumes remain repository-owned and are not labeled.

Before startup or teardown, Stackstead verifies a deterministic labeled claim volume and every project-labeled or exact conventional/custom-name container, network, and volume. A missing or mismatched label fails closed before Compose can target a peer resource. A newly created stackstead with no claim and no candidate Docker resources may be stopped or destroyed without invoking Compose.

### `resources.ports`

The only strategy is `deterministic`. Stackstead scans slots from zero and computes each host port as:

```text
base + slot * stride + service_index
```

`stride` must fit every exposed service, and every resulting port must be in the TCP range, available on `127.0.0.1`, and absent from the per-user Stackstead lease registry. The registry lives under `$XDG_STATE_HOME/stackstead` or `$HOME/.local/state/stackstead`; its resolved directory is persisted in each manifest so later environment drift cannot redirect verification. It serializes allocation across otherwise unrelated repositories, retains stopped-stackstead leases, fails closed if an initialized registry disappears, and releases an exact owner only after successful destroy cleanup. Service indexes use stable lexical key order, so allocation does not depend on YAML parser map ordering. Each key under `expose` is the service-facing Stackstead name. `container` documents the container port; an optional `url` makes the service available to `stackstead open`.

The Compose file must consume the exact generated host-port variable for every configured `ports.<name>` allocation. The Stackstead name is taken from the variable's `{{ ports.<name> }}` target, so it need not equal the Compose service name. Stackstead structurally rejects fixed, missing, non-TCP, ranged, duplicate, disconnected, concrete non-loopback, or unspecified host-IP mappings before creation or Docker startup. `stackstead compose plan` reports discovered mappings without writing, while `stackstead compose apply --yes` rewrites only unambiguous fixed short mappings and long-form `published` values to loopback generated mappings. Changing the configured service names or container-port contract for an existing stackstead requires recreating that stackstead; `up` and `repair` fail rather than guessing a migration. See [Docker Compose](compose.md).

### `dependencies`

`provider: command` optionally runs `install.command` before Compose starts. With `shell: false`, Stackstead parses the command into an executable and arguments; shell operators such as pipes and redirects do not work. Set `shell: true` only when the repository intentionally requires platform-shell semantics.

`provider: yarn-classic` adds a stackstead-local link folder:

```yaml
dependencies:
  provider: yarn-classic
  install:
    command: "yarn install --frozen-lockfile"
    shell: false
  link:
    enabled: true
    link_folder: ".stackstead/yarn-links"
    command: "sh ./scripts/link-packages.sh"
    shell: false
```

Stackstead exposes the resolved folder as `YARN_LINK_FOLDER`, records link state, and can rerun the configured command during repair. It does not infer package relationships. The [Yarn Classic example](../examples/yarn-classic/README.md) is a runnable reference configuration.

### `database.postgres`

Only `strategy: compose-volume` is supported. `service` must identify the Postgres Compose service, while `database`, `user`, and `password` describe its connection settings. A non-empty seed command runs from the worktree with generated environment values available after Postgres becomes reachable. Make seed commands idempotent.

See [Database behavior](database.md).

### `health`

`up` waits for every configured check after Compose, Postgres seeding, and `post_up` hooks. A check must set exactly one of `url` or `command.command`:

```yaml
health:
  timeout_seconds: 60
  interval_millis: 500
  checks:
    - name: web
      url: "{{ urls.web }}/ready"
      expect_status: 204
    - name: worker
      command:
        command: "./scripts/worker-health"
        shell: false
```

HTTP checks support loopback `http` or `https` URL templates and an expected status from 100 through 599. HTTPS uses WebPKI roots; self-signed, mkcert, or private-CA endpoints need a publicly trusted certificate or a command check that invokes the repository's approved CA-aware client. Redirects are not followed, so the configured status is the status being tested. Command checks use the same direct-command model and generated environment as hooks, with trusted manifest-derived `STACKSTEAD_*` and `COMPOSE_PROJECT_NAME` values applied last. A timeout terminates the configured process tree, makes `up` fail, and persists health as `failed`. A new `up` clears stale health/database readiness before doing work. `inspect` passively re-probes HTTP-only health contracts only while the runtime is reported running; it reports persisted status for command-backed checks rather than executing repository code during inspection.

### `env`

`file` is relative to the generated worktree, normally `.stackstead/.env`. Keys under `generate` must be valid environment variable names. The v1 contract rejects process-, state-location-, and Docker-control names such as `PATH`, `HOME`, `XDG_STATE_HOME`, `LD_*`, `DYLD_*`, and `DOCKER_*`/`COMPOSE_*`; a tracked app contract cannot redirect executable lookup, the global lease registry, or the Docker daemon. Every generated file's exact key set is checked against its manifest before use. Output order is deterministic.

Values whose names contain `PASSWORD`, `TOKEN`, `SECRET`, `KEY`, `CREDENTIAL`, or `AUTH` are redacted from normal output and logs, but the generated env file itself is not a secret store. Keep sensitive source material out of version control and protect the local state directory appropriately.

### `agent`

`context_file` is relative to the generated worktree. `rules` are copied into the generated agent context so repository-specific runtime constraints travel with the stackstead.

### `hooks`

Hooks use the same command model as dependencies. They run at `post_create`, `pre_up`, `post_up`, or `pre_destroy`. Hooks are trusted repository configuration and may have side effects; prefer `shell: false` and idempotent commands.

## Template variables

Stackstead supports simple `{{ key }}` interpolation, not a general template language:

```text
{{ project.name }}
{{ stackstead.id }}
{{ stackstead.slug }}
{{ stackstead.short_id }}
{{ ports.<service> }}
{{ urls.<service> }}
{{ paths.repo_root }}
{{ paths.stackstead_root }}
{{ paths.worktree }}
{{ paths.state_dir }}
```

Whitespace inside the braces is allowed. An unknown variable is an error rather than an unresolved string.

## Path resolution

- `state.root` is relative to the canonical repository root.
- Compose files, `env.file`, `agent.context_file`, dependency commands, seed commands, hooks, and command health checks operate relative to the bound source worktree.
- Once created, a stackstead is rediscovered through `.stackstead/stackstead.json`; commands do not reinterpret the copied config to guess its state root.
