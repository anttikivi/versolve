// SPDX-FileCopyrightText: (C) 2026 Antti Kivi <antti@anttikivi.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    cmp::Ordering,
    collections::{HashMap, hash_map::Entry},
    env, io,
    path::PathBuf,
};

use git2::{ErrorCode, ObjectType, Oid, Repository, StatusOptions};
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
    let repo = Repository::discover(&opts.repository)?;

    verbose!(opts.verbose, "opened repository at {:?}", repo.path());

    if repo.is_shallow() {
        return Err(
            "repository is a shallow clone and its history may be incomplete, fetch the full \
            history with \"git fetch --unshallow --tags\""
                .into(),
        );
    }

    let mut tags = collect_tags(
        &repo,
        opts.tag_prefix.as_deref().unwrap_or(""),
        opts.verbose,
    )?;

    let head_ref = match repo.head() {
        Err(e) if e.code() == ErrorCode::UnbornBranch => {
            return Err("repository has no commits".into());
        }
        head => head?,
    };
    let head_commit = head_ref.peel(ObjectType::Commit)?;
    let head_oid = head_commit.id();

    // A tag on the current commit is always used as is, even if a reachable tag has a higher
    // version.
    let mut base = tags.remove(&head_oid).map(|v| (head_oid, v));
    if base.is_none() {
        let mut walk = repo.revwalk()?;
        walk.push(head_oid)?;

        for oid in walk {
            let oid = oid?;
            let Some(v) = tags.remove(&oid) else {
                continue;
            };
            let replace = match &base {
                None => true,
                Some((base_oid, b)) => match v.cmp_precedence(b) {
                    Ordering::Greater => true,
                    Ordering::Equal => repo.graph_descendant_of(oid, *base_oid)?,
                    Ordering::Less => false,
                },
            };
            if replace {
                base = Some((oid, v));
            }
        }
    }

    let mut walk = repo.revwalk()?;
    walk.push(head_oid)?;
    if let Some((oid, _)) = &base {
        walk.hide(*oid)?;
    }
    let distance = walk.try_fold(0usize, |n, id| id.map(|_| n + 1))?;

    let mut version = match base {
        Some((_, mut v)) => {
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
        let branch = match opts.branch.as_deref() {
            Some(s) => s,
            None if head_ref.is_branch() => head_ref.shorthand()?,
            None => {
                return Err(format!(
                    "HEAD is detached at {head_oid}; pass \"--branch\" to name the branch"
                )
                .into());
            }
        };
        if opts.default_branch.as_deref() != Some(branch) {
            pre.push('.');
            if branch.bytes().all(|b| b.is_ascii_digit()) {
                pre.push('b');
            }
            pre.extend(branch.chars().map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' {
                    c
                } else {
                    '-'
                }
            }));
        }
        version.pre = Prerelease::new(&pre)?;
    }

    let mut status_opts = StatusOptions::new();
    let status_opts = status_opts
        .include_untracked(true)
        .recurse_untracked_dirs(true);
    let dirty = !repo.is_bare() && !repo.statuses(Some(status_opts))?.is_empty();

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
    verbose: bool,
) -> Result<HashMap<Oid, Version>, Box<dyn std::error::Error>> {
    let mut tags = HashMap::new();

    for reference in repo.references_glob("refs/tags/*")? {
        let reference = reference?;
        let Ok(tag) = reference.shorthand() else {
            verbose!(
                verbose,
                "skipping tag {:?}, name not valid UTF-8",
                String::from_utf8_lossy(reference.shorthand_bytes())
            );
            continue;
        };
        let Some(name) = tag.strip_prefix(prefix) else {
            verbose!(
                verbose,
                "skipping tag {tag:?}, name does not start with the prefix {prefix:?}"
            );
            continue;
        };
        let version = match Version::parse(name) {
            Ok(v) => v,
            Err(e) => {
                verbose!(
                    verbose,
                    "skipping tag {tag:?}, {name:?} is not a valid version number: {e}"
                );
                continue;
            }
        };
        let target = reference.peel(ObjectType::Any)?;
        let Some(commit) = target.as_commit() else {
            verbose!(
                verbose,
                "skipping tag {tag:?}, it points to {} instead of commit",
                target.kind().map_or("unknown object", |k| k.str())
            );
            continue;
        };
        verbose!(
            verbose,
            "found version {version} from tag {tag:?} on commit {}",
            commit.id()
        );
        match tags.entry(commit.id()) {
            Entry::Occupied(mut entry) => {
                if version > *entry.get() {
                    verbose!(
                        verbose,
                        "version {version} replaces {} on commit {}",
                        entry.get(),
                        commit.id()
                    );
                    entry.insert(version);
                } else {
                    verbose!(
                        verbose,
                        "ignoring version {version}, commit {} already has {}",
                        commit.id(),
                        entry.get()
                    );
                }
            }
            Entry::Vacant(entry) => {
                entry.insert(version);
            }
        }
    }

    Ok(tags)
}

#[cfg(test)]
mod tests {
    use std::{cmp::Ordering, fs, path::Path, process::Command};

    use git2::{Commit, Signature, Time};
    use tempfile::TempDir;

    use super::*;

    struct TestRepo {
        dir: TempDir,
    }

    impl TestRepo {
        fn new() -> Self {
            let repo = Self {
                dir: TempDir::new().expect("failed to create temporary directory"),
            };
            repo.git(&["init", "--initial-branch=main"]);
            repo
        }

        fn path(&self) -> &Path {
            self.dir.path()
        }

        fn options(&self) -> Options {
            Options {
                verbose: false,
                branch: None,
                default_branch: None,
                repository: self.path().to_path_buf(),
                tag_prefix: None,
            }
        }

        fn git(&self, args: &[&str]) {
            let mut cmd = Command::new("git");
            for (key, _) in env::vars_os() {
                if key.to_str().is_some_and(|k| k.starts_with("GIT_")) {
                    cmd.env_remove(key);
                }
            }
            let output = cmd
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
                .args(["-c", "commit.gpgSign=false", "-c", "tag.gpgSign=false"])
                .arg("-C")
                .arg(self.path())
                .args(args)
                .output()
                .expect("failed to run git");
            assert!(
                output.status.success(),
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        fn commit(&self, message: &str) {
            self.git(&["commit", "--allow-empty", "-m", message]);
        }

        fn commit_many(&self, n: usize) {
            for i in 0..n {
                self.commit(&format!("commit {i}"));
            }
        }

        fn commit_all(&self, message: &str) {
            self.git(&["add", "--all"]);
            self.commit(message);
        }

        fn commit_fast_forward(&self, n: usize) {
            let repo = Repository::open(self.path()).unwrap();
            let tree_oid = repo.treebuilder(None).unwrap().write().unwrap();
            let tree = repo.find_tree(tree_oid).unwrap();
            let sig =
                Signature::new("Test", "test@example.com", &Time::new(1735689600, 0)).unwrap();

            let mut parent = repo.head().ok().map(|head| head.peel_to_commit().unwrap());
            for i in 0..n {
                let parents: Vec<&Commit> = parent.iter().collect();
                let oid = repo
                    .commit(None, &sig, &sig, &format!("commit {i}"), &tree, &parents)
                    .unwrap();
                parent = Some(repo.find_commit(oid).unwrap());
            }

            let head = parent.expect("no commits were created").id();
            repo.reference("refs/heads/main", head, true, "fast-forward")
                .unwrap();
        }

        fn tag(&self, name: &str) {
            self.git(&["tag", "-a", name, "-m", name]);
        }

        fn write_file(&self, name: &str, contents: &str) {
            fs::write(self.path().join(name), contents).unwrap();
        }

        fn make_dirty(&self) {
            self.write_file("dirty.txt", "uncommitted changes\n");
        }

        fn short_head_id(&self) -> String {
            let repo = Repository::open(self.path()).unwrap();
            let head = repo.head().unwrap().peel(ObjectType::Commit).unwrap();
            head.short_id().unwrap().as_str().unwrap().to_owned()
        }

        fn resolve(&self) -> Version {
            self.resolve_with(&self.options())
        }

        fn resolve_with(&self, opts: &Options) -> Version {
            from_git(opts).unwrap_or_else(|e| panic!("failed to resolve version: {e}"))
        }
    }

    #[track_caller]
    fn assert_version(actual: &Version, expected: &str) {
        let expected = Version::parse(expected).unwrap();
        assert_eq!(
            actual.cmp_precedence(&expected),
            Ordering::Equal,
            "expected {expected}, found {actual}"
        );
    }

    #[track_caller]
    fn assert_dirty(actual: &Version) {
        assert!(
            actual.build.ends_with(".dirty"),
            "expected dirty build metadata, found {actual}"
        );
    }

    #[track_caller]
    fn assert_clean(actual: &Version) {
        assert!(
            !actual.build.ends_with(".dirty"),
            "expected clean build metadata, found {actual}"
        );
    }

    #[test]
    fn no_tags() {
        let repo = TestRepo::new();

        repo.commit("message");
        assert_version(&repo.resolve(), "0.1.0-0.1.main");

        repo.commit_many(12);
        assert_version(&repo.resolve(), "0.1.0-0.13.main");
    }

    #[test]
    fn no_tags_many_commits() {
        let repo = TestRepo::new();

        repo.commit_fast_forward(12_345);
        assert_version(&repo.resolve(), "0.1.0-0.12345.main");

        repo.commit_fast_forward(3000);
        assert_version(&repo.resolve(), "0.1.0-0.15345.main");
    }

    #[test]
    fn exact_tag() {
        let repo = TestRepo::new();

        repo.commit("commit");
        repo.tag("1.2.3");
        assert_version(&repo.resolve(), "1.2.3");

        repo.commit_many(10);
        repo.tag("1.2.4");
        assert_version(&repo.resolve(), "1.2.4");

        repo.commit_many(15);
        repo.tag("1.3.0-rc.3");
        assert_version(&repo.resolve(), "1.3.0-rc.3");
    }

    #[test]
    fn exact_tag_below_reachable_tag() {
        let repo = TestRepo::new();

        repo.commit("commit");
        repo.tag("0.0.1");
        repo.commit("commit");
        repo.tag("0.0.0");
        assert_version(&repo.resolve(), "0.0.0");

        repo.git(&["checkout", "--detach", "0.0.0"]);
        assert_version(&repo.resolve(), "0.0.0");
    }

    #[test]
    fn build_metadata_does_not_affect_base() {
        let repo = TestRepo::new();

        repo.commit("commit");
        repo.tag("1.0.0+z");
        repo.commit("commit");
        repo.tag("1.0.0+a");
        assert_version(&repo.resolve(), "1.0.0");

        repo.commit("commit");
        assert_version(&repo.resolve(), "1.0.1-0.1.main");
    }

    #[test]
    fn multiple_tags_on_same_commit() {
        let repo = TestRepo::new();

        repo.commit("commit");
        repo.tag("1.0.0-rc.1");
        repo.tag("1.0.0");
        assert_version(&repo.resolve(), "1.0.0");
    }

    #[test]
    fn correct_increment() {
        let repo = TestRepo::new();

        repo.commit("commit");
        repo.tag("1.2.3");
        assert_version(&repo.resolve(), "1.2.3");

        repo.commit("commit");
        assert_version(&repo.resolve(), "1.2.4-0.1.main");

        repo.commit_many(10);
        assert_version(&repo.resolve(), "1.2.4-0.11.main");

        repo.commit_many(10);
        repo.tag("1.2.4-rc.1");
        assert_version(&repo.resolve(), "1.2.4-rc.1");

        repo.commit("commit");
        assert_version(&repo.resolve(), "1.2.4-rc.1.0.1.main");

        repo.commit_many(4);
        assert_version(&repo.resolve(), "1.2.4-rc.1.0.5.main");
    }

    #[test]
    fn build_metadata() {
        let repo = TestRepo::new();

        repo.commit("commit");
        let version = repo.resolve();
        assert_eq!(version.build.as_str(), format!("g{}", repo.short_head_id()));
    }

    #[test]
    fn dirty() {
        let repo = TestRepo::new();

        repo.commit_many(3);
        repo.make_dirty();
        let version = repo.resolve();
        assert_version(&version, "0.1.0-0.3.main");
        assert_dirty(&version);

        repo.commit_all("commit the file");
        repo.tag("1.1.1");
        let version = repo.resolve();
        assert_version(&version, "1.1.1");
        assert_clean(&version);

        repo.write_file("text.txt", "another file\n");
        let version = repo.resolve();
        assert_version(&version, "1.1.1");
        assert_dirty(&version);

        repo.commit_all("commit the second file");
        repo.write_file("third.txt", "third file\n");
        let version = repo.resolve();
        assert_version(&version, "1.1.2-0.1.main");
        assert_dirty(&version);
    }

    #[test]
    fn tag_prefix() {
        let repo = TestRepo::new();
        let opts = Options {
            tag_prefix: Some("v".into()),
            ..repo.options()
        };

        repo.commit("commit");
        repo.tag("v1.0.0");
        assert_version(&repo.resolve_with(&opts), "1.0.0");

        repo.commit_many(10);
        assert_version(&repo.resolve_with(&opts), "1.0.1-0.10.main");

        repo.tag("1.0.1");
        assert_version(&repo.resolve_with(&opts), "1.0.1-0.10.main");

        repo.commit("commit");
        repo.tag("v1.0.1");
        assert_version(&repo.resolve_with(&opts), "1.0.1");
    }

    #[test]
    fn default_branch() {
        let repo = TestRepo::new();
        let opts = Options {
            default_branch: Some("main".into()),
            ..repo.options()
        };

        repo.commit("commit");
        assert_version(&repo.resolve_with(&opts), "0.1.0-0.1");
    }

    #[test]
    fn manual_branch() {
        let repo = TestRepo::new();
        let opts = Options {
            branch: Some("branch".into()),
            ..repo.options()
        };

        repo.commit("commit");
        assert_version(&repo.resolve_with(&opts), "0.1.0-0.1.branch");
    }

    #[test]
    fn invalid_branch_characters() {
        let repo = TestRepo::new();

        repo.commit("commit");
        for (branch, expected) in [
            ("feature/x", "0.1.0-0.1.feature-x"),
            ("my_branch", "0.1.0-0.1.my-branch"),
            ("release.1", "0.1.0-0.1.release-1"),
            ("123", "0.1.0-0.1.b123"),
            ("007", "0.1.0-0.1.b007"),
        ] {
            let opts = Options {
                branch: Some(branch.into()),
                ..repo.options()
            };
            assert_version(&repo.resolve_with(&opts), expected);
        }

        repo.git(&["checkout", "-b", "feature/y"]);
        assert_version(&repo.resolve(), "0.1.0-0.1.feature-y");
    }

    #[test]
    fn default_and_manual_branch() {
        let repo = TestRepo::new();
        let opts = Options {
            branch: Some("master".into()),
            default_branch: Some("master".into()),
            ..repo.options()
        };

        repo.commit("commit");
        assert_version(&repo.resolve_with(&opts), "0.1.0-0.1");
    }

    #[test]
    fn shallow_clone_is_rejected() {
        let repo = TestRepo::new();

        repo.commit_many(3);
        repo.tag("1.0.0");
        repo.commit("commit");

        let clone = TempDir::new().unwrap();
        let clone_path = clone.path().to_str().unwrap();
        let repo_path = repo.path().to_str().unwrap();
        repo.git(&["clone", "--no-local", "--depth=1", repo_path, clone_path]);

        let opts = Options {
            repository: clone.path().to_path_buf(),
            ..repo.options()
        };
        let err = from_git(&opts).unwrap_err();
        assert!(
            err.to_string().contains("shallow"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn subdirectory() {
        let repo = TestRepo::new();

        repo.commit("commit");
        repo.tag("1.0.0");
        fs::create_dir(repo.path().join("sub")).unwrap();
        let opts = Options {
            repository: repo.path().join("sub"),
            ..repo.options()
        };
        assert_version(&repo.resolve_with(&opts), "1.0.0");
    }

    #[test]
    fn bare_repository() {
        let repo = TestRepo::new();

        repo.commit("commit");
        repo.tag("1.0.0");
        repo.commit("commit");

        let bare = TempDir::new().unwrap();
        let bare_path = bare.path().to_str().unwrap();
        repo.git(&["clone", "--bare", ".", bare_path]);

        let opts = Options {
            repository: bare.path().to_path_buf(),
            ..repo.options()
        };
        let version = repo.resolve_with(&opts);
        assert_version(&version, "1.0.1-0.1.main");
        assert_clean(&version);
    }

    #[test]
    fn no_commits() {
        let repo = TestRepo::new();

        let err = from_git(&repo.options()).unwrap_err();
        assert_eq!(err.to_string(), "repository has no commits");
    }

    #[test]
    fn detached_head() {
        let repo = TestRepo::new();

        repo.commit_many(2);
        repo.git(&["checkout", "--detach", "HEAD~1"]);
        assert!(from_git(&repo.options()).is_err());

        let opts = Options {
            branch: Some("main".into()),
            ..repo.options()
        };
        assert_version(&repo.resolve_with(&opts), "0.1.0-0.1.main");
    }

    #[test]
    fn hotfix_branch_ignores_newer_tags() {
        let repo = TestRepo::new();

        repo.commit("commit");
        repo.tag("1.5.0");
        repo.commit("commit");
        repo.tag("1.6.0");
        repo.commit("commit");
        repo.tag("2.0.0");
        repo.commit("commit");

        repo.git(&["checkout", "-b", "hotfix", "1.5.0"]);
        repo.commit_many(2);
        assert_version(&repo.resolve(), "1.5.1-0.2.hotfix");

        repo.tag("1.5.1");
        repo.commit("commit");
        assert_version(&repo.resolve(), "1.5.2-0.1.hotfix");

        repo.git(&["checkout", "main"]);
        assert_version(&repo.resolve(), "2.0.1-0.1.main");
    }

    #[test]
    fn merge_lower_tag_into_main() {
        let repo = TestRepo::new();

        repo.commit("commit");
        repo.tag("1.5.0");

        repo.git(&["checkout", "-b", "hotfix"]);
        repo.commit("fix");
        repo.tag("1.5.1");

        repo.git(&["checkout", "main"]);
        repo.commit("commit");
        repo.tag("2.0.0");
        repo.commit("commit");
        assert_version(&repo.resolve(), "2.0.1-0.1.main");

        repo.git(&["merge", "--no-ff", "-m", "merge hotfix", "hotfix"]);
        assert_version(&repo.resolve(), "2.0.1-0.3.main");
    }

    #[test]
    fn merge_higher_tag_into_hotfix_branch() {
        let repo = TestRepo::new();

        repo.commit("commit");
        repo.tag("1.5.0");

        repo.git(&["checkout", "-b", "hotfix"]);
        repo.commit("fix");
        repo.tag("1.5.1");
        repo.commit("fix");

        repo.git(&["checkout", "main"]);
        repo.commit("commit");
        repo.tag("2.0.0");

        repo.git(&["checkout", "hotfix"]);
        repo.git(&["merge", "--no-ff", "-m", "merge main", "main"]);
        assert_version(&repo.resolve(), "2.0.1-0.3.hotfix");
    }

    #[test]
    fn distance_counts_merged_commits() {
        let repo = TestRepo::new();

        repo.commit("commit");
        repo.git(&["checkout", "-b", "side"]);
        repo.commit_many(2);

        repo.git(&["checkout", "main"]);
        repo.commit("commit");
        repo.tag("1.0.0");
        repo.commit("commit");

        repo.git(&["merge", "--no-ff", "-m", "merge side", "side"]);
        assert_version(&repo.resolve(), "1.0.1-0.4.main");
    }

    #[test]
    fn lightweight_tags() {
        let repo = TestRepo::new();

        repo.commit("commit");
        repo.git(&["tag", "1.0.0"]);
        assert_version(&repo.resolve(), "1.0.0");

        repo.commit("commit");
        assert_version(&repo.resolve(), "1.0.1-0.1.main");
    }

    #[test]
    fn non_version_tags_are_ignored() {
        let repo = TestRepo::new();

        repo.commit("commit");
        repo.tag("release-2020");
        repo.tag("latest");
        assert_version(&repo.resolve(), "0.1.0-0.1.main");

        repo.commit("commit");
        repo.tag("1.0.0");
        repo.tag("stable");
        assert_version(&repo.resolve(), "1.0.0");
    }

    #[test]
    fn invalid_versions_are_ignored() {
        let repo = TestRepo::new();

        repo.commit("commit");
        repo.tag("1.0.0");
        repo.commit("commit");
        for name in ["2.0", "02.0.0", "2.0.0-rc.01", "2.0.0-", "2.0.0+", "v2.0.0"] {
            repo.tag(name);
        }
        assert_version(&repo.resolve(), "1.0.1-0.1.main");
    }

    #[test]
    fn prefixed_tags_require_tag_prefix() {
        let repo = TestRepo::new();
        let opts = Options {
            tag_prefix: Some("v".into()),
            ..repo.options()
        };

        repo.commit("commit");
        repo.tag("v1.0.0");
        assert_version(&repo.resolve(), "0.1.0-0.1.main");
        assert_version(&repo.resolve_with(&opts), "1.0.0");
    }

    #[test]
    fn prefixed_non_version_tags_are_ignored() {
        let repo = TestRepo::new();
        let opts = Options {
            tag_prefix: Some("v".into()),
            ..repo.options()
        };

        repo.commit("commit");
        repo.tag("v1.0.0");
        repo.commit("commit");
        repo.tag("very-old");
        repo.tag("vnext");
        repo.tag("v2");
        assert_version(&repo.resolve_with(&opts), "1.0.1-0.1.main");
    }

    #[test]
    fn non_commit_tags_are_ignored() {
        let repo = TestRepo::new();

        repo.write_file("file.txt", "contents\n");
        repo.commit_all("commit");
        repo.tag("1.0.0");
        repo.commit("commit");
        repo.git(&["tag", "2.0.0", "HEAD^{tree}"]);
        repo.git(&["tag", "-a", "3.0.0", "-m", "3.0.0", "HEAD^{tree}"]);
        repo.git(&["tag", "-a", "4.0.0", "-m", "4.0.0", "HEAD:file.txt"]);
        assert_version(&repo.resolve(), "1.0.1-0.1.main");
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_tag_names_are_ignored() {
        let repo = TestRepo::new();

        repo.commit("commit");
        repo.tag("1.0.0");
        repo.commit("commit");
        let oid = Repository::open(repo.path())
            .unwrap()
            .refname_to_id("HEAD")
            .unwrap();
        let mut line = format!("{oid} refs/tags/").into_bytes();
        line.extend_from_slice(b"2.0.0\xff");
        line.push(b'\n');
        fs::write(repo.path().join(".git/packed-refs"), line).unwrap();
        assert_version(&repo.resolve(), "1.0.1-0.1.main");
    }
}
