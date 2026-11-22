use std::{
    ffi::{OsStr, OsString},
    path::PathBuf,
};

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

pub const USAGE: &str = "Resolve a unique version number for a Git revision

Usage: versolve [options]

Options:

    -b, --branch <name>         Manually override the current branch name
    -B, --default-branch <name> Omit the branch name in version numbers for this branch
    -h, --help                  Print this help message
    -C, --repository <path>     Path to the Git repository to inspect
    -T, --tag-prefix <prefix>   Consider only tags with this prefix
    -v, --verbose               Print debug information
    --no-verbose                Do not print debug information
    -V, --version               Print Versolve's version";

/// Reports whether `argv` asks for the help message with "-h" or "--help"
/// before "--". Unlike `parse_args`, this works even if the other arguments are
/// invalid.
pub fn asks_for_help(argv: &[OsString]) -> bool {
    argv.iter()
        .take_while(|a| a.as_os_str() != "--")
        .any(|a| a == "-h" || a == "--help")
}

pub fn parse_args<I>(argv: I) -> Result<Args, String>
where
    I: IntoIterator<Item = OsString>,
{
    let mut args = Args::default();
    let mut it = argv.into_iter();

    while let Some(arg) = it.next() {
        // Only the option names need to be valid UTF-8. Look at the encoded
        // bytes so that values attached to the options like "-C<path>" don't
        // need to be.
        let bytes = arg.as_encoded_bytes();

        if bytes == b"--" {
            return match it.next() {
                Some(extra) => Err(format!("unexpected argument: {extra:?}")),
                None => Ok(args),
            };
        }

        if bytes.starts_with(b"--") {
            let (name, mut inline_val) = match bytes.iter().position(|&b| b == b'=') {
                Some(i) => {
                    let (n, v) = split_at_ascii(&arg, i);
                    (n, Some(v.to_os_string()))
                }
                None => (arg.as_os_str(), None),
            };
            let Some(name) = name.to_str() else {
                return Err(format!("unknown command-line argument: {arg:?}"));
            };

            match name {
                "--help" => set_args_value(&mut args.help, name, flag(name, &inline_val)?)?,
                "--version" => set_args_value(&mut args.version, name, flag(name, &inline_val)?)?,
                "--verbose" => set_args_value(&mut args.verbose, name, flag(name, &inline_val)?)?,
                "--no-verbose" => {
                    no_value_flag(name, &inline_val)?;
                    set_args_value(&mut args.verbose, "--verbose", false)?;
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
        } else if let Some(cluster) = bytes.strip_prefix(b"-").filter(|c| !c.is_empty()) {
            for (i, &c) in cluster.iter().enumerate() {
                match c {
                    b'h' => set_args_value(&mut args.help, "-h", true)?,
                    b'v' => set_args_value(&mut args.verbose, "-v", true)?,
                    b'V' => set_args_value(&mut args.version, "-V", true)?,
                    b'b' | b'B' | b'C' | b'T' => {
                        let c = char::from(c);
                        let (_, rest) = split_at_ascii(&arg, i + 1);
                        let val = if rest.is_empty() {
                            it.next()
                                .ok_or_else(|| format!("option \"-{c}\" requires a value"))?
                        } else {
                            rest.to_os_string()
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
                    _ => {
                        let c = String::from_utf8_lossy(&cluster[i..]);
                        let c = c.chars().next().unwrap_or(char::REPLACEMENT_CHARACTER);
                        return Err(format!("unknown option: -{c}"));
                    }
                }
            }
        } else {
            return Err(format!("unknown command-line argument: {arg:?}"));
        }
    }

    Ok(args)
}

/// Splits `s` into the parts before and after the ASCII character at the index
/// `i` of its encoded bytes. The parts don't need to be valid UTF-8.
///
/// # Panics
///
/// Panics if the byte at the index `i` is not an ASCII character.
fn split_at_ascii(s: &OsStr, i: usize) -> (&OsStr, &OsStr) {
    let bytes = s.as_encoded_bytes();
    assert!(bytes[i].is_ascii());
    // SAFETY: The parts are split from an `OsStr` immediately before and after
    // an ASCII character, which `OsStr::from_encoded_bytes_unchecked` allows.
    unsafe {
        (
            OsStr::from_encoded_bytes_unchecked(&bytes[..i]),
            OsStr::from_encoded_bytes_unchecked(&bytes[i + 1..]),
        )
    }
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
        assert_eq!(parse(&["-vé"]), Err("unknown option: -é".into()));
    }

    #[test]
    fn asks_for_help_despite_invalid_arguments() {
        for (argv, expected) in [
            (&["--nope", "--help"][..], true),
            (&["-x", "-h"], true),
            (&["--nope"], false),
            (&["--", "--help"], false),
            (&["--branch=--help"], false),
        ] {
            let argv: Vec<OsString> = argv.iter().map(OsString::from).collect();
            assert_eq!(asks_for_help(&argv), expected, "{argv:?}");
        }
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
    fn bad() -> OsString {
        use std::os::unix::ffi::OsStringExt;
        OsString::from_vec(vec![0x66, 0x6f, 0xff]) // "fo\xFF"
    }

    #[cfg(windows)]
    fn bad() -> OsString {
        use std::os::windows::ffi::OsStringExt;
        OsString::from_wide(&[0x66, 0x6f, 0xD800])
    }

    fn with_bad(prefix: &str) -> OsString {
        let mut s = OsString::from(prefix);
        s.push(bad());
        s
    }

    #[test]
    fn non_utf8_branch_is_rejected() {
        for argv in [
            vec![OsString::from("--branch"), bad()],
            vec![with_bad("--branch=")],
            vec![OsString::from("-b"), bad()],
            vec![with_bad("-b")],
        ] {
            let err = parse_args(argv.clone()).unwrap_err();
            assert!(err.contains("not valid UTF-8"), "{argv:?}: {err}");
        }
    }

    #[test]
    fn non_utf8_repository_path_is_accepted() {
        for argv in [
            vec![OsString::from("--repository"), bad()],
            vec![with_bad("--repository=")],
            vec![OsString::from("-C"), bad()],
            vec![with_bad("-C")],
            vec![with_bad("-vC")],
        ] {
            let args = parse_args(argv.clone()).unwrap();
            assert_eq!(args.repository, Some(PathBuf::from(bad())), "{argv:?}");
        }
    }

    #[test]
    fn non_utf8_argument_itself_is_rejected() {
        for arg in [bad(), with_bad("--"), with_bad("-v")] {
            assert!(parse_args([arg.clone()]).is_err(), "{arg:?} should fail");
        }
        let mut arg = with_bad("--");
        arg.push("=value");
        assert!(parse_args([arg]).is_err());
    }
}
