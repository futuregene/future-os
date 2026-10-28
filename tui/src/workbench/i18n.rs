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

/// The reader's locale: the per-process POSIX overrides (`LC_ALL`, then
/// `LC_MESSAGES`), then the system's own default, then `LANG`. A tag that names
/// Chinese (Simplified or Traditional — the bundles ship one Chinese) picks
/// Chinese; anything else, including `C`/`POSIX` and a missing locale, is
/// English.
///
/// `LANG` is last on purpose. It is the broad default a terminal *environment*
/// carries, not a choice the reader made: a Mac takes it from `/etc/zprofile`
/// (`C.UTF-8`, or whatever the shell profile exports), so a Mac whose language
/// is Chinese would otherwise draw English, and the same happens on Windows
/// under a shell that exports its own `LANG`. The system's answer is the macOS
/// preferred-language list and the Windows UI language; on the other Unixes the
/// variables already are the system's answer, so the platform step is empty and
/// `LANG` decides as it always did.
pub fn system_language() -> Language {
    explicit_locale_language()
        .or_else(platform_language)
        .or_else(lang_language)
        .unwrap_or(Language::En)
}

/// `LC_ALL` and `LC_MESSAGES` are set per process, by a shell or a launcher, so
/// they mean what they say even where `LANG` is inherited noise.
fn explicit_locale_language() -> Option<Language> {
    locale_language(&["LC_ALL", "LC_MESSAGES"])
}

fn lang_language() -> Option<Language> {
    locale_language(&["LANG"])
}

fn locale_language(names: &[&str]) -> Option<Language> {
    for name in names {
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

/// The OS's own answer when no locale variable is set: the user's UI language
/// on Windows, the preferred-language list on macOS, and nothing elsewhere (a
/// POSIX terminal's locale variables *are* its answer).
fn platform_language() -> Option<Language> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Globalization::GetUserDefaultUILanguage;
        // A LANGID's language is its low ten bits; 0x04 is Chinese.
        let langid = unsafe { GetUserDefaultUILanguage() };
        Some(if langid & 0x3ff == 0x04 {
            Language::Zh
        } else {
            Language::En
        })
    }
    #[cfg(target_os = "macos")]
    {
        macos::preferred_language()
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        None
    }
}

/// The system language as the Mac's own preferences report it.
///
/// macOS keeps it in `AppleLanguages` (`defaults read -g AppleLanguages`) and
/// hands it out through CoreFoundation, which is what every other bundle on the
/// machine reads; `LC_ALL`/`LANG` describe the terminal, not the Mac, which is
/// why they only get the last word (see [`system_language`]).
#[cfg(target_os = "macos")]
mod macos {
    use super::{is_chinese_tag, Language};
    use std::ffi::{c_char, c_void, CStr};

    // CoreFoundation's own types: an array is a plain `CFTypeRef` here, and the
    // one element we read is a string, so no stronger typing buys anything.
    type CFTypeRef = *const c_void;

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        /// The reader's ordered language preferences, e.g. `["zh-Hans-CN"]`;
        /// owned, so the caller releases it.
        fn CFLocaleCopyPreferredLanguages() -> CFTypeRef;
        fn CFArrayGetCount(array: CFTypeRef) -> isize;
        fn CFArrayGetValueAtIndex(array: CFTypeRef, index: isize) -> *const c_void;
        fn CFStringGetCString(
            string: CFTypeRef,
            buffer: *mut c_char,
            buffer_size: isize,
            encoding: u32,
        ) -> u8;
        fn CFRelease(reference: CFTypeRef);
    }

    /// `kCFStringEncodingUTF8`, which is what the tags are.
    const UTF8: u32 = 0x0800_0100;

    /// The first preferred language, or `None` if the system does not answer.
    pub(super) fn preferred_language() -> Option<Language> {
        // A BCP-47 tag (`zh-Hans-CN`, `en-US`) is a handful of bytes; anything
        // longer is not a language tag and is not worth a second allocation.
        let mut buffer = [0i8; 64];
        // SAFETY: every call is a plain CoreFoundation getter on an object we
        // either just created (and release) or borrowed from the returned
        // array, and the buffer handed to `CFStringGetCString` is ours.
        unsafe {
            let languages = CFLocaleCopyPreferredLanguages();
            if languages.is_null() {
                return None;
            }
            let mut answer = None;
            if CFArrayGetCount(languages) > 0 {
                let first = CFArrayGetValueAtIndex(languages, 0);
                if !first.is_null()
                    && CFStringGetCString(first, buffer.as_mut_ptr(), buffer.len() as isize, UTF8)
                        != 0
                {
                    let tag = CStr::from_ptr(buffer.as_ptr()).to_string_lossy();
                    answer = Some(if is_chinese_tag(&tag) {
                        Language::Zh
                    } else {
                        Language::En
                    });
                }
            }
            CFRelease(languages);
            answer
        }
    }
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
    TabRecent,
    TabWorkspaces,
    /// The files panel's title, its header button and the help row's name.
    FilesTitle,
    // ── the workspace tab's sections ──────────────────────────────────
    SectionPinned,
    SectionWorkspaces,
    SectionChats,
    // ── the recent tab's two stretches ────────────────────────────────
    SectionActive,
    SectionEarlier,
    SummaryUnitsSessions,
    PanelEmptyWorkspaces,
    PanelEmptyFiles,
    PanelTreeTruncated,
    // ── the sidebar's panels ──────────────────────────────────────────
    SummaryTodo,
    SummaryJobs,
    SummarySkills,
    UsageContext,
    UsageCost,
    UsageCache,
    UsageOutput,
    PanelEmptyTodo,
    PanelGoalLabel,
    PanelNoGoal,
    PanelNoGoalHint,
    PanelEmptyGoalTodo,
    PanelEmptyJobs,
    PanelEmptySessions,
    PanelEmptySearch,
    PanelEmptySkills,
    // ── the picker overlay ────────────────────────────────────────────
    PickerTitleModel,
    PickerTitleTheme,
    PickerTitleSessions,
    PickerTitleLanguage,
    PickerTitleThinking,
    PickerTitleApproval,
    PickerTitleWorkspace,
    PickerHintModel,
    PickerHintTheme,
    PickerHintSessions,
    PickerHintLanguage,
    PickerHintThinking,
    PickerHintApproval,
    PickerHintWorkspace,
    PickerEmptyModel,
    PickerEmptyTheme,
    PickerEmptySessions,
    PickerEmptyLanguage,
    PickerEmptyThinking,
    PickerEmptyApproval,
    PickerEmptyWorkspace,
    PickerNewWorkspace,
    PickerFilter,
    PickerUngrouped,
    PickerCanvasLight,
    PickerCanvasDark,
    PickerEffort,
    ModeManual,
    ModeSandbox,
    ModeAuto,
    ModeOff,
    ModeManualDesc,
    ModeSandboxDesc,
    ModeAutoDesc,
    ModeOffDesc,
    RailHint,
    /// The search line's placeholder, and the keys it answers while it is open.
    RailSearch,
    RailSearchHint,
    // ── the files panel (its own overlay, not a rail tab) ──────────
    FilesHint,
    FilesNoWorkspace,
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
    HintSessions,
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
    PaletteCommandsHeading,
    PaletteSkillsHeading,
    // ── the `@` / `#` mention menu ────────────────────────────────────
    MentionTitleFile,
    MentionTitleSession,
    MentionHintFile,
    MentionHintSession,
    MentionEmpty,
    // ── the launchpad ─────────────────────────────────────────────────
    ChipWorkspace,
    ChipModel,
    ChipEffort,
    ChipApproval,
    LaunchpadTagline,
    LaunchpadNewChat,
    LaunchpadNewChatIn,
    WorkspaceChosen,
    WorkspaceChat,
    WorkspaceChatDetail,
    WorkspaceNone,
    // ── the palette's verbs ───────────────────────────────────────────
    CmdNew,
    CmdDiff,
    CmdApproval,
    CmdThinking,
    CmdMode,
    CmdHistory,
    CmdTree,
    CmdFork,
    CmdWorktree,
    CmdSandbox,
    CmdCompact,
    CmdTools,
    CmdProviders,
    DiffTitle,
    DiffHint,
    DiffClean,
    DiffNoWorkspace,
    ApprovalTitle,
    ApprovalHint,
    ApprovalAllowOnce,
    ApprovalAllowRule,
    ApprovalDeny,
    DiffFiles,
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
    // ── the `/git` panel ───────────────────────────────────────────────
    CmdGit,
    CmdFiles,
    GitHint,
    GitTabCommits,
    GitTabRefs,
    GitTabStatus,
    GitTabStash,
    GitTabWorktrees,
    GitTabBranches,
    GitEmptyCommits,
    GitEmptyRefs,
    GitCleanStatus,
    GitEmptyStash,
    GitEmptyWorktrees,
    GitEmptyBranches,
    GitNoRepo,
    /// A `git` that failed with nothing on stderr.
    GitNoMessage,
    /// A branch's two counts against its upstream, in the detail pane's header.
    GitAheadBehind,
    /// The `git show --format=` template behind the tip of a ref: its labels
    /// (`Author:`, `Date:`) are drawn, so they are the table's to spell.
    GitTipFormat,
    GitFilter,
    GitDetail,
    GitNoSelection,
    GitCopied,
    CompactDone,
    ToolsDone,
    ToolsUsage,
    WorkflowFailed,
    ForkEmptyId,
    CmdQueue,
    CmdCopy,
    CmdPaste,
    CmdPreview,
    CmdRefresh,
    CmdMouse,
    CmdDelete,
    DeleteTitle,
    DeleteBody,
    DeleteWhere,
    DeleteHint,
    NoticeDeleted,
    NoticeDeleteFailed,
    NoticeNothingToDelete,
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
    ChatNewOutput,
    CmdSessions,
    CmdModel,
    CmdLanguage,
    CmdReplyDetail,
    CmdToolOutput,
    CmdWorkspace,
    CmdTheme,
    CmdUsage,
    CmdSkills,
    CmdTodo,
    CmdExport,
    CmdTranscript,
    CmdHelp,
    CmdQuit,
    CmdPreferences,
    PrefPosition,
    PrefWidth,
    PrefMouse,
    PrefNotifications,
    PrefNotificationBell,
    PrefMotion,
    PrefSidebarVisible,
    PrefTheme,
    PrefEnabled,
    PrefDisabled,
    PrefShortcuts,
    PrefReset,
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
    HelpKeyDelete,
    HelpKeySidebar,
    HelpKeyFocus,
    HelpKeyScroll,
    HelpKeyPage,
    HelpKeyPalette,
    HelpKeyMentionFile,
    HelpKeyMentionSession,
    HelpKeyHelp,
    HelpKeyReplyDetail,
    HelpKeyReplyDetailSet,
    HelpKeyPrevSession,
    HelpKeyNextSession,
    HelpKeyPin,
    HelpKeyThinking,
    HelpKeyApproval,
    HelpKeyWorkspaces,
    HelpKeyPanelRecent,
    HelpKeyPanelWorkspaces,
    HelpKeyFiles,
    HelpKeyPrevPanel,
    HelpKeyNextPanel,
    // ── the preview box and clipboard paste ──────────────────────────
    PreviewZoomIn,
    PreviewZoomOut,
    PreviewFit,
    PreviewPrev,
    PreviewNext,
    PreviewHintImage,
    PreviewHintText,
    PreviewLoading,
    PreviewImageTooLarge,
    PreviewImageChanged,
    PreviewNothing,
    PreviewImageUnsupported,
    PreviewTruncated,
    PreviewEmptyFile,
    AttachmentsInDraft,
    HelpKeyPasteImage,
    HelpKeyPreviewAttachment,
    NoticeClipboardEmpty,
    NoticeClipboardUnavailable,
    NoticeSessionNotStarted,
    NoticeSendUnacknowledged,
    NoticeSettingsLoadFailed,
    NoticeSettingsSaveFailed,
    HelpGroupSessions,
    HelpGroupChoice,
    HelpGroupPanels,
    HelpGroupInterface,
    HelpGroupInline,
    HelpKeyClick,
    HelpKeyDrag,
    HelpKeyQuit,
    // ── the notice row ────────────────────────────────────────────────
    NoticeTranscript,
    NoticePinned,
    NoticeUnpinned,
    NoticePinUnavailable,
    NoticeModeUnknown,
    NoticeOpenLinkFailed,
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
        TabRecent => ("最近", "Recent"),
        TabWorkspaces => ("工作区", "Workspaces"),

        SectionPinned => ("置顶", "Pinned"),
        SectionWorkspaces => ("工作区", "Workspaces"),
        SectionChats => ("对话会话", "Chats"),
        SectionActive => ("活跃", "Active"),
        SectionEarlier => ("更早", "Earlier"),
        SummaryUnitsSessions => ("会话", "chats"),
        PanelEmptyWorkspaces => ("没有工作区", "no workspaces"),
        FilesTitle => ("文件", "Files"),
        PanelEmptyFiles => ("目录是空的", "empty directory"),
        PanelTreeTruncated => ("只列出前 {} 项", "listing stopped at {} entries"),

        SummaryTodo => ("{} 待办 / {} 合计", "{} open / {} total"),
        SummaryJobs => ("{} 个任务", "{} jobs"),
        SummarySkills => ("{} 个技能", "{} skills"),
        UsageContext => ("上下文", "Context"),
        UsageCost => ("成本", "Cost"),
        UsageCache => ("缓存命中", "Cache hit"),
        UsageOutput => ("输出", "Output"),
        PanelEmptyTodo => ("没有待办", "no todos"),
        PanelGoalLabel => ("目标", "goal"),
        PanelNoGoal => (
            "本仓库没有进行中的目标",
            "no goal in progress in this repository",
        ),
        PanelNoGoalHint => ("future goal init 创建一个", "future goal init creates one"),
        PanelEmptyGoalTodo => ("这个目标还没有待办", "this goal has no todos yet"),
        PanelEmptyJobs => ("没有定时任务", "no scheduled jobs"),
        PanelEmptySessions => ("没有会话", "no sessions"),
        PanelEmptySearch => ("没有匹配的会话", "no matching sessions"),
        PanelEmptySkills => ("没有技能", "no skills"),

        PickerTitleModel => ("模型", "Models"),
        PickerTitleTheme => ("主题", "Themes"),
        PickerTitleSessions => ("会话", "Sessions"),
        PickerTitleLanguage => ("语言", "Language"),
        PickerTitleThinking => ("思考等级", "Reasoning effort"),
        PickerTitleApproval => ("审批模式", "Approval mode"),
        PickerTitleWorkspace => ("工作区", "Workspaces"),
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
        PickerHintThinking => (
            "↑↓ 选择 · ⏎ 应用思考等级 · esc 取消",
            "↑↓ choose · ⏎ applies the effort · esc cancels",
        ),
        PickerHintApproval => (
            "↑↓ 选择 · ⏎ 应用审批模式 · esc 取消",
            "↑↓ choose · ⏎ applies the approval mode · esc cancels",
        ),
        PickerHintWorkspace => (
            "↑↓ 选择 · ⏎ 切到该工作区 · 输入路径后 ⏎ 新建 · esc 取消",
            "↑↓ choose · ⏎ switches workspace · type a path and ⏎ creates one · esc cancels",
        ),
        PickerEmptyModel => ("没有匹配的模型", "No matching model"),
        PickerEmptyTheme => ("没有匹配的主题", "No matching theme"),
        PickerEmptySessions => ("没有匹配的会话", "No matching session"),
        PickerEmptyLanguage => ("没有匹配的语言", "No matching language"),
        PickerEmptyThinking => ("没有匹配的思考等级", "No matching effort level"),
        PickerEmptyApproval => ("没有匹配的审批模式", "No matching approval mode"),
        PickerEmptyWorkspace => (
            "没有工作区，输入路径新建一个",
            "No workspace — type a path to create one",
        ),
        PickerNewWorkspace => ("＋ 新建工作区 {}", "＋ New workspace {}"),
        PickerFilter => ("输入以过滤", "Type to filter"),
        PickerUngrouped => ("未分组", "Ungrouped"),
        PickerCanvasLight => ("浅色画布", "light canvas"),
        PickerCanvasDark => ("深色画布", "dark canvas"),
        PickerEffort => ("推理强度", "Effort"),
        ModeManual => ("手动审批", "Manual"),
        ModeSandbox => ("沙箱保护", "Sandboxed"),
        ModeAuto => ("自动审批", "Automatic review"),
        ModeOff => ("完全访问", "Full access"),
        ModeManualDesc => ("每步操作前询问", "Asks before each action"),
        ModeSandboxDesc => (
            "在系统沙箱里运行，必要时询问",
            "Runs inside the OS sandbox, asks when needed",
        ),
        ModeAutoDesc => (
            "沙箱保护，由模型判断是否放行",
            "Sandboxed; the model decides what goes through",
        ),
        ModeOffDesc => (
            "不审批、不沙箱，全部放行",
            "No approval, no sandbox — everything runs",
        ),
        LangSystem => ("跟随系统", "Follow system"),

        InputHintChat => (
            "输入 · enter 发送 · ctrl+j 换行",
            "Message · enter sends · ctrl+j newline",
        ),
        ChatOlder => ("↑ 更早", "↑ older"),
        ChatOlderHint => ("↑ 更早 · page up", "↑ older · page up"),
        ActivityThinking => ("思考", "Thinking"),
        ActivityTool => ("工具", "Tool"),
        ActivityStreaming => ("生成", "Writing"),
        ActivityTools => (" · {}s · {} 工具", " · {}s · {} tools"),
        // `esc` walks back one level — a preview to the panel it was opened
        // from, a panel to the chat — so the hint names the walk, not one of
        // its stops.
        HintBackToChat => ("返回上一级 / 回到聊天", "Back one level / to the chat"),
        HintSidebar => ("侧栏", "Sidebar"),
        HintSessions => ("会话", "Sessions"),
        HintFocus => ("焦点", "Focus"),
        HintCommands => ("命令", "Commands"),
        HintHelp => ("帮助", "Help"),
        HintNewChat => ("新会话", "New Chat"),
        HintSettings => ("偏好设置", "Preferences"),
        RailHint => (
            "shift+←→ 页签 · shift+↑↓ 切换会话",
            "shift+←→ tabs · shift+↑↓ switches session",
        ),
        RailSearch => ("搜索会话", "Search sessions"),
        RailSearchHint => (
            "输入筛选 · ⏎ 保留 · esc 清除",
            "type to filter · enter keeps · esc clears",
        ),
        FilesHint => (
            "↑↓ 选择 · ⏎ 打开 · ⌫ 上一层 · r 刷新 · esc 关闭",
            "↑↓ select · enter opens · ⌫ up a level · r refreshes · esc closes",
        ),
        FilesNoWorkspace => ("这个会话没有工作区", "This conversation has no workspace"),
        ShortcutRefresh => ("刷新会话和用量", "Refresh sessions and usage"),
        ShortcutStop => ("停止当前生成", "Stop the current run"),

        HelpTitle => ("帮助", "Help"),
        HelpFooter => (
            " 按键与快捷键 · ↑↓ 滚动 · esc 关闭",
            " Keys and shortcuts · ↑↓ scrolls · esc closes",
        ),
        PalettePanelTitle => (" 命令面板（/）", " Command palette (/)"),
        PaletteTitle => ("命令", "Commands"),
        PaletteFilter => (
            "输入以过滤命令与技能 · tab 补全 · ⏎ 运行",
            "Type to filter commands and skills · tab completes · enter runs",
        ),
        PaletteEmpty => (" 没有匹配的命令或技能", " No matching command or skill"),
        PaletteCommandsHeading => (" 命令", " Commands"),
        PaletteSkillsHeading => (
            " 技能（进草稿后由 agent 展开）",
            " Skills (a prompt token the agent expands)",
        ),

        // The menu completes a token the reader is typing, so its own copy is
        // chrome: what the sigil means, and which keys do what while it is up.
        MentionTitleFile => ("文件", "Files"),
        MentionTitleSession => ("会话", "Sessions"),
        MentionHintFile => (
            "↑↓ 选择 · ⏎ / tab 插入 · esc 关闭（不带 @ 提交则原样发送）",
            "↑↓ select · enter / tab insert · esc close",
        ),
        MentionHintSession => (
            "↑↓ 选择 · ⏎ / tab 插入会话链接 · esc 关闭",
            "↑↓ select · enter / tab insert a link · esc close",
        ),
        MentionEmpty => ("没有匹配", "no matches"),

        ChipWorkspace => ("工作区", "Workspace"),
        ChipModel => ("模型", "Model"),
        ChipEffort => ("强度", "Effort"),
        ChipApproval => ("审批", "Approval"),
        LaunchpadTagline => (
            "一个 AI agent，终端、桌面、手机都在",
            "One AI agent — terminal, desktop, phone",
        ),
        LaunchpadNewChat => ("新会话", "New Chat"),
        LaunchpadNewChatIn => ("新会话 · {}", "New Chat · {}"),
        WorkspaceChosen => ("工作区 · {}", "Workspace · {}"),
        // The one chooser row that is not a directory, and the name the rail's
        // last section already gives such a conversation.
        WorkspaceChat => ("对话会话", "Chat"),
        // Beside it, where the workspaces print their paths: the sentence that
        // makes the row a *choice* rather than a folder.
        WorkspaceChatDetail => ("不绑工作区", "no workspace"),
        WorkspaceNone => (
            "没有其它工作区可切换",
            "No other workspace to switch to",
        ),

        CmdNew => ("新建会话", "New chat"),
        CmdDiff => ("审查工作区改动", "Review working-tree changes"),
        CmdApproval => ("查看完整审批详情", "Inspect the pending approval"),
        CmdHistory => ("搜索历史会话", "Search session history"),
        CmdTree => ("浏览会话父子树", "Browse the session tree"),
        CmdFork => ("从用户消息节点创建分支会话", "Fork from a user-message point"),
        CmdWorktree => ("列出、进入或新建 git worktree", "List, enter or create a git worktree"),
        CmdSandbox => (
            "审批模式的旧写法（等同于上面的选择器）",
            "The older spelling of the approval mode (same chooser)",
        ),
        CmdThinking => ("选择思考等级", "Choose the reasoning effort"),
        CmdMode => (
            "选择审批模式：手动 / 沙箱 / 自动 / 完全访问",
            "Choose the approval mode: manual / sandbox / auto / full access",
        ),
        CmdCompact => ("压缩上下文，可附自定义说明", "Compact context, optionally with instructions"),
        CmdTools => ("设置工具列表，off 禁用全部", "Set the tool list; off disables all"),
        CmdProviders => ("列出模型提供商", "List model providers"),
        DiffTitle => ("Diff 审查 · 工作区未提交改动", "Diff review · uncommitted workspace changes"),
        DiffHint => ("tab 文件/变更 · ↑↓ 操作 · ←→ 选择文件 · r 刷新 · esc 关闭", "tab files/patch · ↑↓ navigate · ←→ choose file · r refreshes · esc closes"),
        DiffClean => ("工作区没有未提交改动", "No uncommitted changes in this workspace"),
        DiffNoWorkspace => ("当前会话没有工作区", "The current session has no workspace"),
        ApprovalTitle => ("审批详情", "Approval details"),
        ApprovalHint => ("↑↓ 滚动 · esc 返回", "↑↓ scroll · esc back"),
        ApprovalAllowOnce => ("允许一次", "Allow once"),
        ApprovalAllowRule => ("规则并允许", "Rule + allow"),
        ApprovalDeny => ("拒绝", "Deny"),
        DiffFiles => ("文件", "Files"),
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
        CmdGit => (
            "浏览 git 仓库：提交、引用、改动、stash、worktree、分支",
            "Browse the git repository: commits, refs, changes, stashes, worktrees, branches",
        ),
        CmdFiles => (
            "打开本会话的工作区文件树",
            "Open this conversation's workspace files",
        ),
        GitHint => (
            "1-6 视图 · ↑↓ 选择 · tab 列表/详情 · ←→ 横向滚动 · PgUp/PgDn 详情翻页 · ⏎ 打开 · y 复制 · r 刷新 · esc 关闭",
            "1-6 views · ↑↓ select · tab list/detail · ←→ scrolls horizontally · PgUp/PgDn pages the detail · enter opens · y copies · r refreshes · esc closes",
        ),
        GitTabCommits => ("提交", "commits"),
        GitTabRefs => ("引用", "refs"),
        GitTabStatus => ("改动", "changes"),
        GitTabStash => ("储藏", "stash"),
        GitTabWorktrees => ("工作区", "worktrees"),
        GitTabBranches => ("分支", "branches"),
        GitEmptyCommits => ("没有提交", "No commits"),
        GitEmptyRefs => ("没有分支或标签", "No branches or tags"),
        GitCleanStatus => ("工作区没有未提交改动", "No uncommitted changes"),
        GitEmptyStash => ("没有 stash", "No stashes"),
        GitEmptyWorktrees => ("没有 worktree", "No worktrees"),
        GitEmptyBranches => ("没有分支", "No branches"),
        GitNoRepo => (
            "当前工作区不是 git 仓库",
            "This workspace is not a git repository",
        ),
        GitNoMessage => ("git 没有任何输出", "git exited without a message"),
        GitAheadBehind => ("领先 {} · 落后 {}", "ahead {} · behind {}"),
        GitTipFormat => (
            "--format=commit %h%n作者: %an <%ae>%n日期:   %ad%n%n    %s%n%n%b",
            "--format=commit %h%nAuthor: %an <%ae>%nDate:   %ad%n%n    %s%n%n%b",
        ),
        GitFilter => ("过滤：{}", "filter: {}"),
        GitDetail => ("详情", "Detail"),
        GitNoSelection => ("选择一个条目查看详情", "Select an entry to see its detail"),
        GitCopied => ("已复制 {}", "copied {}"),
        CompactDone => ("上下文已压缩", "Context compacted"),
        ToolsDone => ("工具设置已更新", "Tool settings updated"),
        ToolsUsage => ("用法：/tools read,shell · /tools off", "Usage: /tools read,shell · /tools off"),
        WorkflowFailed => ("操作失败：{}", "Operation failed: {}"),
        ForkEmptyId => ("fork 未返回会话 id", "Fork returned no session id"),
        CmdQueue => ("查看、取消或取回排队消息", "List, cancel or recall queued prompts"),
        CmdDelete => ("删除这条会话（先确认）", "Delete this conversation (asks first)"),
        CmdCopy => ("复制最后一条回答", "Copy the last assistant answer"),
        CmdPaste => (
            "粘贴剪贴板（图片会变成附件）",
            "Paste the clipboard (an image becomes an attachment)",
        ),
        CmdPreview => (
            "预览草稿里最新的附件",
            "Preview the newest attachment in the draft",
        ),
        CmdRefresh => (
            "重新读取会话、用量与目录树",
            "Re-read sessions, usage and the file tree",
        ),
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
        ChatNewOutput => ("↓ 新输出 · esc 回到最新", "↓ new output · esc returns to latest"),
        CmdSessions => ("浏览并切换会话", "Browse and switch sessions"),
        CmdModel => ("选择模型", "Choose a model"),
        CmdLanguage => ("界面语言", "Interface language"),
        CmdReplyDetail => ("最终输出 / 完整过程", "Final answer / full process"),
        CmdToolOutput => ("工具调用正文：展开 / 收起", "Tool call bodies: expand / collapse"),
        CmdWorkspace => ("workspace 目录", "Workspace directory"),
        CmdTheme => ("切换主题", "Switch theme"),
        CmdUsage => ("用量与成本", "Usage and cost"),
        CmdTodo => (
            "侧栏的待办面板（这个仓库的目标看板）",
            "The sidebar's to-do panel (this repository's goal board)",
        ),
        CmdSkills => ("技能列表", "Installed skills"),
        CmdExport => ("导出会话", "Export the session"),
        CmdTranscript => ("完整记录", "The whole record"),
        CmdHelp => ("帮助与快捷键", "Help and shortcuts"),
        CmdQuit => ("退出 workbench", "Leave the workbench"),
        CmdPreferences => ("布局、外观、通知、可访问性和快捷键", "Layout, appearance, notifications, accessibility and shortcuts"),
        PrefPosition => ("侧栏位置", "Sidebar position"),
        PrefWidth => ("侧栏宽度", "Sidebar width"),
        PrefMouse => ("启动与运行时使用鼠标", "Mouse capture"),
        PrefNotifications => ("完成时通知", "Completion notifications"),
        PrefNotificationBell => ("通知包含提示音", "Notification bell"),
        PrefMotion => ("减少动态", "Reduce motion"),
        PrefSidebarVisible => ("显示侧栏", "Show sidebar"),
        PrefTheme => ("配色主题", "Color theme"),
        PrefEnabled => ("开启", "On"),
        PrefDisabled => ("关闭", "Off"),
        PrefShortcuts => ("自定义快捷键", "Customize shortcuts"),
        PrefReset => ("恢复默认偏好", "Restore defaults"),
        PrefLeft => ("左侧", "Left"),
        PrefRight => ("右侧", "Right"),
        PreferencesTitle => ("偏好设置", "Preferences"),
        PreferencesHint => ("↑↓ 选择 · ←→ 调整 · ⏎ 打开/恢复 · esc 关闭", "↑↓ choose · ←→ adjust · ⏎ open/reset · esc closes"),
        KeybindingsTitle => ("快捷键", "Shortcuts"),
        KeybindingsHint => ("↑↓ 选择 · ⏎ 后按新快捷键 · r 恢复全部 · esc 关闭", "↑↓ choose · ⏎ then press a new chord · r resets all · esc closes"),
        NoticePreferencesSaved => ("偏好已保存", "Preferences saved"),
        NoticePreferenceInvalid => ("不支持的按键，请使用 Ctrl/Alt+字母、F1–F24 或 Tab", "Unsupported key — use Ctrl/Alt + a letter, F1–F24 or Tab"),

        HelpKeyNew => ("新会话（任意位置）", "New chat (from anywhere)"),
        HelpKeyDelete => (
            "删除选中的会话（先确认）",
            "Delete the selected conversation (asks first)",
        ),
        HelpKeySidebar => ("侧栏：打开 / 聚焦 / 收起", "Sidebar: show / focus / hide"),
        HelpKeyFocus => (
            "在对话里逐个选中链接与图片，⏎ 打开",
            "Step through the conversation's links and pictures, enter opens one",
        ),
        HelpKeyScroll => ("滚动聊天，或侧栏列表", "Scroll the chat, or the sidebar's list"),
        HelpKeyPage => ("整页滚动聊天", "Scroll the chat a page at a time"),
        HelpKeyPalette => ("命令面板", "Command palette"),
        HelpKeyMentionFile => (
            "引用文件（插入 @path，提交时带内容）",
            "Reference a file (@path, sent with its content)",
        ),
        HelpKeyMentionSession => (
            "引用会话（插入会话链接）",
            "Reference a conversation (inserts its link)",
        ),
        HelpKeyHelp => ("这份帮助", "This help"),
        HelpKeyReplyDetail => ("最终输出 / 完整过程", "Final answer / full process"),
        HelpKeyReplyDetailSet => ("同上，直接设置", "The same, set directly"),
        HelpKeyPrevSession => ("上一个会话（侧栏随之切换）", "Previous conversation (the chat follows)"),
        HelpKeyNextSession => ("下一个会话（侧栏随之切换）", "Next conversation (the chat follows)"),
        HelpKeyPin => ("置顶 / 取消置顶该工作区", "Pin / unpin this workspace"),
        HelpKeyThinking => (
            "思考等级（弹出选择）",
            "Reasoning effort (opens the chooser)",
        ),
        HelpKeyApproval => (
            "审批模式（弹出选择）",
            "Approval mode (opens the chooser)",
        ),
        HelpKeyWorkspaces => (
            "工作区：切换到已有的或新建一个",
            "Workspaces: switch to one or create one",
        ),
        HelpKeyPanelRecent => ("切到「最近」面板", "Show the Recent panel"),
        HelpKeyPanelWorkspaces => ("切到「工作区」面板", "Show the Workspaces panel"),
        HelpKeyFiles => (
            "打开本会话的文件面板",
            "Show this conversation's files",
        ),
        HelpKeyPrevPanel => ("上一个页签：最近 / 工作区", "Previous tab: Recent / Workspaces"),
        HelpKeyNextPanel => ("下一个页签：最近 / 工作区", "Next tab: Recent / Workspaces"),
        HelpKeyPasteImage => (
            "粘贴：图片直接成为草稿附件，文字粘进草稿",
            "Paste: an image becomes a draft attachment, text is pasted",
        ),
        HelpKeyPreviewAttachment => (
            "查看草稿里最新那张图片",
            "View the newest image attached to the draft",
        ),
        PreviewZoomIn => ("放大", "Zoom in"),
        PreviewZoomOut => ("缩小", "Zoom out"),
        PreviewFit => ("适应", "Fit"),
        PreviewPrev => ("上一张", "Previous"),
        PreviewNext => ("下一张", "Next"),
        PreviewHintImage => (
            "＋ / － 缩放 · 拖动或方向键平移 · [ ] 上一张/下一张 · 0 复位 · esc 关闭",
            "+ / - zoom · drag or arrows to pan · [ ] previous/next · 0 resets · esc closes",
        ),
        PreviewHintText => (
            "↑↓ 滚动 · pageUp/pageDown 翻页 · esc 关闭",
            "↑↓ scroll · pageUp/pageDown page · esc closes",
        ),
        PreviewLoading => ("正在读取…", "Loading…"),
        PreviewImageTooLarge => (
            "图片有 {} MB；预览最多接受 {} MB",
            "image is {} MB; the preview accepts at most {} MB",
        ),
        PreviewImageChanged => (
            "图片在读取过程中变大了，超出预览上限",
            "the image grew past the preview limit while it was being read",
        ),
        PreviewNothing => ("没有可预览的内容", "nothing to preview"),
        PreviewImageUnsupported => (
            "这个终端不能显示图片。在终端之外打开这个文件就能看到它。",
            "This terminal cannot show pictures. Open the file outside the TUI to see it.",
        ),
        PreviewTruncated => ("（只显示开头）", "(showing the beginning)"),
        PreviewEmptyFile => ("空文件", "empty file"),
        AttachmentsInDraft => (" · 📎 {} 张图片", " · 📎 {} images"),
        NoticeClipboardEmpty => (
            "剪贴板里没有图片也没有文本",
            "The clipboard has no image and no text",
        ),
        NoticeClipboardUnavailable => ("读不到剪贴板：{}", "Could not read the clipboard: {}"),
        NoticeSessionNotStarted => (
            "没能开启新会话：代理没有返回可用的会话（草稿还在，可以直接再发一次）",
            "Could not start a session: the agent returned none (the draft is kept, try again)",
        ),
        NoticeSendUnacknowledged => (
            "发送未得到确认：{}。代理可能已经收到，请先刷新核对，再决定是否重发",
            "Send was not acknowledged: {}. The agent may have received it; refresh before retrying",
        ),
        NoticeSettingsLoadFailed => (
            "读取偏好设置失败，正在使用默认值：{}",
            "Could not read the TUI settings; defaults are in use: {}",
        ),
        NoticeSettingsSaveFailed => (
            "保存偏好设置失败：{}",
            "Could not save the TUI settings: {}",
        ),
        HelpGroupSessions => ("会话", "Chats"),
        HelpGroupChoice => ("选择", "Choices"),
        HelpGroupPanels => ("面板", "Panels"),
        HelpGroupInterface => ("界面", "Interface"),
        HelpGroupInline => (
            "行内按键（不可改键）",
            "Inline keys (not rebindable)",
        ),
        HelpKeyClick => ("切页签、点行、点参数", "Tabs, rows and chips"),
        HelpKeyDrag => ("侧栏分隔条调宽", "Widen the sidebar at its divider"),
        HelpKeyQuit => ("退出", "Quit"),

        NoticeTranscript => (
            "完整记录：/export 导出整份会话（HTML） · /reply-detail 在聊天里看完整过程",
            "The whole record: /export writes the session as HTML · /reply-detail shows the full process in the chat",
        ),
        NoticePinned => ("已置顶 {} · 在 工作区 页签的 置顶 区", "Pinned {} · it leads the Workspaces tab"),
        NoticeUnpinned => ("已取消置顶 {}", "Unpinned {}"),
        NoticePinUnavailable => ("这条会话不属于工作区，无法置顶", "This conversation belongs to no workspace"),
        DeleteTitle => ("删除会话", "Delete conversation"),
        DeleteBody => (
            "它会连同记录和文件一起删除，无法撤销。",
            "Its record and its file go with it. This cannot be undone.",
        ),
        DeleteWhere => ("工作区：{}", "Workspace: {}"),
        DeleteHint => ("⏎ 删除 · esc 保留", "enter deletes · esc keeps it"),
        NoticeDeleted => ("已删除 {}", "Deleted {}"),
        NoticeDeleteFailed => ("无法删除会话：{}", "Could not delete the conversation: {}"),
        NoticeNothingToDelete => ("这里没有可删除的会话", "Nothing to delete here"),
        NoticeModeUnknown => (
            "未知的审批模式：{}",
            "Unknown approval mode: {}",
        ),
        NoticeOpenLinkFailed => ("打不开 {}：{}", "Could not open {}: {}"),

        InputHintLaunchpad => (
            // What the *draft* can hold, in the order the reader types it: the
            // task itself, then the three sigils the box answers. `/ 命令` is
            // here rather than in a shortcut row under the box, because it is
            // typed into the box — it is not a key binding to memorize.
            "描述你要做的事 · / 命令 · @ 文件 · # 会话",
            "Describe the task · / commands · @ files · # sessions",
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
        // `zh-Hans-CN` is what macOS's `AppleLanguages` reports.
        for tag in [
            "zh_CN.UTF-8",
            "zh",
            "zh-Hans",
            "zh-Hans-CN",
            "ZH_TW",
            "zh_SG.utf8",
        ] {
            assert_eq!(
                env_language_of(tag),
                Some(Language::Zh),
                "{tag} should read as Chinese"
            );
        }
    }

    #[test]
    fn anything_else_is_english() {
        for tag in ["en_US.UTF-8", "de_DE.UTF-8", "en", "en-US"] {
            assert_eq!(env_language_of(tag), Some(Language::En), "{tag}");
        }
        // `C` and `POSIX` are "no locale chosen", not "English chosen": they
        // fall through to the platform, which is what a bare terminal means.
        for tag in ["C", "POSIX", "c", "  "] {
            assert_eq!(env_language_of(tag), None, "{tag}");
        }
    }

    /// The macOS path asks CoreFoundation instead of the shell, so the call
    /// itself has to work: a wrong signature is a crash or a garbage tag, and
    /// the locale tests above would not notice either. *Which* language it
    /// answers belongs to the machine running the test, not to the test.
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_reads_the_system_language() {
        let language = macos::preferred_language().expect("CoreFoundation answers");
        assert!(matches!(language, Language::Zh | Language::En));
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
    const ALL_KEYS: [Key; 318] = {
        use Key::*;
        [
            TabTodo,
            TabJobs,
            TabSessions,
            TabUsage,
            TabSkills,
            TabRecent,
            TabWorkspaces,
            FilesTitle,
            SectionPinned,
            SectionWorkspaces,
            SectionChats,
            SectionActive,
            SectionEarlier,
            SummaryUnitsSessions,
            PanelEmptyWorkspaces,
            PanelEmptyFiles,
            PanelTreeTruncated,
            SummaryTodo,
            SummaryJobs,
            SummarySkills,
            UsageContext,
            UsageCost,
            UsageCache,
            UsageOutput,
            PanelEmptyTodo,
            PanelGoalLabel,
            PanelNoGoal,
            PanelNoGoalHint,
            PanelEmptyGoalTodo,
            PanelEmptyJobs,
            PanelEmptySessions,
            PanelEmptySearch,
            PanelEmptySkills,
            PickerTitleModel,
            PickerTitleTheme,
            PickerTitleSessions,
            PickerTitleLanguage,
            PickerTitleThinking,
            PickerTitleApproval,
            PickerTitleWorkspace,
            PickerHintModel,
            PickerHintTheme,
            PickerHintSessions,
            PickerHintLanguage,
            PickerHintThinking,
            PickerHintApproval,
            PickerHintWorkspace,
            PickerEmptyModel,
            PickerEmptyTheme,
            PickerEmptySessions,
            PickerEmptyLanguage,
            PickerEmptyThinking,
            PickerEmptyApproval,
            PickerEmptyWorkspace,
            PickerNewWorkspace,
            PickerFilter,
            PickerUngrouped,
            PickerCanvasLight,
            PickerCanvasDark,
            PickerEffort,
            ModeManual,
            ModeSandbox,
            ModeAuto,
            ModeOff,
            ModeManualDesc,
            ModeSandboxDesc,
            ModeAutoDesc,
            ModeOffDesc,
            RailHint,
            RailSearch,
            RailSearchHint,
            FilesHint,
            FilesNoWorkspace,
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
            HintSessions,
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
            PaletteCommandsHeading,
            PaletteSkillsHeading,
            MentionTitleFile,
            MentionTitleSession,
            MentionHintFile,
            MentionHintSession,
            MentionEmpty,
            ChipWorkspace,
            ChipModel,
            ChipEffort,
            ChipApproval,
            LaunchpadTagline,
            LaunchpadNewChat,
            LaunchpadNewChatIn,
            WorkspaceChosen,
            WorkspaceChat,
            WorkspaceChatDetail,
            WorkspaceNone,
            CmdNew,
            CmdDiff,
            CmdApproval,
            CmdThinking,
            CmdMode,
            CmdHistory,
            CmdTree,
            CmdFork,
            CmdWorktree,
            CmdSandbox,
            CmdCompact,
            CmdTools,
            CmdProviders,
            DiffTitle,
            DiffHint,
            DiffClean,
            DiffNoWorkspace,
            ApprovalTitle,
            ApprovalHint,
            ApprovalAllowOnce,
            ApprovalAllowRule,
            ApprovalDeny,
            DiffFiles,
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
            CmdGit,
            CmdFiles,
            GitHint,
            GitTabCommits,
            GitTabRefs,
            GitTabStatus,
            GitTabStash,
            GitTabWorktrees,
            GitTabBranches,
            GitEmptyCommits,
            GitEmptyRefs,
            GitCleanStatus,
            GitEmptyStash,
            GitEmptyWorktrees,
            GitEmptyBranches,
            GitNoRepo,
            GitNoMessage,
            GitAheadBehind,
            GitTipFormat,
            GitFilter,
            GitDetail,
            GitNoSelection,
            GitCopied,
            CompactDone,
            ToolsDone,
            ToolsUsage,
            WorkflowFailed,
            ForkEmptyId,
            CmdQueue,
            CmdDelete,
            CmdCopy,
            CmdPaste,
            CmdPreview,
            CmdRefresh,
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
            ChatNewOutput,
            CmdSessions,
            CmdModel,
            CmdLanguage,
            CmdReplyDetail,
            CmdToolOutput,
            CmdWorkspace,
            CmdTheme,
            CmdUsage,
            CmdSkills,
            CmdTodo,
            CmdExport,
            CmdTranscript,
            CmdHelp,
            CmdQuit,
            CmdPreferences,
            PrefPosition,
            PrefWidth,
            PrefMouse,
            PrefNotifications,
            PrefNotificationBell,
            PrefMotion,
            PrefSidebarVisible,
            PrefTheme,
            PrefEnabled,
            PrefDisabled,
            PrefShortcuts,
            PrefReset,
            PrefLeft,
            PrefRight,
            PreferencesTitle,
            PreferencesHint,
            KeybindingsTitle,
            KeybindingsHint,
            NoticePreferencesSaved,
            NoticePreferenceInvalid,
            HelpKeyNew,
            HelpKeyDelete,
            HelpKeySidebar,
            HelpKeyFocus,
            HelpKeyScroll,
            HelpKeyPage,
            HelpKeyPalette,
            HelpKeyMentionFile,
            HelpKeyMentionSession,
            HelpKeyHelp,
            HelpKeyReplyDetail,
            HelpKeyReplyDetailSet,
            HelpKeyPrevSession,
            HelpKeyNextSession,
            HelpKeyPin,
            HelpKeyThinking,
            HelpKeyApproval,
            HelpKeyWorkspaces,
            HelpKeyPanelRecent,
            HelpKeyPanelWorkspaces,
            HelpKeyFiles,
            HelpKeyPrevPanel,
            HelpKeyNextPanel,
            HelpKeyPasteImage,
            HelpKeyPreviewAttachment,
            PreviewZoomIn,
            PreviewZoomOut,
            PreviewFit,
            PreviewPrev,
            PreviewNext,
            PreviewHintImage,
            PreviewHintText,
            PreviewLoading,
            PreviewImageTooLarge,
            PreviewImageChanged,
            PreviewNothing,
            PreviewImageUnsupported,
            PreviewTruncated,
            PreviewEmptyFile,
            AttachmentsInDraft,
            NoticeClipboardEmpty,
            NoticeClipboardUnavailable,
            NoticeSessionNotStarted,
            NoticeSendUnacknowledged,
            NoticeSettingsLoadFailed,
            NoticeSettingsSaveFailed,
            HelpGroupSessions,
            HelpGroupChoice,
            HelpGroupPanels,
            HelpGroupInterface,
            HelpGroupInline,
            HelpKeyClick,
            HelpKeyDrag,
            HelpKeyQuit,
            NoticeTranscript,
            NoticePinned,
            NoticeUnpinned,
            NoticePinUnavailable,
            NoticeOpenLinkFailed,
            DeleteTitle,
            DeleteBody,
            DeleteWhere,
            DeleteHint,
            NoticeDeleted,
            NoticeDeleteFailed,
            NoticeNothingToDelete,
            NoticeModeUnknown,
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
