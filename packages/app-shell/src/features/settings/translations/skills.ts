// Pure translation data: safe to compose without importing feature implementation.
export const skillTranslations = {
  "zh-CN": {
    "errors.skill_name_blank": "技能名称不能为空。",
    "errors.skill_name_invalid":
      "技能名称只能使用安全 slug 字符，且不能使用系统保留名称。",
    "errors.skill_name_too_long": "技能名称过长。",
    "errors.skill_description_blank": "技能描述不能为空。",
    "errors.skill_description_too_large": "技能描述超过最大长度。",
    "errors.skill_name_conflict": "已存在同名技能。",
    "errors.skill_not_found": "未找到该技能。",
    "errors.skill_manifest_missing": "缺少技能清单。",
    "errors.skill_manifest_invalid": "技能清单格式无效。",
    "errors.skill_manifest_name_blank": "技能清单名称不能为空。",
    "errors.skill_manifest_description_blank": "技能清单描述不能为空。",
    "errors.skill_manifest_name_invalid": "技能清单名称无效。",
    "errors.skill_folder_conflict": "技能目录 {{name}} 已存在。",
    "errors.skill_manifest_not_found": "未在导入来源中找到 SKILL.md。",
    "errors.skill_manifest_too_large": "SKILL.md 超过最大大小。",
    "errors.too_many_skills": "导入来源包含过多技能。",
    "errors.archive_format_unsupported":
      "仅支持 ZIP、.skill、.tar.gz 和 .tgz 压缩包。",
    "errors.archive_format_mismatch": "压缩包内容与其扩展名不匹配。",
    "errors.archive_corrupt": "压缩包已损坏或无法读取。",
    "errors.archive_encrypted_unsupported": "不支持加密压缩包。",
    "errors.archive_special_entry_unsupported":
      "压缩包包含不安全的特殊文件项。",
    "errors.archive_path_encoding_invalid": "压缩包包含无效文件路径编码。",
    "errors.archive_path_case_conflict":
      "导入来源包含在不同平台上会冲突的路径。",
    "errors.path_segment_too_long": "文件路径片段过长。",
    "errors.path_too_long": "文件路径过长。",
    "errors.path_too_deep": "文件夹嵌套层级过深。",
    "errors.archive_expansion_ratio_exceeded": "压缩包展开后超过安全容量限制。",
    "errors.import_preparation_timeout": "导入预检超时。",
    "errors.import_session_expired": "导入会话已过期。",
    "errors.import_session_cancelled": "导入会话已取消。",
    "errors.import_session_commit_in_progress": "导入正在进行，暂时不能取消。",
    "errors.import_session_already_committed": "该导入会话已使用其他决策提交。",
    "errors.skill_storage_inconsistent": "技能的数据库记录与文件存储不一致。",
    "settings.skills.title": "技能",
    "settings.skills.description":
      "管理可在 Ora 会话或工作流节点中调用的技能。",
    "settings.skills.sectionLabel": "Skills",
    "settings.skills.search": "搜索名称或描述",
    "settings.skills.new": "新建 Skill",
    "settings.skills.loading": "正在加载技能…",
    "settings.skills.empty": "还没有技能，点击「新建 Skill」创建一个。",
    "settings.skills.loadError": "无法加载技能。",
    "settings.skills.saveError": "保存技能失败，请重试。",
    "settings.skills.deleteError": "删除技能失败，请重试。",
    "settings.skills.createTitle": "新建技能",
    "settings.skills.editTitle": "编辑技能",
    "settings.skills.nameLabel": "名称",
    "settings.skills.namePlaceholder": "给这个技能起个名字",
    "settings.skills.descriptionLabel": "描述",
    "settings.skills.descriptionPlaceholder": "一句话说明它的用途",
    "settings.skills.nameInvalid":
      "名称只能使用英文字母、数字、点、下划线和连字符。",
    "settings.skills.descriptionInvalid":
      "描述不能为空，且最多 4096 个 UTF-8 字节。",
    "settings.skills.contentLabel": "内容",
    "settings.skills.contentLoading": "正在加载内容…",
    "settings.skills.contentHint": "支持 Markdown；留空可清除内容。",
    "settings.skills.contentLoadError": "无法加载内容。",
    "settings.skills.import": "导入 Skill",
    "settings.skills.importTitle": "导入技能",
    "settings.skills.importDescription":
      "选择一个技能文件夹或 ZIP、.skill、.tar.gz、.tgz 压缩包。导入前会先检查所有候选项。",
    "settings.skills.importFolder": "选择文件夹",
    "settings.skills.importArchive": "选择压缩包",
    "settings.skills.importing": "导入中…",
    "settings.skills.importDiscovered": "共 {{count}} 个技能",
    "settings.skills.importResultLine": "{{name}}：{{reason}}",
    "settings.skills.importProgress": "导入中… {{processed}} / {{total}}",
    "settings.skills.importFiles": "个文件",
    "settings.skills.importExisting": "现有描述：{{description}}",
    "settings.skills.importSkip": "跳过",
    "settings.skills.importOverwrite": "覆盖",
    "settings.skills.importCommit": "确认导入",
    "settings.skills.importCompleted": "导入已完成。",
    "settings.skills.importAnother": "继续导入",
    "settings.skills.importChooseAnother": "重新选择",
    "settings.skills.deleteTitle": "删除“{{name}}”？",
    "settings.skills.deleteDescription":
      "该技能将从可用命令中移除，此操作无法撤销。",
    "settings.skills.unavailable": "不可用",
    "settings.skills.unavailableTitle": "“{{name}}”的技能包已丢失",
    "settings.skills.unavailableDescription":
      "这个技能还在列表里，但本地文件找不到了。请删除，或重新上传同名技能包。",
    "settings.skills.unavailableRestore": "重新上传",
    "settings.skills.unavailableAction": "处理",
    "settings.skills.unavailableBanner":
      "{{count}} 个技能的本地文件已丢失，请删除或重新上传。",
    "settings.skills.importRestoreHint": "请导入名为“{{name}}”的技能包以恢复。",
    "settings.skills.importRestoreMissing":
      "导入内容里没有名为“{{name}}”的技能。",
    "settings.skills.importCompletedWithFailures": "{{count}} 个技能导入失败。",
    "settings.skills.importStatus.ready": "待导入",
    "settings.skills.importStatus.conflict": "同名冲突",
    "settings.skills.importStatus.invalid": "无效",
    "settings.skills.importStatus.imported": "已导入",
    "settings.skills.importStatus.overwritten": "已覆盖",
    "settings.skills.importStatus.skipped": "已跳过",
    "settings.skills.importStatus.failed": "导入失败",
    "settings.skills.importStatus.staleconflict": "目标已变化",
    "settings.skills.importStatus.prepared": "待确认",
    "settings.skills.importStatus.committing": "导入中",
    "settings.skills.importStatus.completed": "已完成",
    "settings.skills.importStatus.cancelled": "已取消",
    "settings.skills.importStatus.unknown": "未知状态",
    "settings.skills.importReason.yaml_invalid": "SKILL.md 格式无效。",
    "settings.skills.importReason.name_missing": "SKILL.md 缺少名称。",
    "settings.skills.importReason.name_invalid":
      "技能名称只能使用字母、数字、点、下划线和连字符。",
    "settings.skills.importReason.description_missing": "SKILL.md 缺少描述。",
    "settings.skills.importReason.description_too_large":
      "技能描述超过最大长度。",
    "settings.skills.importReason.skill_manifest_too_large":
      "SKILL.md 超过最大大小。",
    "settings.skills.importReason.invalid_candidate": "该技能无效，无法导入。",
    "settings.skills.importReason.decision_missing":
      "同名技能尚未选择跳过或覆盖。",
    "settings.skills.importReason.skill_storage_error": "无法写入技能文件。",
    "settings.skills.importReason.skill_repository_error": "无法保存技能记录。",
    "settings.skills.importReason.skill_name_invalid": "技能名称无效。",
    "settings.skills.importReason.stale_conflict":
      "目标技能已变化，请重新导入。",
    "settings.skills.importReason.unknown": "导入失败。",
  },
  "en-US": {
    "errors.skill_name_blank": "Skill name cannot be blank.",
    "errors.skill_name_invalid":
      "Skill name must use safe slug characters and cannot be a system-reserved name.",
    "errors.skill_name_too_long": "Skill name is too long.",
    "errors.skill_description_blank": "Skill description cannot be blank.",
    "errors.skill_description_too_large":
      "Skill description exceeds the maximum length.",
    "errors.skill_name_conflict": "A skill with this name already exists.",
    "errors.skill_not_found": "The skill was not found.",
    "errors.skill_manifest_missing": "The skill manifest is missing.",
    "errors.skill_manifest_invalid": "The skill manifest is invalid.",
    "errors.skill_manifest_name_blank":
      "The skill manifest name cannot be blank.",
    "errors.skill_manifest_description_blank":
      "The skill manifest description cannot be blank.",
    "errors.skill_manifest_name_invalid": "The skill manifest name is invalid.",
    "errors.skill_folder_conflict": "The skill folder {{name}} already exists.",
    "errors.skill_manifest_not_found":
      "No SKILL.md was found in the import source.",
    "errors.skill_manifest_too_large": "SKILL.md exceeds the maximum size.",
    "errors.too_many_skills": "The import source contains too many skills.",
    "errors.archive_format_unsupported":
      "Only ZIP, .skill, .tar.gz, and .tgz archives are supported.",
    "errors.archive_format_mismatch":
      "The archive contents do not match its extension.",
    "errors.archive_corrupt": "The archive is corrupt or unreadable.",
    "errors.archive_encrypted_unsupported":
      "Encrypted archives are not supported.",
    "errors.archive_special_entry_unsupported":
      "The archive contains an unsafe special entry.",
    "errors.archive_path_encoding_invalid":
      "The archive contains an invalid path encoding.",
    "errors.archive_path_case_conflict":
      "Source paths would conflict on another supported platform.",
    "errors.path_segment_too_long": "A file path segment is too long.",
    "errors.path_too_long": "A file path is too long.",
    "errors.path_too_deep": "The folder nesting is too deep.",
    "errors.archive_expansion_ratio_exceeded":
      "The archive exceeds the safe expansion limit.",
    "errors.import_preparation_timeout": "Import preparation timed out.",
    "errors.import_session_expired": "The import session has expired.",
    "errors.import_session_cancelled": "The import session was cancelled.",
    "errors.import_session_commit_in_progress":
      "The import is running and cannot be cancelled.",
    "errors.import_session_already_committed":
      "This import session was committed with different decisions.",
    "errors.skill_storage_inconsistent":
      "The skill database record and package storage are inconsistent.",
    "settings.skills.title": "Skills",
    "settings.skills.description":
      "Manage the skills callable from Ora sessions and workflow nodes.",
    "settings.skills.sectionLabel": "Skills",
    "settings.skills.search": "Search name or description",
    "settings.skills.new": "New skill",
    "settings.skills.loading": "Loading skills...",
    "settings.skills.empty": "No skills yet. Use “New skill” to create one.",
    "settings.skills.loadError": "Unable to load skills.",
    "settings.skills.saveError": "Unable to save the skill. Try again.",
    "settings.skills.deleteError": "Unable to delete the skill. Try again.",
    "settings.skills.createTitle": "New skill",
    "settings.skills.editTitle": "Edit skill",
    "settings.skills.nameLabel": "Name",
    "settings.skills.namePlaceholder": "Name this skill",
    "settings.skills.descriptionLabel": "Description",
    "settings.skills.descriptionPlaceholder": "One line on what it is for",
    "settings.skills.nameInvalid":
      "Use only letters, numbers, dots, underscores, and hyphens.",
    "settings.skills.descriptionInvalid":
      "Description is required and limited to 4096 UTF-8 bytes.",
    "settings.skills.contentLabel": "Content",
    "settings.skills.contentLoading": "Loading content...",
    "settings.skills.contentHint":
      "Markdown is supported; leave empty to clear the content.",
    "settings.skills.contentLoadError": "Unable to load content.",
    "settings.skills.import": "Import skill",
    "settings.skills.importTitle": "Import skills",
    "settings.skills.importDescription":
      "Select one skill folder or a ZIP, .skill, .tar.gz, or .tgz archive. Ora validates every candidate before importing.",
    "settings.skills.importFolder": "Choose folder",
    "settings.skills.importArchive": "Choose archive",
    "settings.skills.importing": "Importing...",
    "settings.skills.importDiscovered": "{{count}} skill(s)",
    "settings.skills.importResultLine": "{{name}}: {{reason}}",
    "settings.skills.importProgress": "Importing... {{processed}} / {{total}}",
    "settings.skills.importFiles": "files",
    "settings.skills.importExisting": "Existing description: {{description}}",
    "settings.skills.importSkip": "Skip",
    "settings.skills.importOverwrite": "Overwrite",
    "settings.skills.importCommit": "Confirm import",
    "settings.skills.importCompleted": "Import completed.",
    "settings.skills.importAnother": "Import another",
    "settings.skills.importChooseAnother": "Choose another",
    "settings.skills.deleteTitle": "Delete “{{name}}”?",
    "settings.skills.deleteDescription":
      "This skill will be removed from available commands. This cannot be undone.",
    "settings.skills.unavailable": "Unavailable",
    "settings.skills.unavailableTitle":
      "The “{{name}}” skill package is missing",
    "settings.skills.unavailableDescription":
      "The skill is still listed, but its local files are gone. Delete it, or re-upload a package with the same name.",
    "settings.skills.unavailableRestore": "Re-upload",
    "settings.skills.unavailableAction": "Fix",
    "settings.skills.unavailableBanner":
      "{{count}} skill(s) lost local files. Delete or re-upload.",
    "settings.skills.importRestoreHint":
      "Import a package named “{{name}}” to restore it.",
    "settings.skills.importRestoreMissing":
      "This import does not include a skill named “{{name}}”.",
    "settings.skills.importCompletedWithFailures":
      "{{count}} skill(s) failed to import.",
    "settings.skills.importStatus.ready": "Ready",
    "settings.skills.importStatus.conflict": "Name conflict",
    "settings.skills.importStatus.invalid": "Invalid",
    "settings.skills.importStatus.imported": "Imported",
    "settings.skills.importStatus.overwritten": "Overwritten",
    "settings.skills.importStatus.skipped": "Skipped",
    "settings.skills.importStatus.failed": "Import failed",
    "settings.skills.importStatus.staleconflict": "Target changed",
    "settings.skills.importStatus.prepared": "Ready to confirm",
    "settings.skills.importStatus.committing": "Importing",
    "settings.skills.importStatus.completed": "Completed",
    "settings.skills.importStatus.cancelled": "Cancelled",
    "settings.skills.importStatus.unknown": "Unknown status",
    "settings.skills.importReason.yaml_invalid": "SKILL.md is not valid YAML.",
    "settings.skills.importReason.name_missing": "SKILL.md is missing a name.",
    "settings.skills.importReason.name_invalid":
      "Skill names may only use letters, numbers, dots, underscores, and hyphens.",
    "settings.skills.importReason.description_missing":
      "SKILL.md is missing a description.",
    "settings.skills.importReason.description_too_large":
      "The skill description exceeds the maximum length.",
    "settings.skills.importReason.skill_manifest_too_large":
      "SKILL.md exceeds the maximum size.",
    "settings.skills.importReason.invalid_candidate":
      "This skill is invalid and cannot be imported.",
    "settings.skills.importReason.decision_missing":
      "Choose skip or overwrite for the existing skill.",
    "settings.skills.importReason.skill_storage_error":
      "The skill files could not be written.",
    "settings.skills.importReason.skill_repository_error":
      "The skill record could not be saved.",
    "settings.skills.importReason.skill_name_invalid":
      "The skill name is invalid.",
    "settings.skills.importReason.stale_conflict":
      "The target skill changed. Import it again.",
    "settings.skills.importReason.unknown": "Import failed.",
  },
} as const;
