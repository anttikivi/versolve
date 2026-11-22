mod args;

use std::{
    env,
    ffi::OsString,
    fmt,
    io::{self, Write},
    process::ExitCode,
};

use log::{LevelFilter, Log, Metadata, Record};
use versolve::{
    ErrorKind, Resolver,
    semver::{BuildMetadata, Version},
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
    init_logger(args.verbose.unwrap_or(false));

    log::debug!("running versolve {version}");
    log::debug!("command-line arguments parsed: {args:?}");

    if args.help.unwrap_or(false) {
        print_line(args::USAGE)?;
        return Ok(());
    }

    if args.version.unwrap_or(false) {
        let commit_hash = option_env!("VERSOLVE_VERSION_COMMIT_HASH");
        let commit_short_hash = option_env!("VERSOLVE_VERSION_COMMIT_SHORT_HASH");
        let commit_date = option_env!("VERSOLVE_VERSION_COMMIT_DATE");
        let release = Version {
            build: BuildMetadata::EMPTY,
            ..version.clone()
        };

        match commit_short_hash.zip(commit_date) {
            Some((hash, date)) => print_line(format_args!("versolve {release} ({hash} {date})"))?,
            None => print_line(format_args!("versolve {release}"))?,
        }

        if args.verbose.unwrap_or(false) {
            print_line("binary: versolve")?;
            print_line(format_args!(
                "commit-hash: {}",
                commit_hash.unwrap_or("unknown")
            ))?;
            print_line(format_args!(
                "commit-date: {}",
                commit_date.unwrap_or("unknown")
            ))?;
            print_line(format_args!("host: {}", env!("VERSOLVE_HOST")))?;
            print_line(format_args!("release: {version}"))?;
        }

        return Ok(());
    }

    let resolver = resolver(args)?;

    log::debug!("options parsed: {resolver:?}");

    match resolver.resolve() {
        Ok(v) => print_line(v)?,
        Err(e) if e.kind() == ErrorKind::DetachedHead => {
            return Err(format!(
                "failed to resolve version: {e}; pass \"--branch\" to name the branch"
            )
            .into());
        }
        Err(e) => {
            return Err(format!("failed to resolve version: {e}").into());
        }
    }

    Ok(())
}

fn resolver(args: args::Args) -> io::Result<Resolver> {
    let repository = match args.repository {
        Some(path) => path,
        None => env::current_dir()?,
    };
    let mut resolver = Resolver::new(repository);
    if let Some(prefix) = args.tag_prefix {
        resolver = resolver.tag_prefix(prefix);
    }
    if let Some(name) = args.default_branch {
        resolver = resolver.default_branch(name);
    }
    if let Some(name) = args.branch {
        resolver = resolver.branch(name);
    }
    Ok(resolver)
}

/// Prints the debug messages from Versolve to the standard error.
struct Logger;

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        // Leave out the messages from the dependencies, such as git2.
        metadata.target().split("::").next() == Some("versolve")
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            eprint_line(record.args());
        }
    }

    fn flush(&self) {}
}

fn init_logger(verbose: bool) {
    static LOGGER: Logger = Logger;
    // This fails only if a logger is already set, and this is the only place
    // that sets one.
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(if verbose {
        LevelFilter::Debug
    } else {
        LevelFilter::Off
    });
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
