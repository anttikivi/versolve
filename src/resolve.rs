// SPDX-FileCopyrightText: (C) 2026 Antti Kivi <antti@anttikivi.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    collections::{HashMap, hash_map::Entry},
    env, io,
    path::PathBuf,
};

use git2::{ObjectType, Oid, Repository, Sort, StatusOptions};
use semver::{BuildMetadata, Prerelease, Version};

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
            repository,
            tag_prefix: value.tag_prefix,
            verbose: value.verbose.unwrap_or(false),
        })
    }
}

pub fn from_git(opts: &Options) -> Result<Version, Box<dyn std::error::Error>> {
    let repo = Repository::open(&opts.repository)?;

    if opts.verbose {
        eprintln!("opened repository at {:?}", repo.path());
    }

    let mut tags = collect_tags(&repo, opts.tag_prefix.as_deref().unwrap_or(""))?;

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
    let head_commit = head_ref.peel(ObjectType::Commit)?;
    let head_oid = head_commit.id();

    let mut it = repo.revwalk()?;
    it.set_sorting(Sort::TOPOLOGICAL)?;
    it.push(head_oid)?;

    let mut distance: usize = 0;
    let mut base = None;
    for oid in it {
        let oid = oid?;
        if let Some(version) = tags.remove(&oid) {
            base = Some(version);
            break;
        }
        distance += 1;
    }

    let mut version = match base {
        Some(mut v) => {
            if distance > 0 && v.pre.is_empty() {
                v.patch += 1;
            }
            v
        }
        None => Version::new(0, 1, 0),
    };

    if distance > 0 {
        let mut pre = match version.pre.as_str() {
            "" => format!("0.{distance}"),
            p => format!("{p}.0.{distance}"),
        };
        if opts.default_branch.as_deref() != Some(branch) {
            // TODO: Make sure the branch is a valid pre-release identifier.
            pre.push('.');
            pre.push_str(branch);
        }
        version.pre = Prerelease::new(&pre)?;
    }

    let mut status_opts = StatusOptions::new();
    let status_opts = status_opts
        .include_untracked(true)
        .recurse_untracked_dirs(true);
    let dirty = !repo.statuses(Some(status_opts))?.is_empty();

    let short_id = head_commit.short_id()?;
    let short_id = short_id.as_str()?;

    let build = if dirty {
        format!("g{short_id}.dirty")
    } else {
        format!("g{short_id}")
    };

    version.build = BuildMetadata::new(&build)?;

    Ok(version)
}

fn collect_tags(
    repo: &Repository,
    prefix: &str,
) -> Result<HashMap<Oid, Version>, Box<dyn std::error::Error>> {
    let mut tags = HashMap::new();

    for rf in repo.references_glob("refs/tags/*")? {
        let rf = rf?;
        let commit = rf.peel_to_commit()?;
        let Some(name) = rf.shorthand()?.strip_prefix(prefix) else {
            continue;
        };
        let version = Version::parse(name)?;
        match tags.entry(commit.id()) {
            Entry::Occupied(mut entry) => {
                if version > *entry.get() {
                    entry.insert(version);
                }
            }
            Entry::Vacant(entry) => {
                entry.insert(version);
            }
        }
    }

    Ok(tags)
}
