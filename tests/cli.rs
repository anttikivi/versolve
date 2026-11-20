// SPDX-FileCopyrightText: (C) 2026 Antti Kivi <antti@anttikivi.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    io,
    process::{Command, Output},
};

use tempfile::TempDir;

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
fn help_is_shown_despite_invalid_arguments() {
    for args in [&["--nope", "--help"][..], &["-h", "--nope"], &["-h", "-h"]] {
        let output = versolve(args);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_eq!(output.status.code(), Some(0), "{args:?}");
        assert!(stdout.contains("Usage: versolve"), "{args:?}: {stdout}");
    }
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
