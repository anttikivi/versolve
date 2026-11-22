use std::{error, fmt};

/// An error returned while resolving a version.
///
/// [`Display`](fmt::Display) formats a user-facing message and suggests a fix
/// where possible. Use [`Error::kind`] to handle specific error cases in code.
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

    /// Returns the kind of this error.
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

/// The category of an [`Error`].
///
/// Additional variants may be added in future versions, so matching on this
/// enum requires a wildcard arm.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorKind {
    /// No Git repository was found at the given path or in any parent
    /// directory.
    RepositoryNotFound,
    /// The repository is a shallow clone so its history may be incomplete.
    ShallowClone,
    /// The current branch has no commits.
    NoCommits,
    /// `HEAD` is detached at a commit without a version tag and no branch name
    /// was supplied.
    DetachedHead,
    /// The repository uses an unsupported feature (such as the reftable
    /// reference format or a split index).
    UnsupportedRepository,
    /// A version tag cannot be read (e.g. it points to a missing object).
    InvalidTag,
    /// The patch number of the base tag is already `u64::MAX` and cannot be
    /// incremented.
    VersionOverflow,
    /// An input argument is invalid (e.g. an empty branch name).
    InvalidInput,
    /// Any other error from libgit2.
    Git,
}
