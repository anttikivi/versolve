# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.4.1] - 2026-11-26

### Changed

- Lower the MSRV to 1.87.0.

## [0.4.0] - 2026-11-24

Versolve now focuses on resolving versions: the build script integration returns
the version instead of setting environment variables for the package. See the
migration guide in the README.

### Changed

- **Breaking:** `versolve::build::Config` is now `versolve::cargo::Resolver`,
  and `Config::emit(var)` is replaced by `Resolver::resolve()`, which returns
  the version and prints only Cargo rerun instructions and warnings. Print
  `cargo:rustc-env=<VAR>={version}` from the build script to keep embedding the
  version. Fallback, strict mode, and the branch variable work as before.
- `--version` now prints the full embedded version, including its build
  metadata, such as `versolve 0.4.0+g1a2b3c4`, instead of the version without
  build metadata followed by the commit hash and date.
- The documentation now lists the changes that don't rerun the build script by
  themselves.

### Removed

- **Breaking:** the `<VAR>_COMMIT_HASH`, `<VAR>_COMMIT_SHORT_HASH`, and
  `<VAR>_COMMIT_DATE` variables set by `emit`, along with `Config::time_zone`
  and `TimeZone`. Use a build information crate such as vergen or shadow-rs, or
  collect the details in your build script.
- The `commit-hash` and `commit-date` fields of `--verbose --version`.

## [0.3.0] - 2026-11-22

### Added

- Library API (`Resolver`, `Error`, `ErrorKind`, and re-exported `semver`) with
  debug logging via `log`.
- `versolve::build` module for embedding the resolved version plus
  `<VAR>_COMMIT_HASH`, `<VAR>_COMMIT_SHORT_HASH`, and `<VAR>_COMMIT_DATE` (UTC
  by default, configurable via `Config::time_zone`) from `build.rs`. The build
  script reruns when `HEAD`, refs, Git configuration, or the shallow-clone state
  changes.
- `-V` short option for `--version`, and `-vV` / `--verbose --version` output
  matching the style of `rustc -vV`.
- `vendored-libgit2` Cargo feature (on by default) for building the bundled
  libgit2.

### Changed

- Relicensed under `MIT OR Apache-2.0` (earlier releases remain under
  `GPL-3.0-or-later`).
- `--version` output now matches the style of `rustc --version`, e.g.
  `versolve 0.3.0 (1a2b3c4 2026-11-21)`, omitting the commit info in packaged
  builds.
- SHA-256 repository support, which was always built in to 0.2.0, now requires
  the opt-in `sha256` feature so that library users don't force
  `git2/unstable-sha256` on every `git2` user in their build. A plain
  `cargo install versolve` no longer supports SHA-256 repositories; install with
  `cargo install versolve --features sha256` to keep it.
- Build-script version resolution discovers repositories in parent directories
  while ignoring packaged crates and `.gitignore`d directories.

### Fixed

- `--no-verbose` can now override an earlier `-v` / `--verbose` flag, and vice
  versa, instead of failing as a duplicate option. Repeating `-v` / `--verbose`
  is still an error.
- Detached-`HEAD` warnings when building Versolve from source now suggest
  `VERSOLVE_BUILD_BRANCH` instead of `--branch`.

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

[Unreleased]: https://github.com/anttikivi/versolve/compare/v0.4.1...HEAD
[0.4.1]: https://github.com/anttikivi/versolve/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/anttikivi/versolve/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/anttikivi/versolve/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/anttikivi/versolve/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/anttikivi/versolve/releases/tag/v0.1.0
