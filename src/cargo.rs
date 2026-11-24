//! Resolve a package's version from its Cargo build script.
//!
//! Unlike [`versolve::Resolver`](crate::Resolver), which resolves any
//! repository path, [`Resolver`] finds the package through Cargo's build script
//! environment, falls back to the `Cargo.toml` version when needed, and prints
//! Cargo instructions so that the build script reruns when the version may
//! change.
//!
//! Add Versolve to `[build-dependencies]`, call [`Resolver::resolve`] in
//! `build.rs`, and pass the returned version to the package however you like,
//! for example as an environment variable:
//!
//! ```no_run
//! fn main() {
//!     let version = versolve::cargo::Resolver::new()
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
//! When the package is outside a Git repository (such as a crates.io tarball)
//! or version resolution fails, the version from `Cargo.toml` is used instead.
//! See [`Resolver::resolve`] for details.
//!
//! # Cargo package versions
//!
//! The resolved version is only what the build script passes to the package.
//! Versolve doesn't change `Cargo.toml`, so `CARGO_PKG_VERSION`, dependency
//! resolution, and the version published to a registry still come from there.
//!
//! # Other build information
//!
//! Versolve only resolves the version. Collect other build information, such
//! as the full commit hash or the commit date, in the build script or with a
//! build information crate such as `vergen` or `shadow-rs`, and pass the
//! resolved version to that tool wherever it accepts custom values.
//!
//! A version resolved from Git ends with an abbreviated commit hash, such as
//! `+g1a2b3c4`, but fallback versions have none, an abbreviated hash isn't
//! guaranteed to stay unique, and SemVer precedence ignores build metadata, so
//! don't use it as the commit identifier.

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
    resolve,
};

/// Resolves the version of a package from its Cargo build script.
///
/// Configure it with the builder methods and call [`Resolver::resolve`]. See
/// the [module documentation](self) for an example.
#[derive(Clone, Debug, Default)]
pub struct Resolver {
    tag_prefix: String,
    default_branch: Option<String>,
    branch_env: Option<String>,
    strict: bool,
}

impl Resolver {
    /// Creates a resolver with default settings.
    ///
    /// Unlike [`versolve::Resolver::new`](crate::Resolver::new), this takes no
    /// path: the package directory comes from `CARGO_MANIFEST_DIR` when
    /// [`resolve`](Resolver::resolve) is called.
    pub fn new() -> Self {
        Self::default()
    }

    /// Considers only tags starting with `prefix` (e.g. `v`). Empty by default.
    /// See [`versolve::Resolver::tag_prefix`](crate::Resolver::tag_prefix).
    pub fn tag_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.tag_prefix = prefix.into();
        self
    }

    /// Omits the branch name from versions resolved on `name`. See
    /// [`versolve::Resolver::default_branch`](crate::Resolver::default_branch).
    pub fn default_branch(mut self, name: impl Into<String>) -> Self {
        self.default_branch = Some(name.into());
        self
    }

    /// Names an environment variable that overrides the branch name.
    ///
    /// Useful when `HEAD` may be detached (e.g. in CI, `git rebase`, or
    /// `git bisect`). An empty value counts as unset. When resolving from a Git
    /// repository, changes to this variable rerun the build script.
    pub fn branch_env(mut self, var: impl Into<String>) -> Self {
        self.branch_env = Some(var.into());
        self
    }

    /// Fails instead of falling back to `Cargo.toml` when a repository is found
    /// but version resolution fails.
    ///
    /// Useful for CI release builds where a shallow clone or detached `HEAD`
    /// should abort the build rather than silently embed `CARGO_PKG_VERSION`.
    /// Packages outside a Git repository still fall back to `Cargo.toml`. Off
    /// by default.
    pub fn strict(mut self, strict: bool) -> Self {
        self.strict = strict;
        self
    }

    /// Resolves the version of the package whose build script is running.
    ///
    /// The package is located with `CARGO_MANIFEST_DIR`, and the repository is
    /// discovered from there. Resolution inspects only committed repository
    /// state, so `.dirty` is never appended.
    ///
    /// Unlike [`versolve::Resolver::resolve`](crate::Resolver::resolve), this
    /// has build-script side effects: it prints Cargo instructions to stdout so
    /// that Cargo reruns the build script when the version may change, and a
    /// `cargo:warning` when it falls back. It doesn't pass the version to the
    /// package; print your own instruction for that, such as
    /// `cargo:rustc-env=MYAPP_VERSION={version}`.
    ///
    /// # Fallback behavior
    ///
    /// `CARGO_PKG_VERSION` is used silently (and returned as `Ok`) if the
    /// package:
    ///
    /// - was packaged with `cargo package` (registry downloads, `cargo vendor`,
    ///   or `cargo package`/`cargo publish` verification builds);
    /// - is not inside a Git repository; or
    /// - is in a directory ignored by the repository (such as a `.gitignore`d
    ///   vendor directory).
    ///
    /// If the package is in a repository but resolution fails (e.g. a shallow
    /// clone or detached `HEAD` without a tag or branch override), `resolve`
    /// prints a `cargo:warning` and falls back to `CARGO_PKG_VERSION`, unless
    /// [strict mode](Resolver::strict) is enabled.
    ///
    /// # Build script reruns
    ///
    /// When resolving from a Git repository, `resolve` prints
    /// `rerun-if-changed` for `HEAD`, the `refs` directory, `packed-refs` and
    /// `shallow` (if present), Git configuration, and `Cargo.toml`, plus
    /// `rerun-if-env-changed` for the configured
    /// [`branch_env`](Resolver::branch_env). Changes to the working tree don't
    /// rerun the build script, since they don't affect the version.
    ///
    /// Printing any `rerun-if-*` instruction disables Cargo's default behavior
    /// of rerunning `build.rs` whenever any package file changes. If your build
    /// script reads other files or variables, emit rerun instructions for them
    /// as well.
    ///
    /// Some changes don't rerun the build script by themselves, so the
    /// previous version stays until a package file changes or you run
    /// `cargo clean`:
    ///
    /// - When the package is outside a repository, in an ignored directory, or
    ///   in a repository that can't be opened (for example because it uses the
    ///   reftable format), no Git paths are watched. Creating the repository,
    ///   un-ignoring the directory, or fixing the repository goes unnoticed.
    /// - `packed-refs` and `shallow` are watched only if they exist, because
    ///   Cargo reruns a build script on every build while a watched path is
    ///   missing. `git pack-refs --all` and `git gc` also change `refs` when
    ///   they create `packed-refs`, but if a tool creates it without changing
    ///   `refs`, later changes to it go unnoticed until another watched path
    ///   changes. Likewise, a full clone becoming shallow may go unnoticed,
    ///   which keeps the version resolved from the full history.
    ///
    /// # Errors
    ///
    /// Returns an error only in [strict mode](Resolver::strict), when the
    /// package is in a Git repository and version resolution fails.
    ///
    /// # Panics
    ///
    /// Panics if not called from a Cargo build script, or if writing to stdout
    /// fails.
    pub fn resolve(&self) -> Result<Version, Error> {
        self.resolve_with(&mut io::stdout().lock(), &|name| env::var_os(name))
    }

    fn resolve_with(
        &self,
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

        match self.resolve_from_git(&manifest_dir, env, &mut out) {
            Ok(Some(version)) => Ok(version),
            Ok(None) => Ok(pkg_version),
            Err(e) if self.strict => Err(e),
            Err(e) => {
                let hint = match &self.branch_env {
                    Some(name) if e.kind() == ErrorKind::DetachedHead => {
                        format!("; set {name} to name the branch")
                    }
                    _ => String::new(),
                };
                out.print(format_args!("warning=failed to resolve version: {e}{hint}"));
                Ok(pkg_version)
            }
        }
    }

    /// Returns `None` when `CARGO_PKG_VERSION` should be used without warning.
    fn resolve_from_git(
        &self,
        manifest_dir: &Path,
        env: &dyn Fn(&str) -> Option<OsString>,
        out: &mut Instructions<'_>,
    ) -> Result<Option<Version>, Error> {
        // `cargo package` always writes `Cargo.toml.orig`, and writes
        // `.cargo_vcs_info.json` when packaged from a Git repository. Watching
        // whichever marker exists also forces a rebuild when returning from a
        // `cargo package` verification build to the working checkout in the
        // same target directory.
        let mut packaged = false;
        for marker in [".cargo_vcs_info.json", "Cargo.toml.orig"] {
            if manifest_dir.join(marker).exists() {
                out.print(format_args!("rerun-if-changed={marker}"));
                packaged = true;
            }
        }
        if packaged {
            return Ok(None);
        }

        let repo = match resolve::open_repository(manifest_dir) {
            Err(e) if e.kind() == ErrorKind::RepositoryNotFound => return Ok(None),
            repo => repo?,
        };

        if is_ignored(&repo, manifest_dir)? {
            return Ok(None);
        }

        if let Some(name) = &self.branch_env {
            out.print(format_args!("rerun-if-env-changed={name}"));
        }
        print_rerun_instructions(&repo, out);

        let branch = match &self.branch_env {
            Some(name) => branch_from_env(env, name)?,
            None => None,
        };

        let mut resolver = resolve::Resolver::new(manifest_dir)
            .tag_prefix(self.tag_prefix.clone())
            .check_dirty(false);
        if let Some(name) = &self.default_branch {
            resolver = resolver.default_branch(name.clone());
        }
        if let Some(name) = branch {
            resolver = resolver.branch(name);
        }

        resolver.resolve_repository(&repo).map(Some)
    }
}

/// Uses `cargo:` rather than `cargo::` because Cargo rejects `cargo::` in
/// packages whose `rust-version` is older than 1.77.
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
        panic!(
            "{name} is not set; versolve::cargo::Resolver::resolve must be called from a Cargo \
            build script"
        )
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

fn is_ignored(repo: &Repository, dir: &Path) -> Result<bool, Error> {
    let Some(workdir) = repo.workdir() else {
        return Ok(false);
    };
    // Canonicalize both paths to handle symlinks such as `/tmp` on macOS.
    let (Ok(workdir), Ok(dir)) = (workdir.canonicalize(), dir.canonicalize()) else {
        return Ok(false);
    };
    match dir.strip_prefix(&workdir) {
        Ok(relative) if !relative.as_os_str().is_empty() => Ok(repo.is_path_ignored(relative)?),
        _ => Ok(false),
    }
}

fn print_rerun_instructions(repo: &Repository, out: &mut Instructions<'_>) {
    let git_dir = repo.path();
    let common_dir = repo.commondir();
    // Cargo treats missing rerun-if-changed paths as always changed, so only
    // emit optional paths that exist.
    let optional = [
        Ok(common_dir.join("packed-refs")),
        Ok(common_dir.join("shallow")),
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

    const BRANCH_VAR: &str = "TEST_BRANCH";

    fn resolver() -> Resolver {
        Resolver::new()
            .tag_prefix("v")
            .default_branch("main")
            .branch_env(BRANCH_VAR)
    }

    fn resolve(
        resolver: &Resolver,
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
        let result = resolver.resolve_with(&mut out, &env);
        let output = String::from_utf8(out).unwrap();
        for line in output.lines() {
            assert!(
                line.starts_with("cargo:") && !line.starts_with("cargo::"),
                "{line:?} doesn't use the \"cargo:\" syntax"
            );
            assert!(
                !line.starts_with("cargo:rustc-env="),
                "{line:?} sets a variable for the package"
            );
        }
        (result, output)
    }

    #[track_caller]
    fn assert_fallback(result: Result<Version, Error>) {
        assert_eq!(result.unwrap(), Version::new(9, 9, 9));
    }

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

        let (result, output) = resolve(&resolver(), repo.path(), &[]);
        let expected = format!("1.2.4-0.1+g{}", repo.short_head_id());
        assert_eq!(result.unwrap().to_string(), expected);
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

        let (_, output) = resolve(&resolver(), repo.path(), &[]);
        assert!(!output.contains("packed-refs"), "{output}");

        repo.git(&["pack-refs", "--all"]);
        let (_, output) = resolve(&resolver(), repo.path(), &[]);
        assert!(
            rerun_paths(&output).contains(&canonical(packed_refs)),
            "{output}"
        );
    }

    #[test]
    fn shallow_marker_only_if_it_exists() {
        let repo = tagged_repo();
        let (_, output) = resolve(&resolver(), repo.path(), &[]);
        assert!(!output.contains("shallow"), "{output}");

        let clone = TempDir::new().unwrap();
        repo.git(&[
            "clone",
            "--no-local",
            "--depth=1",
            repo.path().to_str().unwrap(),
            clone.path().to_str().unwrap(),
        ]);
        let shallow = canonical(clone.path().join(".git/shallow"));
        let (result, output) = resolve(&resolver(), clone.path(), &[]);
        assert_fallback(result);
        assert!(rerun_paths(&output).contains(&shallow), "{output}");
    }

    #[test]
    fn package_in_subdirectory() {
        let repo = tagged_repo();
        let package = repo.path().join("crates/app");
        fs::create_dir_all(&package).unwrap();

        let (result, output) = resolve(&resolver(), &package, &[]);
        assert_eq!(
            result.unwrap().to_string(),
            format!("1.2.4-0.1+g{}", repo.short_head_id())
        );
        assert!(!output.contains("cargo:warning="), "{output}");
    }

    #[test]
    fn branch_variable_overrides_current_branch() {
        let repo = tagged_repo();
        repo.git(&["checkout", "-b", "feature"]);
        let short_id = repo.short_head_id();

        for (vars, expected) in [
            (vec![], format!("1.2.4-0.1.feature+g{short_id}")),
            (
                vec![(BRANCH_VAR, OsString::new())],
                format!("1.2.4-0.1.feature+g{short_id}"),
            ),
            (
                vec![(BRANCH_VAR, "main".into())],
                format!("1.2.4-0.1+g{short_id}"),
            ),
            (
                vec![(BRANCH_VAR, "other".into())],
                format!("1.2.4-0.1.other+g{short_id}"),
            ),
        ] {
            let (result, output) = resolve(&resolver(), repo.path(), &vars);
            assert_eq!(result.unwrap().to_string(), expected, "{vars:?}");
            assert!(!output.contains("cargo:warning="), "{vars:?}: {output}");
        }
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

        let (result, output) = resolve(&resolver(), &worktree, &[]);
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

            let (result, output) = resolve(&resolver(), repo.path(), &[]);
            assert_fallback(result);
            assert_eq!(
                output,
                format!("cargo:rerun-if-changed={marker}\n"),
                "{marker}"
            );
        }
    }

    #[test]
    fn no_repository_falls_back() {
        let dir = TempDir::new().unwrap();

        let (result, output) = resolve(&resolver(), dir.path(), &[]);
        assert_fallback(result);
        assert_eq!(output, "");
    }

    #[test]
    fn ignored_directory_falls_back() {
        let repo = tagged_repo();
        repo.write_file(".gitignore", "/vendor/\ntarget\n");
        repo.commit_all("ignore");

        for dir in ["vendor/app", "target", "crates/target/app"] {
            let package = repo.path().join(dir);
            fs::create_dir_all(&package).unwrap();
            let (result, output) = resolve(&resolver(), &package, &[]);
            assert_fallback(result);
            assert_eq!(output, "", "{dir}");
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
            let (result, output) = resolve(&resolver(), repo.path(), &vars);
            assert_fallback(result);
            assert!(
                output.contains(&format!("{warning}; set {BRANCH_VAR} to name the branch\n")),
                "{vars:?}: {output}"
            );
            assert!(rerun_paths(&output).len() > 1, "{vars:?}: {output}");
        }

        let (result, output) = resolve(&Resolver::new(), repo.path(), &[]);
        assert_fallback(result);
        assert!(output.contains(&format!("{warning}\n")), "{output}");

        let (result, output) = resolve(&resolver(), repo.path(), &[(BRANCH_VAR, "main".into())]);
        assert_eq!(
            result.unwrap().to_string(),
            format!("0.1.0-0.1+g{}", repo.short_head_id())
        );
        assert!(!output.contains("cargo:warning="), "{output}");
    }

    #[test]
    fn strict_mode() {
        let repo = tagged_repo();
        let strict = resolver().strict(true);

        let clone = TempDir::new().unwrap();
        repo.git(&[
            "clone",
            "--no-local",
            "--depth=1",
            repo.path().to_str().unwrap(),
            clone.path().to_str().unwrap(),
        ]);
        let (result, output) = resolve(&strict, clone.path(), &[]);
        assert_eq!(result.unwrap_err().kind(), ErrorKind::ShallowClone);
        assert!(!output.contains("cargo:warning="), "{output}");

        repo.git(&["checkout", "--detach", "HEAD"]);
        let (result, output) = resolve(&strict, repo.path(), &[]);
        assert_eq!(result.unwrap_err().kind(), ErrorKind::DetachedHead);
        assert!(!output.contains("cargo:warning="), "{output}");

        let dir = TempDir::new().unwrap();
        let (result, output) = resolve(&strict, dir.path(), &[]);
        assert_fallback(result);
        assert_eq!(output, "");
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_branch_variable() {
        use std::os::unix::ffi::OsStringExt;

        let repo = tagged_repo();
        let vars = [(BRANCH_VAR, OsString::from_vec(b"fo\xff".to_vec()))];

        let (result, output) = resolve(&resolver(), repo.path(), &vars);
        assert_fallback(result);
        assert!(
            output.contains(&format!(
                "cargo:warning=failed to resolve version: environment variable {BRANCH_VAR} is \
                not valid UTF-8"
            )),
            "{output}"
        );

        let strict = resolver().strict(true);
        let (result, _) = resolve(&strict, repo.path(), &vars);
        assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidInput);

        let dir = TempDir::new().unwrap();
        let (result, output) = resolve(&strict, dir.path(), &vars);
        assert_fallback(result);
        assert_eq!(output, "");
    }
}
