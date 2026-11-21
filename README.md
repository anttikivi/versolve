# Versolve

`versolve` resolves a unique version number that follows
[semantic versioning](https://semver.org/) for every revision of a Git
repository. It takes the version from the repository's tags, and for commits
between tags it builds a pre-release version from the commit's distance to the
latest tag. The resolved version numbers are ordered according to semantic
versioning precedence rules so that later revisions always get a higher version.

## Installation

```sh
cargo install versolve
```

Building Versolve requires Rust 1.88.0 or later.

When built from a Git checkout, Versolve resolves its own version from the
committed state of the repository, so the version never has `.dirty` in it. If
`HEAD` is detached, set the branch name with the `VERSOLVE_BUILD_BRANCH`
environment variable. If the version cannot be resolved, the version from
`Cargo.toml` is used.

## Usage

Run Versolve in a repository. It prints the resolved version to the standard
output.

```console
$ versolve --tag-prefix v --default-branch main
1.2.4-0.3+g1a2b3c4
```

| Option                          | Description                                          |
| ------------------------------- | ---------------------------------------------------- |
| `-b`, `--branch <name>`         | Use this branch name instead of the current branch   |
| `-B`, `--default-branch <name>` | Omit the branch name from versions on this branch    |
| `-C`, `--repository <path>`     | Resolve the version for this repository              |
| `-T`, `--tag-prefix <prefix>`   | Consider only tags with this prefix, for example `v` |
| `-v`, `--verbose`               | Print debug information to standard error            |
| `--no-verbose`                  | Do not print debug information                       |
| `-h`, `--help`                  | Print the help message                               |
| `--version`                     | Print the version of Versolve                        |

Versolve exits with status 0 on success, 1 if the version cannot be resolved,
and 2 if the command-line arguments are invalid.

### How the version is resolved

If the current commit has a version tag, Versolve uses it. Otherwise it finds
the highest version tag that is reachable from the current commit. Build
metadata does not affect which tag is the highest, and of tags with equal
precedence the one farthest from the current commit is used, so that merging a
branch with such a tag cannot lower the version. A tag is a version tag if its
name, after removing the prefix given with `--tag-prefix`, is a valid semantic
version number. All other tags are ignored. Note that this means tags like
`v1.2.3` are only used if you pass `--tag-prefix v`.

The distance is the number of commits that are reachable from the current commit
but not from the tag, the same number that `git describe` reports. The version
is then resolved with these rules:

- The current commit is tagged: the version of the tag.
- The latest tag is a release version:
  `<major>.<minor>.<patch + 1>-0.<distance>.<branch>`.
- The latest tag is a pre-release version:
  `<tag version>.0.<distance>.<branch>`.
- There are no version tags: `0.1.0-0.<distance>.<branch>`.

For example, with `--tag-prefix v --default-branch main`:

| Revision                                       | Version                            |
| ---------------------------------------------- | ---------------------------------- |
| Two commits, no tags                           | `0.1.0-0.2+g1a2b3c4`               |
| Tagged `v1.2.3`                                | `1.2.3+g1a2b3c4`                   |
| Three commits after `v1.2.3` on `main`         | `1.2.4-0.3+g1a2b3c4`               |
| Five commits after `v1.2.3` on `feature/login` | `1.2.4-0.5.feature-login+g1a2b3c4` |
| Two commits after `v2.0.0-rc.1` on `main`      | `2.0.0-rc.1.0.2+g1a2b3c4`          |

A development version always sorts after the tag it is based on and before any
later release: `1.2.3` < `1.2.4-0.3` < `1.2.4-0.4` < `1.2.4`, and `2.0.0-rc.1` <
`2.0.0-rc.1.0.2` < `2.0.0-rc.2`.

> [!NOTE]
> This ordering holds as long as each version tag has a higher version than the
> commits it is based on and than every commit, on any branch, whose latest tag
> is lower than it. Otherwise, merging the tagged commit into such a branch
> gives the merge commit a lower version than its other parent. Versolve does
> not check this. In practice, avoid these tags:
>
> - Tags lower than `0.1.0`, such as `0.0.1`, as commits without version tags
>   resolve to `0.1.0-0.<distance>`.
> - Pre-release tags that start with `0`, such as `1.2.4-0.1`, as after `1.2.3`
>   commits resolve to `1.2.4-0.<distance>`, which can sort after the tag.
> - Tags that extend a lower pre-release tag with `.0`, such as `2.0.0-rc.1.0`
>   or `2.0.0-rc.1.0.1`, as after `2.0.0-rc.1` commits resolve to
>   `2.0.0-rc.1.0.<distance>`, which can sort after the tag.
>
> For example, if `main` resolves to `1.2.4-0.4` and a branch with the tag
> `1.2.4-0.1` is merged into it, the merge commit resolves to `1.2.4-0.1.0.5`.
> Tags such as `1.2.4-rc.1`, `1.2.4`, and `2.0.0-rc.2` are safe.

### Branches

The branch name keeps the versions of commits on different branches unique. Use
`--default-branch` to leave it out on your main branch.

Characters that are not allowed in a pre-release identifier, that is anything
other than ASCII letters, digits, and `-`, and bytes that are not valid UTF-8
are replaced with `-`, so `feature/example` becomes `feature-example`. A branch
name with only digits is prefixed with `b`. Because of this, different branch
names can produce the same identifier, for example `feature/example` and
`feature-example`.

If `HEAD` is detached and the current commit is not tagged, pass the branch name
with `--branch`.

### Build metadata

The build metadata contains `g` followed by the abbreviated commit ID. If the
working tree has uncommitted changes, including untracked files that are not
ignored, `.dirty` is appended to it, for example `1.2.4-0.3+g1a2b3c4.dirty`.
Build metadata does not affect the ordering of the versions.

### Repository formats

Versolve reads the repository with [libgit2](https://libgit2.org/). It supports
repositories that use SHA-1 or SHA-256 object IDs. The support for SHA-256 in
libgit2 is still experimental.

Repositories that store their references in the reftable format, for example
ones created with `git init --ref-format=reftable`, are not supported, as
libgit2 doesn't have support for them. To use Versolve in such a repository,
convert its references to the files format with
`git refs migrate --ref-format=files`, which requires Git 2.46 or later. libgit2
also rejects repositories that explicitly set `extensions.refStorage` to
`files`. Remove the setting with `git config --unset extensions.refStorage`.

A split index, enabled with `git update-index --split-index` or
`core.splitIndex`, is not supported either. Turn it off with
`git update-index --no-split-index`.

### GitHub Actions

Versolve needs the full history and the tags of the repository. It refuses to
run in a shallow clone. Partial clones, such as ones made with
`--filter=blob:none`, work. A clone without tags cannot be detected and resolves
to a `0.1.0` pre-release, so make sure the tags are fetched.

With GitHub Actions, check out the repository and pass the branch name, as pull
requests are checked out with a detached `HEAD`:

```yaml
- uses: actions/checkout@v7
  with:
    ref: ${{ github.event.pull_request.head.sha || github.sha }}
    fetch-depth: 0
    filter: blob:none
- run: versolve -T v -B main -b "${GITHUB_HEAD_REF:-$GITHUB_REF_NAME}"
```

## Using Versolve from a build script

Versolve is also a library. To embed the resolved version in your own program,
add Versolve to the build dependencies:

```toml
[build-dependencies]
versolve = "0.3"
```

Then resolve the version in `build.rs`:

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

`emit` sets the environment variable `MYAPP_VERSION` for the package, so the
program can read the version at compile time:

```rust
const VERSION: &str = env!("MYAPP_VERSION");
```

The version is resolved from the committed state of the repository that contains
the package, so it never has `.dirty` in it: a build script can't track every
change in the working tree. The build script is run again when `HEAD`, the
references, the Git configuration, `Cargo.toml`, or the branch variable changes.

> [!IMPORTANT]
> Printing any `rerun-if-changed` or `rerun-if-env-changed` instruction turns
> off Cargo's default of running the build script again whenever any file in the
> package changes, and `emit` prints them. If your build script has other
> inputs, print `cargo:rerun-if-changed` for them too.

### Falling back to the version in `Cargo.toml`

`emit` uses the version from `Cargo.toml` without a warning if the package:

- was packaged with `cargo package`, so it was downloaded from crates.io or
  another registry, vendored with `cargo vendor`, or is the verification build
  of `cargo package` or `cargo publish`;
- is not in a Git repository; or
- is in a directory that the repository ignores, such as a vendor directory
  listed in `.gitignore`.

If the package is in a repository but the version can't be resolved, `emit`
prints a warning and uses the version from `Cargo.toml`. This happens, for
example, in a shallow clone or when `HEAD` is detached at a commit that has no
version tag. `HEAD` is detached not only when CI checks out a pull request but
also in the middle of `git rebase` or `git bisect`. Set the branch variable,
here `MYAPP_BUILD_BRANCH`, to name the branch in those cases. An empty value
counts as unset.

To fail the build instead, turn on strict mode with `.strict(true)`. Use it, for
example, when building releases in CI, so that a shallow clone fails the build
instead of embedding the version from `Cargo.toml`. In strict mode, a build with
a detached `HEAD` fails unless the commit has a version tag or the branch
variable is set. Packages that are not in a repository still use the version
from `Cargo.toml`.

Versolve resolves the version of the repository that contains the package. If
the package's sources are copied into another repository, for example with
`git subtree`, the version is resolved from that repository's tags.

### Building in GitHub Actions

Like the command-line tool, the build script needs the full history and the
tags. Check out the repository and pass the branch name in the variable:

```yaml
- uses: actions/checkout@v7
  with:
    ref: ${{ github.event.pull_request.head.sha || github.sha }}
    fetch-depth: 0
    filter: blob:none
- run: cargo build --release
  env:
    MYAPP_BUILD_BRANCH: ${{ github.head_ref || github.ref_name }}
```

### Requirements and features

The library requires Rust 1.88 or later and a C compiler, as libgit2 is built
from the sources bundled with `libgit2-sys` by default. This makes the first
build slower.

Versolve has two features, both on by default:

- `vendored-libgit2`: always build libgit2 from the bundled sources. Without it,
  a compatible libgit2 installed on the system is used if one is found, and the
  bundled sources otherwise.
- `sha256`: support repositories that use SHA-256 object IDs. This builds
  libgit2 with its experimental SHA-256 support, which is not compatible with
  the regular library, so without `vendored-libgit2` a system library is used
  only if it is built with SHA-256 support.

Cargo combines the features that all packages enable for a dependency. With the
default features, every other build dependency that uses `git2` also gets the
bundled libgit2 with SHA-256 support. To avoid this, turn off the default
features:

```toml
[build-dependencies]
versolve = { version = "0.3", default-features = false }
```

### Minimum supported Rust version

The minimum supported Rust version is raised only in a minor release.

## License

Versolve is distributed under the terms of both the MIT license and the Apache
License (Version 2.0). See [LICENSE-APACHE](LICENSE-APACHE) and
[LICENSE-MIT](LICENSE-MIT) for details.

## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
