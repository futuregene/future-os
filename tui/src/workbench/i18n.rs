//! The workbench's own copy, in the two languages it ships.
//!
//! What lives here is the *chrome*: tab labels, pickers, key hints, empty
//! states, notices. What does not live here is anything the reader or the agent
//! wrote — a conversation, a session title, a file path — because that is
//! content, and translating content would be rewriting it.
//!
//! Both spellings sit side by side in [`pair`], one key per row. A key added
//! with only one language does not compile, which is the property that keeps a
//! half-translated surface from shipping: the other checks (a rendered frame, a
//! screenshot) only ever show the language they were run in.
//!
//! The language follows the system until the reader picks one (`/lang`, or the
//! `language` key in `~/.future/tui/settings.json`, the same `system` / `zh` /
//! `en` vocabulary the desktop and the phone use).

use std::fmt::Display;

/// The language a frame is drawn in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Language {
    Zh,
    /// The fallback: English is what a terminal with no usable locale gets.
    #[default]
    En,
}

impl Language {
    pub fn code(self) -> &'static str {
        match self {
            Self::Zh => "zh",
            Self::En => "en",
        }
    }
}

/// What the reader chose — `system` defers to the locale, and is what an
/// untouched settings file means.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LanguagePreference {
    #[default]
    System,
    Zh,
    En,
}

impl LanguagePreference {
    /// The picker's rows, in the order they read: the default first.
    pub const ALL: [LanguagePreference; 3] = [Self::System, Self::Zh, Self::En];

    /// The wire spelling (`settings.json`, `/lang`), shared with the other
    /// clients so one vocabulary covers all three surfaces.
    pub fn as_wire(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Zh => "zh",
            Self::En => "en",
        }
    }

    /// The spelling a settings file carries; anything else is `system`, which is
    /// also what an absent key means.
    pub fn from_wire(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "zh" | "cn" => Self::Zh,
            "en" => Self::En,
            _ => Self::System,
        }
    }

    /// The language this preference stands for right now.
    pub fn resolve(self) -> Language {
        match self {
            Self::System => system_language(),
            Self::Zh => Language::Zh,
            Self::En => Language::En,
        }
    }

    /// The row's own label in `lang`.
    pub fn label(self, lang: Language) -> &'static str {
        match self {
            Self::System => t(lang, Key::LangSystem),
            // The two languages name themselves, in themselves: `中文` is not
            // "Chinese" to a reader who is looking for it while the surface is
            // in a language they do not read.
            Self::Zh => "中文",
            Self::En => "English",
        }
    }
}

/// The reader's locale: `LC_ALL`, then `LC_MESSAGES`, then `LANG`, and the
/// Windows user UI language when none of them is set. A tag that names Chinese
/// (Simplified or Traditional — the bundles ship one Chinese) picks Chinese;
/// anything else, including `C`/`POSIX` and a missing locale, is English.
pub fn system_language() -> Language {
    env_language().unwrap_or_else(platform_language)
}

fn env_language() -> Option<Language> {
    for name in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        let Ok(value) = std::env::var(name) else {
            continue;
        };
        let value = value.trim();
        // An empty or `C` locale is not a choice the reader made; the next
        // variable, or the platform, still might be.
        if value.is_empty()
            || value.eq_ignore_ascii_case("c")
            || value.eq_ignore_ascii_case("posix")
        {
            continue;
        }
        return Some(if is_chinese_tag(value) {
            Language::Zh
        } else {
            Language::En
        });
    }
    None
}

/// `zh`, `zh_CN.UTF-8`, `zh-Hans`, `Chinese (Simplified)_China.936` — the
/// language subtag before any region, charset or modifier.
fn is_chinese_tag(tag: &str) -> bool {
    let tag = tag.to_ascii_lowercase();
    let tag = tag.split(['.', '@']).next().unwrap_or("");
    let primary = tag.split(['_', '-']).next().unwrap_or("");
    matches!(
        primary,
        "zh" | "chinese" | "chinese-simplified" | "chinese-traditional"
    )
}

/// The OS's own answer when no locale variable is set: on Windows the user's UI
/// language, and elsewhere English (a POSIX terminal without `LANG` is C, and
/// C is English).
fn platform_language() -> Language {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Globalization::GetUserDefaultUILanguage;
        // A LANGID's language is its low ten bits; 0x04 is Chinese.
        let langid = unsafe { GetUserDefaultUILanguage() };
        if langid & 0x3ff == 0x04 {
            return Language::Zh;
        }
    }
    Language::En
}

/// One piece of chrome. Every variant is a row in [`pair`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    // ── the sidebar's tabs ────────────────────────────────────────────
    TabTodo,
    TabJobs,
    TabSessions,
    TabUsage,
    TabSkills,
    // ── the sidebar's panels ──────────────────────────────────────────
    SummaryTodo,
    SummaryJobs,
    SummarySessions,
    SummarySkills,
    UsageContext,
    UsageCost,
    UsageCache,
    UsageOutput,
    PanelEmptyTodo,
    PanelEmptyJobs,
    PanelEmptySessions,
    PanelEmptySkills,
    // ── the picker overlay ────────────────────────────────────────────
    PickerTitleModel,
    PickerTitleTheme,
    PickerTitleSessions,
    PickerTitleLanguage,
    PickerHintModel,
    PickerHintTheme,
    PickerHintSessions,
    PickerHintLanguage,
    PickerEmptyModel,
    PickerEmptyTheme,
    PickerEmptySessions,
    PickerEmptyLanguage,
    PickerFilter,
    PickerUngrouped,
    PickerCanvasLight,
    PickerCanvasDark,
    PickerEffort,
    LangSystem,
    // ── the conversation surface ──────────────────────────────────────
    InputHintChat,
    ChatOlder,
    ChatOlderHint,
    ActivityThinking,
    ActivityTool,
    ActivityStreaming,
    ActivityTools,
    HintBackToChat,
    HintSidebar,
    HintFocus,
    HintCommands,
    HintHelp,
    HintNewChat,
    HintSettings,
    ShortcutRefresh,
    ShortcutStop,
    // ── the overlays ──────────────────────────────────────────────────
    HelpTitle,
    HelpFooter,
    PalettePanelTitle,
    PaletteTitle,
    PaletteFilter,
    PaletteEmpty,
    // ── the launchpad ─────────────────────────────────────────────────
    ChipModel,
    ChipEffort,
    ChipMode,
    ChipAccess,
    ShortcutCommands,
    ShortcutHelp,
    ShortcutNewChat,
    ShortcutSidebar,
    LaunchpadTagline,
    LaunchpadNewChat,
    LaunchpadNewChatIn,
    LaunchpadRecent,
    // ── the palette's verbs ───────────────────────────────────────────
    CmdNew,
    CmdDiff,
    CmdApproval,
    CmdHistory,
    CmdTree,
    CmdFork,
    CmdWorktree,
    CmdSandbox,
    CmdPermission,
    CmdCompact,
    CmdTools,
    CmdProviders,
    DiffTitle,
    DiffHint,
    DiffClean,
    DiffNoWorkspace,
    ApprovalTitle,
    ApprovalHint,
    ApprovalNone,
    ApprovalAction,
    ApprovalRequested,
    ApprovalBoundary,
    ApprovalRules,
    ApprovalNoRule,
    ApprovalRuleFailed,
    ApprovalRuleAdded,
    HistoryTitle,
    HistoryHint,
    HistoryEmpty,
    TreeTitle,
    TreeHint,
    ForkTitle,
    ForkHint,
    ForkEmpty,
    WorktreeTitle,
    WorktreeHint,
    WorktreeEmpty,
    WorktreeCreating,
    CompactDone,
    ToolsDone,
    ToolsUsage,
    WorkflowFailed,
    ForkEmptyId,
    CmdQueue,
    CmdCopy,
    CmdMouse,
    QueueTitle,
    QueueHint,
    QueueEmpty,
    QueueCancelled,
    QueueRecalled,
    QueueCancelFailed,
    QueueRecallFailed,
    CopyFailed,
    CopyDone,
    CopyEmpty,
    MouseOn,
    MouseOff,
    NotificationTitle,
    NotificationBody,
    LinearYou,
    LinearApproval,
    LinearTool,
    LinearError,
    LinearUnknownError,
    ChatNewOutput,
    CmdSessions,
    CmdModel,
    CmdLanguage,
    CmdReplyDetail,
    CmdWorkspace,
    CmdTheme,
    CmdUsage,
    CmdSkills,
    CmdExport,
    CmdTranscript,
    CmdHelp,
    CmdQuit,
    CmdPreferences,
    PrefDensity,
    PrefPosition,
    PrefWidth,
    PrefMouse,
    PrefNotifications,
    PrefNotificationBell,
    PrefMotion,
    PrefLinear,
    PrefEnabled,
    PrefDisabled,
    PrefShortcuts,
    PrefReset,
    PrefFocused,
    PrefComfortable,
    PrefLeft,
    PrefRight,
    PreferencesTitle,
    PreferencesHint,
    KeybindingsTitle,
    KeybindingsHint,
    NoticePreferencesSaved,
    NoticePreferenceInvalid,
    // ── the help overlay's key table ──────────────────────────────────
    HelpKeyNew,
    HelpKeySidebar,
    HelpKeyFocus,
    HelpKeyScroll,
    HelpKeyPage,
    HelpKeyPalette,
    HelpKeyHelp,
    HelpKeyReplyDetail,
    HelpKeyReplyDetailSet,
    HelpKeyClick,
    HelpKeyDrag,
    HelpKeyQuit,
    // ── the notice row ────────────────────────────────────────────────
    NoticeWorkspace,
    NoticeTranscript,
    Unspecified,
    // ── the adapter (`live`, `local`, `widgets`) ──────────────────────
    InputHintLaunchpad,
    ApprovalWaiting,
    ExportDone,
    ExportDonePath,
    JobPaused,
    JobScheduled,
    JobManual,
    CacheLabel,
}

/// The two spellings of one key: `(中文, English)`.
///
/// `{}` marks a value the caller substitutes (see [`render`]); the number and
/// order of them is the same in both languages, and keeping them that way is a
/// reviewer's job in the one place a reviewer can see both at once.
fn pair(key: Key) -> (&'static str, &'static str) {
    use Key::*;
    match key {
        TabTodo => ("待办", "To-do"),
        TabJobs => ("任务", "Jobs"),
        TabSessions => ("会话", "Sessions"),
        TabUsage => ("用量", "Usage"),
        TabSkills => ("技能", "Skills"),

        SummaryTodo => ("{} 待办 / {} 合计", "{} open / {} total"),
        SummaryJobs => ("{} 个任务", "{} jobs"),
        SummarySessions => ("{} 个会话", "{} sessions"),
        SummarySkills => ("{} 个技能", "{} skills"),
        UsageContext => ("上下文", "Context"),
        UsageCost => ("成本", "Cost"),
        UsageCache => ("缓存命中", "Cache hit"),
        UsageOutput => ("输出", "Output"),
        PanelEmptyTodo => ("没有待办", "no todos"),
        PanelEmptyJobs => ("没有定时任务", "no scheduled jobs"),
        PanelEmptySessions => ("没有会话", "no sessions"),
        PanelEmptySkills => ("没有技能", "no skills"),

        PickerTitleModel => ("模型", "Models"),
        PickerTitleTheme => ("主题", "Themes"),
        PickerTitleSessions => ("会话", "Sessions"),
        PickerTitleLanguage => ("语言", "Language"),
        PickerHintModel => (
            "↑↓ 选择 · ←→ 推理强度 · ⏎ 应用模型与强度 · esc 取消",
            "↑↓ choose · ←→ effort · ⏎ apply model and effort · esc cancels",
        ),
        PickerHintTheme => (
            "↑↓ 移动即预览 · ⏎ 保存 · esc 恢复进入前的主题",
            "↑↓ move to preview · ⏎ saves · esc restores the theme you had",
        ),
        PickerHintSessions => (
            "↑↓ 选择 · ⏎ 切换会话 · esc 取消",
            "↑↓ choose · ⏎ switches session · esc cancels",
        ),
        PickerHintLanguage => (
            "↑↓ 选择 · ⏎ 应用 · esc 取消",
            "↑↓ choose · ⏎ applies · esc cancels",
        ),
        PickerEmptyModel => ("没有匹配的模型", "No matching model"),
        PickerEmptyTheme => ("没有匹配的主题", "No matching theme"),
        PickerEmptySessions => ("没有匹配的会话", "No matching session"),
        PickerEmptyLanguage => ("没有匹配的语言", "No matching language"),
        PickerFilter => ("输入以过滤", "Type to filter"),
        PickerUngrouped => ("未分组", "Ungrouped"),
        PickerCanvasLight => ("浅色画布", "light canvas"),
        PickerCanvasDark => ("深色画布", "dark canvas"),
        PickerEffort => ("推理强度", "Effort"),
        LangSystem => ("跟随系统", "Follow system"),

        InputHintChat => (
            "输入 · enter 发送 · alt+enter 换行 · / 命令",
            "Message · enter sends · alt+enter newline · / commands",
        ),
        ChatOlder => ("↑ 更早", "↑ older"),
        ChatOlderHint => ("↑ 更早 · page up", "↑ older · page up"),
        ActivityThinking => ("思考", "Thinking"),
        ActivityTool => ("工具", "Tool"),
        ActivityStreaming => ("生成", "Writing"),
        ActivityTools => (" · {}s · {} 工具", " · {}s · {} tools"),
        HintBackToChat => ("回到聊天", "Back to chat"),
        HintSidebar => ("侧栏", "Sidebar"),
        HintFocus => ("焦点", "Focus"),
        HintCommands => ("命令", "Commands"),
        HintHelp => ("帮助", "Help"),
        HintNewChat => ("新会话", "New Chat"),
        HintSettings => ("偏好设置", "Preferences"),
        ShortcutRefresh => ("刷新会话和用量", "Refresh sessions and usage"),
        ShortcutStop => ("停止当前生成", "Stop the current run"),

        HelpTitle => ("帮助", "Help"),
        HelpFooter => (
            " 按键与快捷键 · ↑↓ 滚动 · esc 关闭",
            " Keys and shortcuts · ↑↓ scrolls · esc closes",
        ),
        PalettePanelTitle => (" 命令面板（/）", " Command palette (/)"),
        PaletteTitle => ("命令", "Commands"),
        PaletteFilter => ("输入以过滤命令", "Type to filter commands"),
        PaletteEmpty => (" 没有匹配的命令", " No matching command"),

        ChipModel => ("模型", "Model"),
        ChipEffort => ("强度", "Effort"),
        ChipMode => ("模式", "Mode"),
        ChipAccess => ("权限", "Access"),
        ShortcutCommands => ("命令", "Commands"),
        ShortcutHelp => ("帮助", "Help"),
        ShortcutNewChat => ("新会话", "New Chat"),
        ShortcutSidebar => ("侧栏", "Sidebar"),
        LaunchpadTagline => (
            "一个 AI agent，终端、桌面、手机都在",
            "One AI agent — terminal, desktop, phone",
        ),
        LaunchpadNewChat => ("新会话", "New Chat"),
        LaunchpadNewChatIn => ("新会话 · {}", "New Chat · {}"),
        LaunchpadRecent => ("最近会话", "Recent sessions"),

        CmdNew => ("新建会话", "New chat"),
        CmdDiff => ("审查工作区改动", "Review working-tree changes"),
        CmdApproval => ("查看完整审批详情", "Inspect the pending approval"),
        CmdHistory => ("搜索历史会话", "Search session history"),
        CmdTree => ("浏览会话父子树", "Browse the session tree"),
        CmdFork => ("从用户消息节点创建分支会话", "Fork from a user-message point"),
        CmdWorktree => ("列出、进入或新建 git worktree", "List, enter or create a git worktree"),
        CmdSandbox => ("切换 sandbox 审批模式", "Cycle sandbox approval mode"),
        CmdPermission => ("设置文件权限级别", "Set the file permission level"),
        CmdCompact => ("压缩上下文，可附自定义说明", "Compact context, optionally with instructions"),
        CmdTools => ("设置工具列表，off 禁用全部", "Set the tool list; off disables all"),
        CmdProviders => ("列出模型提供商", "List model providers"),
        DiffTitle => ("Diff 审查 · 工作区未提交改动", "Diff review · uncommitted workspace changes"),
        DiffHint => ("←→ 选择文件 · ↑↓ 滚动 · r 刷新 · esc 关闭", "←→ choose a file · ↑↓ scroll · r refreshes · esc closes"),
        DiffClean => ("工作区没有未提交改动", "No uncommitted changes in this workspace"),
        DiffNoWorkspace => ("当前会话没有工作区", "The current session has no workspace"),
        ApprovalTitle => ("审批详情", "Approval details"),
        ApprovalHint => ("y 允许一次 · n 拒绝 · a 添加本次运行工作区规则并允许 · ↑↓ 滚动 · esc 返回", "y allow once · n deny · a add this-run workspace rule and allow · ↑↓ scroll · esc back"),
        ApprovalNone => ("当前没有等待审批的请求", "No approval is waiting"),
        ApprovalAction => ("动作与路径", "Action and paths"),
        ApprovalRequested => ("模型原始请求", "Requested action"),
        ApprovalBoundary => ("sandbox 边界", "Sandbox boundary"),
        ApprovalRules => ("可添加的本次运行规则（不会永久保存）", "Available this-run rules (not persisted)"),
        ApprovalNoRule => ("此审批没有可用的工作区允许规则", "This approval has no workspace allow rule"),
        ApprovalRuleFailed => ("无法添加允许规则，审批仍未决定：{}", "Could not add the allow rule; approval is still pending: {}"),
        ApprovalRuleAdded => ("已添加本次运行规则并批准请求", "This-run rule added and request approved"),
        HistoryTitle => ("历史搜索", "History search"),
        HistoryHint => ("输入筛选标题、首条消息、目录或 id · ↑↓ 选择 · ⏎ 打开 · esc 关闭", "Filter title, first message, directory or id · ↑↓ choose · enter opens · esc closes"),
        HistoryEmpty => ("没有匹配的会话", "No matching sessions"),
        TreeTitle => ("会话树", "Session tree"),
        TreeHint => ("↑↓ 选择 · ⏎ 打开 · esc 关闭", "↑↓ choose · enter opens · esc closes"),
        ForkTitle => ("选择 fork 节点", "Choose a fork point"),
        ForkHint => ("↑↓ 选择真实用户消息 · ⏎ 创建并打开分支会话 · esc 取消", "↑↓ choose a real user message · enter creates and opens a fork · esc cancels"),
        ForkEmpty => ("没有可 fork 的用户消息", "No user-message fork points"),
        WorktreeTitle => ("git worktree", "Git worktrees"),
        WorktreeHint => ("↑↓ 选择 · ⏎ 设为会话工作区 · r 刷新 · /worktree new <branch> 新建 · esc 关闭", "↑↓ choose · enter sets session cwd · r refreshes · /worktree new <branch> creates · esc closes"),
        WorktreeEmpty => ("没有可显示的 worktree", "No worktrees to show"),
        WorktreeCreating => ("正在创建 worktree…", "Creating worktree…"),
        CompactDone => ("上下文已压缩", "Context compacted"),
        ToolsDone => ("工具设置已更新", "Tool settings updated"),
        ToolsUsage => ("用法：/tools read,shell · /tools off", "Usage: /tools read,shell · /tools off"),
        WorkflowFailed => ("操作失败：{}", "Operation failed: {}"),
        ForkEmptyId => ("fork 未返回会话 id", "Fork returned no session id"),
        CmdQueue => ("查看、取消或取回排队消息", "List, cancel or recall queued prompts"),
        CmdCopy => ("复制最后一条回答", "Copy the last assistant answer"),
        CmdMouse => ("切换鼠标捕获，便于选择复制", "Toggle mouse capture for native selection"),
        QueueTitle => ("排队消息", "Queued prompts"),
        QueueHint => ("↑↓ 选择 · x 取消 · r/⏎ 取消并取回草稿 · esc 关闭 · ctrl+x 停止当前运行", "↑↓ choose · x cancels · r/enter recalls to draft · esc closes · ctrl+x stops the active run"),
        QueueEmpty => ("没有排队消息", "No queued prompts"),
        QueueCancelled => ("已取消排队消息", "Queued prompt cancelled"),
        QueueRecalled => ("已取消排队消息并取回草稿", "Queued prompt cancelled and recalled to draft"),
        QueueCancelFailed => ("无法取消排队消息：{}", "Could not cancel queued run: {}"),
        QueueRecallFailed => ("无法取回排队消息：{}", "Could not recall queued run: {}"),
        CopyFailed => ("无法复制：{}", "Could not copy: {}"),
        CopyDone => ("已复制最后一条回答", "Last assistant answer copied"),
        CopyEmpty => ("还没有可复制的回答", "No assistant answer to copy yet"),
        MouseOn => ("鼠标捕获已开启", "Mouse capture enabled"),
        MouseOff => ("鼠标捕获已关闭，可使用终端原生选择", "Mouse capture disabled; native terminal selection is available"),
        NotificationTitle => ("FutureOS：运行完成", "FutureOS: run complete"),
        NotificationBody => ("本轮已成功完成。", "This turn finished successfully."),
        LinearYou => ("你：{}", "You: {}"),
        LinearApproval => ("需要审批：{}", "Approval needed: {}"),
        LinearTool => ("工具执行", "tool execution"),
        LinearError => ("错误：{}", "Error: {}"),
        LinearUnknownError => ("未知错误", "unknown error"),
        ChatNewOutput => ("↓ 新输出 · esc 回到最新", "↓ new output · esc returns to latest"),
        CmdSessions => ("浏览并切换会话", "Browse and switch sessions"),
        CmdModel => ("选择模型", "Choose a model"),
        CmdLanguage => ("界面语言", "Interface language"),
        CmdReplyDetail => ("最终输出 / 完整过程", "Final answer / full process"),
        CmdWorkspace => ("workspace 目录", "Workspace directory"),
        CmdTheme => ("切换主题", "Switch theme"),
        CmdUsage => ("用量与成本", "Usage and cost"),
        CmdSkills => ("技能列表", "Installed skills"),
        CmdExport => ("导出会话", "Export the session"),
        CmdTranscript => ("完整记录", "The whole record"),
        CmdHelp => ("帮助与快捷键", "Help and shortcuts"),
        CmdQuit => ("退出 workbench", "Leave the workbench"),
        CmdPreferences => ("布局、通知、可访问性和快捷键", "Layout, notifications, accessibility and shortcuts"),
        PrefDensity => ("界面密度", "Density"),
        PrefPosition => ("侧栏位置", "Sidebar position"),
        PrefWidth => ("侧栏宽度", "Sidebar width"),
        PrefMouse => ("启动与运行时使用鼠标", "Mouse capture"),
        PrefNotifications => ("成功完成时通知（终端支持时）", "Successful-run notifications (terminal permitting)"),
        PrefNotificationBell => ("通知包含提示音", "Notification bell"),
        PrefMotion => ("减少动态", "Reduce motion"),
        PrefLinear => ("线性阅读（单栏且静态）", "Linear reading (single column, static)"),
        PrefEnabled => ("开启", "On"),
        PrefDisabled => ("关闭", "Off"),
        PrefShortcuts => ("自定义快捷键", "Customize shortcuts"),
        PrefReset => ("恢复默认偏好", "Restore defaults"),
        PrefFocused => ("专注：更多对话空间", "Focused: more room for conversation"),
        PrefComfortable => ("舒适：完整品牌头部", "Comfortable: full identity header"),
        PrefLeft => ("左侧", "Left"),
        PrefRight => ("右侧", "Right"),
        PreferencesTitle => ("偏好设置", "Preferences"),
        PreferencesHint => ("↑↓ 选择 · ←→ 调整 · ⏎ 打开/恢复 · esc 关闭", "↑↓ choose · ←→ adjust · ⏎ open/reset · esc closes"),
        KeybindingsTitle => ("快捷键", "Shortcuts"),
        KeybindingsHint => ("↑↓ 选择 · ⏎ 后按新快捷键 · r 恢复全部 · esc 关闭", "↑↓ choose · ⏎ then press a new chord · r resets all · esc closes"),
        NoticePreferencesSaved => ("偏好已保存", "Preferences saved"),
        NoticePreferenceInvalid => ("不支持的按键，请使用 Ctrl/Alt+字母、F1–F24 或 Tab", "Unsupported key — use Ctrl/Alt + a letter, F1–F24 or Tab"),

        HelpKeyNew => ("新会话（任意位置）", "New chat (from anywhere)"),
        HelpKeySidebar => ("侧栏：打开 / 聚焦 / 收起", "Sidebar: show / focus / hide"),
        HelpKeyFocus => ("在输入框与侧栏之间移动", "Move between the input and the sidebar"),
        HelpKeyScroll => ("滚动聊天，或侧栏列表", "Scroll the chat, or the sidebar's list"),
        HelpKeyPage => ("整页滚动聊天", "Scroll the chat a page at a time"),
        HelpKeyPalette => ("命令面板", "Command palette"),
        HelpKeyHelp => ("这份帮助", "This help"),
        HelpKeyReplyDetail => ("最终输出 / 完整过程", "Final answer / full process"),
        HelpKeyReplyDetailSet => ("同上，直接设置", "The same, set directly"),
        HelpKeyClick => ("切页签、点行、点参数", "Tabs, rows and chips"),
        HelpKeyDrag => ("侧栏分隔条调宽", "Widen the sidebar at its divider"),
        HelpKeyQuit => ("退出", "Quit"),

        NoticeWorkspace => (
            "工作区 {} · 随会话固定 · 换目录：在目标目录启动 future tui，/sessions 换已有会话",
            "Workspace {} · fixed for this session · to change directory, start future tui there; /sessions switches to an existing session",
        ),
        NoticeTranscript => (
            "完整记录：/export 导出整份会话（HTML） · /reply-detail 在聊天里看完整过程",
            "The whole record: /export writes the session as HTML · /reply-detail shows the full process in the chat",
        ),
        Unspecified => ("未指定", "unset"),

        InputHintLaunchpad => (
            "描述你要做的事 · @ 文件 · # 会话 · / 命令",
            "Describe the task · @ files · # sessions · / commands",
        ),
        ApprovalWaiting => (
            "等待批准：{}{} — y 允许 · n 拒绝",
            "Waiting for approval: {}{} — y allows · n denies",
        ),
        ExportDone => ("会话已导出", "Session exported"),
        ExportDonePath => ("会话已导出 → {}", "Session exported → {}"),
        JobPaused => ("暂停", "paused"),
        JobScheduled => ("已排期", "scheduled"),
        JobManual => ("手动", "manual"),
        CacheLabel => ("缓存 {}%", "cache {}%"),
    }
}

/// One piece of chrome in `lang`.
pub fn t(lang: Language, key: Key) -> &'static str {
    let (zh, en) = pair(key);
    match lang {
        Language::Zh => zh,
        Language::En => en,
    }
}

/// One piece of chrome with its `{}` values filled in, left to right.
///
/// A template with more holes than arguments leaves the extra `{}` standing
/// rather than panicking: a frame is not worth taking the surface down for, and
/// the stray braces are visible in a test's output.
pub fn render(lang: Language, key: Key, values: &[&dyn Display]) -> String {
    let mut out = String::new();
    let mut parts = t(lang, key).split("{}");
    out.push_str(parts.next().unwrap_or(""));
    let mut used = 0;
    for part in parts {
        match values.get(used) {
            Some(value) => {
                use std::fmt::Write;
                let _ = write!(out, "{value}");
                used += 1;
            }
            None => out.push_str("{}"),
        }
        out.push_str(part);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_languages_are_actually_different() {
        // A pair that was copied rather than translated is invisible in a
        // frame: both look "translated". Latin script in the Chinese column is
        // the signal (a proper noun — `中文`, `English`, `workspace` — is
        // allowed to be identical, so only pure-CJK-vs-identical is checked).
        let identical: Vec<&str> = ALL_KEYS
            .iter()
            .filter(|key| {
                let (zh, en) = pair(**key);
                zh == en && zh.chars().any(|ch| ch.is_ascii_alphabetic())
            })
            .map(|key| t(Language::Zh, *key))
            .collect();
        assert!(identical.is_empty(), "not translated: {identical:?}");
    }

    #[test]
    fn both_languages_have_the_same_number_of_values() {
        for key in ALL_KEYS {
            let (zh, en) = pair(key);
            assert_eq!(
                zh.matches("{}").count(),
                en.matches("{}").count(),
                "{} has a different hole count per language",
                t(Language::En, key)
            );
        }
    }

    #[test]
    fn every_key_renders_in_both_languages() {
        for key in ALL_KEYS {
            for lang in [Language::Zh, Language::En] {
                assert!(!t(lang, key).is_empty());
            }
        }
    }

    #[test]
    fn render_fills_the_holes_in_order() {
        assert_eq!(
            render(Language::En, Key::SummaryTodo, &[&3, &7]),
            "3 open / 7 total"
        );
        assert_eq!(
            render(Language::Zh, Key::SummaryTodo, &[&3, &7]),
            "3 待办 / 7 合计"
        );
        assert_eq!(
            render(Language::En, Key::ExportDonePath, &[&"/tmp/a.html"]),
            "Session exported → /tmp/a.html"
        );
        // No holes, no values: the template comes back as-is.
        assert_eq!(render(Language::Zh, Key::TabTodo, &[]), "待办");
    }

    #[test]
    fn a_missing_value_leaves_the_hole_visible() {
        assert_eq!(render(Language::En, Key::SummaryJobs, &[]), "{} jobs");
    }

    #[test]
    fn a_chinese_system_picks_chinese() {
        for tag in ["zh_CN.UTF-8", "zh", "zh-Hans", "ZH_TW", "zh_SG.utf8"] {
            assert_eq!(
                env_language_of(tag),
                Some(Language::Zh),
                "{tag} should read as Chinese"
            );
        }
    }

    #[test]
    fn anything_else_is_english() {
        for tag in ["en_US.UTF-8", "de_DE.UTF-8", "en"] {
            assert_eq!(env_language_of(tag), Some(Language::En), "{tag}");
        }
        // `C` and `POSIX` are "no locale chosen", not "English chosen": they
        // fall through to the platform, which is what a bare terminal means.
        for tag in ["C", "POSIX", "c", "  "] {
            assert_eq!(env_language_of(tag), None, "{tag}");
        }
    }

    #[test]
    fn the_preference_wire_spelling_matches_the_other_clients() {
        for preference in LanguagePreference::ALL {
            assert_eq!(
                LanguagePreference::from_wire(preference.as_wire()),
                preference
            );
        }
        // Absent or unknown is `system`, never a silent English.
        assert_eq!(
            LanguagePreference::from_wire(""),
            LanguagePreference::System
        );
        assert_eq!(
            LanguagePreference::from_wire("fr"),
            LanguagePreference::System
        );
        // An explicit choice does not consult the environment at all.
        assert_eq!(LanguagePreference::Zh.resolve(), Language::Zh);
        assert_eq!(LanguagePreference::En.resolve(), Language::En);
    }

    #[test]
    fn the_system_row_is_localized_and_the_languages_name_themselves() {
        assert_eq!(
            LanguagePreference::System.label(Language::En),
            "Follow system"
        );
        assert_eq!(LanguagePreference::System.label(Language::Zh), "跟随系统");
        for lang in [Language::Zh, Language::En] {
            assert_eq!(LanguagePreference::Zh.label(lang), "中文");
            assert_eq!(LanguagePreference::En.label(lang), "English");
        }
    }

    /// `env_language` with one variable set, so the test does not depend on the
    /// locale of the machine running it.
    fn env_language_of(tag: &str) -> Option<Language> {
        // `is_chinese_tag` + the empty/`C` rule, without touching the process
        // environment (a test that set `LANG` would race every other test in
        // this binary).
        let value = tag.trim();
        if value.is_empty()
            || value.eq_ignore_ascii_case("c")
            || value.eq_ignore_ascii_case("posix")
        {
            return None;
        }
        Some(if is_chinese_tag(value) {
            Language::Zh
        } else {
            Language::En
        })
    }

    /// Every key, in the order [`pair`] lists them. Kept beside the match so a
    /// new key is an obvious addition in two places rather than a silent gap in
    /// the tests above.
    const ALL_KEYS: [Key; 193] = {
        use Key::*;
        [
            TabTodo,
            TabJobs,
            TabSessions,
            TabUsage,
            TabSkills,
            SummaryTodo,
            SummaryJobs,
            SummarySessions,
            SummarySkills,
            UsageContext,
            UsageCost,
            UsageCache,
            UsageOutput,
            PanelEmptyTodo,
            PanelEmptyJobs,
            PanelEmptySessions,
            PanelEmptySkills,
            PickerTitleModel,
            PickerTitleTheme,
            PickerTitleSessions,
            PickerTitleLanguage,
            PickerHintModel,
            PickerHintTheme,
            PickerHintSessions,
            PickerHintLanguage,
            PickerEmptyModel,
            PickerEmptyTheme,
            PickerEmptySessions,
            PickerEmptyLanguage,
            PickerFilter,
            PickerUngrouped,
            PickerCanvasLight,
            PickerCanvasDark,
            PickerEffort,
            LangSystem,
            InputHintChat,
            ChatOlder,
            ChatOlderHint,
            ActivityThinking,
            ActivityTool,
            ActivityStreaming,
            ActivityTools,
            HintBackToChat,
            HintSidebar,
            HintFocus,
            HintCommands,
            HintHelp,
            HintNewChat,
            HintSettings,
            ShortcutRefresh,
            ShortcutStop,
            HelpTitle,
            HelpFooter,
            PalettePanelTitle,
            PaletteTitle,
            PaletteFilter,
            PaletteEmpty,
            ChipModel,
            ChipEffort,
            ChipMode,
            ChipAccess,
            ShortcutCommands,
            ShortcutHelp,
            ShortcutNewChat,
            ShortcutSidebar,
            LaunchpadTagline,
            LaunchpadNewChat,
            LaunchpadNewChatIn,
            LaunchpadRecent,
            CmdNew,
            CmdDiff,
            CmdApproval,
            CmdHistory,
            CmdTree,
            CmdFork,
            CmdWorktree,
            CmdSandbox,
            CmdPermission,
            CmdCompact,
            CmdTools,
            CmdProviders,
            DiffTitle,
            DiffHint,
            DiffClean,
            DiffNoWorkspace,
            ApprovalTitle,
            ApprovalHint,
            ApprovalNone,
            ApprovalAction,
            ApprovalRequested,
            ApprovalBoundary,
            ApprovalRules,
            ApprovalNoRule,
            ApprovalRuleFailed,
            ApprovalRuleAdded,
            HistoryTitle,
            HistoryHint,
            HistoryEmpty,
            TreeTitle,
            TreeHint,
            ForkTitle,
            ForkHint,
            ForkEmpty,
            WorktreeTitle,
            WorktreeHint,
            WorktreeEmpty,
            WorktreeCreating,
            CompactDone,
            ToolsDone,
            ToolsUsage,
            WorkflowFailed,
            ForkEmptyId,
            CmdQueue,
            CmdCopy,
            CmdMouse,
            QueueTitle,
            QueueHint,
            QueueEmpty,
            QueueCancelled,
            QueueRecalled,
            QueueCancelFailed,
            QueueRecallFailed,
            CopyFailed,
            CopyDone,
            CopyEmpty,
            MouseOn,
            MouseOff,
            NotificationTitle,
            NotificationBody,
            LinearYou,
            LinearApproval,
            LinearTool,
            LinearError,
            LinearUnknownError,
            ChatNewOutput,
            CmdSessions,
            CmdModel,
            CmdLanguage,
            CmdReplyDetail,
            CmdWorkspace,
            CmdTheme,
            CmdUsage,
            CmdSkills,
            CmdExport,
            CmdTranscript,
            CmdHelp,
            CmdQuit,
            CmdPreferences,
            PrefDensity,
            PrefPosition,
            PrefWidth,
            PrefMouse,
            PrefNotifications,
            PrefNotificationBell,
            PrefMotion,
            PrefLinear,
            PrefEnabled,
            PrefDisabled,
            PrefShortcuts,
            PrefReset,
            PrefFocused,
            PrefComfortable,
            PrefLeft,
            PrefRight,
            PreferencesTitle,
            PreferencesHint,
            KeybindingsTitle,
            KeybindingsHint,
            NoticePreferencesSaved,
            NoticePreferenceInvalid,
            HelpKeyNew,
            HelpKeySidebar,
            HelpKeyFocus,
            HelpKeyScroll,
            HelpKeyPage,
            HelpKeyPalette,
            HelpKeyHelp,
            HelpKeyReplyDetail,
            HelpKeyReplyDetailSet,
            HelpKeyClick,
            HelpKeyDrag,
            HelpKeyQuit,
            NoticeWorkspace,
            NoticeTranscript,
            Unspecified,
            InputHintLaunchpad,
            ApprovalWaiting,
            ExportDone,
            ExportDonePath,
            JobPaused,
            JobScheduled,
            JobManual,
            CacheLabel,
        ]
    };
}
