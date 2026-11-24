use std::{
    env,
    ffi::OsString,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::{self, Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

const REUSE: &str = "--from=reuse[charset-normalizer]==6.3.0";

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Result<Self> {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let mut name = OsString::from("versolve-e2e-");
        name.push(format!("{}-{nanos}", process::id()));
        let path = env::temp_dir().join(name);
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run() -> Result {
    let mut it = env::args().skip(1);
    match it.next().as_deref() {
        Some("reuse") => sh("uvx", &[REUSE, "reuse", "lint"]),
        Some("e2e") => e2e(),
        _ => {
            eprintln!("usage: cargo xtask <reuse|e2e>");
            process::exit(2)
        }
    }
}

fn main() {
    if let Err(e) = run() {
        eprintln!("xtask: {e}");
        process::exit(1)
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn sh(prog: &str, args: &[&str]) -> Result {
    eprintln!("+ {prog} {}", args.join(" "));
    let status = Command::new(prog).args(args).current_dir(root()).status()?;
    if !status.success() {
        return Err(format!("`{prog}` failed with {status}").into());
    }
    Ok(())
}

const BUILD_SCRIPT: &str = r#"use std::{env, fs::OpenOptions, io::Write};

fn main() {
    // Record each run so that the test can tell whether Cargo reran the build
    // script. Changes to this variable intentionally don't rerun it.
    if let Some(log) = env::var_os("APP_BUILD_LOG") {
        let mut log = OpenOptions::new().create(true).append(true).open(log).unwrap();
        writeln!(log, "run").unwrap();
    }
    let version = versolve::cargo::Resolver::new()
        .tag_prefix("v")
        .default_branch("main")
        .branch_env("APP_BUILD_BRANCH")
        .resolve()
        .unwrap();
    println!("cargo:rustc-env=APP_VERSION={version}");
}
"#;

const MAIN_RS: &str = "fn main() {\n    println!(\"{}\", env!(\"APP_VERSION\"));\n}\n";

/// The version an [`App`] build is expected to embed.
#[derive(Clone, Copy)]
enum Expect<'a> {
    /// Resolved from Git: `<base>+g<abbreviated HEAD>`.
    Git(&'a str),
    /// The `Cargo.toml` version.
    Fallback,
}

/// Whether a build is expected to run the build script. `Any` is for steps
/// where a rerun is harmless but not required.
#[derive(Clone, Copy, PartialEq)]
enum Rerun {
    Yes,
    No,
    Any,
}

/// A package that uses `versolve::cargo` from its build script, built with its
/// own target directory so that fixtures don't share build script results.
struct App {
    name: &'static str,
    /// The workspace root, which is also the repository root once initialized.
    dir: PathBuf,
    target: PathBuf,
    log: PathBuf,
}

impl App {
    fn new(name: &'static str, dir: PathBuf, temp: &Path) -> Self {
        Self {
            name,
            dir,
            target: temp.join(format!("target-{name}")),
            log: temp.join(format!("{name}-build.log")),
        }
    }

    /// Writes the workspace with the `app` package into `self.dir`.
    fn write(&self, root: &Path) -> Result {
        let versolve = root
            .to_str()
            .filter(|p| !p.contains('\''))
            .ok_or("the repository path can't be written as a TOML literal string")?;
        fs::create_dir_all(self.dir.join("app/src"))?;
        fs::write(
            self.dir.join("Cargo.toml"),
            "[workspace]\nmembers = [\"app\"]\nresolver = \"2\"\n",
        )?;
        // The old `rust-version` makes Cargo reject the `cargo::` syntax.
        fs::write(
            self.dir.join("app/Cargo.toml"),
            format!(
                "[package]\n\
                name = \"app\"\n\
                version = \"0.0.0\"\n\
                edition = \"2021\"\n\
                rust-version = \"1.70\"\n\
                publish = false\n\
                \n\
                [build-dependencies]\n\
                versolve = {{ path = '{versolve}' }}\n"
            ),
        )?;
        fs::write(self.dir.join("app/build.rs"), BUILD_SCRIPT)?;
        fs::write(self.dir.join("app/src/main.rs"), MAIN_RS)?;
        fs::copy(root.join("Cargo.lock"), self.dir.join("Cargo.lock"))?;
        Ok(())
    }

    fn cargo_run(&self, env: &[(&str, &str)]) -> Result<Output> {
        let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let mut cmd = Command::new(cargo);
        cmd.args(["run", "--quiet", "--package", "app"])
            .env("CARGO_TARGET_DIR", &self.target)
            .env("APP_BUILD_LOG", &self.log)
            .env_remove("APP_BUILD_BRANCH")
            .current_dir(&self.dir);
        for (key, value) in env {
            cmd.env(key, value);
        }
        // Capture stderr so that failures can be checked, but still show it.
        let output = cmd.output()?;
        io::stderr().write_all(&output.stderr)?;
        Ok(output)
    }

    fn build_script_runs(&self) -> Result<usize> {
        match fs::read_to_string(&self.log) {
            Ok(log) => Ok(log.lines().count()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(0),
            Err(e) => Err(e.into()),
        }
    }

    /// Builds and runs the app, checks the embedded version and whether the
    /// build script ran, and returns the version.
    fn check(
        &self,
        step: &str,
        rerun: Rerun,
        expect: Expect,
        env: &[(&str, &str)],
    ) -> Result<String> {
        let runs = self.build_script_runs()?;
        let output = self.cargo_run(env)?;
        if !output.status.success() {
            return Err(format!(
                "{}: {step}: `cargo run` failed with {}",
                self.name, output.status
            )
            .into());
        }
        let version = String::from_utf8(output.stdout)?.trim_end().to_owned();
        let ran = self.build_script_runs()? > runs;
        eprintln!(
            "e2e: {}: {step}: {version} ({})",
            self.name,
            if ran { "build script ran" } else { "fresh" }
        );

        if (rerun == Rerun::Yes && !ran) || (rerun == Rerun::No && ran) {
            return Err(format!(
                "{}: {step}: expected the build script {}to run",
                self.name,
                if ran { "not " } else { "" }
            )
            .into());
        }

        let matches = match expect {
            Expect::Fallback => version == "0.0.0",
            Expect::Git(base) => {
                let head = git(&self.dir, &["rev-parse", "HEAD"])?;
                version
                    .strip_prefix(base)
                    .and_then(|rest| rest.strip_prefix("+g"))
                    .is_some_and(|id| id.len() >= 7 && head.starts_with(id))
            }
        };
        if !matches {
            let expected = match expect {
                Expect::Fallback => "0.0.0".to_owned(),
                Expect::Git(base) => format!("{base}+g<abbreviated HEAD>"),
            };
            return Err(
                format!("{}: {step}: expected {expected}, got {version}", self.name).into(),
            );
        }
        Ok(version)
    }

    /// Checks that the build fails with an error containing `error`, as it
    /// should in strict mode.
    fn check_fails(&self, step: &str, error: &str) -> Result {
        eprintln!("e2e: {}: {step}: expecting the build to fail", self.name);
        let output = self.cargo_run(&[])?;
        eprintln!("e2e: {}: {step}: {}", self.name, output.status);
        if output.status.success() {
            return Err(format!("{}: {step}: expected `cargo run` to fail", self.name).into());
        }
        if !String::from_utf8_lossy(&output.stderr).contains(error) {
            return Err(format!("{}: {step}: expected the error {error:?}", self.name).into());
        }
        Ok(())
    }

    fn git(&self, args: &[&str]) -> Result<String> {
        git(&self.dir, args)
    }
}

/// Builds packages that use `versolve::cargo` from their build scripts, changes
/// their repositories between builds, and checks both the embedded version and
/// whether Cargo reran the build script.
fn e2e() -> Result {
    use Expect::{Fallback, Git};
    use Rerun::{Any, No, Yes};

    let root = root();
    let temp = TempDir::new()?;
    let temp = temp.0.as_path();

    // Outside a repository, the version falls back to `Cargo.toml` and Cargo's
    // default of rerunning the build script when package files change applies.
    let app = App::new("main", temp.join("repo"), temp);
    app.write(&root)?;
    if git(&app.dir, &["rev-parse", "--git-dir"]).is_ok() {
        return Err(format!("{} is inside a Git repository", app.dir.display()).into());
    }
    app.check("no repository", Yes, Fallback, &[])?;
    app.check("no repository, no changes", No, Fallback, &[])?;

    // Creating a repository around the package doesn't rerun the build script
    // by itself; changing a package file does. This is a documented
    // limitation, so update the documentation if Cargo starts detecting it.
    app.git(&["init", "--initial-branch=main"])?;
    app.git(&["add", "--all"])?;
    app.git(&["commit", "-m", "initial"])?;
    app.git(&["tag", "-a", "v1.2.3", "-m", "v1.2.3"])?;
    app.check("repository created", No, Fallback, &[])?;
    fs::write(
        app.dir.join("app/src/main.rs"),
        format!("{MAIN_RS}// edited\n"),
    )?;
    app.check("package file changed", Yes, Git("1.2.3"), &[])?;
    app.check("no changes", No, Git("1.2.3"), &[])?;

    // Only committed state matters, so a dirty working tree doesn't rerun the
    // build script.
    fs::write(app.dir.join("untracked.txt"), "untracked\n")?;
    fs::write(
        app.dir.join("app/src/main.rs"),
        format!("{MAIN_RS}// edited again\n"),
    )?;
    app.check("dirty working tree", No, Git("1.2.3"), &[])?;

    app.git(&["commit", "--allow-empty", "-m", "commit"])?;
    app.check("commit", Yes, Git("1.2.4-0.1"), &[])?;
    app.git(&["checkout", "-b", "feature"])?;
    app.check("new branch", Yes, Git("1.2.4-0.1.feature"), &[])?;
    app.git(&["commit", "--allow-empty", "-m", "commit"])?;
    app.check("commit on branch", Yes, Git("1.2.4-0.2.feature"), &[])?;
    app.git(&["checkout", "main"])?;
    app.check("switch branch", Yes, Git("1.2.4-0.1"), &[])?;

    app.git(&["tag", "-a", "v1.3.0", "-m", "v1.3.0"])?;
    app.check("create tag", Yes, Git("1.3.0"), &[])?;
    app.git(&["tag", "-d", "v1.3.0"])?;
    app.check("delete tag", Yes, Git("1.2.4-0.1"), &[])?;
    app.git(&["pack-refs", "--all"])?;
    app.check("pack refs", Any, Git("1.2.4-0.1"), &[])?;
    app.git(&["tag", "-d", "v1.2.3"])?;
    app.check("delete packed tag", Yes, Git("0.1.0-0.2"), &[])?;

    let release = [("APP_BUILD_BRANCH", "release")];
    app.check(
        "set branch variable",
        Yes,
        Git("0.1.0-0.2.release"),
        &release,
    )?;
    app.check(
        "same branch variable",
        No,
        Git("0.1.0-0.2.release"),
        &release,
    )?;
    app.check("unset branch variable", Yes, Git("0.1.0-0.2"), &[])?;

    app.git(&["config", "core.abbrev", "12"])?;
    let version = app.check("change core.abbrev", Yes, Git("0.1.0-0.2"), &[])?;
    if !version.ends_with(&app.git(&["rev-parse", "--short=12", "HEAD"])?) {
        return Err(
            format!("main: change core.abbrev: expected a 12-digit hash, got {version}").into(),
        );
    }

    app.git(&["checkout", "--detach"])?;
    app.check("detach HEAD", Yes, Fallback, &[])?;
    app.git(&["checkout", "main"])?;
    app.check("reattach HEAD", Yes, Git("0.1.0-0.2"), &[])?;

    // A linked worktree has its own HEAD but shares refs with the main
    // worktree.
    let worktree = App::new("worktree", temp.join("worktree"), temp);
    app.git(&["worktree", "add", "-b", "wt", path_str(&worktree.dir)?])?;
    worktree.check("initial", Yes, Git("0.1.0-0.2.wt"), &[])?;
    worktree.check("no changes", No, Git("0.1.0-0.2.wt"), &[])?;
    // Distinct messages keep these empty commits on the same parent from being
    // identical when they're made within the same second.
    worktree.git(&["commit", "--allow-empty", "-m", "commit in worktree"])?;
    worktree.check("commit", Yes, Git("0.1.0-0.3.wt"), &[])?;
    app.git(&["commit", "--allow-empty", "-m", "commit in main worktree"])?;
    app.git(&["tag", "-a", "v2.0.0", "-m", "v2.0.0"])?;
    worktree.check("tag in the main worktree", Any, Git("0.1.0-0.3.wt"), &[])?;
    worktree.git(&["merge", "--no-ff", "-m", "merge", "main"])?;
    worktree.check("merge the tag", Yes, Git("2.0.1-0.2.wt"), &[])?;

    // A shallow clone falls back, or fails in strict mode, until unshallowed
    // even without moving HEAD or fetching tags.
    let shallow = App::new("shallow", temp.join("shallow"), temp);
    let source = path_str(&app.dir)?;
    let clone_args = [
        "clone",
        "--no-local",
        "--depth=1",
        "--no-tags",
        "--branch=main",
    ];
    git(
        temp,
        &[&clone_args[..], &[source, path_str(&shallow.dir)?]].concat(),
    )?;
    shallow.check("initial", Yes, Fallback, &[])?;
    shallow.check("no changes", No, Fallback, &[])?;
    let strict = BUILD_SCRIPT.replace(".resolve()", ".strict(true)\n        .resolve()");
    if strict == BUILD_SCRIPT {
        return Err("failed to enable strict mode in the build script".into());
    }
    fs::write(shallow.dir.join("app/build.rs"), strict)?;
    shallow.check_fails("strict mode", "repository is a shallow clone")?;
    shallow.git(&["checkout", "--", "app/build.rs"])?;
    shallow.check("strict mode off", Yes, Fallback, &[])?;
    shallow.git(&["fetch", "--unshallow", "--no-tags"])?;
    shallow.check("unshallow", Yes, Git("0.1.0-0.3"), &[])?;

    Ok(())
}

fn path_str(path: &Path) -> Result<&str> {
    path.to_str()
        .ok_or_else(|| format!("non-UTF-8 path: {}", path.display()).into())
}

fn git(repo: &Path, args: &[&str]) -> Result<String> {
    let mut cmd = Command::new("git");
    for (key, _) in env::vars_os() {
        if key.to_str().is_some_and(|k| k.starts_with("GIT_")) {
            cmd.env_remove(key);
        }
    }
    let output = cmd
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
        .args(["-c", "commit.gpgSign=false", "-c", "tag.gpgSign=false"])
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "`git {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(String::from_utf8(output.stdout)?.trim_end().to_owned())
}
