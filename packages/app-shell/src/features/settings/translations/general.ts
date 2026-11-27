// Pure translation data: safe to compose without importing feature implementation.
export const settingsTranslations = {
  "zh-CN": {
    "settings.description":
      "配置 Ora 的界面、角色、技能、模型和 Agent 执行策略。",
    "settings.nav.appearance": "外观",
    "settings.nav.roles": "角色",
    "settings.nav.skills": "技能",
    "settings.nav.proxy": "代理",
    "settings.nav.plugins": "插件",
    "settings.nav.permissions": "权限与执行",
    "settings.nav.privacy": "数据与隐私",
    "settings.nav.developer": "开发者选项",
    "settings.appearance.title": "外观",
    "settings.appearance.theme": "主题",
    "settings.appearance.themeDescription":
      "选择界面的明暗外观，或跟随操作系统设置。",
    "settings.appearance.system": "跟随系统",
    "settings.appearance.light": "浅色",
    "settings.appearance.dark": "深色",
    "settings.appearance.language": "界面语言",
    "settings.appearance.languageDescription":
      "设置菜单、提示和 Agent 工作区使用的语言。",
    "settings.nav.cloud": "Cloud 连接",
    "settings.cloud.title": "Cloud 连接",
    "settings.cloud.description":
      "以企业账号登录团队的 Cloud 服务器。没有连接时，本地的会话、Agent 和工作区照常可用。",
    "settings.cloud.loading": "加载中",
    "settings.cloud.loadError": "无法读取 Cloud 连接。",
    "settings.cloud.server": "服务器地址",
    "settings.cloud.serverHint":
      "填 https 地址，例如 https://cloud.example.com；只有本机地址可以用 http。更换或清空地址会退出当前服务器。",
    "settings.cloud.save": "保存",
    "settings.cloud.saveError": "无法保存服务器地址：{{reason}}",
    "settings.cloud.signIn": "登录",
    "settings.cloud.cancel": "取消登录",
    "settings.cloud.signOut": "退出登录",
    "settings.cloud.startError": "无法打开浏览器登录，请重试。",
    "settings.cloud.revocationUnconfirmed":
      "已在本机退出，但原服务器暂时连不上，没有确认登录已吊销。",
    "settings.cloud.status.notConfigured": "尚未设置服务器。",
    "settings.cloud.status.signedOut": "未登录。",
    "settings.cloud.status.signingIn": "请在浏览器中完成登录并点「允许」。",
    "settings.cloud.status.signedIn": "已登录为 {{name}}。",
    "settings.cloud.status.unreachable":
      "已登录为 {{name}}，但服务器暂时连不上。",
    "settings.cloud.status.needsSignIn": "登录已失效，请重新登录。",
    "settings.cloud.status.storageUnavailable":
      "系统凭据存储不可用（{{reason}}），无法保存登录，Cloud 功能已关闭。",
    "settings.cloud.failure.denied":
      "已在浏览器中取消登录，或授权没有完成，未登录。",
    "settings.cloud.failure.timedOut": "等待浏览器授权超时，未登录。",
    "settings.cloud.failure.rejected": "服务器拒绝了这次登录，请重试。",
    "settings.cloud.failure.unreachable": "登录时连不上服务器，请稍后重试。",
    "settings.proxy.title": "网络代理",
    "settings.proxy.description": "用于桌面更新，以及开启“使用代理”的市场源。",
    "settings.proxy.host": "主机",
    "settings.proxy.hostPlaceholder": "127.0.0.1",
    "settings.proxy.port": "端口",
    "settings.proxy.portPlaceholder": "7890",
    "settings.proxy.username": "用户名",
    "settings.proxy.password": "密码",
    "settings.proxy.optional": "可选",
    "settings.proxy.save": "保存",
    "settings.proxy.saving": "保存中",
    "settings.proxy.saved": "代理配置已保存。",
    "settings.proxy.invalid": "请输入有效的主机和端口。",
    "settings.proxy.mode": "代理模式",
    "settings.proxy.modeSystem": "跟随系统代理（默认）",
    "settings.proxy.systemHint":
      "在 Windows 上使用系统的静态代理。仅开启自动检测时将直连，不运行 WPAD。",
    "settings.proxy.replaceUnreadable": "替换已保存的代理配置",
    "settings.proxy.modeManual": "手动代理",
    "settings.proxy.modeDirect": "直连",
    "settings.proxy.routeSystem": "系统代理",
    "settings.proxy.routeEnvironment": "环境变量代理",
    "settings.proxy.checkRouteDetail":
      "最终请求使用{{route}} · HTTP 状态码 {{status}}。",
    "settings.proxy.clear": "清空",
    "settings.proxy.cleared": "已清空代理配置。",
    "settings.proxy.clearError": "无法清空代理配置。",
    "settings.proxy.check": "验证代理",
    "settings.proxy.checkTitle": "验证代理设置",
    "settings.proxy.checkDescription":
      "验证会保存探测网址，使用当前代理设置跟随跳转，最终收到 HTTP 2xx 时显示成功。代理配置需单独保存。",
    "settings.proxy.checkUrl": "探测网址",
    "settings.proxy.checkUrlRequired": "请输入要探测的网址。",
    "settings.proxy.checkUrlLoadError":
      "无法读取已保存的探测网址，请重试；代理配置仍可编辑。",
    "settings.proxy.checkUrlSaveError":
      "探测网址未保存，未执行探测。请检查网址并重试。",
    "settings.proxy.checkConfirm": "验证",
    "settings.proxy.checking": "验证中",
    "settings.proxy.checkSuccess": "代理验证成功。",
    "settings.proxy.checkSuccessDetail": "HTTP 状态码 {{status}}。",
    "settings.proxy.checkHttpError": "目标网址未返回成功状态。",
    "settings.proxy.checkFailed":
      "未能通过当前代理连接探测网址；这不代表代理配置有误。",
    "settings.proxy.loading": "加载中",
    "settings.proxy.loadError": "无法读取代理配置。",
    "settings.proxy.updateError": "无法保存代理配置。",
    "settings.permissions.title": "权限与执行",
    "settings.permissions.description":
      "定义 Agent 执行命令前需要确认的范围，以及默认可用的运行时能力。",
    "settings.permissions.approval": "审批策略",
    "settings.permissions.approvalDescription":
      "控制 Agent 在执行工具和命令前何时请求确认。",
    "settings.permissions.always": "每次询问",
    "settings.permissions.risky": "仅高风险操作",
    "settings.permissions.trusted": "信任工作区",
    "settings.permissions.terminal": "终端命令",
    "settings.permissions.terminalDescription":
      "允许 Agent 在当前工作区运行终端命令。",
    "settings.permissions.files": "文件写入",
    "settings.permissions.filesDescription":
      "允许 Agent 创建和修改工作区文件。",
    "settings.permissions.network": "网络访问",
    "settings.permissions.networkDescription":
      "允许 Agent 请求外部服务和下载资源。",
    "settings.permissions.timeout": "命令超时",
    "settings.permissions.timeoutDescription":
      "单个命令默认允许的最长运行时间。",
    "settings.permissions.timeoutSeconds": "{{count}} 秒",
    "settings.permissions.timeoutMinutes": "{{count}} 分钟",
    "settings.permissions.noTimeout": "不限制",
    "settings.privacy.title": "数据与隐私",
    "settings.privacy.description": "管理工作树的存储位置。",
    "settings.privacy.worktreeRoot": "工作树存储位置",
    "settings.privacy.worktreeRootDescription":
      "新建工作树会存储在此目录；更改位置不会移动已有工作树。",
    "settings.privacy.worktreeRootLoading": "正在读取…",
    "settings.privacy.changeWorktreeRoot": "更改位置",
    "settings.privacy.retention": "历史记录保留",
    "settings.privacy.retentionDescription":
      "决定本地会话历史默认保留多长时间。",
    "settings.privacy.days30": "30 天",
    "settings.privacy.days90": "90 天",
    "settings.privacy.forever": "永久保留",
    "settings.privacy.diagnostics": "共享诊断数据",
    "settings.privacy.diagnosticsDescription":
      "发送匿名性能和错误信息，帮助改进 Ora。",
    "settings.developer.developerMode": "开发者模式",
    "settings.developer.developerModeDescription":
      "开启后，在此页面显示日志级别等用于诊断和开发的设置。此选项只控制界面可见性，不是安全权限。",
    "settings.developer.developerModeLoading": "正在读取开发者模式…",
    "settings.developer.developerModeSaving": "正在保存开发者模式…",
    "settings.developer.developerModeLoadError": "无法读取开发者模式，请重试。",
    "settings.developer.developerModeUpdateError":
      "开发者模式更新失败，已保留上次生效的设置。",
    "settings.developer.title": "开发者选项",
    "settings.developer.description":
      "开启开发者模式后，可在此页面调整当前 Ora 进程的诊断行为。此模式不会改变访问权限。",
    "settings.developer.logLevel": "日志级别",
    "settings.developer.logLevelDescription":
      "立即调整当前 Ora 进程记录的日志详细程度，并保存为下次启动的偏好。",
    "settings.developer.logLevel.trace": "Trace（最详细）",
    "settings.developer.logLevel.debug": "Debug",
    "settings.developer.logLevel.info": "Info（推荐）",
    "settings.developer.logLevel.warn": "Warn",
    "settings.developer.logLevel.error": "Error（最精简）",
    "settings.developer.logLevelLoading": "正在读取…",
    "settings.developer.logLevelSaving": "正在应用日志级别…",
    "settings.developer.logLevelUnavailable": "暂时不可用",
    "settings.developer.logLevelLoadError":
      "无法读取当前日志级别，请稍后重试。",
    "settings.developer.logLevelUpdateError":
      "日志级别更新失败，已恢复为上次生效的设置。",
    "settings.developer.logLevelVolumeWarning":
      "Trace 和 Debug 会显著增加日志量，建议仅在诊断问题时临时使用。",
    "settings.developer.diagnosticLogs": "诊断日志",
    "settings.developer.diagnosticLogsDescription":
      "将当前 Ora 进程今日的诊断日志保存到你选择的位置，便于排查问题。",
    "settings.developer.downloadLogs": "下载日志",
    "settings.developer.downloadLogsInProgress": "正在下载…",
    "settings.privacy.clearHistory": "清除会话历史",
    "settings.privacy.clearHistoryDescription":
      "清除当前运行期内存中的所有 Agent 对话，不会删除项目和工作树。",
    "settings.privacy.clear": "清除历史",
    "settings.privacy.clearTitle": "清除所有会话历史？",
    "settings.privacy.clearConfirm":
      "当前运行期内存中的所有 Agent 对话都将被清除。",
  },
  "en-US": {
    "settings.description":
      "Configure Ora appearance, roles, skills, models, and agent execution policies.",
    "settings.nav.appearance": "Appearance",
    "settings.nav.roles": "Roles",
    "settings.nav.skills": "Skills",
    "settings.nav.proxy": "Proxy",
    "settings.nav.plugins": "Plugins",
    "settings.nav.permissions": "Permissions",
    "settings.nav.privacy": "Data & privacy",
    "settings.nav.developer": "Developer options",
    "settings.appearance.title": "Appearance",
    "settings.appearance.theme": "Theme",
    "settings.appearance.themeDescription":
      "Choose a light or dark interface, or follow the operating system.",
    "settings.appearance.system": "System",
    "settings.appearance.light": "Light",
    "settings.appearance.dark": "Dark",
    "settings.appearance.language": "Display language",
    "settings.appearance.languageDescription":
      "Set the language used by menus, prompts, and the agent workspace.",
    "settings.nav.cloud": "Cloud connection",
    "settings.cloud.title": "Cloud connection",
    "settings.cloud.description":
      "Sign in to your team's Cloud server with your company account. Without a connection, local sessions, agents and workspaces work as usual.",
    "settings.cloud.loading": "Loading",
    "settings.cloud.loadError": "Could not read the Cloud connection.",
    "settings.cloud.server": "Server address",
    "settings.cloud.serverHint":
      "Use an https address such as https://cloud.example.com; plain http works only for this computer. Changing or clearing the address signs out of the current server.",
    "settings.cloud.save": "Save",
    "settings.cloud.saveError": "Could not save the server address: {{reason}}",
    "settings.cloud.signIn": "Sign in",
    "settings.cloud.cancel": "Cancel sign-in",
    "settings.cloud.signOut": "Sign out",
    "settings.cloud.startError":
      "Could not open the browser to sign in. Try again.",
    "settings.cloud.revocationUnconfirmed":
      "Signed out on this device, but the previous server could not be reached to confirm the sign-in was revoked.",
    "settings.cloud.status.notConfigured": "No server is set.",
    "settings.cloud.status.signedOut": "Not signed in.",
    "settings.cloud.status.signingIn":
      "Finish signing in in your browser and click Allow.",
    "settings.cloud.status.signedIn": "Signed in as {{name}}.",
    "settings.cloud.status.unreachable":
      "Signed in as {{name}}, but the server cannot be reached right now.",
    "settings.cloud.status.needsSignIn":
      "Your sign-in has expired. Sign in again.",
    "settings.cloud.status.storageUnavailable":
      "The system credential store is unavailable ({{reason}}), so the sign-in cannot be kept and Cloud features are off.",
    "settings.cloud.failure.denied":
      "The sign-in was cancelled in the browser or not approved.",
    "settings.cloud.failure.timedOut":
      "Timed out waiting for approval in the browser.",
    "settings.cloud.failure.rejected":
      "The server refused this sign-in. Try again.",
    "settings.cloud.failure.unreachable":
      "The server could not be reached while signing in. Try again later.",
    "settings.proxy.title": "Network proxy",
    "settings.proxy.description":
      "Used for Desktop updates and marketplace sources that enable proxy routing.",
    "settings.proxy.host": "Host",
    "settings.proxy.hostPlaceholder": "127.0.0.1",
    "settings.proxy.port": "Port",
    "settings.proxy.portPlaceholder": "7890",
    "settings.proxy.username": "Username",
    "settings.proxy.password": "Password",
    "settings.proxy.optional": "Optional",
    "settings.proxy.save": "Save",
    "settings.proxy.saving": "Saving",
    "settings.proxy.saved": "Proxy settings saved.",
    "settings.proxy.invalid": "Enter a valid host and port.",
    "settings.proxy.mode": "Proxy mode",
    "settings.proxy.modeSystem": "Follow system proxy (default)",
    "settings.proxy.systemHint":
      "On Windows, Ora uses the static system proxy. With only automatic detection enabled, it connects directly without running WPAD.",
    "settings.proxy.replaceUnreadable": "Replace saved proxy settings",
    "settings.proxy.modeManual": "Manual proxy",
    "settings.proxy.modeDirect": "Direct connection",
    "settings.proxy.routeSystem": "System proxy",
    "settings.proxy.routeEnvironment": "Environment proxy",
    "settings.proxy.checkRouteDetail":
      "Final request via {{route}} · HTTP status {{status}}.",
    "settings.proxy.clear": "Clear",
    "settings.proxy.cleared": "Proxy settings cleared.",
    "settings.proxy.clearError": "Could not clear proxy settings.",
    "settings.proxy.check": "Check connection",
    "settings.proxy.checkTitle": "Check proxy settings",
    "settings.proxy.checkDescription":
      "Check saves the test URL and follows redirects using the current proxy settings. A final HTTP 2xx response counts as success. Save proxy settings separately.",
    "settings.proxy.checkUrl": "URL to check",
    "settings.proxy.checkUrlRequired": "Enter a URL to check.",
    "settings.proxy.checkUrlLoadError":
      "Could not read the saved check URL. Retry to check; proxy settings can still be edited.",
    "settings.proxy.checkUrlSaveError":
      "The URL was not saved and no check was run. Check the URL and retry.",
    "settings.proxy.checkConfirm": "Check",
    "settings.proxy.checking": "Checking",
    "settings.proxy.checkSuccess": "Proxy check succeeded.",
    "settings.proxy.checkSuccessDetail": "HTTP status {{status}}.",
    "settings.proxy.checkHttpError":
      "The test URL did not return a successful status.",
    "settings.proxy.checkFailed":
      "Could not reach this URL through the proxy; the proxy settings may still be valid.",
    "settings.proxy.loading": "Loading",
    "settings.proxy.loadError": "Could not read proxy settings.",
    "settings.proxy.updateError": "Could not save proxy settings.",
    "settings.permissions.title": "Permissions & execution",
    "settings.permissions.description":
      "Define when agents ask before commands and which runtime capabilities are available by default.",
    "settings.permissions.approval": "Approval policy",
    "settings.permissions.approvalDescription":
      "Control when an agent requests confirmation before using tools and commands.",
    "settings.permissions.always": "Ask every time",
    "settings.permissions.risky": "Risky actions only",
    "settings.permissions.trusted": "Trusted workspace",
    "settings.permissions.terminal": "Terminal commands",
    "settings.permissions.terminalDescription":
      "Allow agents to run terminal commands in the current workspace.",
    "settings.permissions.files": "File writes",
    "settings.permissions.filesDescription":
      "Allow agents to create and modify workspace files.",
    "settings.permissions.network": "Network access",
    "settings.permissions.networkDescription":
      "Allow agents to call external services and download resources.",
    "settings.permissions.timeout": "Command timeout",
    "settings.permissions.timeoutDescription":
      "The default maximum runtime for an individual command.",
    "settings.permissions.timeoutSeconds": "{{count}}s",
    "settings.permissions.timeoutMinutes": "{{count}} min",
    "settings.permissions.noTimeout": "No limit",
    "settings.privacy.title": "Data & privacy",
    "settings.privacy.description": "Manage where new worktrees are stored.",
    "settings.privacy.worktreeRoot": "Worktree storage location",
    "settings.privacy.worktreeRootDescription":
      "New worktrees are stored here. Changing it does not move existing worktrees.",
    "settings.privacy.worktreeRootLoading": "Loading…",
    "settings.privacy.changeWorktreeRoot": "Change location",
    "settings.privacy.retention": "History retention",
    "settings.privacy.retentionDescription":
      "Choose how long local conversation history is retained by default.",
    "settings.privacy.days30": "30 days",
    "settings.privacy.days90": "90 days",
    "settings.privacy.forever": "Keep forever",
    "settings.privacy.diagnostics": "Share diagnostics",
    "settings.privacy.diagnosticsDescription":
      "Send anonymous performance and error data to help improve Ora.",
    "settings.developer.developerMode": "Developer mode",
    "settings.developer.developerModeDescription":
      "When enabled, this page shows log-level and other diagnostic or development settings. This controls UI visibility, not security permissions.",
    "settings.developer.developerModeLoading": "Loading developer mode…",
    "settings.developer.developerModeSaving": "Saving developer mode…",
    "settings.developer.developerModeLoadError":
      "Developer mode could not be loaded. Try again.",
    "settings.developer.developerModeUpdateError":
      "The developer-mode update failed. The last effective setting has been retained.",
    "settings.developer.title": "Developer options",
    "settings.developer.description":
      "Enable developer mode to adjust diagnostic behavior for the current Ora process on this page. This mode does not change access permissions.",
    "settings.developer.logLevel": "Log level",
    "settings.developer.logLevelDescription":
      "Change how much the current Ora process logs immediately and save the preference for its next start.",
    "settings.developer.logLevel.trace": "Trace (most detailed)",
    "settings.developer.logLevel.debug": "Debug",
    "settings.developer.logLevel.info": "Info (recommended)",
    "settings.developer.logLevel.warn": "Warn",
    "settings.developer.logLevel.error": "Error (least detailed)",
    "settings.developer.logLevelLoading": "Loading…",
    "settings.developer.logLevelSaving": "Applying log level…",
    "settings.developer.logLevelUnavailable": "Unavailable",
    "settings.developer.logLevelLoadError":
      "The current log level could not be loaded. Try again later.",
    "settings.developer.logLevelUpdateError":
      "The log level update failed. The last effective setting has been restored.",
    "settings.developer.logLevelVolumeWarning":
      "Trace and Debug can produce substantially more logs. Use them temporarily while diagnosing a problem.",
    "settings.developer.diagnosticLogs": "Diagnostic logs",
    "settings.developer.diagnosticLogsDescription":
      "Save today's diagnostic log from the current Ora process to a location you choose for troubleshooting.",
    "settings.developer.downloadLogs": "Download logs",
    "settings.developer.downloadLogsInProgress": "Downloading…",
    "settings.privacy.clearHistory": "Clear conversation history",
    "settings.privacy.clearHistoryDescription":
      "Clear all Agent conversations held in memory for this runtime without removing projects or worktrees.",
    "settings.privacy.clear": "Clear history",
    "settings.privacy.clearTitle": "Clear all conversation history?",
    "settings.privacy.clearConfirm":
      "All Agent conversations held in memory for this runtime will be cleared.",
  },
} as const;
