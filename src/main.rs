// SPDX-FileCopyrightText: (C) 2026 Antti Kivi <antti@anttikivi.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

mod args;

use std::{env, io, process::ExitCode};

fn main() -> ExitCode {
    match args::parse_args(env::args_os().skip(1)) {
        Ok(args) => match run(args) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("it failed: {e}");
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
        eprintln!("parsed command-line arguments: {args:#?}")
    }

    if args.help.unwrap_or(false) {
        println!("{}", args::USAGE);
    }

    Ok(())
}
