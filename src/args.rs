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
    pub version: Option<bool>,
}

pub const USAGE: &str = "Resolve unique version number for Git revision

Usage: versolve [options]

Options:

    -b, --branch <name>         Manually override the current branch name
    -B, --default-branch <name> Omit the branch name in version numbers for this branch
    -h, --help                  Print this help message
    -C, --repository <path>     Path to the Git repository to inspect
    -T, --tag-prefix <prefix>   Consider only tags with this prefix
    -v, --verbose               Print debug information
    --version                   Print Versolve's version";

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
                "--version" => set_args_value(&mut args.version, name, flag(name, &inline_val)?)?,
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
                        "--branch" => {
                            let val = utf8(name, val)?;
                            if val.is_empty() {
                                return Err(format!("value for \"{name}\" cannot be empty"));
                            }
                            set_args_value(&mut args.branch, name, val)?
                        }
                        "--default-branch" => {
                            set_args_value(&mut args.default_branch, name, utf8(name, val)?)?;
                        }
                        "--repository" => {
                            set_args_value(&mut args.repository, name, PathBuf::from(val))?;
                        }
                        "--tag-prefix" => {
                            set_args_value(&mut args.tag_prefix, name, utf8(name, val)?)?;
                        }
                        _ => unreachable!(),
                    }
                }
                _ => return Err(format!("unknown command-line argument: {name:?}")),
            }
        } else if let Some(cluster) = s.strip_prefix('-').filter(|c| !c.is_empty()) {
            for (i, c) in cluster.char_indices() {
                match c {
                    'h' => set_args_value(&mut args.help, "-h", true)?,
                    'v' => set_args_value(&mut args.verbose, "-v", true)?,
                    'b' | 'B' | 'C' | 'T' => {
                        let rest = &cluster[i + c.len_utf8()..];
                        let val = if rest.is_empty() {
                            it.next()
                                .ok_or_else(|| format!("option \"-{c}\" requires a value"))?
                        } else {
                            OsString::from(rest)
                        };
                        match c {
                            'b' => {
                                let val = utf8("-b", val)?;
                                if val.is_empty() {
                                    return Err("value for \"-b\" cannot be empty".into());
                                }
                                set_args_value(&mut args.branch, "-b", val)?;
                            }
                            'B' => {
                                set_args_value(&mut args.default_branch, "-B", utf8("-B", val)?)?;
                            }
                            'C' => set_args_value(&mut args.repository, "-C", PathBuf::from(val))?,
                            'T' => set_args_value(&mut args.tag_prefix, "-T", utf8("-T", val)?)?,
                            _ => unreachable!(),
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
        return Err(format!("option \"{name}\" is set twice"));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(argv: &[&str]) -> Result<Args, String> {
        parse_args(argv.iter().map(OsString::from))
    }

    #[test]
    fn no_args_returns_defaults() {
        assert_eq!(parse(&[]), Ok(Args::default()));
    }

    #[test]
    fn long_options_separate_value() {
        let parsed = parse(&[
            "--branch",
            "feature",
            "--default-branch",
            "main",
            "--repository",
            "/tmp/repo",
            "--tag-prefix",
            "v",
        ])
        .unwrap();
        assert_eq!(
            parsed,
            Args {
                branch: Some("feature".into()),
                default_branch: Some("main".into()),
                repository: Some(PathBuf::from("/tmp/repo")),
                tag_prefix: Some("v".into()),
                ..Args::default()
            }
        );
    }

    #[test]
    fn long_options_inline_values() {
        let parsed = parse(&[
            "--branch=feature",
            "--default-branch=main",
            "--repository=/tmp/repo",
            "--tag-prefix=v",
        ])
        .unwrap();
        assert_eq!(
            parsed,
            Args {
                branch: Some("feature".into()),
                default_branch: Some("main".into()),
                repository: Some(PathBuf::from("/tmp/repo")),
                tag_prefix: Some("v".into()),
                ..Args::default()
            }
        );
    }

    #[test]
    fn long_options_with_flags() {
        let parsed = parse(&[
            "--branch=feature",
            "--default-branch=main",
            "--repository=/tmp/repo",
            "--help",
            "--verbose=false",
            "--tag-prefix=v",
        ])
        .unwrap();
        assert_eq!(
            parsed,
            Args {
                branch: Some("feature".into()),
                default_branch: Some("main".into()),
                help: Some(true),
                repository: Some(PathBuf::from("/tmp/repo")),
                tag_prefix: Some("v".into()),
                verbose: Some(false),
                ..Args::default()
            }
        );
    }

    #[test]
    fn inline_value_may_contain_equals_sign() {
        let args = parse(&["--branch=a=b"]).unwrap();
        assert_eq!(args.branch.as_deref(), Some("a=b"));
    }

    #[test]
    fn short_options_separate_and_attached() {
        assert_eq!(
            parse(&["-b", "main"]).unwrap().branch.as_deref(),
            Some("main")
        );
        assert_eq!(parse(&["-bmain"]).unwrap().branch.as_deref(), Some("main"));
        assert_eq!(
            parse(&["-C/tmp/repo"]).unwrap().repository,
            Some(PathBuf::from("/tmp/repo"))
        );
    }

    #[test]
    fn short_flags_options_with_trailing_value() {
        let args = parse(&["-vbmain"]).unwrap();
        assert_eq!(args.verbose, Some(true));
        assert_eq!(args.branch.as_deref(), Some("main"));

        let args = parse(&["-vb", "main"]).unwrap();
        assert_eq!(args.branch.as_deref(), Some("main"));
    }

    #[test]
    fn boolean_flag_values() {
        for (input, expected) in [
            ("--verbose", true),
            ("--verbose=true", true),
            ("--verbose=1", true),
            ("--verbose=false", false),
            ("--verbose=0", false),
            ("--no-verbose", false),
        ] {
            assert_eq!(parse(&[input]).unwrap().verbose, Some(expected), "{input}");
        }
        assert_eq!(parse(&["--help"]).unwrap().help, Some(true));
        assert_eq!(parse(&["-h"]).unwrap().help, Some(true));
        assert!(parse(&["-h", "--help"]).is_err());
    }

    #[test]
    fn boolean_flag_rejects_other_values() {
        assert!(parse(&["--verbose=yes"]).is_err());
        assert!(parse(&["--verbose="]).is_err());
        assert!(parse(&["--no-verbose=true"]).is_err());
    }

    #[test]
    fn duplicate_options_are_rejected() {
        for argv in [
            &["--branch", "a", "--branch", "b"][..],
            &["--branch", "a", "-b", "b"],
            &["-vv"],
            &["-v", "--verbose"],
            &["--verbose", "--no-verbose"],
            &["-C", "a", "--repository", "b"],
        ] {
            assert!(parse(argv).is_err(), "{argv:?} should fail");
        }
    }

    #[test]
    fn missing_values_are_rejected() {
        assert_eq!(
            parse(&["--branch"]),
            Err("option \"--branch\" requires a value".into())
        );
        assert_eq!(parse(&["-b"]), Err("option \"-b\" requires a value".into()));
        assert!(parse(&["-vC"]).is_err());
    }

    #[test]
    fn unknown_arguments_are_rejected() {
        assert!(parse(&["--nope"]).is_err());
        assert!(parse(&["-x"]).is_err());
        assert!(parse(&["-vx"]).is_err());
        assert!(parse(&["positional"]).is_err());
        assert!(parse(&["-"]).is_err());
    }

    #[test]
    fn double_dash() {
        assert_eq!(parse(&["-v", "--"]).unwrap().verbose, Some(true));
        assert!(parse(&["--", "extra"]).is_err());
    }

    #[test]
    fn option_consumes_next_arg_even_if_it_looks_like_a_flag() {
        assert_eq!(
            parse(&["--branch", "--verbose"]).unwrap().branch.as_deref(),
            Some("--verbose")
        );
    }

    #[test]
    fn empty_value_is_accepted() {
        assert_eq!(
            parse(&["--tag-prefix="]).unwrap().tag_prefix.as_deref(),
            Some("")
        );
    }

    #[test]
    fn empty_branch_is_rejected() {
        for argv in [&["--branch="][..], &["--branch", ""], &["-b", ""]] {
            assert!(parse(argv).is_err(), "{argv:?} should fail");
        }
    }

    #[cfg(unix)]
    mod non_utf8 {
        use super::*;
        use std::os::unix::ffi::OsStringExt;

        fn bad() -> OsString {
            OsString::from_vec(vec![0x66, 0x6f, 0xff]) // "fo\xFF"
        }

        #[test]
        fn non_utf8_branch_is_rejected() {
            let err = parse_args([OsString::from("--branch"), bad()]).unwrap_err();
            assert!(err.contains("not valid UTF-8"));
        }

        #[test]
        fn non_utf8_repository_path_is_accepted() {
            let args = parse_args([OsString::from("-C"), bad()]).unwrap();
            assert_eq!(args.repository, Some(PathBuf::from(bad())));
        }

        #[test]
        fn non_utf8_argument_itself_is_rejected() {
            assert!(parse_args([bad()]).is_err());
        }
    }

    #[cfg(windows)]
    mod non_utf8 {
        use super::*;
        use std::os::windows::ffi::OsStringExt;

        fn bad() -> OsString {
            OsString::from_wide(&[0x66, 0x6f, 0xD800])
        }

        #[test]
        fn non_utf8_branch_is_rejected() {
            let err = parse_args([OsString::from("--branch"), bad()]).unwrap_err();
            assert!(err.contains("not valid UTF-8"));
        }

        #[test]
        fn non_utf8_repository_path_is_accepted() {
            let args = parse_args([OsString::from("-C"), bad()]).unwrap();
            assert_eq!(args.repository, Some(PathBuf::from(bad())));
        }

        #[test]
        fn non_utf8_argument_itself_is_rejected() {
            assert!(parse_args([bad()]).is_err());
        }
    }
}
