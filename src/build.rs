//! Embed the resolved version in a package from its build script.
//!
//! Add Versolve to `[build-dependencies]` and call [`Config::emit`] in
//! `build.rs`:
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
//! When the package is outside a Git repository (such as a crates.io tarball)
//! or version resolution fails, the version from `Cargo.toml` is used instead.
//! See [`Config::emit`] for details.
//!
//! When resolved from Git, `emit` also sets variables for the commit hash and
//! date, which lets you print version output in the style of
//! `rustc --version`:
//!
//! ```
//! # macro_rules! env { ($var:literal) => { "1.2.3+g1a2b3c4" } }
//! // Strip build metadata because the short hash is printed separately.
//! let version = env!("MYAPP_VERSION").split('+').next().unwrap();
//! let commit = option_env!("MYAPP_VERSION_COMMIT_SHORT_HASH")
//!     .zip(option_env!("MYAPP_VERSION_COMMIT_DATE"));
//! match commit {
//!     Some((hash, date)) => println!("myapp {version} ({hash} {date})"),
//!     None => println!("myapp {version}"),
//! }
//! ```

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
    resolve::{self, Resolved, Resolver},
};

/// Configures how [`Config::emit`] resolves and embeds a package version.
///
/// See the [module documentation](self) for an example.
#[derive(Clone, Debug, Default)]
pub struct Config {
    tag_prefix: String,
    default_branch: Option<String>,
    branch_env: Option<String>,
    strict: bool,
    time_zone: TimeZone,
}

impl Config {
    /// Creates a configuration with default settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Considers only tags starting with `prefix` (e.g. `v`). Empty by default.
    /// See [`Resolver::tag_prefix`].
    pub fn tag_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.tag_prefix = prefix.into();
        self
    }

    /// Omits the branch name from versions resolved on `name`.
    /// See [`Resolver::default_branch`].
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

    /// Sets the time zone used for `<var>_COMMIT_DATE`. Defaults to
    /// [`TimeZone::Utc`].
    pub fn time_zone(mut self, time_zone: TimeZone) -> Self {
        self.time_zone = time_zone;
        self
    }

    /// Emits Cargo instructions to set `var` to the resolved version and
    /// returns the emitted [`Version`].
    ///
    /// The package reads the version via `env!(<var>)`. Resolution inspects
    /// only committed repository state, so `.dirty` is never appended.
    ///
    /// When the version comes from Git, `emit` also sets three optional
    /// variables readable with `option_env!`:
    ///
    /// - `<var>_COMMIT_HASH`: full commit hash;
    /// - `<var>_COMMIT_SHORT_HASH`: abbreviated commit hash (matching the
    ///   version's build metadata);
    /// - `<var>_COMMIT_DATE`: committer date as `YYYY-MM-DD`, in UTC unless
    ///   [overridden](Config::time_zone).
    ///
    /// These are omitted when falling back to `CARGO_PKG_VERSION`.
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
    /// clone or detached `HEAD` without a tag or branch override), `emit`
    /// prints a `cargo:warning` and falls back to `CARGO_PKG_VERSION`, unless
    /// [strict mode](Config::strict) is enabled.
    ///
    /// # Build script reruns
    ///
    /// When resolving from a Git repository, `emit` prints `rerun-if-changed`
    /// for `HEAD`, refs, Git configuration, `shallow` (if present), and
    /// `Cargo.toml`, plus `rerun-if-env-changed` for the configured
    /// [`branch_env`](Config::branch_env).
    ///
    /// Printing any `rerun-if-*` instruction disables Cargo's default behavior
    /// of rerunning `build.rs` whenever any package file changes. If your build
    /// script reads other files or variables, emit rerun instructions for them
    /// as well.
    ///
    /// If the repository can't be opened (for example because it uses the
    /// reftable format), no Git paths are watched, so fixing the repository
    /// doesn't rerun the build script by itself. The fallback version stays
    /// until a package file changes or you run `cargo clean`.
    ///
    /// # Errors
    ///
    /// Returns an error only in [strict mode](Config::strict), when the package
    /// is in a Git repository and version resolution fails.
    ///
    /// # Panics
    ///
    /// Panics if not called from a Cargo build script, or if writing to stdout
    /// fails.
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

        let (version, commit) = match self.resolve(&manifest_dir, env, &mut out) {
            Ok(Some((version, commit))) => (version, Some(commit)),
            Ok(None) => (pkg_version, None),
            Err(e) if self.strict => return Err(e),
            Err(e) => {
                let hint = match &self.branch_env {
                    Some(name) if e.kind() == ErrorKind::DetachedHead => {
                        format!("; set {name} to name the branch")
                    }
                    _ => String::new(),
                };
                out.print(format_args!("warning=failed to resolve version: {e}{hint}"));
                (pkg_version, None)
            }
        };

        out.print(format_args!("rustc-env={var}={version}"));
        if let Some(commit) = commit {
            out.print(format_args!("rustc-env={var}_COMMIT_HASH={}", commit.hash));
            out.print(format_args!(
                "rustc-env={var}_COMMIT_SHORT_HASH={}",
                commit.short_hash
            ));
            out.print(format_args!("rustc-env={var}_COMMIT_DATE={}", commit.date));
        }

        Ok(version)
    }

    /// Returns `None` when `CARGO_PKG_VERSION` should be used without warning.
    fn resolve(
        &self,
        manifest_dir: &Path,
        env: &dyn Fn(&str) -> Option<OsString>,
        out: &mut Instructions<'_>,
    ) -> Result<Option<(Version, CommitInfo)>, Error> {
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

        let mut resolver = Resolver::new(manifest_dir)
            .tag_prefix(self.tag_prefix.clone())
            .check_dirty(false);
        if let Some(name) = &self.default_branch {
            resolver = resolver.default_branch(name.clone());
        }
        if let Some(name) = branch {
            resolver = resolver.branch(name);
        }

        let resolved = resolver.resolve_repository(&repo)?;
        let commit = CommitInfo::from_resolved(&resolved, self.time_zone);
        Ok(Some((resolved.version, commit)))
    }
}

/// Time zone used for `<var>_COMMIT_DATE` emitted by [`Config::emit`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TimeZone {
    /// Coordinated Universal Time, independent of the committer's local clock.
    #[default]
    Utc,
    /// The time zone recorded in the commit, matching `git log --date=short`.
    Committer,
}

struct CommitInfo {
    hash: String,
    short_hash: String,
    date: String,
}

impl CommitInfo {
    fn from_resolved(resolved: &Resolved, time_zone: TimeZone) -> Self {
        let offset = match time_zone {
            TimeZone::Utc => 0,
            TimeZone::Committer => i64::from(resolved.commit_time.offset_minutes()) * 60,
        };
        let (year, month, day) = civil_date(resolved.commit_time.seconds() + offset);
        Self {
            hash: resolved.commit_id.to_string(),
            short_hash: resolved.short_id.clone(),
            date: format!("{year:04}-{month:02}-{day:02}"),
        }
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

/// Converts seconds since the Unix epoch to a proleptic Gregorian date:
/// <https://howardhinnant.github.io/date_algorithms.html#civil_from_days>.
fn civil_date(seconds: i64) -> (i64, i64, i64) {
    let days = seconds.div_euclid(86_400) + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

fn cargo_env(env: &dyn Fn(&str) -> Option<OsString>, name: &str) -> OsString {
    env(name).unwrap_or_else(|| {
        panic!(
            "{name} is not set; versolve::build::Config::emit must be called from a Cargo build \
            script"
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

    use git2::{Commit, Signature, Time};
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
        assert!(!output.contains("_COMMIT_"), "{output}");
        assert!(
            output.ends_with(&format!("cargo:rustc-env={VAR}=9.9.9\n")),
            "{output}"
        );
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

    fn commit_with_time(repo: &TestRepo, time: Time) {
        let git = Repository::open(repo.path()).unwrap();
        let tree_oid = git.treebuilder(None).unwrap().write().unwrap();
        let tree = git.find_tree(tree_oid).unwrap();
        let sig = Signature::new("Test", "test@example.com", &time).unwrap();
        let parent = git.head().ok().map(|head| head.peel_to_commit().unwrap());
        let parents: Vec<&Commit> = parent.iter().collect();
        git.commit(Some("HEAD"), &sig, &sig, "commit", &tree, &parents)
            .unwrap();
    }

    #[test]
    fn commit_information() {
        // 2025-01-01T00:00:00Z with the committer's time zone offset in
        // minutes, and the expected dates in UTC and in the committer's time
        // zone.
        for (offset, utc_date, committer_date) in [
            (0, "2025-01-01", "2025-01-01"),
            (-60, "2025-01-01", "2024-12-31"),
            (14 * 60, "2025-01-01", "2025-01-01"),
        ] {
            let repo = TestRepo::new();
            commit_with_time(&repo, Time::new(1_735_689_600, offset));
            let short_id = repo.short_head_id();

            for (config, date) in [
                (config(), utc_date),
                (config().time_zone(TimeZone::Utc), utc_date),
                (config().time_zone(TimeZone::Committer), committer_date),
            ] {
                let (result, output) = emit(&config, repo.path(), &[]);
                assert_eq!(result.unwrap().build.as_str(), format!("g{short_id}"));
                assert!(
                    output.ends_with(&format!(
                        "cargo:rustc-env={VAR}_COMMIT_HASH={}\n\
                        cargo:rustc-env={VAR}_COMMIT_SHORT_HASH={short_id}\n\
                        cargo:rustc-env={VAR}_COMMIT_DATE={date}\n",
                        repo.head_id()
                    )),
                    "{offset}, {config:?}: {output}"
                );
            }
        }
    }

    #[test]
    fn civil_dates() {
        for (seconds, date) in [
            (0, (1970, 1, 1)),
            (-1, (1969, 12, 31)),
            (951_782_400, (2000, 2, 29)),
            (1_735_689_599, (2024, 12, 31)),
            (4_107_542_400, (2100, 3, 1)),
            (-11_670_998_400, (1600, 2, 29)),
            (-62_135_596_800, (1, 1, 1)),
            (253_402_300_799, (9999, 12, 31)),
        ] {
            assert_eq!(civil_date(seconds), date, "{seconds}");
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
    fn shallow_marker_only_if_it_exists() {
        let repo = tagged_repo();
        let (_, output) = emit(&config(), repo.path(), &[]);
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
        let (result, output) = emit(&config(), clone.path(), &[]);
        assert_fallback(result, &output);
        assert!(rerun_paths(&output).contains(&shallow), "{output}");
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
                format!(
                    "cargo:rerun-if-changed={marker}\n\
                    cargo:rustc-env={VAR}=9.9.9\n"
                ),
                "{marker}"
            );
        }
    }

    #[test]
    fn no_repository_falls_back() {
        let dir = TempDir::new().unwrap();

        let (result, output) = emit(&config(), dir.path(), &[]);
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
            let (result, output) = emit(&config(), &package, &[]);
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
