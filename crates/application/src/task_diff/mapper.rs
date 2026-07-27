use ora_contracts::{
    TaskDiffComment as ContractComment, TaskDiffCommentAnchor as ContractAnchor,
    TaskDiffCommentKind as ContractKind, TaskDiffSide as ContractSide,
    TaskDiffThreadStatus as ContractStatus,
};
use ora_domain::{TaskDiffComment, TaskDiffCommentKind, TaskDiffSide, TaskDiffThreadStatus};

/// Maps one domain discussion message into the transport-neutral public contract.
pub fn map_task_diff_comment(comment: TaskDiffComment) -> ContractComment {
    ContractComment {
        id: comment.id.to_string(),
        task_id: comment.task_id.to_string(),
        kind: match comment.kind {
            TaskDiffCommentKind::Thread { anchor, status } => ContractKind::Thread {
                anchor: ContractAnchor {
                    diff_id: anchor.diff_id,
                    path: anchor.path,
                    side: map_diff_side(anchor.side),
                    start_line: anchor.start_line,
                    end_line: anchor.end_line,
                    hunk_header: anchor.hunk_header,
                    line_content: anchor.line_content,
                },
                status: map_thread_status(status),
            },
            TaskDiffCommentKind::Reply { parent_comment_id } => ContractKind::Reply {
                parent_comment_id: parent_comment_id.to_string(),
            },
        },
        body: comment.body,
        created_at: comment.audit_fields.created_at,
        updated_at: comment.audit_fields.updated_at,
    }
}

/// Maps the domain diff side into the public contract enum.
fn map_diff_side(side: TaskDiffSide) -> ContractSide {
    match side {
        TaskDiffSide::Old => ContractSide::Old,
        TaskDiffSide::New => ContractSide::New,
    }
}

/// Maps the domain thread status into the public contract enum.
fn map_thread_status(status: TaskDiffThreadStatus) -> ContractStatus {
    match status {
        TaskDiffThreadStatus::Open => ContractStatus::Open,
        TaskDiffThreadStatus::Resolved => ContractStatus::Resolved,
    }
}

/// Maps a public diff side into the domain model.
pub fn map_contract_diff_side(side: ContractSide) -> TaskDiffSide {
    match side {
        ContractSide::Old => TaskDiffSide::Old,
        ContractSide::New => TaskDiffSide::New,
    }
}

/// Maps a public thread status into the domain model.
pub fn map_contract_thread_status(status: ContractStatus) -> TaskDiffThreadStatus {
    match status {
        ContractStatus::Open => TaskDiffThreadStatus::Open,
        ContractStatus::Resolved => TaskDiffThreadStatus::Resolved,
    }
}
