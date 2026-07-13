# Database support

The production database scope is deliberately narrow: one configured Postgres service using a normal Docker Compose-managed volume.

```yaml
database:
  postgres:
    strategy: compose-volume
    service: postgres
    database: app
    user: app
    password: app
    seed:
      command: "sh ./scripts/db/seed-dev.sh"
      shell: false
```

The Postgres host port must also be exposed and generated:

```yaml
resources:
  ports:
    expose:
      postgres:
        container: 5432

env:
  generate:
    POSTGRES_PORT: "{{ ports.postgres }}"
    DATABASE_URL: "postgres://app:app@127.0.0.1:{{ ports.postgres }}/app"
```

The Compose service consumes the generated port:

```yaml
services:
  postgres:
    image: postgres:16-alpine
    environment:
      POSTGRES_DB: app
      POSTGRES_USER: app
      POSTGRES_PASSWORD: app
    ports:
      - "127.0.0.1:${POSTGRES_PORT}:5432"
    volumes:
      - postgres-data:/var/lib/postgresql/data

volumes:
  postgres-data: {}
```

## Startup sequence

`stackstead up <name>`:

1. Generates the environment file.
2. Runs configured dependency setup.
3. Starts the manifest's Compose project.
4. Verifies Compose publishes the configured Postgres container port on the manifest's allocated host port, requires that host port to accept TCP connections, and runs `pg_isready` inside the service until that database/user reports ready.
5. Runs a non-empty seed command from the generated worktree with the generated environment loaded.
6. Records database and seed status in the manifest and event log.

`pg_isready` is Postgres-aware startup readiness, but it is not a schema, migration, credential, or application-query health check. Applications remain responsible for migrations and deeper readiness checks.

`database.postgres.password` documents the configured local service but is not exported automatically. Generate the matching service variables explicitly under `env.generate`, and do not put real secrets in a tracked `stackstead.yaml`.

Seed commands should be safe to rerun. Stackstead records known seed status and last seed time, but it is not a migration framework and does not infer whether application data is current.

## Status

```sh
stackstead db status feature-a
stackstead db status feature-a --json
```

The version 1 `DatabaseStatus` response reports the configured strategy, service, host, allocated port, database, known seed state, and `reachable` TCP probe. `identity_status` is `reachable` only when the manifest's exact Compose project is running, the configured service and container port publish the allocated host endpoint, and that endpoint accepts a TCP connection; it is otherwise `unreachable` or `unknown`. `StacksteadInspection` exposes the same proof under `live.database.{reachable,status}`. Human output labels the raw TCP probe separately and uses the identity-aware status, so a listener owned by another process is not presented as the stackstead database. Status does not connect with a SQL client or expose the configured password.

## State ownership

With a normal Compose-managed volume, the unique Compose project name makes database storage stackstead-local. `stackstead stop` preserves the volume. `stackstead destroy` runs `down -v --remove-orphans --rmi local`, permanently removing that stackstead's Compose volumes and local build images without a custom image tag after confirmation.

External volumes, globally named volumes, and shared host bind mounts bypass project-name isolation. Stackstead does not claim those are branch-local and does not attempt to delete them.

## Not included

Stackstead does not implement schema diffs, snapshots, rollback testing, database forks, shared Postgres administration, query analysis, managed cloud databases, or production deployment. Future database adapters can build on the manifest and command boundaries after real use proves the need; they are not plugins in the current application.
