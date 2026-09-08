# Yarn Classic example

This repository runs `scripts/link-packages.sh` through `dependencies.install`. The script installs dependencies and owns the repository's link recipe. It derives `.stackstead/yarn-links` from `STACKSTEAD_WORKTREE`, rejects symlinked or non-directory paths, and passes that folder explicitly to Yarn. Failures stop the script and fail dependency installation.

The readiness declaration requires the actual `web` Compose service to remain
running. Compose defines its instance count. Package installation is a host
command, not a Compose job, so it is not a required service. There are no
application health checks; application health stays unconfigured with status
`unknown`, independently of runtime readiness.

Prerequisites are Git, Docker Compose, Stackstead, `jq`, and Yarn Classic. `yarn --version` must report 1.x.

Copy the example into its own test repository:

```sh
scratch=$(mktemp -d "${TMPDIR:-/tmp}/stackstead-yarn-classic.XXXXXX")
cp -R examples/yarn-classic "$scratch/project"
cd "$scratch/project"
git init -b main
git add .
git commit -m "Yarn Classic Stackstead example"

stackstead doctor
id=$(stackstead --json create linked-app | jq -er '
  select(.kind == "StacksteadChange" and .version == "1" and .action == "created")
  | .stackstead.stackstead_id')
stackstead up "$id"
stackstead inspect "$id"
stackstead open "$id" web
```

The example has no packages to link. Add your repository's package registration and consumer commands after installation in `scripts/link-packages.sh`. Every `yarn link` invocation must use `--link-folder "$YARN_LINK_FOLDER"`; the script sets this variable only for itself and its children. Keep package paths inside `STACKSTEAD_WORKTREE`, not a shared checkout.

`stackstead up "$id"` and `stackstead repair "$id"` rerun the install script. Make the repository's link commands safe to repeat. To invoke the recipe directly in the environment, use:

```sh
stackstead run "$id" -- sh ./scripts/link-packages.sh
```
