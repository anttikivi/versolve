// SPDX-FileCopyrightText: (C) 2026 Antti Kivi <antti@anttikivi.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

macro_rules! verbose {
    ($enabled:expr, $($args:tt)*) => {
        if $enabled {
            eprintln!($($args)*);
        }
    };
}

mod args;
mod resolve;

use std::{env, process::ExitCode};

fn main() -> ExitCode {
    match args::parse_args(env::args_os().skip(1)) {
        Ok(args) => match run(args) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{e}");
                ExitCode::FAILURE
            }
        },
        Err(s) => {
            eprintln!("failed to parse command-line arguments: {s}");
            ExitCode::from(2)
        }
    }
}

fn run(args: args::Args) -> Result<(), Box<dyn std::error::Error>> {
    verbose!(
        args.verbose.unwrap_or(false),
        "running versolve {}",
        env!("VERSOLVE_VERSION")
    );
    verbose!(
        args.verbose.unwrap_or(false),
        "command-line arguments parsed: {args:?}"
    );

    if args.help.unwrap_or(false) {
        println!("{}", args::USAGE);
        return Ok(());
    }

    if args.version.unwrap_or(false) {
        println!("versolve {}", env!("VERSOLVE_VERSION"));
        return Ok(());
    }

    let opts = resolve::Options::try_from(args)?;

    verbose!(opts.verbose, "options parsed: {opts:?}");

    match resolve::from_git(&opts) {
        Ok(v) => println!("{v}"),
        Err(s) => {
            return Err(format!("failed to resolve version: {s}").into());
        }
    }

    Ok(())
}
