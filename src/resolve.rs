// SPDX-FileCopyrightText: (C) 2026 Antti Kivi <antti@anttikivi.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    borrow::Cow,
    cmp::Ordering,
    collections::{HashMap, hash_map::Entry},
    env, fs, io,
    path::PathBuf,
};

use git2::{Config, ErrorCode, ObjectType, Oid, Repository, StatusOptions};
use semver::{BuildMetadata, Prerelease, Version};

use crate::args::Args;

#[derive(Debug)]
pub struct Options {
    pub verbose: bool,
    /// Whether to append `.dirty` to the build metadata if the working tree has
    /// uncommitted changes.
    pub check_dirty: bool,
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
            check_dirty: true,
            default_branch: value.default_branch,
            repository,
            tag_prefix: value.tag_prefix,
            verbose: value.verbose.unwrap_or(false),
        })
    }
}

pub fn from_git(opts: &Options) -> Result<Version, Box<dyn std::error::Error>> {
    resolve_from_git(opts).map_err(|e| match e.downcast::<git2::Error>() {
        Ok(e) => {
            verbose!(opts.verbose, "libgit2 error: {e}");
            e.message().into()
        }
        Err(e) => e,
    })
}

fn resolve_from_git(opts: &Options) -> Result<Version, Box<dyn std::error::Error>> {
    let repo = match Repository::discover(&opts.repository) {
        Err(e) if e.message().contains("extensions.refstorage") => {
            verbose!(opts.verbose, "libgit2 error: {e}");
            return Err(ref_storage_error(opts).into());
        }
        repo => repo?,
    };

    verbose!(opts.verbose, "opened repository at {:?}", repo.path());

    let shallow_in_commondir =
        || std::fs::metadata(repo.commondir().join("shallow")).is_ok_and(|m| m.len() > 0);
    if repo.is_shallow() || shallow_in_commondir() {
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
            let head = repo.find_reference("HEAD")?;
            let branch = head
                .symbolic_target()
                .ok()
                .flatten()
                .and_then(|t| t.strip_prefix("refs/heads/"));
            return Err(match branch {
                Some(b) => format!("the current branch \"{b}\" has no commits"),
                None => "the current branch has no commits".into(),
            }
            .into());
        }
        head => head?,
    };
    let head_commit = head_ref.peel(ObjectType::Commit)?;
    let head_oid = head_commit.id();

    let (mut base, mut base_oids) = match tags.remove(&head_oid) {
        Some(v) => (Some(v), vec![head_oid]),
        None => (None, Vec::new()),
    };
    if base.is_none() && !tags.is_empty() {
        let mut walk = repo.revwalk()?;
        walk.push(head_oid)?;

        for oid in walk {
            if tags.is_empty() {
                break;
            }
            let oid = oid?;
            let Some(v) = tags.remove(&oid) else {
                continue;
            };
            match base.as_ref().map(|b| v.cmp_precedence(b)) {
                None | Some(Ordering::Greater) => {
                    tags.retain(|_, t| t.cmp_precedence(&v) != Ordering::Less);
                    base = Some(v);
                    base_oids = vec![oid];
                }
                Some(Ordering::Equal) => base_oids.push(oid),
                Some(Ordering::Less) => {}
            }
        }
    }

    let distance_from = |base_oid: Option<Oid>| -> Result<usize, git2::Error> {
        let mut walk = repo.revwalk()?;
        walk.push(head_oid)?;
        if let Some(oid) = base_oid {
            walk.hide(oid)?;
        }
        walk.try_fold(0, |n, id| id.map(|_| n + 1))
    };
    let distance = match base_oids.as_slice() {
        [] => distance_from(None)?,
        oids => oids
            .iter()
            .try_fold(0, |d, &oid| distance_from(Some(oid)).map(|n| d.max(n)))?,
    };

    let mut version = match base {
        Some(mut v) => {
            if distance > 0 && v.pre.is_empty() {
                v.patch = v.patch.checked_add(1).ok_or_else(|| {
                    format!("cannot increment the patch version of {v}, it is the maximum value")
                })?;
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
            Some(s) => Cow::Borrowed(s),
            None if head_ref.is_branch() => String::from_utf8_lossy(head_ref.shorthand_bytes()),
            None => {
                return Err(format!(
                    "HEAD is detached at {head_oid}; pass \"--branch\" to name the branch"
                )
                .into());
            }
        };
        if opts.default_branch.as_deref() != Some(&*branch) {
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
    let status_opts = status_opts.include_untracked(true);
    let dirty = opts.check_dirty && !repo.is_bare() && {
        let statuses =
            repo.statuses(Some(status_opts))
                .map_err(|e| -> Box<dyn std::error::Error> {
                    if e.class() == git2::ErrorClass::Index && e.message().contains("'link'") {
                        verbose!(opts.verbose, "libgit2 error: {e}");
                        "repository uses a split index, which is not supported, turn it off with \
                        \"git update-index --no-split-index\""
                            .into()
                    } else {
                        e.into()
                    }
                })?;
        !statuses.is_empty()
    };

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

/// Explains why the repository with the unsupported `extensions.refStorage`
/// setting cannot be opened. libgit2 rejects the setting regardless of its
/// value, so read the value from the configuration to give the right advice.
fn ref_storage_error(opts: &Options) -> String {
    let value = Repository::discover_path(&opts.repository, None::<&str>)
        .ok()
        .and_then(|git_dir| {
            let common_dir = match fs::read_to_string(git_dir.join("commondir")) {
                Ok(s) => git_dir.join(s.trim_end_matches(['\n', '\r'])),
                Err(_) => git_dir,
            };
            Config::open(&common_dir.join("config"))
                .and_then(|c| c.get_string("extensions.refStorage"))
                .ok()
        });
    verbose!(opts.verbose, "extensions.refStorage is {value:?}");

    match value.as_deref() {
        Some("reftable") => {
            "repository stores its references in the reftable format, which is not supported, \
            convert them to the files format with \"git refs migrate --ref-format=files\""
                .into()
        }
        Some("files") => "repository sets \"extensions.refStorage\", which is not supported even \
            with the value \"files\", remove the setting with \
            \"git config --unset extensions.refStorage\""
            .into(),
        Some(v) => format!(
            "repository stores its references in the unsupported format {v:?}, convert them to the \
            files format with \"git refs migrate --ref-format=files\""
        ),
        None => "repository sets \"extensions.refStorage\", which is not supported, convert its \
            references to the files format with \"git refs migrate --ref-format=files\" and remove \
            the setting"
            .into(),
    }
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
        let target = reference.peel(ObjectType::Any).map_err(|e| {
            verbose!(verbose, "libgit2 error: {e}");
            if e.code() == ErrorCode::NotFound {
                format!(
                    "tag \"{tag}\" points to an object that is missing from the repository, fetch \
                    the object or delete the tag with \"git tag -d {tag}\""
                )
            } else {
                format!("failed to read tag \"{tag}\": {}", e.message())
            }
        })?;
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
    use std::{cmp::Ordering, fs, path::Path, process::Command, sync::Once};

    use git2::{Commit, ConfigLevel, Signature, Time};
    use tempfile::TempDir;

    use super::*;

    fn isolate_libgit2_config() {
        static ISOLATE: Once = Once::new();
        ISOLATE.call_once(|| {
            for level in [
                ConfigLevel::System,
                ConfigLevel::Global,
                ConfigLevel::XDG,
                ConfigLevel::ProgramData,
            ] {
                // SAFETY: Every test creates a `TestRepo` before it uses
                // libgit2, and `Once` blocks the other tests until the search
                // paths are set, so nothing reads them concurrently.
                unsafe { git2::opts::set_search_path(level, "") }
                    .expect("failed to clear the libgit2 search path");
            }
        });
    }

    struct TestRepo {
        dir: TempDir,
    }

    impl TestRepo {
        fn new() -> Self {
            Self::init(&[])
        }

        fn init(args: &[&str]) -> Self {
            isolate_libgit2_config();
            let repo = Self {
                dir: TempDir::new().expect("failed to create temporary directory"),
            };
            let mut init = vec!["init", "--initial-branch=main"];
            init.extend_from_slice(args);
            repo.git(&init);
            repo
        }

        fn path(&self) -> &Path {
            self.dir.path()
        }

        fn options(&self) -> Options {
            Options {
                verbose: false,
                check_dirty: true,
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

        /// Creates a commit with the given parents and commit time without
        /// updating any references, so that the tests control the order in
        /// which the commits are walked.
        fn commit_at(&self, parents: &[Oid], time: i64) -> Oid {
            let repo = Repository::open(self.path()).unwrap();
            let tree_oid = repo.treebuilder(None).unwrap().write().unwrap();
            let tree = repo.find_tree(tree_oid).unwrap();
            let sig = Signature::new("Test", "test@example.com", &Time::new(time, 0)).unwrap();
            let parents: Vec<Commit> = parents
                .iter()
                .map(|&oid| repo.find_commit(oid).unwrap())
                .collect();
            let parents: Vec<&Commit> = parents.iter().collect();
            repo.commit(None, &sig, &sig, "commit", &tree, &parents)
                .unwrap()
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
        repo.tag("1.0.0+a");
        repo.commit("commit");
        repo.tag("1.0.0+z");
        assert_version(&repo.resolve(), "1.0.0");

        repo.commit("commit");
        assert_version(&repo.resolve(), "1.0.1-0.2.main");
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
    fn maximum_patch_version() {
        let repo = TestRepo::new();

        repo.commit("commit");
        repo.tag("1.0.18446744073709551615");
        assert_version(&repo.resolve(), "1.0.18446744073709551615");

        repo.commit("commit");
        let err = from_git(&repo.options()).unwrap_err();
        assert_eq!(
            err.to_string(),
            "cannot increment the patch version of 1.0.18446744073709551615, it is the maximum \
            value"
        );
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
    fn dirty_untracked_directories() {
        let repo = TestRepo::new();

        repo.write_file(".gitignore", "*.log\n");
        repo.commit_all("commit");
        fs::create_dir_all(repo.path().join("empty/nested")).unwrap();
        fs::create_dir_all(repo.path().join("logs/nested")).unwrap();
        repo.write_file("logs/nested/build.log", "ignored\n");
        assert_clean(&repo.resolve());

        repo.write_file("logs/nested/file.txt", "untracked\n");
        assert_dirty(&repo.resolve());
    }

    #[test]
    fn dirty_check_disabled() {
        let repo = TestRepo::new();
        let opts = Options {
            check_dirty: false,
            ..repo.options()
        };

        repo.commit("commit");
        repo.make_dirty();
        let version = repo.resolve_with(&opts);
        assert_eq!(version.build.as_str(), format!("g{}", repo.short_head_id()));
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
    fn non_utf8_branch_name() {
        let repo = TestRepo::new();

        repo.commit("commit");
        let head = Repository::open(repo.path())
            .unwrap()
            .head()
            .unwrap()
            .target()
            .unwrap();
        let git_dir = repo.path().join(".git");
        let mut packed = b"# pack-refs with: peeled fully-peeled sorted \n".to_vec();
        packed.extend_from_slice(format!("{head} refs/heads/caf").as_bytes());
        packed.extend_from_slice(b"\xe9\n");
        fs::write(git_dir.join("packed-refs"), packed).unwrap();
        fs::write(git_dir.join("HEAD"), b"ref: refs/heads/caf\xe9\n").unwrap();

        assert_version(&repo.resolve(), "0.1.0-0.1.caf-");
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

        let worktree = TempDir::new().unwrap();
        let worktree_path = worktree.path().join("worktree");
        repo.git(&[
            "-C",
            clone_path,
            "worktree",
            "add",
            "--detach",
            worktree_path.to_str().unwrap(),
        ]);

        for path in [clone.path(), &worktree_path] {
            let opts = Options {
                repository: path.to_path_buf(),
                ..repo.options()
            };
            let err = from_git(&opts).unwrap_err();
            assert!(
                err.to_string().contains("git fetch --unshallow --tags"),
                "unexpected error for {path:?}: {err}"
            );
        }
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
        assert_eq!(
            err.to_string(),
            "the current branch \"main\" has no commits"
        );
    }

    #[test]
    fn orphan_branch() {
        let repo = TestRepo::new();

        repo.commit("commit");
        repo.tag("1.0.0");
        repo.git(&["checkout", "--orphan", "orphan"]);

        let err = from_git(&repo.options()).unwrap_err();
        assert_eq!(
            err.to_string(),
            "the current branch \"orphan\" has no commits"
        );
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
    fn higher_tag_found_after_lower_tag() {
        let repo = TestRepo::new();

        let root = repo.commit_at(&[], 1735689600);
        let higher = repo.commit_at(&[root], 1735689610);
        let lower = repo.commit_at(&[root], 1735689620);
        let merge = repo.commit_at(&[lower, higher], 1735689630);
        repo.git(&["tag", "2.0.0", &higher.to_string()]);
        repo.git(&["tag", "1.5.0", &lower.to_string()]);
        repo.git(&["update-ref", "refs/heads/main", &merge.to_string()]);
        assert_version(&repo.resolve(), "2.0.1-0.2.main");
    }

    #[test]
    fn equal_tag_on_descendant_found_after_ancestor() {
        let repo = TestRepo::new();

        let ancestor = repo.commit_at(&[], 1735689620);
        let descendant = repo.commit_at(&[ancestor], 1735689610);
        let merge = repo.commit_at(&[ancestor, descendant], 1735689630);
        repo.git(&["tag", "1.0.0+a", &ancestor.to_string()]);
        repo.git(&["tag", "1.0.0+b", &descendant.to_string()]);
        repo.git(&["update-ref", "refs/heads/main", &merge.to_string()]);
        assert_version(&repo.resolve(), "1.0.1-0.2.main");
    }

    #[test]
    fn farthest_equal_tag_on_parallel_branch() {
        for (near_time, far_time) in [(1735689610, 1735689650), (1735689650, 1735689610)] {
            for near_first in [true, false] {
                let repo = TestRepo::new();

                let root = repo.commit_at(&[], 1735689600);
                let mut near = root;
                for i in 0..3 {
                    near = repo.commit_at(&[near], near_time + i);
                }
                let far = repo.commit_at(&[root], far_time);
                let parents = if near_first { [near, far] } else { [far, near] };
                let merge = repo.commit_at(&parents, 1735689700);
                repo.git(&["tag", "1.0.0+near", &near.to_string()]);
                repo.git(&["tag", "1.0.0+far", &far.to_string()]);
                repo.git(&["update-ref", "refs/heads/main", &merge.to_string()]);
                assert_version(&repo.resolve(), "1.0.1-0.4.main");
            }
        }
    }

    #[test]
    fn merging_equal_tag_does_not_lower_version() {
        let repo = TestRepo::new();

        repo.commit("commit");
        repo.git(&["checkout", "-b", "a"]);
        repo.commit("commit");
        repo.tag("1.0.0+a");

        repo.git(&["checkout", "main"]);
        repo.commit_many(8);
        repo.git(&["checkout", "-b", "b"]);
        repo.commit("commit");
        repo.tag("1.0.0+b");

        repo.git(&["checkout", "main"]);
        repo.git(&["merge", "--no-ff", "-m", "merge a", "a"]);
        let first = repo.resolve();
        assert_version(&first, "1.0.1-0.9.main");

        repo.git(&["merge", "--no-ff", "-m", "merge b", "b"]);
        let second = repo.resolve();
        assert_version(&second, "1.0.1-0.11.main");
        assert_eq!(second.cmp_precedence(&first), Ordering::Greater);
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
    fn sha256_repository() {
        let repo = TestRepo::init(&["--object-format=sha256"]);

        repo.commit("commit");
        repo.tag("1.0.0");
        repo.git(&["tag", "2.0.0", "HEAD^{tree}"]);
        assert_version(&repo.resolve(), "1.0.0");

        repo.commit_many(2);
        repo.git(&["checkout", "-b", "feature"]);
        repo.commit("commit");
        repo.make_dirty();
        let version = repo.resolve();
        assert_version(&version, "1.0.1-0.3.feature");
        assert_eq!(
            version.build.as_str(),
            format!("g{}.dirty", repo.short_head_id())
        );
    }

    #[test]
    fn reftable_repository_is_rejected() {
        let repo = TestRepo::init(&["--ref-format=reftable"]);

        repo.commit("commit");
        let err = from_git(&repo.options()).unwrap_err();
        assert!(
            err.to_string().contains("reftable format")
                && err
                    .to_string()
                    .contains("git refs migrate --ref-format=files"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn split_index_is_rejected() {
        let repo = TestRepo::new();

        repo.write_file("file.txt", "contents\n");
        repo.commit_all("commit");
        repo.git(&["update-index", "--split-index"]);
        let err = from_git(&repo.options()).unwrap_err();
        assert!(
            err.to_string()
                .contains("git update-index --no-split-index"),
            "unexpected error: {err}"
        );

        repo.git(&["update-index", "--no-split-index"]);
        assert_version(&repo.resolve(), "0.1.0-0.1.main");
    }

    #[test]
    fn explicit_files_ref_storage_is_rejected() {
        let repo = TestRepo::new();

        repo.commit("commit");
        repo.git(&["config", "core.repositoryFormatVersion", "1"]);
        repo.git(&["config", "extensions.refStorage", "files"]);
        let err = from_git(&repo.options()).unwrap_err();
        assert!(
            err.to_string()
                .contains("git config --unset extensions.refStorage")
                && !err.to_string().contains("reftable"),
            "unexpected error: {err}"
        );

        repo.git(&["config", "--unset", "extensions.refStorage"]);
        assert_version(&repo.resolve(), "0.1.0-0.1.main");
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

    #[test]
    fn missing_tag_target_is_an_error() {
        let repo = TestRepo::new();
        let missing = "1111111111111111111111111111111111111111\n";

        repo.commit("commit");
        repo.tag("1.0.0");
        fs::write(repo.path().join(".git/refs/tags/latest"), missing).unwrap();
        assert_version(&repo.resolve(), "1.0.0");

        fs::write(repo.path().join(".git/refs/tags/2.0.0"), missing).unwrap();
        let err = from_git(&repo.options()).unwrap_err();
        assert_eq!(
            err.to_string(),
            "tag \"2.0.0\" points to an object that is missing from the repository, fetch the \
            object or delete the tag with \"git tag -d 2.0.0\""
        );
    }
}
