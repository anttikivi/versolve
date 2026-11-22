use std::{env, fs, path::Path, process::Command, sync::Once};

use git2::{Commit, ConfigLevel, ObjectType, Oid, Repository, Signature, Time};
use semver::Version;
use tempfile::TempDir;

use super::resolve::Resolver;

/// Clears the search paths of the global, XDG, system, and program data
/// configuration files so that libgit2 doesn't read the user's configuration.
pub(crate) fn isolate_libgit2_config() {
    static ISOLATE: Once = Once::new();
    ISOLATE.call_once(|| {
        for level in [
            ConfigLevel::System,
            ConfigLevel::Global,
            ConfigLevel::XDG,
            ConfigLevel::ProgramData,
        ] {
            // SAFETY: Every test creates a `TestRepo` before it uses libgit2,
            // and `Once` blocks the other tests until the search paths are
            // set, so nothing reads them concurrently.
            unsafe { git2::opts::set_search_path(level, "") }
                .expect("failed to clear the libgit2 search path");
        }
    });
}

pub(crate) struct TestRepo {
    dir: TempDir,
}

impl TestRepo {
    pub(crate) fn new() -> Self {
        Self::init(&[])
    }

    pub(crate) fn init(args: &[&str]) -> Self {
        isolate_libgit2_config();
        let repo = Self {
            dir: TempDir::new().expect("failed to create temporary directory"),
        };
        let mut init = vec!["init", "--initial-branch=main"];
        init.extend_from_slice(args);
        repo.git(&init);
        repo
    }

    pub(crate) fn path(&self) -> &Path {
        self.dir.path()
    }

    pub(crate) fn resolver(&self) -> Resolver {
        Resolver::new(self.path())
    }

    /// Runs `git` in the repository without reading the user's configuration.
    pub(crate) fn git(&self, args: &[&str]) {
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

    pub(crate) fn commit(&self, message: &str) {
        self.git(&["commit", "--allow-empty", "-m", message]);
    }

    pub(crate) fn commit_many(&self, n: usize) {
        for i in 0..n {
            self.commit(&format!("commit {i}"));
        }
    }

    pub(crate) fn commit_all(&self, message: &str) {
        self.git(&["add", "--all"]);
        self.commit(message);
    }

    pub(crate) fn commit_fast_forward(&self, n: usize) {
        let repo = Repository::open(self.path()).unwrap();
        let tree_oid = repo.treebuilder(None).unwrap().write().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        let sig = Signature::new("Test", "test@example.com", &Time::new(1735689600, 0)).unwrap();

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

    /// Creates a commit with the given parents and commit time without updating
    /// any references, so that the tests control the order in which the commits
    /// are walked.
    pub(crate) fn commit_at(&self, parents: &[Oid], time: i64) -> Oid {
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

    pub(crate) fn tag(&self, name: &str) {
        self.git(&["tag", "-a", name, "-m", name]);
    }

    pub(crate) fn write_file(&self, name: &str, contents: &str) {
        fs::write(self.path().join(name), contents).unwrap();
    }

    pub(crate) fn make_dirty(&self) {
        self.write_file("dirty.txt", "uncommitted changes\n");
    }

    pub(crate) fn head_id(&self) -> Oid {
        Repository::open(self.path())
            .unwrap()
            .refname_to_id("HEAD")
            .unwrap()
    }

    pub(crate) fn short_head_id(&self) -> String {
        let repo = Repository::open(self.path()).unwrap();
        let head = repo.head().unwrap().peel(ObjectType::Commit).unwrap();
        head.short_id().unwrap().as_str().unwrap().to_owned()
    }

    pub(crate) fn resolve(&self) -> Version {
        self.resolve_with(&self.resolver())
    }

    pub(crate) fn resolve_with(&self, resolver: &Resolver) -> Version {
        resolver
            .resolve()
            .unwrap_or_else(|e| panic!("failed to resolve version: {e}"))
    }
}
