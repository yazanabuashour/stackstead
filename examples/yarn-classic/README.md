# Yarn Classic example

This example shows Stackstead's project-configured Yarn Classic mode. Stackstead installs dependencies, creates a stackstead-local link folder, exports it as `YARN_LINK_FOLDER`, and invokes the repository's link command. It does not infer workspace or package relationships.

Prerequisites are Git, Docker Compose, Stackstead, and Yarn Classic (`yarn --version` reports 1.x).

Copy the example into its own test repository:

```sh
cp -R examples/yarn-classic /tmp/stackstead-yarn-classic
cd /tmp/stackstead-yarn-classic
git init -b main
git add .
git commit -m "Yarn Classic Stackstead example"

stackstead doctor
stackstead create linked-app
stackstead up linked-app
stackstead inspect linked-app
stackstead env linked-app
stackstead open linked-app web
```

`scripts/link-packages.sh` deliberately does only one honest thing: it verifies the generated `YARN_LINK_FOLDER` and writes a marker there. Replace it with repository-specific `yarn link --link-folder "$YARN_LINK_FOLDER" ...` commands when integrating a real Yarn Classic monorepo.

`stackstead repair linked-app` may rerun the configured link command if link state needs regeneration.
