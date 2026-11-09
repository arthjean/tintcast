use std::io;
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("Another apply is running.")]
    Busy,
    #[error(
        "Tintcast can't read its restore records in {}. Restore is unavailable until the file is fixed.",
        path.display()
    )]
    LedgerUnreadable { path: PathBuf, reason: String },
    #[error("Tintcast can't write {}: {source}", path.display())]
    Io { path: PathBuf, source: io::Error },
    #[error("No interrupted run has the id {0}.")]
    UnknownRun(String),
    #[error("{0} is no longer in your library, so Undo can't apply it again.")]
    ThemeMissing(String),
    #[error("Tintcast can't take its apply lock: {0}")]
    Lock(String),
}

impl EngineError {
    pub(crate) fn io(path: impl Into<PathBuf>) -> impl FnOnce(io::Error) -> Self {
        let path = path.into();
        move |source| Self::Io { path, source }
    }
}
