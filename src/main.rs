// SPDX-FileCopyrightText: (C) 2026 Antti Kivi <antti@anttikivi.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use std::{env, ffi::OsString, io, path::PathBuf, process::ExitCode};

struct Args {
    branch: Option<String>,
    default_branch: Option<String>,
    repository: Option<PathBuf>,
    tag_prefix: Option<String>,
}

fn utf8(name: &str, val: OsString) -> Result<String, String> {
    val.into_string()
        .map_err(|v| format!("value for \"{name}\" is not valid UTF-8: {v:?}"))
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        branch: None,
        default_branch: None,
        repository: None,
        tag_prefix: None,
    };
    let mut it = env::args_os().skip(1);

    while let Some(arg) = it.next() {
        let Some(s) = arg.to_str() else {
            return Err(format!("unknown command-line argument: {arg:?}"));
        };

        if s == "--" {
            return match it.next() {
                Some(extra) => Err(format!("unexpected argument: {extra:?}")),
                None => Ok(args),
            };
        }

        if s.starts_with("--") {
            let (name, mut inline_val) = match s.split_once('=') {
                Some((n, v)) => (n, Some(OsString::from(v))),
                None => (s, None),
            };

            match name {
                "--branch" | "--default-branch" | "--repository" | "--tag-prefix" => {
                    let val = inline_val
                        .take()
                        .or_else(|| it.next())
                        .ok_or_else(|| format!("option \"{name}\" requires a value"))?;
                    match name {
                        "--branch" => args.branch = Some(utf8(name, val)?),
                        "--default-branch" => args.default_branch = Some(utf8(name, val)?),
                        "--repository" => args.repository = Some(PathBuf::from(val)),
                        "--tag-prefix" => args.tag_prefix = Some(utf8(name, val)?),
                        _ => panic!("unexpected command-line option name: {name:?}"),
                    }
                }
                _ if name.starts_with("--") => {
                    return Err(format!("unknown command-line argument: {name:?}"));
                }
                _ => return Err(format!("unknown command-line argument: {name:?}")),
            }
        } else if let Some(cluster) = s.strip_prefix('-').filter(|c| !c.is_empty()) {
            for (i, c) in cluster.char_indices() {
                match c {
                    'b' | 'C' => {
                        let rest = &cluster[i + c.len_utf8()..];
                        let val = if rest.is_empty() {
                            it.next()
                                .ok_or_else(|| format!("option \"-{c}\" requires a value"))?
                        } else {
                            OsString::from(rest)
                        };
                        match c {
                            'b' => args.branch = Some(utf8("-b", val)?),
                            'C' => args.repository = Some(PathBuf::from(val)),
                            _ => panic!("unexpected command-line option: \"-{c}\""),
                        }
                        break; // consumes the whole argument
                    }
                    _ => return Err(format!("unknown option: -{c}")),
                }
            }
        } else {
            return Err(format!("unknown command-line argument: \"{s}\""));
        }
    }

    Ok(args)
}

fn run(_args: Args) -> io::Result<()> {
    Ok(())
}

fn main() -> ExitCode {
    println!("Hello, world!");
    let parsed = parse_args();
    match parsed {
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
