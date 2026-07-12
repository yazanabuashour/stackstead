# Basic Compose example

This example runs a static Nginx site and Postgres with Stackstead-generated host ports. The database volume is managed by the unique Compose project, so two stacksteads do not share it.

Because `stackstead.yaml` belongs at a repository root, copy this directory into a small test repository:

```sh
cp -R examples/basic-compose /tmp/stackstead-basic
cd /tmp/stackstead-basic
git init -b main
git add .
git commit -m "basic Stackstead example"
```

With `stackstead` and Docker Compose installed:

```sh
stackstead doctor
stackstead create feature-a
stackstead up feature-a
stackstead inspect feature-a
stackstead open feature-a web
```

Create a second independent runtime:

```sh
stackstead create feature-b
stackstead up feature-b
stackstead ps
```

The `${WEB_PORT}` and `${POSTGRES_PORT}` mappings in `docker-compose.yml` come from each generated `.stackstead/.env`. Stop preserves the Postgres volume; destroy removes it:

```sh
stackstead stop feature-a
stackstead destroy feature-a
stackstead destroy feature-b
```

The credentials in this example are intentionally trivial local-demo values. Do not reuse them for sensitive data or production.
