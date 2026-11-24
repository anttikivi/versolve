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
versolve = "0.4"
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
`[build-dependencies]`, resolve the version with `versolve::cargo::Resolver` in
`build.rs`, and pass it to your package:

```toml
[build-dependencies]
versolve = "0.4"
```

```rust
fn main() {
    let version = versolve::cargo::Resolver::new()
        .tag_prefix("v")
        .default_branch("main")
        .branch_env("MYAPP_BUILD_BRANCH")
        .resolve()
        .expect("failed to resolve the version");
    println!("cargo:rustc-env=MYAPP_VERSION={version}");
}
```

Then read the version in your code:

```rust
const VERSION: &str = env!("MYAPP_VERSION");
```

The resolved version is the version of your application as derived from Git.
Versolve doesn't change `Cargo.toml`, so `CARGO_PKG_VERSION`, dependency
resolution, and the version you publish to a registry still come from
`Cargo.toml`.

`resolve` uses only committed repository state (never `.dirty`). Unlike
`versolve::Resolver::resolve`, it has build script side effects: it prints
`cargo:rerun-if-changed` / `cargo:rerun-if-env-changed` instructions for `HEAD`,
refs, `packed-refs`, Git config, `shallow`, `Cargo.toml`, and the branch
variable, and a `cargo:warning` when it falls back. It doesn't set any
environment variables for your package; that's up to your build script.

> [!IMPORTANT]
> Printing any `rerun-if-*` instruction turns off Cargo's default of rerunning
> `build.rs` whenever any file in the package changes. If your build script
> reads other files or environment variables, print rerun instructions for them
> too.

#### Fallback and strict mode

`resolve` silently falls back to `CARGO_PKG_VERSION` when the package was
packaged with `cargo package` (crates.io downloads, `cargo vendor`, verification
builds), is outside a Git repository, or sits inside an ignored directory.

If a repository is found but resolution fails (such as a shallow clone or a
detached `HEAD` during CI, `git rebase`, or `git bisect` without `branch_env`
set), `resolve` prints a `cargo:warning` and falls back to `Cargo.toml`. Call
`.strict(true)` to fail the build instead — useful for release builds in CI.
Packages outside a repository still fall back in strict mode.

The branch variable named with `branch_env` overrides the current branch name;
an empty value counts as unset. See the
[`versolve::cargo` API docs](https://docs.rs/versolve/latest/versolve/cargo/)
for full details.

#### Rebuild limitations

The build script reruns when the version may change, but a few changes go
unnoticed until a file in the package changes or you run `cargo clean`:

- Creating a repository around a package that was built outside one, un-ignoring
  the directory that contains the package, or fixing a repository that Versolve
  can't open (such as one using reftable).
- Changes to `packed-refs` if it was created without changing anything under
  `.git/refs`. `git pack-refs --all` and `git gc` do change `refs`, so this
  mostly affects other tools.
- A full clone becoming shallow, which keeps the version resolved from the full
  history.

#### Other build information

Versolve only resolves the version. For the commit hash, commit date, build
time, or other build information, collect it in your build script or use a build
information crate such as [vergen](https://docs.rs/vergen) or
[shadow-rs](https://docs.rs/shadow-rs) alongside Versolve. Neither needs
Versolve-specific support: resolve the version with Versolve and emit it as its
own variable, as above, or pass it to the other tool wherever that tool accepts
custom values.

A version resolved from Git ends with the abbreviated commit hash, such as
`+g1a2b3c4`, but don't rely on it as the commit identifier: fallback versions
have no hash, an abbreviated hash isn't guaranteed to stay unique, and SemVer
precedence ignores build metadata.

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
  versolve = { version = "0.4", default-features = false }
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

## Migrating from 0.3

Version 0.4 narrows the build script integration to resolving the version.
`versolve::build::Config` is now `versolve::cargo::Resolver`, and `emit` is
replaced by `resolve`, which returns the version instead of setting environment
variables. The other options (`tag_prefix`, `default_branch`, `branch_env`, and
`strict`) work as before.

Before:

```rust
versolve::build::Config::new()
    .tag_prefix("v")
    .emit("MYAPP_VERSION")
    .unwrap();
```

After:

```rust
let version = versolve::cargo::Resolver::new()
    .tag_prefix("v")
    .resolve()
    .unwrap();
println!("cargo:rustc-env=MYAPP_VERSION={version}");
```

| Removed                           | Replacement                                                                                            |
| --------------------------------- | ------------------------------------------------------------------------------------------------------ |
| `MYAPP_VERSION`                   | Print it from your build script as shown above.                                                        |
| `MYAPP_VERSION_COMMIT_HASH`       | Collect the full commit hash separately, for example with a build information crate.                   |
| `MYAPP_VERSION_COMMIT_SHORT_HASH` | Collect it separately. A version resolved from Git ends with an abbreviated hash, but fallbacks don't. |
| `MYAPP_VERSION_COMMIT_DATE`       | Collect it separately, for example with a build information crate.                                     |
| `Config::time_zone`, `TimeZone`   | None; they only applied to `MYAPP_VERSION_COMMIT_DATE`.                                                |

See [Other build information](#other-build-information) for collecting the
commit details.

The `versolve --version` output now shows the full embedded version, including
its build metadata (for example `versolve 0.4.0+g1a2b3c4`), and
`versolve --verbose --version` no longer prints `commit-hash` and `commit-date`.

## License

Versolve is distributed under the terms of both the MIT license and the Apache
License (Version 2.0). See [LICENSE-APACHE](LICENSE-APACHE) and
[LICENSE-MIT](LICENSE-MIT) for details.

## Contributing

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
