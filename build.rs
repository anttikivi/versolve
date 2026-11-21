// SPDX-FileCopyrightText: (C) 2026 Antti Kivi <antti@anttikivi.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use std::{env, error::Error, path::Path};

use git2::Repository;

macro_rules! verbose {
    ($enabled:expr, $($args:tt)*) => {
        if $enabled {
            eprintln!($($args)*);
        }
    };
}

#[allow(dead_code)]
#[path = "src/args.rs"]
mod args;
#[allow(dead_code)]
#[path = "src/resolve.rs"]
mod resolve;

fn main() {
    let root = env::var_os("CARGO_MANIFEST_DIR").unwrap();
    let root = Path::new(&root);

    // Don't resolve version from a repository that happens to contain Versolve
    // from e.g. vendor directory.
    let version = match Repository::open(root) {
        Ok(repo) => {
            // The embedded version depends only on the committed state, so
            // changes to the working tree don't need to be tracked.
            println!(
                "cargo::rerun-if-changed={}",
                repo.path().join("HEAD").display()
            );
            println!(
                "cargo::rerun-if-changed={}",
                repo.commondir().join("refs").display()
            );
            let packed_refs = repo.commondir().join("packed-refs");
            if packed_refs.exists() {
                println!("cargo::rerun-if-changed={}", packed_refs.display());
            }
            for path in ["build.rs", "Cargo.toml"] {
                println!("cargo::rerun-if-changed={path}");
            }
            println!("cargo::rerun-if-env-changed=VERSOLVE_BUILD_BRANCH");
            resolve_version(root).unwrap_or_else(|e| {
                println!("cargo::warning=failed to resolve version: {e}");
                env::var("CARGO_PKG_VERSION").unwrap()
            })
        }
        Err(_) => env::var("CARGO_PKG_VERSION").unwrap(),
    };

    println!("cargo::rustc-env=VERSOLVE_VERSION={version}");
}

fn resolve_version(root: &Path) -> Result<String, Box<dyn Error>> {
    let mut opts = resolve::Options::try_from(args::Args {
        branch: env::var("VERSOLVE_BUILD_BRANCH")
            .ok()
            .filter(|b| !b.is_empty()),
        default_branch: Some("master".into()),
        repository: Some(root.to_path_buf()),
        tag_prefix: Some("v".into()),
        ..Default::default()
    })?;
    opts.check_dirty = false;
    Ok(resolve::from_git(&opts)?.to_string())
}
