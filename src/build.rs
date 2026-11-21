//! Embed the resolved version in a package from its build script.
//!
//! Add Versolve to the build dependencies of the package and call
//! [`Config::emit`] in its build script:
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
//! The package can then read the version with `env!("MYAPP_VERSION")`.
//!
//! If the package is not in a Git repository, for example when it is built from
//! crates.io, or if the version can't be resolved, the version from
//! `Cargo.toml` is used instead. See [`Config::emit`] for the details.

use std::{
    env,
    ffi::OsString,
    fmt,
    io::{self, Write},
    path::{Path, PathBuf},
};

use git2::Repository;
use semver::Version;

use super::{
    error::{Error, ErrorKind},
    resolve::{Resolver, open_repository},
};

/// Configures how the version is resolved in a build script.
///
/// See the [module documentation](self) for an example.
#[derive(Clone, Debug, Default)]
pub struct Config {
    tag_prefix: String,
    default_branch: Option<String>,
    branch_env: Option<String>,
    strict: bool,
}

impl Config {
    /// Creates a configuration with the default settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Considers only the tags whose names start with `prefix`, for example
    /// `v`. By default, the prefix is empty. See [`Resolver::tag_prefix`].
    pub fn tag_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.tag_prefix = prefix.into();
        self
    }

    /// Omits the branch name from the versions of commits on the branch `name`.
    /// See [`Resolver::default_branch`].
    pub fn default_branch(mut self, name: impl Into<String>) -> Self {
        self.default_branch = Some(name.into());
        self
    }

    /// Name of an environment variable that overrides the branch name.
    ///
    /// Set the variable when `HEAD` may be detached, for example in CI. An
    /// empty value counts as unset. The build script is run again when the
    /// variable changes.
    pub fn branch_env(mut self, var: impl Into<String>) -> Self {
        self.branch_env = Some(var.into());
        self
    }

    /// Fail instead of falling back when a repository is found but the version
    /// can't be resolved.
    ///
    /// Use this, for example, when building releases in CI, so that a shallow
    /// clone fails the build instead of embedding the version from
    /// `Cargo.toml`. A package that is not in a Git repository still uses the
    /// version from `Cargo.toml`. By default, strict mode is off.
    pub fn strict(mut self, strict: bool) -> Self {
        self.strict = strict;
        self
    }

    /// Prints the Cargo instructions and sets `var` for the package.
    ///
    /// The package can read the version with `env!` using the name `var`. The
    /// version is resolved from the Git repository that contains the package,
    /// from the committed state only, so it never has `.dirty` in its build
    /// metadata. The build script is run again when `HEAD`, the references, the
    /// Git configuration, or `Cargo.toml` changes.
    ///
    /// The version from `Cargo.toml` is used silently if the package:
    ///
    /// - was packaged with `cargo package`, for example when it is downloaded
    ///   from a registry or vendored with `cargo vendor`;
    /// - is not in a Git repository; or
    /// - is in a directory that the repository ignores, such as a vendor
    ///   directory.
    ///
    /// If the version can't be resolved, for example because `HEAD` is detached
    /// at a commit that has no version tag and the branch variable is not set,
    /// the version from `Cargo.toml` is used and a warning is printed, unless
    /// [strict mode](Config::strict) is on.
    ///
    /// The instructions use the `cargo:` syntax so that they work with packages
    /// whose minimum supported Rust version is older than 1.77.
    ///
    /// # Errors
    ///
    /// Returns an error only in strict mode, when the package is in a Git
    /// repository but the version can't be resolved.
    ///
    /// # Panics
    ///
    /// Panics if not called from a Cargo build script, or if writing to the
    /// standard output fails.
    pub fn emit(&self, var: &str) -> Result<Version, Error> {
        self.emit_to(var, &mut io::stdout().lock(), &|name| env::var_os(name))
    }

    fn emit_to(
        &self,
        var: &str,
        out: &mut dyn Write,
        env: &dyn Fn(&str) -> Option<OsString>,
    ) -> Result<Version, Error> {
        let manifest_dir = PathBuf::from(cargo_env(env, "CARGO_MANIFEST_DIR"));
        let pkg_version = cargo_env(env, "CARGO_PKG_VERSION");
        let pkg_version = pkg_version
            .to_str()
            .and_then(|v| Version::parse(v).ok())
            .unwrap_or_else(|| panic!("CARGO_PKG_VERSION is not a valid version: {pkg_version:?}"));
        let mut out = Instructions(out);

        let branch = match &self.branch_env {
            Some(name) => {
                out.print(format_args!("rerun-if-env-changed={name}"));
                branch_from_env(env, name)
            }
            None => Ok(None),
        };

        let version = match self.resolve(&manifest_dir, branch, &mut out) {
            Ok(Some(version)) => version,
            Ok(None) => pkg_version,
            Err(e) if self.strict => return Err(e),
            Err(e) => {
                let hint = match &self.branch_env {
                    Some(name) if e.kind() == ErrorKind::DetachedHead => {
                        format!("; set {name} to name the branch")
                    }
                    _ => String::new(),
                };
                out.print(format_args!("warning=failed to resolve version: {e}{hint}"));
                pkg_version
            }
        };

        out.print(format_args!("rustc-env={var}={version}"));

        Ok(version)
    }

    /// Resolves the version of the repository that contains `manifest_dir`.
    /// Returns `None` if the version from `Cargo.toml` should be used without
    /// a warning.
    fn resolve(
        &self,
        manifest_dir: &Path,
        branch: Result<Option<String>, Error>,
        out: &mut Instructions<'_>,
    ) -> Result<Option<Version>, Error> {
        // `cargo package` always writes `Cargo.toml.orig`, but it writes
        // `.cargo_vcs_info.json` only if the package is in a Git repository.
        // Without these checks, a packaged crate in a Cargo home that is inside
        // a repository would get the version of that repository.
        if [".cargo_vcs_info.json", "Cargo.toml.orig"]
            .iter()
            .any(|f| manifest_dir.join(f).exists())
        {
            return Ok(None);
        }

        let repo = match open_repository(manifest_dir) {
            Err(e) if e.kind() == ErrorKind::RepositoryNotFound => return Ok(None),
            repo => repo?,
        };

        if is_ignored(&repo, manifest_dir)? {
            return Ok(None);
        }

        print_rerun_instructions(&repo, out);

        let mut resolver = Resolver::new(manifest_dir)
            .tag_prefix(self.tag_prefix.clone())
            .check_dirty(false);
        if let Some(name) = &self.default_branch {
            resolver = resolver.default_branch(name.clone());
        }
        if let Some(name) = branch? {
            resolver = resolver.branch(name);
        }

        resolver.resolve_repository(&repo).map(Some)
    }
}

/// Writes Cargo instructions in the `cargo:` syntax.
struct Instructions<'a>(&'a mut dyn Write);

impl Instructions<'_> {
    fn print(&mut self, instruction: fmt::Arguments<'_>) {
        if let Err(e) = writeln!(self.0, "cargo:{instruction}") {
            panic!("failed printing to stdout: {e}");
        }
    }
}

fn cargo_env(env: &dyn Fn(&str) -> Option<OsString>, name: &str) -> OsString {
    env(name).unwrap_or_else(|| {
        panic!("{name} is not set, versolve::build::Config::emit must be called from a Cargo build script")
    })
}

fn branch_from_env(
    env: &dyn Fn(&str) -> Option<OsString>,
    name: &str,
) -> Result<Option<String>, Error> {
    match env(name) {
        None => Ok(None),
        Some(value) if value.is_empty() => Ok(None),
        Some(value) => value.into_string().map(Some).map_err(|value| {
            Error::new(
                ErrorKind::InvalidInput,
                format!("environment variable {name} is not valid UTF-8: {value:?}"),
            )
        }),
    }
}

/// Reports whether `dir` is ignored in the working tree of `repo`, for example
/// because it is a vendor or a target directory.
fn is_ignored(repo: &Repository, dir: &Path) -> Result<bool, Error> {
    let Some(workdir) = repo.workdir() else {
        return Ok(false);
    };
    // Compare the canonical paths, as the paths may go through symbolic links,
    // like `/tmp` on macOS.
    let (Ok(workdir), Ok(dir)) = (workdir.canonicalize(), dir.canonicalize()) else {
        return Ok(false);
    };
    match dir.strip_prefix(&workdir) {
        Ok(relative) if !relative.as_os_str().is_empty() => Ok(repo.is_path_ignored(relative)?),
        _ => Ok(false),
    }
}

/// Prints the instructions to rerun the build script when the committed state
/// of the repository changes. The working tree is not tracked, as the version
/// doesn't depend on it.
fn print_rerun_instructions(repo: &Repository, out: &mut Instructions<'_>) {
    let git_dir = repo.path();
    let common_dir = repo.commondir();
    // Cargo considers a missing path always changed, so print only the paths
    // that exist.
    let optional = [
        Ok(common_dir.join("packed-refs")),
        Ok(common_dir.join("config")),
        Ok(git_dir.join("config.worktree")),
        git2::Config::find_global(),
        git2::Config::find_xdg(),
        git2::Config::find_system(),
    ];
    let paths = [git_dir.join("HEAD"), common_dir.join("refs")]
        .into_iter()
        .chain(optional.into_iter().flatten().filter(|p| p.exists()));
    for path in paths {
        out.print(format_args!("rerun-if-changed={}", path.display()));
    }
    out.print(format_args!("rerun-if-changed=Cargo.toml"));
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use tempfile::TempDir;

    use super::{
        super::test_support::{TestRepo, isolate_libgit2_config},
        *,
    };

    const VAR: &str = "TEST_VERSION";
    const BRANCH_VAR: &str = "TEST_BRANCH";

    fn config() -> Config {
        Config::new()
            .tag_prefix("v")
            .default_branch("main")
            .branch_env(BRANCH_VAR)
    }

    /// Calls `emit_to` for a package in `dir` with the version 9.9.9 and the
    /// additional environment variables `vars`. Returns the result and the
    /// printed instructions.
    fn emit(
        config: &Config,
        dir: &Path,
        vars: &[(&str, OsString)],
    ) -> (Result<Version, Error>, String) {
        isolate_libgit2_config();
        let env = |name: &str| match name {
            "CARGO_MANIFEST_DIR" => Some(dir.into()),
            "CARGO_PKG_VERSION" => Some("9.9.9".into()),
            _ => vars
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.clone()),
        };
        let mut out = Vec::new();
        let result = config.emit_to(VAR, &mut out, &env);
        let output = String::from_utf8(out).unwrap();
        for line in output.lines() {
            assert!(
                line.starts_with("cargo:") && !line.starts_with("cargo::"),
                "{line:?} doesn't use the \"cargo:\" syntax"
            );
        }
        (result, output)
    }

    #[track_caller]
    fn assert_fallback(result: Result<Version, Error>, output: &str) {
        assert_eq!(result.unwrap(), Version::new(9, 9, 9));
        assert!(
            output.ends_with(&format!("cargo:rustc-env={VAR}=9.9.9\n")),
            "{output}"
        );
    }

    /// Returns the canonical paths from the `rerun-if-changed` instructions.
    fn rerun_paths(output: &str) -> Vec<PathBuf> {
        output
            .lines()
            .filter_map(|l| l.strip_prefix("cargo:rerun-if-changed="))
            .map(|p| Path::new(p).canonicalize().unwrap_or_else(|_| p.into()))
            .collect()
    }

    fn canonical(path: impl AsRef<Path>) -> PathBuf {
        path.as_ref().canonicalize().unwrap()
    }

    fn tagged_repo() -> TestRepo {
        let repo = TestRepo::new();
        repo.commit("commit");
        repo.tag("v1.2.3");
        repo.commit("commit");
        repo
    }

    #[test]
    fn repository_at_package_root() {
        let repo = tagged_repo();

        let (result, output) = emit(&config(), repo.path(), &[]);
        let expected = format!("1.2.4-0.1+g{}", repo.short_head_id());
        assert_eq!(result.unwrap().to_string(), expected);
        assert!(
            output.contains(&format!("cargo:rustc-env={VAR}={expected}\n")),
            "{output}"
        );
        assert!(
            output.contains(&format!("cargo:rerun-if-env-changed={BRANCH_VAR}\n")),
            "{output}"
        );
        assert!(
            output.contains("cargo:rerun-if-changed=Cargo.toml\n"),
            "{output}"
        );
        assert!(!output.contains("cargo:warning="), "{output}");
        let paths = rerun_paths(&output);
        for path in [".git/HEAD", ".git/refs", ".git/config"] {
            let path = canonical(repo.path().join(path));
            assert!(paths.contains(&path), "{path:?} not in {output}");
        }
    }

    #[test]
    fn packed_refs_only_if_it_exists() {
        let repo = tagged_repo();
        let packed_refs = repo.path().join(".git/packed-refs");

        let (_, output) = emit(&config(), repo.path(), &[]);
        assert!(!output.contains("packed-refs"), "{output}");

        repo.git(&["pack-refs", "--all"]);
        let (_, output) = emit(&config(), repo.path(), &[]);
        assert!(
            rerun_paths(&output).contains(&canonical(packed_refs)),
            "{output}"
        );
    }

    #[test]
    fn package_in_subdirectory() {
        let repo = tagged_repo();
        let package = repo.path().join("crates/app");
        fs::create_dir_all(&package).unwrap();

        let (result, output) = emit(&config(), &package, &[]);
        assert_eq!(
            result.unwrap().to_string(),
            format!("1.2.4-0.1+g{}", repo.short_head_id())
        );
        assert!(!output.contains("cargo:warning="), "{output}");
    }

    #[test]
    fn linked_worktree() {
        let repo = tagged_repo();
        let dir = TempDir::new().unwrap();
        let worktree = dir.path().join("wt");
        repo.git(&[
            "worktree",
            "add",
            "-b",
            "feature",
            worktree.to_str().unwrap(),
        ]);

        let (result, output) = emit(&config(), &worktree, &[]);
        assert_eq!(
            result.unwrap().to_string(),
            format!("1.2.4-0.1.feature+g{}", repo.short_head_id())
        );
        let paths = rerun_paths(&output);
        for path in [".git/worktrees/wt/HEAD", ".git/refs", ".git/config"] {
            let path = canonical(repo.path().join(path));
            assert!(paths.contains(&path), "{path:?} not in {output}");
        }
        let main_head = canonical(repo.path().join(".git/HEAD"));
        assert!(!paths.contains(&main_head), "{output}");
    }

    #[test]
    fn packaged_crate_falls_back() {
        for marker in [".cargo_vcs_info.json", "Cargo.toml.orig"] {
            let repo = tagged_repo();
            repo.write_file(marker, "\n");

            let (result, output) = emit(&config(), repo.path(), &[]);
            assert_fallback(result, &output);
            assert_eq!(
                output,
                format!("cargo:rerun-if-env-changed={BRANCH_VAR}\ncargo:rustc-env={VAR}=9.9.9\n"),
                "{marker}"
            );
        }
    }

    #[test]
    fn no_repository_falls_back() {
        let dir = TempDir::new().unwrap();

        let (result, output) = emit(&Config::new(), dir.path(), &[]);
        assert_fallback(result, &output);
        assert_eq!(output, format!("cargo:rustc-env={VAR}=9.9.9\n"));
    }

    #[test]
    fn ignored_directory_falls_back() {
        let repo = tagged_repo();
        repo.write_file(".gitignore", "/vendor/\ntarget\n");
        repo.commit_all("ignore");

        for dir in ["vendor/app", "target", "crates/target/app"] {
            let package = repo.path().join(dir);
            fs::create_dir_all(&package).unwrap();
            let (result, output) = emit(&Config::new(), &package, &[]);
            assert_fallback(result, &output);
            assert_eq!(output, format!("cargo:rustc-env={VAR}=9.9.9\n"), "{dir}");
        }
    }

    #[test]
    fn detached_head() {
        let repo = TestRepo::new();
        repo.commit_many(2);
        repo.git(&["checkout", "--detach", "HEAD~1"]);
        let warning = format!(
            "cargo:warning=failed to resolve version: HEAD is detached at {}",
            repo.head_id()
        );

        for vars in [vec![], vec![(BRANCH_VAR, OsString::new())]] {
            let (result, output) = emit(&config(), repo.path(), &vars);
            assert_fallback(result, &output);
            assert!(
                output.contains(&format!("{warning}; set {BRANCH_VAR} to name the branch\n")),
                "{vars:?}: {output}"
            );
            assert!(rerun_paths(&output).len() > 1, "{vars:?}: {output}");
        }

        let (result, output) = emit(&Config::new(), repo.path(), &[]);
        assert_fallback(result, &output);
        assert!(output.contains(&format!("{warning}\n")), "{output}");

        let (result, output) = emit(&config(), repo.path(), &[(BRANCH_VAR, "main".into())]);
        assert_eq!(
            result.unwrap().to_string(),
            format!("0.1.0-0.1+g{}", repo.short_head_id())
        );
        assert!(!output.contains("cargo:warning="), "{output}");
    }

    #[test]
    fn strict_mode() {
        let repo = tagged_repo();
        let strict = config().strict(true);

        let clone = TempDir::new().unwrap();
        repo.git(&[
            "clone",
            "--no-local",
            "--depth=1",
            repo.path().to_str().unwrap(),
            clone.path().to_str().unwrap(),
        ]);
        let (result, output) = emit(&strict, clone.path(), &[]);
        assert_eq!(result.unwrap_err().kind(), ErrorKind::ShallowClone);
        assert!(!output.contains("cargo:rustc-env="), "{output}");
        assert!(!output.contains("cargo:warning="), "{output}");

        repo.git(&["checkout", "--detach", "HEAD"]);
        let (result, output) = emit(&strict, repo.path(), &[]);
        assert_eq!(result.unwrap_err().kind(), ErrorKind::DetachedHead);
        assert!(!output.contains("cargo:rustc-env="), "{output}");

        let dir = TempDir::new().unwrap();
        let (result, output) = emit(&strict, dir.path(), &[]);
        assert_fallback(result, &output);
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_branch_variable() {
        use std::os::unix::ffi::OsStringExt;

        let repo = tagged_repo();
        let vars = [(BRANCH_VAR, OsString::from_vec(b"fo\xff".to_vec()))];

        let (result, output) = emit(&config(), repo.path(), &vars);
        assert_fallback(result, &output);
        assert!(
            output.contains(&format!(
                "cargo:warning=failed to resolve version: environment variable {BRANCH_VAR} is \
                not valid UTF-8"
            )),
            "{output}"
        );

        let strict = config().strict(true);
        let (result, _) = emit(&strict, repo.path(), &vars);
        assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidInput);

        let dir = TempDir::new().unwrap();
        let (result, output) = emit(&strict, dir.path(), &vars);
        assert_fallback(result, &output);
    }
}
