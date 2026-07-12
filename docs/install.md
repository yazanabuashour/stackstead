# Install Stackstead

Release binaries do not require a Rust toolchain. Stackstead itself still expects
Git and Docker with the Compose plugin when it manages a runtime.

## Release installer

Install v0.1.3 from its immutable, checksummed release:

```sh
(
  install_script="$(mktemp)" &&
  trap 'rm -f "$install_script"' 0 &&
  curl --proto '=https' --tlsv1.2 -fsSL \
    --output "$install_script" \
    https://github.com/yazanabuashour/stackstead/releases/download/v0.1.3/install.sh &&
  sh "$install_script" --version 0.1.3
)
```

Each release publishes `install.sh` as an immutable, checksummed release asset.

The installer selects the current macOS or supported glibc Linux architecture, downloads
the matching release binary and `SHA256SUMS`, verifies the exact asset checksum,
then installs `stackstead` to `~/.local/bin`. Add that directory to `PATH` if
needed. Linux x86-64 releases require glibc 2.35 or newer; Linux ARM64 releases
currently require glibc 2.39 or newer. The installer detects musl, unknown
libcs, and older glibc before downloading and fails with a build-from-source
instruction.

`STACKSTEAD_REPOSITORY`, `STACKSTEAD_RELEASE_BASE`, `STACKSTEAD_VERSION`, and
`STACKSTEAD_INSTALL_DIR` are equivalent environment variables. A private mirror
can use `--release-base https://releases.example.com/stackstead`; it must expose
GitHub-style `download/vVERSION/ASSET` and `latest/download/ASSET` paths.
Plain HTTP is rejected. `file://` is accepted only so the installer can be
tested without network access.

Run `sh install.sh --help` for all options. `--version` selects the Stackstead
release to install; `--installer-version` prints the install-script contract
version.

## Build from source

To build from a reviewed checkout with the pinned Rust toolchain:

```sh
cargo install --locked --path .
```

## Homebrew formula

Every release publishes a generated `stackstead.rb` formula alongside the
binaries. A tap can copy that formula without changing its checksums. To render
it locally from a release checksum file:

```sh
STACKSTEAD_REPOSITORY=yazanabuashour/stackstead \
  packaging/homebrew/render-formula.sh 0.1.3 SHA256SUMS stackstead.rb
```

The formula covers Intel and ARM macOS and the documented glibc Linux baselines.
Windows is not published until Stackstead can preserve its agent-run teardown
lease across wrapper termination with native Windows process supervision.

## Publish a release

Publication requires the repository owner's GitHub authority. Create and push
an exact semantic `v`-prefixed tag whose version equals `Cargo.toml`. Preflight resolves
the tag once, verifies that commit is on `origin/main`, and checks the package
version. The complete reusable CI workflow then validates that exact revision
before any matrix build starts. Builds, formula rendering, license assembly, and
checksums all remain read-only; only the final `release` environment job receives
`contents: write`, and it rechecks the live tag before attaching the checksum-bound
bundle to that tag.

The public repository uses a protected `release` environment restricted to `v*`
tags, with no required reviewers or wait timer. A repository ruleset protects
release tags, and immutable releases are enabled. Copy the generated formula
into an owner-controlled Homebrew tap when the tap exists. The workflow never
invents a repository owner or publishes from an untagged local checkout.

## Test packaging locally

The packaging test builds a tiny fixture binary, serves it through `file://`,
checks pinned and latest installation, confirms tampering is rejected, and
renders the Homebrew formula. It makes no network requests:

```sh
scripts/test-install.sh
```

The real-binary clean-install smoke test builds or accepts the native release
binary, creates a local GitHub-style release tree and checksum file, installs it
through `install.sh`, clears the caller environment, and proves the installed
command reports the exact Cargo package version:

```sh
cargo build --locked --release
scripts/test-release-install.sh target/release/stackstead
```

CI runs this test on the native Linux build, and every release matrix runner
runs it against the exact macOS or Linux binary before uploading that artifact.
