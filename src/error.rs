use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum StacksteadError {
    #[error("no stackstead project found from {0}; run `stackstead init` in a Git repository")]
    ProjectNotFound(PathBuf),
    #[error("stackstead `{0}` was not found; run `stackstead ps`")]
    StacksteadNotFound(String),
    #[error("stackstead name `{name}` is ambiguous; candidates: {candidates}")]
    AmbiguousStackstead { name: String, candidates: String },
    #[error("could not acquire {kind} lock at {path}")]
    LockBusy { kind: &'static str, path: PathBuf },
    #[error("unsafe path: {0}")]
    UnsafePath(String),
    #[error("command failed: {command}\n{stderr}")]
    CommandFailed { command: String, stderr: String },
    #[error("service `{0}` is unknown")]
    UnknownService(String),
}
