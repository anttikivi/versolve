# Versolve

`versolve` resolves a unique [semantic version](https://semver.org/) for every
revision of a Git repository. It derives versions from the highest-precedence
version tag reachable from `HEAD`, and for commits after that tag it builds a
pre-release version from the commit distance and branch name. When tags increase
monotonically across history, later revisions sort higher under semantic
versioning precedence rules.

## Installation

```sh
cargo install versolve
```

Building Versolve requires Rust 1.88.0 or later and a C compiler. To include
experimental SHA-256 repository support, install with `--features sha256`.

## CLI usage

Run `versolve` inside a repository to print the resolved version to stdout:

```console
$ versolve --tag-prefix v --default-branch main
1.2.4-0.3+g1a2b3c4
```

| Option                          | Description                                                                    |
| ------------------------------- | ------------------------------------------------------------------------------ |
| `-b`, `--branch <name>`         | Use this branch name instead of the current branch                             |
| `-B`, `--default-branch <name>` | Omit the branch name from versions on this branch                              |
| `-C`, `--repository <path>`     | Resolve the version for this repository                                        |
| `-T`, `--tag-prefix <prefix>`   | Consider only tags with this prefix (e.g. `v`)                                 |
| `-v`, `--verbose`               | Print debug output to stderr, or full version details with `-V`                |
| `--no-verbose`                  | Disable verbose output                                                         |
| `-h`, `--help`                  | Print the help message                                                         |
| `-V`, `--version`               | Print the version of Versolve (combine with `-v` for `rustc -vV` style output) |

Exit status is `0` on success, `1` if the version cannot be resolved, and `2`
for invalid command-line arguments.

## Library usage

Add Versolve to `[dependencies]` to resolve versions directly from Rust code:

```toml
[dependencies]
versolve = "0.3"
```

```rust
let version = versolve::Resolver::new(".")
    .tag_prefix("v")
    .default_branch("main")
    .resolve()?;
println!("{version}");
```

`resolve()` returns a [`semver::Version`](https://docs.rs/semver) (re-exported
by `versolve`). Debug diagnostics are emitted via the
[`log`](https://docs.rs/log) crate.

### Build script integration

To embed the resolved version in your program at compile time, add Versolve to
`[build-dependencies]` and call `versolve::build::Config::emit` in `build.rs`:

```toml
[build-dependencies]
versolve = "0.3"
```

```rust
fn main() {
    versolve::build::Config::new()
        .tag_prefix("v")
        .default_branch("main")
        .branch_env("MYAPP_BUILD_BRANCH")
        .emit("MYAPP_VERSION")
        .unwrap();
}
```

Read the version with `env!("MYAPP_VERSION")`. When resolved from Git, `emit`
also sets `MYAPP_VERSION_COMMIT_HASH`, `MYAPP_VERSION_COMMIT_SHORT_HASH`, and
`MYAPP_VERSION_COMMIT_DATE` (committer date in UTC as `YYYY-MM-DD`; pass
`.time_zone(versolve::build::TimeZone::Committer)` for the committer's time
zone). Read those with `option_env!`, since they are omitted when falling back
to `Cargo.toml`:

```rust
// Strip build metadata because the short hash is printed separately.
let version = env!("MYAPP_VERSION").split('+').next().unwrap();
let commit = option_env!("MYAPP_VERSION_COMMIT_SHORT_HASH")
    .zip(option_env!("MYAPP_VERSION_COMMIT_DATE"));
match commit {
    Some((hash, date)) => println!("myapp {version} ({hash} {date})"),
    None => println!("myapp {version}"),
}
```

`emit` resolves only committed repository state (never `.dirty`) and prints
`cargo:rerun-if-changed` / `cargo:rerun-if-env-changed` instructions for `HEAD`,
refs, Git config, `shallow`, `Cargo.toml`, and the branch variable.

> [!IMPORTANT]
> Printing any `rerun-if-*` instruction turns off Cargo's default of rerunning
> `build.rs` whenever any file in the package changes. If your build script
> reads other files or environment variables, print rerun instructions for them
> too.

#### Fallback and strict mode

`emit` silently falls back to `CARGO_PKG_VERSION` when the package was packaged
with `cargo package` (crates.io downloads, `cargo vendor`, verification builds),
is outside a Git repository, or sits inside an ignored directory.

If a repository is found but resolution fails (such as a shallow clone or a
detached `HEAD` during CI, `git rebase`, or `git bisect` without `branch_env`
set), `emit` prints a `cargo:warning` and falls back to `Cargo.toml`. Call
`.strict(true)` to fail the build instead — useful for release builds in CI. See
the
[`versolve::build` API docs](https://docs.rs/versolve/latest/versolve/build/)
for full details.

## How versions are resolved

If `HEAD` has a version tag, Versolve uses it directly. Otherwise it finds the
highest-precedence version tag reachable from `HEAD`. Build metadata does not
affect precedence; when multiple reachable tags tie on precedence, Versolve uses
the farthest commit so merging a branch with an equal-precedence tag cannot
lower the version.

A tag counts as a version tag if its name, after stripping `--tag-prefix`, is a
valid semantic version. All other tags are ignored — in particular, tags like
`v1.2.3` require `--tag-prefix v`.

Distance is the number of commits reachable from `HEAD` but not from the base
tag (the same count `git describe` reports):

- `HEAD` is tagged: `<tag version>`
- Highest reachable tag is a release:
  `<major>.<minor>.<patch + 1>-0.<distance>[.<branch>]`
- Highest reachable tag is a pre-release:
  `<tag version>.0.<distance>[.<branch>]`
- No reachable version tags: `0.1.0-0.<distance>[.<branch>]`

With `--tag-prefix v --default-branch main`:

| Revision                                       | Version                            |
| ---------------------------------------------- | ---------------------------------- |
| Two commits, no tags                           | `0.1.0-0.2+g1a2b3c4`               |
| Tagged `v1.2.3`                                | `1.2.3+g1a2b3c4`                   |
| Three commits after `v1.2.3` on `main`         | `1.2.4-0.3+g1a2b3c4`               |
| Five commits after `v1.2.3` on `feature/login` | `1.2.4-0.5.feature-login+g1a2b3c4` |
| Two commits after `v2.0.0-rc.1` on `main`      | `2.0.0-rc.1.0.2+g1a2b3c4`          |

Development versions sort after their base tag and before any later release:
`1.2.3` < `1.2.4-0.3` < `1.2.4-0.4` < `1.2.4`, and `2.0.0-rc.1` <
`2.0.0-rc.1.0.2` < `2.0.0-rc.2`.

> [!NOTE]
> Monotonic ordering assumes each tag sorts higher than the commits it is based
> on and higher than any branch it may later merge into. Versolve does not
> enforce this. Avoid tags below `0.1.0` (e.g. `0.0.1`), pre-releases starting
> with `0` (e.g. `1.2.4-0.1`), or tags extending a pre-release with `.0` (e.g.
> `2.0.0-rc.1.0`). For instance, merging `1.2.4-0.1` into a `main` branch
> currently at `1.2.4-0.4` produces `1.2.4-0.1.0.5`, which sorts below
> `1.2.4-0.4`. Conventional tags like `1.2.4-rc.1`, `1.2.4`, and `2.0.0-rc.2`
> are safe.

### Branches

Including the branch name keeps commits on different branches distinct. Use
`--default-branch` to omit it on your main branch.

Characters outside ASCII `[0-9A-Za-z-]` (and invalid UTF-8) are replaced with
`-`, so `feature/example` becomes `feature-example`. All-digit branch names are
prefixed with `b`. Because of sanitization, distinct names such as
`feature/example` and `feature-example` map to the same identifier.

If `HEAD` is detached at an untagged commit, supply the branch name with
`--branch` (or `branch_env` in a build script).

### Build metadata

Versolve always sets the build metadata to `g<short_hash>`, replacing any build
metadata present on the tag itself (so a tag named `1.2.3+vendor` resolves to
`1.2.3+g1a2b3c4`). When the working tree has uncommitted changes — including
untracked files that are not ignored — `.dirty` is appended
(`1.2.4-0.3+g1a2b3c4.dirty`).

### Repository formats

Versolve reads repositories via [libgit2](https://libgit2.org/). Standard SHA-1
repositories work out of the box, and experimental SHA-256 support is available
behind the `sha256` Cargo feature.

Repositories using the reftable reference format, an explicit
`extensions.refStorage` config entry, or a split index are not supported by
libgit2; Versolve detects these and prints the Git command to migrate or disable
them.

### GitHub Actions

Versolve needs full commit history and tags. It refuses shallow clones (partial
clones with `filter: blob:none` work). Because GitHub Actions checks out pull
requests with a detached `HEAD`, pass the branch name explicitly:

```yaml
- uses: actions/checkout@v7
  with:
    ref: ${{ github.event.pull_request.head.sha || github.sha }}
    fetch-depth: 0
    filter: blob:none
- run: versolve -T v -B main -b "${GITHUB_HEAD_REF:-$GITHUB_REF_NAME}"
```

For build-script integration, set your configured `branch_env` instead:

```yaml
- run: cargo build --release
  env:
    MYAPP_BUILD_BRANCH: ${{ github.head_ref || github.ref_name }}
```

## Cargo features

- `vendored-libgit2` _(enabled by default)_: always compile libgit2 from the
  sources bundled with `libgit2-sys` (requires a C compiler). Disable default
  features to link against a compatible system libgit2 when available:

  ```toml
  [build-dependencies]
  versolve = { version = "0.3", default-features = false }
  ```

- `sha256` _(off by default)_: enable experimental SHA-256 object ID support
  (`git2/unstable-sha256`). Because Cargo unifies features across the build
  graph, enabling `sha256` switches every crate using `git2` in the same build
  to the experimental SHA-256 libgit2 ABI.

## Building from source

When built from a Git checkout, Versolve embeds its own version from the
committed repository state (never `.dirty`). If `HEAD` is detached, set
`VERSOLVE_BUILD_BRANCH` to the branch name; otherwise the build falls back to
the version in `Cargo.toml`.

## License

Versolve is distributed under the terms of both the MIT license and the Apache
License (Version 2.0). See [LICENSE-APACHE](LICENSE-APACHE) and
[LICENSE-MIT](LICENSE-MIT) for details.

## Contributing

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
