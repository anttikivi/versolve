mod args;

use std::{
    env,
    ffi::OsString,
    fmt,
    io::{self, Write},
    process::ExitCode,
};

use log::{LevelFilter, Log, Metadata, Record};
use versolve::{ErrorKind, Resolver, semver::Version};

struct Logger;

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        // Leave out messages from dependencies such as git2.
        metadata.target().split("::").next() == Some("versolve")
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            eprint_line(record.args());
        }
    }

    fn flush(&self) {}
}

fn main() -> ExitCode {
    let argv: Vec<OsString> = env::args_os().skip(1).collect();
    let args = match args::parse_args(argv.clone()) {
        Ok(args) => args,
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
        panic!(
            "failed to parse the embedded version; Versolve must be built with a valid version: {e}"
        )
    });

    static LOGGER: Logger = Logger;
    log::set_logger(&LOGGER)
        .unwrap_or_else(|e| panic!("Versolve logger must be the first set logger: {e}"));
    log::set_max_level(if args.verbose.unwrap_or(false) {
        LevelFilter::Debug
    } else {
        LevelFilter::Off
    });

    if args.help.unwrap_or(false) {
        print_line(args::HELP_MESSAGE)?;
        return Ok(());
    }

    if args.version.unwrap_or(false) {
        print_line(format_args!("versolve {version}"))?;

        if args.verbose.unwrap_or(false) {
            print_line("binary: versolve")?;
            print_line(format_args!("host: {}", env!("VERSOLVE_HOST")))?;
            print_line(format_args!("release: {version}"))?;
        }

        return Ok(());
    }

    log::debug!("running versolve {version}");
    log::debug!("command-line arguments parsed: {args:?}");

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

    let resolver = resolver;

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

fn print_line(line: impl fmt::Display) -> Result<(), String> {
    writeln!(io::stdout().lock(), "{line}")
        .map_err(|e| format!("failed to write to standard output: {e}"))
}

fn eprint_line(line: impl fmt::Display) {
    // Ignore stderr write errors so broken pipes do not panic or alter exit
    // status.
    let _ = writeln!(io::stderr().lock(), "{line}");
}
