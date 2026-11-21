macro_rules! verbose {
    ($enabled:expr, $($args:tt)*) => {
        if $enabled {
            crate::eprint_line(format_args!($($args)*));
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

use semver::{BuildMetadata, Version};

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
            eprint_line(format_args!("failed to parse command-line arguments: {s}"));
            eprint_line("run \"versolve --help\" to see the usage");
            return ExitCode::from(2);
        }
    };

    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprint_line(e);
            ExitCode::FAILURE
        }
    }
}

fn run(args: args::Args) -> Result<(), Box<dyn std::error::Error>> {
    let version = Version::parse(env!("VERSOLVE_VERSION")).unwrap_or_else(|e| {
        panic!("failed to parse the embedded Versolve version, Versolve must be built with a valid version: {e}")
    });
    let stripped_version = Version {
        build: BuildMetadata::EMPTY,
        ..version.clone()
    };

    verbose!(args.verbose.unwrap_or(false), "running versolve {version}");
    verbose!(
        args.verbose.unwrap_or(false),
        "command-line arguments parsed: {args:?}"
    );

    if args.help.unwrap_or(false) {
        print_line(args::USAGE)?;
        return Ok(());
    }

    if args.version.unwrap_or(false) {
        if args.verbose.unwrap_or(false) {
            print_line(format_args!("versolve {version}"))?;
        } else {
            print_line(format_args!("versolve {stripped_version}"))?;
        }
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

/// Writes a line to the standard error. Ignores write errors instead of
/// panicking as the diagnostics are best effort and failing to write them
/// should not change the exit status.
fn eprint_line(line: impl fmt::Display) {
    let _ = writeln!(io::stderr().lock(), "{line}");
}
