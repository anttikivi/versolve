// SPDX-FileCopyrightText: (C) 2026 Antti Kivi <antti@anttikivi.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

mod args;
mod resolve;

use std::{env, io, process::ExitCode};

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
            ExitCode::FAILURE
        }
    }
}

fn run(args: args::Args) -> io::Result<()> {
    if args.verbose.unwrap_or(false) {
        // TODO: Use the actual version.
        eprintln!("running versolve 0.1.0-dev");
        eprintln!("command-line arguments parsed: {args:?}");
    }

    if args.help.unwrap_or(false) {
        println!("{}", args::USAGE);
        return Ok(());
    }

    if args.version.unwrap_or(false) {
        // TODO: Print the actual version.
        println!("versolve 0.1.0-dev");
        return Ok(());
    }

    let opts = resolve::Options::try_from(args)?;

    if opts.verbose {
        eprintln!("options parsed: {opts:?}");
    }

    match resolve::from_git(&opts) {
        Ok(_) => {}
        Err(s) => eprintln!("failed to resolve version: {s}"),
    }

    Ok(())
}
