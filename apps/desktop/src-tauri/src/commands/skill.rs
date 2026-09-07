//! Desktop skill operations.

use ora_contracts::*;

backend_command!(
    create_skill,
    CreateSkillRequest,
    CreateSkillResponse,
    skills.create,
    "Creates one skill through the shared Backend."
);
backend_command!(
    get_skill,
    GetSkillRequest,
    GetSkillResponse,
    skills.get,
    "Gets one skill through the shared Backend."
);
backend_command!(
    list_skills,
    ListSkillsRequest,
    ListSkillsResponse,
    skills.list,
    "Lists skills through the shared Backend."
);
backend_command!(
    update_skill,
    UpdateSkillRequest,
    UpdateSkillResponse,
    skills.update,
    "Updates one skill through the shared Backend."
);
backend_command!(
    delete_skill,
    DeleteSkillRequest,
    DeleteSkillResponse,
    skills.delete,
    "Deletes one skill through the shared Backend."
);
backend_command!(
    prepare_skill_import,
    PrepareSkillImportRequest,
    PrepareSkillImportResponse,
    skills.prepare_import,
    "Prepares one skill import source into a previewed session."
);
backend_command!(
    get_skill_import,
    GetSkillImportSessionRequest,
    GetSkillImportSessionResponse,
    skills.get_import,
    "Gets one skill import session with its current progress."
);
backend_command!(
    commit_skill_import,
    CommitSkillImportRequest,
    CommitSkillImportResponse,
    skills.commit_import,
    "Accepts and freezes one skill import commit."
);
backend_command!(
    cancel_skill_import,
    CancelSkillImportRequest,
    CancelSkillImportResponse,
    skills.cancel_import,
    "Cancels one prepared skill import session."
);
