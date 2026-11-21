# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- Print the build metadata with `--version` only when verbose output is enabled.

## [0.2.0] - 2026-11-21

### Added

- Experimental support for repositories that use SHA-256 object IDs.

### Changed

- When several reachable tags have equal precedence, Versolve now uses the one
  farthest from the current commit, so merging a branch with such a tag can't
  lower the version anymore. Version 0.1.0 documented the closest tag but
  actually picked one by commit time, so the development versions of some
  commits may change.
- Resolving the version is much faster in long histories, as Versolve no longer
  walks back to the oldest version tag.
- Checking for uncommitted changes is faster when the working tree has large
  untracked directories.
- libgit2 is always built from the sources bundled with `libgit2-sys`, so
  updating the system library can't break an installed binary anymore.
- Error messages explain how to fix a version tag that points to a missing
  object, a repository that uses reftable or sets `extensions.refStorage`, and a
  split index.
- libgit2 errors show only the message. The full error is in the verbose output.
- `-h` and `--help` show the help even if other arguments are invalid, and
  invalid arguments suggest `--help`.

### Fixed

- Versolve no longer panics when writing to the standard output or standard
  error fails, for example when piped to a command that has already exited, and
  exits with the documented status instead.
- Shallow clones are now detected in linked worktrees too, instead of failing
  with a missing object or resolving a wrong version.
- Branch names that aren't valid UTF-8 no longer fail. The invalid bytes are
  replaced with `-`.
- Repository paths that aren't valid UTF-8 now also work as `-C<path>` and
  `--repository=<path>`, not only as a separate argument.
- A branch with no commits, for example after `git checkout --orphan`, is no
  longer reported as an empty repository.
- The GitHub Actions example in the README now checks out the pull request's own
  commit instead of GitHub's merge commit.
- When building from source, the build script:
  - no longer reruns on every build in a checkout without `packed-refs`
  - picks up Git configuration changes such as `core.abbrev`
  - warns when it can't open the repository instead of silently using the
    version in `Cargo.toml`.

## [0.1.0] - 2026-11-20

### Added

- Initial release of Versolve, a tool that gives every revision of a Git
  repository a unique semantic version number based on the repository's tags.
- Development versions for commits between tags, built from the distance to the
  latest version tag, the branch name and the commit hash, so that they sort
  correctly under semantic versioning precedence.
- Options to set the tag prefix (`--tag-prefix`), the branch name (`--branch`),
  the default branch to leave out of versions (`--default-branch`), the
  repository path (`--repository`) and verbose output (`--verbose`).
- Distinct exit codes: 0 on success, 1 if the version cannot be resolved and 2
  for invalid arguments.

[Unreleased]: https://github.com/anttikivi/versolve/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/anttikivi/versolve/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/anttikivi/versolve/releases/tag/v0.1.0
