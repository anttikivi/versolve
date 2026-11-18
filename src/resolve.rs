use std::{env, io, path::PathBuf};

use git2::{Repository, RepositoryOpenFlags};

use crate::args::Args;

#[derive(Debug)]
pub struct Options {
    pub branch: Option<String>,
    pub default_branch: Option<String>,
    pub repository: PathBuf,
    pub tag_prefix: Option<String>,
    pub verbose: bool,
}

impl TryFrom<Args> for Options {
    type Error = io::Error;

    fn try_from(value: Args) -> Result<Self, Self::Error> {
        let repository = match value.repository {
            Some(path) => path,
            None => env::current_dir()?,
        };

        Ok(Self {
            branch: value.branch,
            default_branch: value.default_branch,
            repository: repository,
            tag_prefix: value.tag_prefix,
            verbose: value.verbose.unwrap_or(false),
        })
    }
}

pub fn from_git(opts: &Options) -> Result<(), Box<dyn std::error::Error>> {
    let repository = Repository::open(&opts.repository)?;

    Ok(())
}
