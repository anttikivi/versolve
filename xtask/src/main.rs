use std::{
    env,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::{self, Command},
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

/// Builds a package that uses `versolve::build` from its build script in a
/// temporary repository, and checks the version it embeds.
fn e2e() -> Result {
    let root = root();
    let versolve = root
        .to_str()
        .filter(|p| !p.contains('\''))
        .ok_or("the repository path can't be written as a TOML literal string")?;
    let dir = TempDir::new()?;
    let repo = dir.0.as_path();

    fs::create_dir_all(repo.join("app/src"))?;
    fs::write(
        repo.join("Cargo.toml"),
        "[workspace]\nmembers = [\"app\"]\nresolver = \"2\"\n",
    )?;
    // The old `rust-version` makes Cargo reject the `cargo::` syntax.
    fs::write(
        repo.join("app/Cargo.toml"),
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
    fs::write(
        repo.join("app/build.rs"),
        r#"fn main() {
    versolve::build::Config::new()
        .tag_prefix("v")
        .default_branch("main")
        .branch_env("APP_BUILD_BRANCH")
        .emit("APP_VERSION")
        .unwrap();
}
"#,
    )?;
    fs::write(
        repo.join("app/src/main.rs"),
        "fn main() {\n    println!(\"{}\", env!(\"APP_VERSION\"));\n}\n",
    )?;
    fs::copy(root.join("Cargo.lock"), repo.join("Cargo.lock"))?;

    git(repo, &["init", "--initial-branch=main"])?;
    git(repo, &["add", "--all"])?;
    git(repo, &["commit", "-m", "commit"])?;
    git(repo, &["tag", "-a", "v1.2.3", "-m", "v1.2.3"])?;

    for distance in 1..=2 {
        git(repo, &["commit", "--allow-empty", "-m", "commit"])?;
        let head = git(repo, &["rev-parse", "HEAD"])?;
        let version = run_app(repo, &root)?;
        eprintln!("app version: {version}");
        let expected = format!("1.2.4-0.{distance}+g");
        let matches = version
            .strip_prefix(&expected)
            .is_some_and(|id| id.len() >= 7 && head.starts_with(id));
        if !matches {
            return Err(format!("expected {expected}<abbreviated {head}>, got {version}").into());
        }
    }

    // Verify that unshallowing without moving HEAD or fetching tags reruns the
    // build script and replaces the fallback version.
    let shallow_dir = TempDir::new()?;
    let shallow = shallow_dir.0.as_path();
    let repo_str = repo.to_str().ok_or("non-UTF-8 temporary path")?;
    let shallow_str = shallow.to_str().ok_or("non-UTF-8 temporary path")?;
    git(
        repo,
        &[
            "clone",
            "--no-local",
            "--depth=1",
            "--no-tags",
            repo_str,
            shallow_str,
        ],
    )?;

    let fallback = run_app(shallow, &root)?;
    eprintln!("shallow app version: {fallback}");
    if fallback != "0.0.0" {
        return Err(format!("expected fallback version 0.0.0, got {fallback}").into());
    }

    git(shallow, &["fetch", "--unshallow", "--no-tags"])?;
    let head = git(shallow, &["rev-parse", "HEAD"])?;
    let unshallowed = run_app(shallow, &root)?;
    eprintln!("unshallowed app version: {unshallowed}");
    let expected = "0.1.0-0.3+g";
    let matches = unshallowed
        .strip_prefix(expected)
        .is_some_and(|id| id.len() >= 7 && head.starts_with(id));
    if !matches {
        return Err(format!("expected {expected}<abbreviated {head}>, got {unshallowed}").into());
    }

    Ok(())
}

fn run_app(repo: &Path, root: &Path) -> Result<String> {
    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    eprintln!("+ cargo run --package app");
    let output = Command::new(cargo)
        .args(["run", "--package", "app"])
        .env("CARGO_TARGET_DIR", root.join("target/e2e"))
        .env_remove("APP_BUILD_BRANCH")
        .current_dir(repo)
        .stderr(process::Stdio::inherit())
        .output()?;
    if !output.status.success() {
        return Err(format!("`cargo run` failed with {}", output.status).into());
    }
    Ok(String::from_utf8(output.stdout)?.trim_end().to_owned())
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
