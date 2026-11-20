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
precedence the one closest to the current commit is used. A tag is a version tag
if its name, after removing the prefix given with `--tag-prefix`, is a valid
semantic version number. All other tags are ignored. Note that this means tags
like `v1.2.3` are only used if you pass `--tag-prefix v`.

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
> This ordering holds as long as each new tag has a higher version than the
> versions of the commits it is based on and the first version tag is at least
> `0.1.0`. Versolve does not check this. For example, tagging a commit `0.0.1`
> after untagged commits that resolved to `0.1.0-0.<distance>` gives that commit
> a lower version than its parent.
>
> A tag that is higher than the previous tag can still be lower than the commits
> between them. After `1.2.3`, a pre-release tag that starts with `0`, such as
> `1.2.4-0.1`, can sort before `1.2.4-0.<distance>`. After `2.0.0-rc.1`, a tag
> such as `2.0.0-rc.1.0` or `2.0.0-rc.1.0.1` can sort before
> `2.0.0-rc.1.0.<distance>`. Tags such as `1.2.4-rc.1`, `1.2.4`, and
> `2.0.0-rc.2` are safe.

### Branches

The branch name keeps the versions of commits on different branches unique. Use
`--default-branch` to leave it out on your main branch.

Characters that are not allowed in a pre-release identifier, that is anything
other than ASCII letters, digits, and `-`, are replaced with `-`, so
`feature/example` becomes `feature-example`. A branch name with only digits is
prefixed with `b`. Because of this, different branch names can produce the same
identifier, for example `feature/example` and `feature-example`.

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
`git refs migrate --ref-format=files`, which requires Git 2.46 or later.

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

## License

© 2026 Antti Kivi

Versolve is licensed under the
[GNU General Public License v3.0 or later](https://www.gnu.org/licenses/).
