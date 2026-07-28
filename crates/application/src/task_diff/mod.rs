mod anchor;
mod git_reader;
mod git_writer;
mod handlers;
mod id_generator;
mod mapper;
mod ports;

#[cfg(test)]
mod tests;

pub use git_reader::GitTaskDiffReader;
pub use git_writer::GitTaskGitWriter;
pub use handlers::{
    CommitTaskChangesHandler, CreateTaskDiffCommentHandler, GetTaskDiffHandler,
    ListTaskDiffCommentsHandler, PushTaskBranchHandler, ReplyTaskDiffCommentHandler,
    SetTaskDiffCommentStatusHandler, task_diff_id,
};
pub use id_generator::UuidTaskDiffCommentIdGenerator;
pub use ports::{
    CommitTaskGitRequest, PushTaskGitRequest, ReadTaskDiffRequest, ReadTaskDiffScope,
    TaskDiffCommentIdGenerator, TaskDiffCommentRepository, TaskDiffCommentRepositoryError,
    TaskDiffReader, TaskDiffReaderError, TaskDiffSnapshot, TaskGitCommit, TaskGitPush,
    TaskGitWriter, TaskGitWriterError,
};
