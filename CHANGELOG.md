# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- Always build libgit2 from the sources bundled with the `libgit2-sys` crate
  instead of linking to a compatible version of the system library when one is
  found, so that the installed binary does not break when the system library is
  updated.

### Fixed

- Exit with status 1 and an error message instead of panicking when writing to
  the standard output fails, for example when the output is piped to a command
  that has already exited.

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

[Unreleased]: https://github.com/anttikivi/versolve/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/anttikivi/versolve/releases/tag/v0.1.0
