# Installation

Stackstead supports Intel and Apple Silicon macOS, plus x86-64 and ARM64
glibc Linux. It requires Git and Docker with the Compose plugin to manage a
runtime; release binaries do not require Rust.

## Install the latest release

```sh
curl --proto '=https' --tlsv1.2 -fsSL \
  https://github.com/yazanabuashour/stackstead/releases/latest/download/install.sh | sh
```

The installer downloads the binary and release checksums, verifies the exact
asset, and installs `stackstead` to `~/.local/bin`. Add that directory to
`PATH` if needed, then verify the installation:

```sh
stackstead --version
```

Prefer to inspect the [installer source](../install.sh) first?

```sh
curl --proto '=https' --tlsv1.2 -fsSLo stackstead-install.sh \
  https://github.com/yazanabuashour/stackstead/releases/latest/download/install.sh
less stackstead-install.sh
sh stackstead-install.sh
```

Run `sh stackstead-install.sh --help` to select a release, install directory,
fork, or private mirror. The corresponding environment variables are
`STACKSTEAD_VERSION`, `STACKSTEAD_INSTALL_DIR`, `STACKSTEAD_REPOSITORY`, and
`STACKSTEAD_RELEASE_BASE`. Mirrors must use HTTPS and expose GitHub-style
release paths; `file://` is reserved for local installer tests.

Linux x86-64 releases require glibc 2.35 or newer, and Linux ARM64 releases
require glibc 2.39 or newer. Build from source on musl, older glibc, or another
unsupported platform. Windows binaries are not currently published.

## Build from source

From a reviewed checkout, use the repository's pinned Rust toolchain:

```sh
cargo install --locked --path .
```

## Upgrade

Run the latest-release installer again, then run `stackstead doctor`. Also
compare the repository policy in your agent instruction file with the current
[agent setup guide](agent-setup.md#repository-policy); agent instructions can
change independently of the binary.

## Uninstall

Run `stackstead ps` and destroy any retained environments you no longer need,
using each exact full ID. Then remove `stackstead` from `~/.local/bin`, or from
the custom directory passed to the installer. Removing only the binary leaves
worktrees, runtime state, and volumes intact.

## Homebrew formula

Each release includes a generated `stackstead.rb` formula for maintainers of
Homebrew taps. The project does not yet publish its own tap. To render the
formula locally from a release checksum file:

```sh
packaging/homebrew/render-formula.sh VERSION SHA256SUMS stackstead.rb
```

Use the release version without a `v` prefix. Set `STACKSTEAD_REPOSITORY` for a
fork, or pass a release base as the script's fourth argument for a mirror.

## Maintainers

### Publish a release

Create and push a `v`-prefixed semantic version tag that exactly matches
`Cargo.toml`. The release workflow verifies that the tagged commit is on
`origin/main`, runs the full CI workflow, builds and tests every supported
binary, and publishes the checksum-bound bundle through the protected
`release` environment.

### Test packaging locally

The fixture test exercises pinned and latest installation, checksum failure,
and Homebrew formula rendering without network access:

```sh
scripts/test-install.sh
```

The clean-install smoke test uses the real release binary:

```sh
cargo build --locked --release
scripts/test-release-install.sh target/release/stackstead
```
