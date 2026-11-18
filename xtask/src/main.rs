// SPDX-FileCopyrightText: (C) 2026 Antti Kivi <antti@anttikivi.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    env,
    path::{Path, PathBuf},
    process::{self, Command},
};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn run() -> Result {
    let mut it = env::args().skip(1);
    match it.next().as_deref() {
        Some("reuse") => sh(
            "uvx",
            &["--from=reuse[charset-normalizer]", "reuse", "lint"],
        ),
        Some("reuse-annotate") => {
            reuse_annotate(&[
                "--recursive",
                ".builds",
                ".github",
                "src",
                "xtask/src",
                ".cargo/config.toml",
                ".gitignore",
                "Cargo.toml",
                "xtask/Cargo.toml",
            ])?;
            reuse_annotate(&["--force-dot-license", "Cargo.lock"])
        }
        _ => {
            eprintln!("usage: cargo xtask <reuse|reuse-annotate>");
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
    let status = Command::new(&prog)
        .args(args)
        .current_dir(root())
        .status()?;
    if !status.success() {
        return Err(format!("`{prog}` failed with {status}").into());
    }
    Ok(())
}

fn reuse_annotate(args: &[&str]) -> Result {
    let mut all = vec![
        "--from=reuse[charset-normalizer]",
        "reuse",
        "annotate",
        "--copyright=Antti Kivi <antti@anttikivi.com>",
        "--license=GPL-3.0-or-later",
        "--copyright-prefix=spdx-c",
    ];
    all.extend_from_slice(args);
    sh("uvx", &all)
}
