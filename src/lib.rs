//! Resolve a unique [semantic version](https://semver.org/) for every revision
//! of a Git repository.
//!
//! Versolve derives versions from the highest-precedence version tag reachable
//! from `HEAD`. For commits after that tag, it constructs a pre-release version
//! from the commit distance and branch name. When tags increase monotonically
//! across history, later revisions sort higher under SemVer precedence rules.
//!
//! # Resolving a version
//!
//! Use [`Resolver`] to resolve a repository directly:
//!
//! ```no_run
//! let version = versolve::Resolver::new(".")
//!     .tag_prefix("v")
//!     .default_branch("main")
//!     .resolve()?;
//! println!("{version}");
//! # Ok::<(), versolve::Error>(())
//! ```
//!
//! The returned version is a [`semver::Version`], re-exported by this crate.
//!
//! ## Versioning rules
//!
//! - **`HEAD` has a version tag:** use that tag's version.
//! - **Highest reachable tag is a release:** `<major>.<minor>.<patch + 1>-0.<distance>[.<branch>]`.
//! - **Highest reachable tag is a pre-release:** `<tag>.0.<distance>[.<branch>]`.
//! - **No reachable version tags:** `0.1.0-0.<distance>[.<branch>]`.
//!
//! Any build metadata on a tag (such as `1.2.3+vendor`) is replaced with
//! `+g<short_hash>` (plus `.dirty` when dirty checking is enabled and the
//! working tree has uncommitted changes). Among reachable tags with equal
//! precedence, Versolve picks the farthest commit so merging an equal-precedence
//! tag cannot lower the version.
//!
//! Branch names are omitted on the configured
//! [`default_branch`](Resolver::default_branch). Otherwise, characters outside
//! `[0-9A-Za-z-]` (and invalid UTF-8) are replaced with `-`, and all-digit
//! branch names are prefixed with `b`.
//!
//! # Build scripts
//!
//! To embed the resolved version in a binary or library, add Versolve to
//! `[build-dependencies]`, call [`build::Config::resolve`] from `build.rs`, and
//! pass the version to the package yourself:
//!
//! ```no_run
//! fn main() {
//!     let version = versolve::build::Config::new()
//!         .tag_prefix("v")
//!         .default_branch("main")
//!         .branch_env("MYAPP_BUILD_BRANCH")
//!         .resolve()
//!         .expect("failed to resolve the version");
//!     println!("cargo:rustc-env=MYAPP_VERSION={version}");
//! }
//! ```
//!
//! The package can then read the version with `env!("MYAPP_VERSION")`.
//!
//! # Requirements and features
//!
//! Versolve requires Rust 1.88.0 or later. With default features it also
//! requires a C compiler to build the bundled libgit2 sources.
//!
//! - `vendored-libgit2` *(enabled by default)*: build libgit2 from the sources
//!   bundled with `libgit2-sys`. Disable default features to link against a
//!   compatible system libgit2 when available.
//! - `sha256` *(off by default)*: enable experimental SHA-256 repository
//!   support in libgit2 (`git2/unstable-sha256`). Because Cargo unifies
//!   features across dependencies, enabling this applies the experimental
//!   SHA-256 libgit2 ABI to every package in the build that uses `git2`.
//!
//! # Logging
//!
//! Versolve logs debug details (such as skipped tags and raw libgit2 errors)
//! through the [`log`](https://docs.rs/log) crate.

#![warn(missing_docs)]
// The build script examples show `fn main` to make clear where the code goes.
#![allow(clippy::needless_doctest_main)]

pub mod build;
mod error;
mod resolve;
#[cfg(test)]
mod test_support;

pub use error::{Error, ErrorKind};
pub use resolve::Resolver;
pub use semver;
