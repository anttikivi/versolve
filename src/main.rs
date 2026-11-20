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

use std::{
    env,
    ffi::OsString,
    fmt,
    io::{self, Write},
    process::ExitCode,
};

fn main() -> ExitCode {
    let argv: Vec<OsString> = env::args_os().skip(1).collect();
    let args = match args::parse_args(argv.clone()) {
        Ok(args) => args,
        // Show the help message when it is asked for even if the other
        // arguments are invalid so that the --help flag effectively takes
        // precedence.
        Err(_) if args::asks_for_help(&argv) => args::Args {
            help: Some(true),
            ..Default::default()
        },
        Err(s) => {
            eprintln!("failed to parse command-line arguments: {s}");
            eprintln!("run \"versolve --help\" to see the usage");
            return ExitCode::from(2);
        }
    };

    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
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
        print_line(args::USAGE)?;
        return Ok(());
    }

    if args.version.unwrap_or(false) {
        print_line(format_args!("versolve {}", env!("VERSOLVE_VERSION")))?;
        return Ok(());
    }

    let opts = resolve::Options::try_from(args)?;

    verbose!(opts.verbose, "options parsed: {opts:?}");

    match resolve::from_git(&opts) {
        Ok(v) => print_line(v)?,
        Err(s) => {
            return Err(format!("failed to resolve version: {s}").into());
        }
    }

    Ok(())
}

fn print_line(line: impl fmt::Display) -> Result<(), String> {
    writeln!(io::stdout().lock(), "{line}")
        .map_err(|e| format!("failed to write to standard output: {e}"))
}
