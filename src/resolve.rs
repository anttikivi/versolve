// SPDX-FileCopyrightText: (C) 2026 Antti Kivi <antti@anttikivi.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use std::{collections::HashMap, env, io, path::PathBuf};

use git2::{ObjectType, Oid, Repository, Sort};

use crate::args::Args;

#[derive(Debug)]
pub struct Options {
    pub verbose: bool,
    branch: Option<String>,
    default_branch: Option<String>,
    repository: PathBuf,
    tag_prefix: Option<String>,
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
    let repo = Repository::open(&opts.repository)?;

    if opts.verbose {
        eprintln!("opened repository at {:?}", repo.path());
    }

    let tags = collect_tags(&repo, opts.tag_prefix.as_deref().unwrap_or(""))?;

    let head_ref = repo.head()?;
    let branch = match opts.branch.as_deref() {
        Some(s) => s,
        None if head_ref.is_branch() => head_ref.shorthand()?,
        None => {
            return Err(format!(
                "repository in detached HEAD in reference {}",
                head_ref.name()?
            )
            .into());
        }
    };
    let head_obj = head_ref.peel(ObjectType::Commit)?;
    let head_oid = head_obj.id();

    let mut it = repo.revwalk()?;
    it.set_sorting(Sort::TOPOLOGICAL)?;
    it.push(head_oid)?;

    let mut found_tag = None;
    for (distance, oid) in it.enumerate() {
        let oid = oid?;
        if let Some(names) = tags.get(&oid) {
            found_tag = Some((names, distance));
            break;
        }
    }

    Ok(())
}

fn collect_tags(
    repo: &Repository,
    prefix: &str,
) -> Result<HashMap<Oid, Vec<String>>, Box<dyn std::error::Error>> {
    let mut tags: HashMap<Oid, Vec<String>> = HashMap::new();

    for rf in repo.references_glob("refs/tags/*")? {
        let rf = rf?;
        let Some(name) = rf.shorthand()?.strip_prefix(prefix) else {
            continue;
        };
        let commit = rf.peel_to_commit()?;
        tags.entry(commit.id()).or_default().push(name.to_owned());
    }

    Ok(tags)
}
