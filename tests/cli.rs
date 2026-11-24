use std::{
    env, io,
    path::Path,
    process::{Command, Output},
};

use tempfile::TempDir;
use versolve::semver::Version;

fn versolve(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_versolve"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn closed_stdout_is_an_error() {
    for arg in ["--help", "--version"] {
        let (reader, writer) = io::pipe().unwrap();
        drop(reader);
        let output = Command::new(env!("CARGO_BIN_EXE_versolve"))
            .arg(arg)
            .stdout(writer)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(1), "{arg}: {stderr}");
        assert!(
            stderr.starts_with("failed to write to standard output: "),
            "{arg}: {stderr}"
        );
    }
}

#[test]
fn closed_stderr_keeps_exit_status() {
    let dir = TempDir::new().unwrap();
    let missing = dir.path().join("missing");
    let missing = missing.to_str().unwrap();

    for (args, code, stdout) in [
        (&["--bogus"][..], 2, ""),
        (&["--verbose", "--version"], 0, "versolve "),
        (&["-C", missing], 1, ""),
    ] {
        let (reader, writer) = io::pipe().unwrap();
        drop(reader);
        let output = Command::new(env!("CARGO_BIN_EXE_versolve"))
            .args(args)
            .stderr(writer)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(code), "{args:?}");
        assert!(output.stdout.starts_with(stdout.as_bytes()), "{args:?}");
    }
}

#[test]
fn help_is_shown_despite_invalid_arguments() {
    for args in [&["--nope", "--help"][..], &["-h", "--nope"], &["-h", "-h"]] {
        let output = versolve(args);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_eq!(output.status.code(), Some(0), "{args:?}");
        assert!(stdout.contains("Usage: versolve"), "{args:?}: {stdout}");
    }
}

fn versolve_stdout(args: &[&str]) -> String {
    let output = versolve(args);
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn version_shows_the_embedded_version() {
    let stdout = versolve_stdout(&["--version"]);
    let first_line = stdout
        .strip_suffix('\n')
        .filter(|s| !s.contains('\n'))
        .unwrap_or_else(|| panic!("expected one line: {stdout:?}"));
    let version = first_line
        .strip_prefix("versolve ")
        .unwrap_or_else(|| panic!("{first_line:?}"));
    Version::parse(version).unwrap_or_else(|e| panic!("{first_line:?}: {e}"));

    let stdout = versolve_stdout(&["--verbose", "--version"]);
    let mut lines = stdout.lines();
    assert_eq!(lines.next(), Some(first_line));
    let fields: Vec<(&str, &str)> = lines
        .map(|l| l.split_once(": ").unwrap_or_else(|| panic!("{l:?}")))
        .collect();
    let keys: Vec<&str> = fields.iter().map(|(k, _)| *k).collect();
    assert_eq!(keys, ["binary", "host", "release"]);
    let field = |key| fields.iter().find(|(k, _)| *k == key).unwrap().1;

    assert_eq!(field("binary"), "versolve");
    assert!(!field("host").is_empty(), "{stdout}");
    assert_eq!(field("release"), version);
}

#[test]
fn invalid_arguments_point_to_help() {
    let output = versolve(&["--nope"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(2), "{stderr}");
    assert!(
        stderr.ends_with("run \"versolve --help\" to see the usage\n"),
        "{stderr}"
    );
}

#[test]
fn libgit2_errors_omit_class_and_code() {
    let dir = TempDir::new().unwrap();
    let missing = dir.path().join("missing");
    let missing = missing.to_str().unwrap();

    let output = versolve(&["-C", missing]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.starts_with("failed to resolve version: "),
        "{stderr}"
    );
    assert!(!stderr.contains("class="), "{stderr}");

    let output = versolve(&["-v", "-C", missing]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("libgit2 error: "), "{stderr}");
    assert!(stderr.contains("class="), "{stderr}");
}

/// Runs `git` in `dir` without reading the user's configuration and returns its
/// standard output.
fn git(dir: &Path, args: &[&str]) -> String {
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
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn detached_head_suggests_branch() {
    let dir = TempDir::new().unwrap();
    git(dir.path(), &["init", "--initial-branch=main"]);
    git(dir.path(), &["commit", "--allow-empty", "-m", "commit"]);
    git(dir.path(), &["commit", "--allow-empty", "-m", "commit"]);
    git(dir.path(), &["checkout", "--detach", "HEAD~1"]);
    let head = git(dir.path(), &["rev-parse", "HEAD"]);

    // Keep libgit2 from reading the user's global configuration.
    let output = Command::new(env!("CARGO_BIN_EXE_versolve"))
        .arg("-C")
        .arg(dir.path())
        .env("HOME", dir.path())
        .env("XDG_CONFIG_HOME", dir.path())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert_eq!(
        stderr,
        format!(
            "failed to resolve version: HEAD is detached at {}; pass \"--branch\" to name the \
            branch\n",
            head.trim_end()
        )
    );
}
