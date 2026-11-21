//! Resolve a unique [semantic version](https://semver.org/) for every revision
//! of a Git repository.
//!
//! Versolve takes the version from the repository's tags. For commits between
//! tags, it builds a pre-release version from the commit's distance to the
//! latest tag and the branch name, so later revisions always get a higher
//! version. See the
//! [README](https://github.com/anttikivi/versolve#how-the-version-is-resolved)
//! for the rules.
//!
//! Use [`Resolver`] to resolve the version of a repository:
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
//! The version is a [`semver::Version`], which this crate re-exports.
//!
//! # Build scripts
//!
//! To embed the version in a program, add Versolve to the build dependencies
//! and use the [`build`] module in the build script:
//!
//! ```no_run
//! fn main() {
//!     versolve::build::Config::new()
//!         .tag_prefix("v")
//!         .default_branch("main")
//!         .branch_env("MYAPP_BUILD_BRANCH")
//!         .emit("MYAPP_VERSION")
//!         .unwrap();
//! }
//! ```
//!
//! The program can then read the version with `env!("MYAPP_VERSION")`.
//!
//! # Logging
//!
//! Versolve logs debug messages about how it resolves the version, such as
//! the tags it skips, with the [`log`](https://docs.rs/log) crate.

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
