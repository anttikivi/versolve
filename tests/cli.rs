// SPDX-FileCopyrightText: (C) 2026 Antti Kivi <antti@anttikivi.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use std::{io, process::Command};

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
