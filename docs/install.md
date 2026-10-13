# Installation

Stackstead supports Intel and Apple Silicon macOS, plus x86-64 and ARM64
glibc Linux. It requires Git and Docker with the Compose plugin to manage a
runtime; release binaries do not require Rust.

## Install the latest release

```sh
curl -fsSL https://github.com/yazanabuashour/stackstead/releases/latest/download/install.sh | sh
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

Before replacing the binary, finish active agent commands and inspect every
retained environment. Complete any started or failed destroy with the same old
binary; retain that pinned binary until the upgrade is verified. Pre-1.0
teardown recovery formats are not imported across releases.

If a worktree manager uses trusted Stackstead hooks, pause its lifecycle
automation and update the binary plus every installed hook copy as one
maintenance operation. Mismatched hook and binary contracts fail closed, and a
rollback must restore the matching pair. Follow the release-specific upgrade
guide before replacing either side. For 0.1.3, use the complete
[0.1.4 upgrade procedure](upgrade-0.1.4.md), including exact-target recovery for
an already failed teardown. For 0.1.4, follow the
[0.1.5 binary and hook rollout](upgrade-0.1.5.md).

After upgrading, run `stackstead doctor` and compare the repository policy in
your agent instruction file with the current
[agent setup guide](agent-setup.md#repository-policy); agent instructions can
change independently of the binary.

## Uninstall

Run `stackstead ps` and destroy any retained environments you no longer need,
using each exact full ID. Then remove `stackstead` from `~/.local/bin`, or from
the custom directory passed to the installer. Removing only the binary leaves
worktrees, runtime state, and volumes intact.

## Homebrew

The project does not publish a Homebrew tap. Releases include a checksummed
`stackstead.rb` formula for tap maintainers; see [release packaging](../CONTRIBUTING.md#release-packaging).
