mod git_reader;
mod git_writer;
mod handlers;
mod ports;

pub use git_reader::GitWorkspaceDiffReader;
pub use git_writer::GitWorkspaceGitWriter;
pub use handlers::{
    CommitWorkspaceChangesHandler, PushWorkspaceBranchHandler, StageWorkspaceChangesHandler,
    UnstageWorkspaceChangesHandler,
};
pub use ports::{
    CommitWorkspaceGitRequest, PushWorkspaceGitRequest, ReadWorkspaceDiffRequest,
    ReadWorkspaceDiffScope, ReadWorkspaceDiffSelection, StageWorkspaceGitRequest,
    UnstageWorkspaceGitRequest, WorkspaceDiffContent, WorkspaceDiffReader,
    WorkspaceDiffReaderError, WorkspaceDiffSnapshot, WorkspaceDiffStatus, WorkspaceGitCommit,
    WorkspaceGitPush, WorkspaceGitStage, WorkspaceGitUnstage, WorkspaceGitWriter,
    WorkspaceGitWriterError, WorkspaceStatusFile, WorkspaceStatusSnapshot,
};
