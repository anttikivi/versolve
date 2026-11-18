// SPDX-FileCopyrightText: (C) 2026 Antti Kivi <antti@anttikivi.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use std::{ffi::OsString, path::PathBuf};

#[derive(Debug, Default, PartialEq)]
pub struct Args {
    pub branch: Option<String>,
    pub default_branch: Option<String>,
    pub help: Option<bool>,
    pub repository: Option<PathBuf>,
    pub tag_prefix: Option<String>,
    pub verbose: Option<bool>,
}

pub fn parse_args<I>(argv: I) -> Result<Args, String>
where
    I: IntoIterator<Item = OsString>,
{
    let mut args = Args::default();
    let mut it = argv.into_iter();

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
                "--help" => set_args_value(&mut args.help, name, flag(name, &inline_val)?)?,
                "--verbose" => set_args_value(&mut args.verbose, name, flag(name, &inline_val)?)?,
                "--no-verbose" => {
                    no_value_flag(name, &inline_val)?;
                    set_args_value(&mut args.verbose, name, false)?;
                }
                "--branch" | "--default-branch" | "--repository" | "--tag-prefix" => {
                    let val = inline_val
                        .take()
                        .or_else(|| it.next())
                        .ok_or_else(|| format!("option \"{name}\" requires a value"))?;
                    match name {
                        "--branch" => set_args_value(&mut args.branch, name, utf8(name, val)?)?,
                        "--default-branch" => {
                            set_args_value(&mut args.default_branch, name, utf8(name, val)?)?;
                        }
                        "--repository" => {
                            set_args_value(&mut args.repository, name, PathBuf::from(val))?;
                        }
                        "--tag-prefix" => {
                            set_args_value(&mut args.tag_prefix, name, utf8(name, val)?)?;
                        }
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
                    'v' => set_args_value(&mut args.verbose, "-v", true)?,
                    'b' | 'C' => {
                        let rest = &cluster[i + c.len_utf8()..];
                        let val = if rest.is_empty() {
                            it.next()
                                .ok_or_else(|| format!("option \"-{c}\" requires a value"))?
                        } else {
                            OsString::from(rest)
                        };
                        match c {
                            'b' => set_args_value(&mut args.branch, "-b", utf8("-b", val)?)?,
                            'C' => set_args_value(&mut args.repository, "-C", PathBuf::from(val))?,
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

fn set_args_value<T>(field: &mut Option<T>, name: &str, val: T) -> Result<(), String> {
    if field.is_some() {
        return Err(format!("option \"{name}\" is se twice"));
    }

    *field = Some(val);
    Ok(())
}

fn utf8(name: &str, val: OsString) -> Result<String, String> {
    val.into_string()
        .map_err(|v| format!("value for \"{name}\" is not valid UTF-8: {v:?}"))
}

fn flag(name: &str, inline_val: &Option<OsString>) -> Result<bool, String> {
    match inline_val {
        None => Ok(true),
        Some(v) => match v.to_str() {
            Some("true" | "1") => Ok(true),
            Some("false" | "0") => Ok(false),
            _ => Err(format!(
                "option \"{name}\" expects true or false, got {v:?}"
            )),
        },
    }
}

fn no_value_flag(name: &str, inline_val: &Option<OsString>) -> Result<(), String> {
    match inline_val {
        None => Ok(()),
        Some(_) => Err(format!("option \"{name}\" does not take a value")),
    }
}
