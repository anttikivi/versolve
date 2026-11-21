use std::{error, fmt};

/// An error that occurred while resolving a version.
///
/// The [`Display`](fmt::Display) implementation prints a message that is meant
/// for the user and suggests a fix where possible. Use [`Error::kind`] to
/// handle specific errors in code.
#[derive(Debug)]
pub struct Error {
    kind: ErrorKind,
    message: String,
    source: Option<git2::Error>,
}

impl Error {
    pub(crate) fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            source: None,
        }
    }

    /// Creates an error caused by the libgit2 error `source`. The caller logs
    /// `source`, as the full error is shown only in the debug output.
    pub(crate) fn with_source(
        kind: ErrorKind,
        message: impl Into<String>,
        source: git2::Error,
    ) -> Self {
        Self {
            kind,
            message: message.into(),
            source: Some(source),
        }
    }

    /// Returns the kind of the error.
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl error::Error for Error {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|e| e as &(dyn error::Error + 'static))
    }
}

impl From<git2::Error> for Error {
    fn from(e: git2::Error) -> Self {
        log::debug!("libgit2 error: {e}");
        Self::with_source(ErrorKind::Git, e.message().to_owned(), e)
    }
}

/// The kind of an [`Error`].
///
/// More kinds may be added in the future, so matching on this type requires a
/// wildcard arm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ErrorKind {
    /// No Git repository was found at the given path or in any of its parent
    /// directories.
    RepositoryNotFound,
    /// The repository is a shallow clone, so its history may be incomplete.
    ShallowClone,
    /// The current branch has no commits.
    NoCommits,
    /// `HEAD` is detached at a commit that has no version tag, and no branch
    /// name was given.
    DetachedHead,
    /// The repository uses a feature that is not supported, such as the
    /// reftable reference format or a split index.
    UnsupportedRepository,
    /// A version tag can't be read, for example because it points to an
    /// object that is missing from the repository.
    InvalidTag,
    /// The patch version of the latest tag can't be incremented, as it is
    /// the maximum value.
    VersionOverflow,
    /// The input is invalid, for example the branch name is empty.
    InvalidInput,
    /// Any other error from libgit2.
    Git,
}
