//! Authoritative keyboard shortcut catalog and context resolver.
//!
//! Text editing and PTY byte translation intentionally remain outside this
//! module. Everything here represents an application command that can be
//! discovered in the footer, a modal hint row, or the help overlay.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use protocol::Route;

use crate::app::{BranchSubPane, Focus, LogItem, Settings, SidebarMode, TuiApp};

/// Identifies one semantic application action independently of its bindings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShortcutId {
    OpenHelp,
    CloseHelp,
    ToggleHelpView,
    HelpScrollDown,
    HelpScrollUp,
    HelpPageDown,
    HelpPageUp,
    HelpTop,
    HelpBottom,
    Quit,
    PreviousWorkspace,
    NextWorkspace,
    CycleSidebar,
    MoveDown,
    MoveUp,
    Open,
    Back,
    NewWorkspace,
    AddRepository,
    AddSshRepository,
    Delete,
    Review,
    ToggleReady,
    ToggleReviewFilter,
    RefreshGit,
    OpenSettings,
    Collapse,
    Expand,
    ExpandOrOpen,
    ClosePopout,
    Confirm,
    Cancel,
    NextField,
    PreviousField,
    PreviousChoice,
    NextChoice,
    MoveWorkspace,
    ToggleSetting,
    AdjustSettingLeft,
    AdjustSettingRight,
    NewAgent,
    DeleteAgent,
    SelectNewSsh,
    GoToParent,
    ToggleHidden,
    EditPath,
    EnterDirectory,
    SelectDirectory,
    ToggleTerminalCommandMode,
    ScrollTerminalToBottom,
    ToggleFullscreen,
    ToggleYolo,
    NextPane,
    PreviousPane,
    OpenWorkspaceCommand,
    SelectFirstTab,
    SelectSecondTab,
    NextTerminalTab,
    PreviousTerminalTab,
    NewShellTab,
    CloseTerminalTab,
    RenameTerminalTab,
    SwitchAgent,
    StartActiveTerminal,
    StopActiveTerminal,
    ToggleLogItem,
    ToggleStage,
    StageAll,
    UnstageAll,
    Commit,
    Discard,
    DiscardAll,
    Stash,
    StashAll,
    ToggleTags,
    SelectLocalBranches,
    SelectRemoteBranches,
    CheckoutBranch,
    CreateBranch,
    DeleteBranch,
    Pull,
    Fetch,
    Push,
    ToggleReviewPane,
    OpenReviewFile,
    ReviewPageDown,
    ReviewPageUp,
    OpenPullRequest,
    RunCommand,
    OutputUp,
    OutputDown,
    OutputPageUp,
    OutputPageDown,
    AttachSession,
    NewSession,
    DeleteSession,
    RefreshSessions,
}

/// Identifies a precedence-resolved leaf of the keyboard input state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ShortcutContext {
    Help,
    HomeSidebar,
    HomeRail,
    HomeRailPopout,
    RepoSummary,
    WorkspaceSidebar,
    WorkspaceRail,
    WorkspaceRailPopout,
    TerminalPassthrough,
    TerminalCommand,
    TerminalTabs,
    GitLogHeader,
    GitLogFile,
    GitLogDirectory,
    GitLogCommit,
    GitLogCommitFile,
    Branches,
    Diff,
    ReviewFiles,
    ReviewDiff,
    Settings,
    SettingsConfirmDeleteAgent,
    SshHistory,
    DirectoryBrowser,
    MovingWorkspace,
    DeleteConfirm,
    DiscardConfirm,
    DiscardAllConfirm,
    StashPullPopConfirm,
    DeleteBranchConfirm,
    AgentPicker,
    ResurrectConfirm,
    QuickCreateText,
    SshText,
    DirectoryPathText,
    RenameWorkspaceText,
    RenameTabText,
    CreateBranchText,
    CommitText,
    StashText,
    AgentCommandText,
    WorkspaceCommandText,
    SettingsText,
    SettingsKeyCapture,
    NewAgentText,
    ChooserBrowse,
    ChooserNewText,
    ChooserDeleteConfirm,
}

impl ShortcutContext {
    pub fn title(self) -> &'static str {
        match self {
            Self::Help => "Help",
            Self::HomeSidebar => "Home · sidebar",
            Self::HomeRail => "Home · repository rail",
            Self::HomeRailPopout => "Home · rail workspace list",
            Self::RepoSummary => "Repository summary",
            Self::WorkspaceSidebar => "Workspace · sidebar",
            Self::WorkspaceRail => "Workspace · repository rail",
            Self::WorkspaceRailPopout => "Workspace · rail workspace list",
            Self::TerminalPassthrough => "Workspace · terminal",
            Self::TerminalCommand => "Workspace · terminal command mode",
            Self::TerminalTabs => "Workspace · terminal tabs",
            Self::GitLogHeader => "Workspace · Git log · changes",
            Self::GitLogFile => "Workspace · Git log · changed file",
            Self::GitLogDirectory => "Workspace · Git log · changed directory",
            Self::GitLogCommit => "Workspace · Git log · commit",
            Self::GitLogCommitFile => "Workspace · Git log · commit file",
            Self::Branches => "Workspace · branches",
            Self::Diff => "Workspace · diff",
            Self::ReviewFiles => "Review · files",
            Self::ReviewDiff => "Review · diff",
            Self::Settings => "Settings",
            Self::SettingsConfirmDeleteAgent => "Settings · delete agent",
            Self::SshHistory => "Recent SSH workspaces",
            Self::DirectoryBrowser => "Add repository",
            Self::MovingWorkspace => "Move workspace",
            Self::DeleteConfirm => "Delete confirmation",
            Self::DiscardConfirm => "Discard file confirmation",
            Self::DiscardAllConfirm => "Discard all confirmation",
            Self::StashPullPopConfirm => "Stash, pull, and pop confirmation",
            Self::DeleteBranchConfirm => "Delete branch confirmation",
            Self::AgentPicker => "Switch agent",
            Self::ResurrectConfirm => "Resume command",
            Self::QuickCreateText => "New workspace",
            Self::SshText => "Add SSH workspace",
            Self::DirectoryPathText => "Add repository · path",
            Self::RenameWorkspaceText => "Rename workspace",
            Self::RenameTabText => "Rename terminal tab",
            Self::CreateBranchText => "Create branch",
            Self::CommitText => "Commit message",
            Self::StashText => "Stash message",
            Self::AgentCommandText => "Custom agent command",
            Self::WorkspaceCommandText => "Workspace command",
            Self::SettingsText => "Settings · edit value",
            Self::SettingsKeyCapture => "Settings · capture key",
            Self::NewAgentText => "Settings · new agent",
            Self::ChooserBrowse => "Session chooser",
            Self::ChooserNewText => "Session chooser · new session",
            Self::ChooserDeleteConfirm => "Session chooser · delete session",
        }
    }

    pub fn help_available(self) -> bool {
        !matches!(
            self,
            Self::TerminalPassthrough
                | Self::ResurrectConfirm
                | Self::QuickCreateText
                | Self::SshText
                | Self::DirectoryPathText
                | Self::RenameWorkspaceText
                | Self::RenameTabText
                | Self::CreateBranchText
                | Self::CommitText
                | Self::StashText
                | Self::AgentCommandText
                | Self::WorkspaceCommandText
                | Self::SettingsText
                | Self::SettingsKeyCapture
                | Self::NewAgentText
                | Self::ChooserNewText
        )
    }
}

/// Groups related actions into stable sections in the exhaustive help view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum HelpGroup {
    Discovery,
    Global,
    Home,
    RepositorySummary,
    Sidebar,
    Workspace,
    Terminal,
    TerminalTabs,
    GitLog,
    Branches,
    Review,
    Settings,
    Dialogs,
    SessionChooser,
}

impl HelpGroup {
    pub fn title(self) -> &'static str {
        match self {
            Self::Discovery => "Help",
            Self::Global => "Global",
            Self::Home => "Home",
            Self::RepositorySummary => "Repository summary",
            Self::Sidebar => "Sidebar and repository rail",
            Self::Workspace => "Workspace panes",
            Self::Terminal => "Terminal",
            Self::TerminalTabs => "Terminal tabs",
            Self::GitLog => "Git log",
            Self::Branches => "Branches",
            Self::Review => "Review",
            Self::Settings => "Settings",
            Self::Dialogs => "Dialogs",
            Self::SessionChooser => "Session chooser",
        }
    }
}

/// Marks the discovery surfaces on which a shortcut may be rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HintSurface {
    Footer,
    Modal,
    Help,
}

/// Selects a user-configurable binding from effective TUI settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingBinding {
    PreviousWorkspace,
    NextWorkspace,
    TerminalCommandMode,
    ScrollToBottom,
    Fullscreen,
}

/// Supplies either static aliases or one user-configurable binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingSource {
    Static(&'static [&'static str]),
    Setting(SettingBinding),
}

/// Describes selection-dependent visibility for discovery surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
    Always,
    SidebarSelection,
    SidebarWorkspace,
    SidebarRepository,
    SidebarRepositoryRow,
    HasSecondTab,
    LogFileContext,
    LogChangedPath,
    LogHasChanges,
    LocalBranchPane,
    SettingsAgentRow,
}

/// Defines one context-specific catalog entry for a semantic action.
#[derive(Debug, Clone, Copy)]
pub struct ShortcutSpec {
    pub id: ShortcutId,
    pub label: &'static str,
    pub contexts: &'static [ShortcutContext],
    pub bindings: BindingSource,
    pub help_group: HelpGroup,
    pub footer_priority: Option<u8>,
    pub surfaces: &'static [HintSurface],
    pub availability: Availability,
}

const F: &[HintSurface] = &[HintSurface::Footer, HintSurface::Help];
const M: &[HintSurface] = &[HintSurface::Modal, HintSurface::Help];
const FM: &[HintSurface] = &[HintSurface::Footer, HintSurface::Modal, HintSurface::Help];
const H: &[HintSurface] = &[HintSurface::Help];

macro_rules! shortcut {
    ($id:ident, $label:literal, [$($context:ident),+], $bindings:expr, $group:ident, $priority:expr, $surfaces:expr) => {
        ShortcutSpec {
            id: ShortcutId::$id,
            label: $label,
            contexts: &[$(ShortcutContext::$context),+],
            bindings: $bindings,
            help_group: HelpGroup::$group,
            footer_priority: $priority,
            surfaces: $surfaces,
            availability: Availability::Always,
        }
    };
    ($id:ident, $label:literal, [$($context:ident),+], $bindings:expr, $group:ident, $priority:expr, $surfaces:expr, $availability:ident) => {
        ShortcutSpec {
            id: ShortcutId::$id,
            label: $label,
            contexts: &[$(ShortcutContext::$context),+],
            bindings: $bindings,
            help_group: HelpGroup::$group,
            footer_priority: $priority,
            surfaces: $surfaces,
            availability: Availability::$availability,
        }
    };
}

const APP_SCREENS: &[ShortcutContext] = &[
    ShortcutContext::HomeSidebar,
    ShortcutContext::HomeRail,
    ShortcutContext::HomeRailPopout,
    ShortcutContext::RepoSummary,
    ShortcutContext::WorkspaceSidebar,
    ShortcutContext::WorkspaceRail,
    ShortcutContext::WorkspaceRailPopout,
    ShortcutContext::TerminalCommand,
    ShortcutContext::TerminalTabs,
    ShortcutContext::GitLogHeader,
    ShortcutContext::GitLogFile,
    ShortcutContext::GitLogDirectory,
    ShortcutContext::GitLogCommit,
    ShortcutContext::GitLogCommitFile,
    ShortcutContext::Branches,
    ShortcutContext::Diff,
    ShortcutContext::ReviewFiles,
    ShortcutContext::ReviewDiff,
];

const QUIT_CONTEXTS: &[ShortcutContext] = &[
    ShortcutContext::HomeSidebar,
    ShortcutContext::HomeRail,
    ShortcutContext::HomeRailPopout,
    ShortcutContext::RepoSummary,
    ShortcutContext::WorkspaceSidebar,
    ShortcutContext::WorkspaceRail,
    ShortcutContext::WorkspaceRailPopout,
    ShortcutContext::TerminalTabs,
    ShortcutContext::GitLogHeader,
    ShortcutContext::GitLogFile,
    ShortcutContext::GitLogDirectory,
    ShortcutContext::GitLogCommit,
    ShortcutContext::GitLogCommitFile,
    ShortcutContext::Branches,
    ShortcutContext::Diff,
    ShortcutContext::ReviewFiles,
    ShortcutContext::ReviewDiff,
];

const HELPABLE_CONTEXTS: &[ShortcutContext] = &[
    ShortcutContext::HomeSidebar,
    ShortcutContext::HomeRail,
    ShortcutContext::HomeRailPopout,
    ShortcutContext::RepoSummary,
    ShortcutContext::WorkspaceSidebar,
    ShortcutContext::WorkspaceRail,
    ShortcutContext::WorkspaceRailPopout,
    ShortcutContext::TerminalCommand,
    ShortcutContext::TerminalTabs,
    ShortcutContext::GitLogHeader,
    ShortcutContext::GitLogFile,
    ShortcutContext::GitLogDirectory,
    ShortcutContext::GitLogCommit,
    ShortcutContext::GitLogCommitFile,
    ShortcutContext::Branches,
    ShortcutContext::Diff,
    ShortcutContext::ReviewFiles,
    ShortcutContext::ReviewDiff,
    ShortcutContext::Settings,
    ShortcutContext::SettingsConfirmDeleteAgent,
    ShortcutContext::SshHistory,
    ShortcutContext::DirectoryBrowser,
    ShortcutContext::MovingWorkspace,
    ShortcutContext::DeleteConfirm,
    ShortcutContext::DiscardConfirm,
    ShortcutContext::DiscardAllConfirm,
    ShortcutContext::StashPullPopConfirm,
    ShortcutContext::DeleteBranchConfirm,
    ShortcutContext::AgentPicker,
    ShortcutContext::ChooserBrowse,
    ShortcutContext::ChooserDeleteConfirm,
];

/// Contains every discoverable, non-PTY application command.
pub static CATALOG: &[ShortcutSpec] = &[
    ShortcutSpec {
        id: ShortcutId::OpenHelp,
        label: "Help",
        contexts: HELPABLE_CONTEXTS,
        bindings: BindingSource::Static(&["?"]),
        help_group: HelpGroup::Discovery,
        footer_priority: Some(0),
        surfaces: F,
        availability: Availability::Always,
    },
    shortcut!(
        CloseHelp,
        "Close help",
        [Help],
        BindingSource::Static(&["esc", "?"]),
        Discovery,
        Some(0),
        H
    ),
    shortcut!(
        ToggleHelpView,
        "Current / All",
        [Help],
        BindingSource::Static(&["tab"]),
        Discovery,
        Some(1),
        H
    ),
    shortcut!(
        HelpScrollDown,
        "Scroll down",
        [Help],
        BindingSource::Static(&["j", "down"]),
        Discovery,
        Some(2),
        H
    ),
    shortcut!(
        HelpScrollUp,
        "Scroll up",
        [Help],
        BindingSource::Static(&["k", "up"]),
        Discovery,
        Some(2),
        H
    ),
    shortcut!(
        HelpPageDown,
        "Page down",
        [Help],
        BindingSource::Static(&["pagedown"]),
        Discovery,
        Some(3),
        H
    ),
    shortcut!(
        HelpPageUp,
        "Page up",
        [Help],
        BindingSource::Static(&["pageup"]),
        Discovery,
        Some(3),
        H
    ),
    shortcut!(
        HelpTop,
        "Top",
        [Help],
        BindingSource::Static(&["home"]),
        Discovery,
        None,
        H
    ),
    shortcut!(
        HelpBottom,
        "Bottom",
        [Help],
        BindingSource::Static(&["end"]),
        Discovery,
        None,
        H
    ),
    ShortcutSpec {
        id: ShortcutId::Quit,
        label: "Quit",
        contexts: QUIT_CONTEXTS,
        bindings: BindingSource::Static(&["q"]),
        help_group: HelpGroup::Global,
        footer_priority: Some(90),
        surfaces: F,
        availability: Availability::Always,
    },
    ShortcutSpec {
        id: ShortcutId::PreviousWorkspace,
        label: "Previous workspace",
        contexts: APP_SCREENS,
        bindings: BindingSource::Setting(SettingBinding::PreviousWorkspace),
        help_group: HelpGroup::Global,
        footer_priority: Some(70),
        surfaces: F,
        availability: Availability::Always,
    },
    shortcut!(
        PreviousWorkspace,
        "Previous workspace",
        [TerminalPassthrough],
        BindingSource::Setting(SettingBinding::PreviousWorkspace),
        Global,
        Some(20),
        F
    ),
    ShortcutSpec {
        id: ShortcutId::NextWorkspace,
        label: "Next workspace",
        contexts: APP_SCREENS,
        bindings: BindingSource::Setting(SettingBinding::NextWorkspace),
        help_group: HelpGroup::Global,
        footer_priority: Some(71),
        surfaces: F,
        availability: Availability::Always,
    },
    shortcut!(
        NextWorkspace,
        "Next workspace",
        [TerminalPassthrough],
        BindingSource::Setting(SettingBinding::NextWorkspace),
        Global,
        Some(21),
        F
    ),
    ShortcutSpec {
        id: ShortcutId::CycleSidebar,
        label: "Cycle sidebar",
        contexts: APP_SCREENS,
        bindings: BindingSource::Static(&["ctrl+b"]),
        help_group: HelpGroup::Global,
        footer_priority: Some(15),
        surfaces: F,
        availability: Availability::Always,
    },
    shortcut!(
        MoveDown,
        "Move down",
        [
            HomeSidebar,
            HomeRail,
            HomeRailPopout,
            RepoSummary,
            WorkspaceSidebar,
            WorkspaceRail,
            WorkspaceRailPopout,
            GitLogHeader,
            GitLogFile,
            GitLogDirectory,
            GitLogCommit,
            GitLogCommitFile,
            Branches,
            Diff,
            ReviewFiles,
            ReviewDiff,
            Settings,
            SshHistory,
            DirectoryBrowser,
            MovingWorkspace,
            AgentPicker,
            ChooserBrowse
        ],
        BindingSource::Static(&["j", "down"]),
        Global,
        Some(5),
        FM
    ),
    shortcut!(
        MoveUp,
        "Move up",
        [
            HomeSidebar,
            HomeRail,
            HomeRailPopout,
            RepoSummary,
            WorkspaceSidebar,
            WorkspaceRail,
            WorkspaceRailPopout,
            GitLogHeader,
            GitLogFile,
            GitLogDirectory,
            GitLogCommit,
            GitLogCommitFile,
            Branches,
            Diff,
            ReviewFiles,
            ReviewDiff,
            Settings,
            SshHistory,
            DirectoryBrowser,
            MovingWorkspace,
            AgentPicker,
            ChooserBrowse
        ],
        BindingSource::Static(&["k", "up"]),
        Global,
        Some(6),
        FM
    ),
    shortcut!(
        Open,
        "Open",
        [HomeSidebar, WorkspaceSidebar],
        BindingSource::Static(&["enter"]),
        Sidebar,
        Some(1),
        F,
        SidebarSelection
    ),
    shortcut!(
        Open,
        "Open",
        [HomeRailPopout, WorkspaceRailPopout],
        BindingSource::Static(&["enter", "l"]),
        Sidebar,
        Some(1),
        F,
        SidebarSelection
    ),
    shortcut!(
        Open,
        "Open",
        [RepoSummary],
        BindingSource::Static(&["enter", "l", "right"]),
        RepositorySummary,
        Some(1),
        F,
        SidebarSelection
    ),
    shortcut!(
        Open,
        "Open workspace list",
        [HomeRail, WorkspaceRail],
        BindingSource::Static(&["enter", "l", "right"]),
        Sidebar,
        Some(1),
        F,
        SidebarRepository
    ),
    shortcut!(
        Back,
        "Back",
        [RepoSummary],
        BindingSource::Static(&["esc", "h", "left"]),
        RepositorySummary,
        Some(2),
        F
    ),
    shortcut!(
        Back,
        "Home",
        [
            WorkspaceSidebar,
            WorkspaceRail,
            TerminalTabs,
            GitLogHeader,
            GitLogFile,
            GitLogDirectory,
            GitLogCommit,
            GitLogCommitFile,
            Branches,
            Diff
        ],
        BindingSource::Static(&["esc"]),
        Workspace,
        Some(30),
        F
    ),
    shortcut!(
        Back,
        "Unfocus terminal",
        [TerminalCommand],
        BindingSource::Static(&["esc"]),
        Terminal,
        Some(30),
        F
    ),
    shortcut!(
        NewWorkspace,
        "New workspace",
        [
            HomeSidebar,
            HomeRail,
            RepoSummary,
            WorkspaceSidebar,
            WorkspaceRail
        ],
        BindingSource::Static(&["n"]),
        Home,
        Some(10),
        F,
        SidebarRepository
    ),
    shortcut!(
        AddRepository,
        "Add repository",
        [HomeSidebar],
        BindingSource::Static(&["N", "a"]),
        Home,
        Some(40),
        F
    ),
    shortcut!(
        AddRepository,
        "Add repository",
        [HomeRail],
        BindingSource::Static(&["N"]),
        Home,
        Some(40),
        F
    ),
    shortcut!(
        AddSshRepository,
        "Add SSH repository",
        [HomeSidebar],
        BindingSource::Static(&["A"]),
        Home,
        Some(45),
        F
    ),
    shortcut!(
        Delete,
        "Delete",
        [
            HomeSidebar,
            HomeRail,
            HomeRailPopout,
            RepoSummary,
            WorkspaceSidebar,
            WorkspaceRail,
            WorkspaceRailPopout
        ],
        BindingSource::Static(&["D"]),
        Sidebar,
        Some(35),
        F,
        SidebarSelection
    ),
    shortcut!(
        Review,
        "Review workspace",
        [HomeSidebar, WorkspaceSidebar],
        BindingSource::Static(&["R"]),
        Sidebar,
        Some(25),
        F,
        SidebarWorkspace
    ),
    shortcut!(
        ToggleReady,
        "Toggle ready for review",
        [HomeSidebar, WorkspaceSidebar],
        BindingSource::Static(&["space"]),
        Sidebar,
        Some(26),
        F,
        SidebarWorkspace
    ),
    shortcut!(
        ToggleReviewFilter,
        "Filter ready workspaces",
        [HomeSidebar, HomeRail, WorkspaceSidebar, WorkspaceRail],
        BindingSource::Static(&["f"]),
        Sidebar,
        Some(50),
        F
    ),
    shortcut!(
        RefreshGit,
        "Refresh Git",
        [
            HomeSidebar,
            RepoSummary,
            WorkspaceSidebar,
            TerminalCommand,
            TerminalTabs,
            GitLogHeader,
            GitLogFile,
            GitLogDirectory,
            GitLogCommit,
            GitLogCommitFile,
            Branches,
            Diff
        ],
        BindingSource::Static(&["g"]),
        Global,
        Some(60),
        F
    ),
    shortcut!(
        OpenSettings,
        "Settings",
        [HomeSidebar, HomeRail],
        BindingSource::Static(&["S"]),
        Home,
        Some(80),
        F
    ),
    shortcut!(
        Collapse,
        "Collapse repository",
        [HomeSidebar, WorkspaceSidebar],
        BindingSource::Static(&["h", "left"]),
        Sidebar,
        Some(20),
        F,
        SidebarRepositoryRow
    ),
    shortcut!(
        Expand,
        "Expand repository",
        [HomeSidebar, WorkspaceSidebar],
        BindingSource::Static(&["right"]),
        Sidebar,
        Some(21),
        F,
        SidebarRepositoryRow
    ),
    shortcut!(
        ExpandOrOpen,
        "Expand repository / open workspace",
        [HomeSidebar, WorkspaceSidebar],
        BindingSource::Static(&["l"]),
        Sidebar,
        Some(22),
        F,
        SidebarSelection
    ),
    shortcut!(
        ClosePopout,
        "Close workspace list",
        [HomeRailPopout, WorkspaceRailPopout],
        BindingSource::Static(&["esc", "h", "left"]),
        Sidebar,
        Some(2),
        F
    ),
    shortcut!(
        Confirm,
        "Confirm",
        [DeleteConfirm],
        BindingSource::Static(&["y", "Y"]),
        Dialogs,
        Some(0),
        FM
    ),
    shortcut!(
        Cancel,
        "Cancel",
        [DeleteConfirm],
        BindingSource::Static(&["n", "N", "esc"]),
        Dialogs,
        Some(1),
        FM
    ),
    shortcut!(
        Confirm,
        "Confirm",
        [DiscardConfirm, DiscardAllConfirm, StashPullPopConfirm],
        BindingSource::Static(&["y", "enter"]),
        Dialogs,
        Some(0),
        FM
    ),
    shortcut!(
        Cancel,
        "Cancel",
        [DiscardConfirm, DiscardAllConfirm, StashPullPopConfirm],
        BindingSource::Static(&["n", "esc"]),
        Dialogs,
        Some(1),
        FM
    ),
    shortcut!(
        Confirm,
        "Delete branch",
        [DeleteBranchConfirm],
        BindingSource::Static(&["y"]),
        Dialogs,
        Some(0),
        FM
    ),
    shortcut!(
        Cancel,
        "Cancel",
        [DeleteBranchConfirm],
        BindingSource::Static(&["n", "esc", "enter"]),
        Dialogs,
        Some(1),
        FM
    ),
    shortcut!(
        Confirm,
        "Delete agent",
        [SettingsConfirmDeleteAgent],
        BindingSource::Static(&["y", "Y"]),
        Settings,
        Some(0),
        FM
    ),
    shortcut!(
        Cancel,
        "Keep agent",
        [SettingsConfirmDeleteAgent],
        BindingSource::Static(&["esc", "n"]),
        Settings,
        Some(1),
        FM
    ),
    shortcut!(
        Confirm,
        "Create",
        [QuickCreateText],
        BindingSource::Static(&["enter"]),
        Dialogs,
        Some(0),
        FM
    ),
    shortcut!(
        Cancel,
        "Cancel",
        [
            QuickCreateText,
            SshText,
            DirectoryPathText,
            RenameWorkspaceText,
            RenameTabText,
            CreateBranchText,
            CommitText,
            StashText,
            AgentCommandText,
            WorkspaceCommandText,
            NewAgentText
        ],
        BindingSource::Static(&["esc"]),
        Dialogs,
        Some(2),
        FM
    ),
    shortcut!(
        NextField,
        "More options / next field",
        [QuickCreateText],
        BindingSource::Static(&["tab"]),
        Dialogs,
        Some(1),
        FM
    ),
    shortcut!(
        MoveDown,
        "Next matching branch",
        [QuickCreateText],
        BindingSource::Static(&["down"]),
        Dialogs,
        Some(5),
        FM
    ),
    shortcut!(
        MoveUp,
        "Previous matching branch",
        [QuickCreateText],
        BindingSource::Static(&["up"]),
        Dialogs,
        Some(6),
        FM
    ),
    shortcut!(
        PreviousField,
        "Previous field",
        [QuickCreateText],
        BindingSource::Static(&["backtab"]),
        Dialogs,
        Some(3),
        M
    ),
    shortcut!(
        PreviousChoice,
        "Previous choice",
        [QuickCreateText],
        BindingSource::Static(&["left"]),
        Dialogs,
        Some(7),
        M
    ),
    shortcut!(
        NextChoice,
        "Next choice",
        [QuickCreateText],
        BindingSource::Static(&["right"]),
        Dialogs,
        Some(7),
        M
    ),
    shortcut!(
        Confirm,
        "Add SSH repository",
        [SshText],
        BindingSource::Static(&["enter"]),
        Dialogs,
        Some(0),
        FM
    ),
    shortcut!(
        NextField,
        "Next field",
        [SshText],
        BindingSource::Static(&["tab", "backtab"]),
        Dialogs,
        Some(1),
        FM
    ),
    shortcut!(
        Confirm,
        "Browse path",
        [DirectoryPathText],
        BindingSource::Static(&["enter"]),
        Dialogs,
        Some(0),
        FM
    ),
    shortcut!(
        NextField,
        "Complete path",
        [DirectoryPathText],
        BindingSource::Static(&["tab"]),
        Dialogs,
        Some(1),
        FM
    ),
    shortcut!(
        Confirm,
        "Rename",
        [RenameWorkspaceText, RenameTabText],
        BindingSource::Static(&["enter"]),
        Dialogs,
        Some(0),
        FM
    ),
    shortcut!(
        Confirm,
        "Create branch",
        [CreateBranchText],
        BindingSource::Static(&["enter"]),
        Dialogs,
        Some(0),
        FM
    ),
    shortcut!(
        Confirm,
        "Commit",
        [CommitText],
        BindingSource::Static(&["enter"]),
        Dialogs,
        Some(0),
        FM
    ),
    shortcut!(
        Confirm,
        "Stash",
        [StashText],
        BindingSource::Static(&["enter"]),
        Dialogs,
        Some(0),
        FM
    ),
    shortcut!(
        Confirm,
        "Switch agent",
        [AgentCommandText],
        BindingSource::Static(&["enter"]),
        Dialogs,
        Some(0),
        FM
    ),
    shortcut!(
        Confirm,
        "Next step",
        [NewAgentText],
        BindingSource::Static(&["enter"]),
        Settings,
        Some(0),
        FM
    ),
    shortcut!(
        Confirm,
        "Save value",
        [SettingsText],
        BindingSource::Static(&["enter"]),
        Settings,
        Some(0),
        FM
    ),
    shortcut!(
        Cancel,
        "Cancel edit",
        [SettingsText, SettingsKeyCapture],
        BindingSource::Static(&["esc"]),
        Settings,
        Some(1),
        FM
    ),
    shortcut!(
        MoveWorkspace,
        "Finish moving",
        [MovingWorkspace],
        BindingSource::Static(&["enter", "esc", "M"]),
        Home,
        Some(1),
        F
    ),
    shortcut!(
        ToggleSetting,
        "Edit / toggle",
        [Settings],
        BindingSource::Static(&["enter", "space"]),
        Settings,
        Some(1),
        F
    ),
    shortcut!(
        AdjustSettingLeft,
        "Previous value",
        [Settings],
        BindingSource::Static(&["h", "left"]),
        Settings,
        Some(2),
        F
    ),
    shortcut!(
        AdjustSettingRight,
        "Next value",
        [Settings],
        BindingSource::Static(&["l", "right"]),
        Settings,
        Some(3),
        F
    ),
    shortcut!(
        NewAgent,
        "New agent",
        [Settings],
        BindingSource::Static(&["n"]),
        Settings,
        Some(20),
        F,
        SettingsAgentRow
    ),
    shortcut!(
        DeleteAgent,
        "Delete agent",
        [Settings],
        BindingSource::Static(&["d"]),
        Settings,
        Some(21),
        F,
        SettingsAgentRow
    ),
    shortcut!(
        Cancel,
        "Close settings",
        [Settings],
        BindingSource::Static(&["esc", "S"]),
        Settings,
        Some(4),
        F
    ),
    shortcut!(
        Confirm,
        "Select",
        [SshHistory, AgentPicker],
        BindingSource::Static(&["enter"]),
        Dialogs,
        Some(1),
        FM
    ),
    shortcut!(
        SelectNewSsh,
        "New SSH target",
        [SshHistory],
        BindingSource::Static(&["n"]),
        Dialogs,
        Some(2),
        FM
    ),
    shortcut!(
        Cancel,
        "Cancel",
        [SshHistory, AgentPicker],
        BindingSource::Static(&["esc"]),
        Dialogs,
        Some(3),
        FM
    ),
    shortcut!(
        GoToParent,
        "Parent directory",
        [DirectoryBrowser],
        BindingSource::Static(&["backspace"]),
        Dialogs,
        Some(20),
        FM
    ),
    shortcut!(
        ToggleHidden,
        "Toggle hidden directories",
        [DirectoryBrowser],
        BindingSource::Static(&["."]),
        Dialogs,
        Some(21),
        FM
    ),
    shortcut!(
        EditPath,
        "Edit path",
        [DirectoryBrowser],
        BindingSource::Static(&["/"]),
        Dialogs,
        Some(22),
        FM
    ),
    shortcut!(
        EnterDirectory,
        "Enter directory",
        [DirectoryBrowser],
        BindingSource::Static(&["tab"]),
        Dialogs,
        Some(23),
        FM
    ),
    shortcut!(
        SelectDirectory,
        "Add selected directory",
        [DirectoryBrowser],
        BindingSource::Static(&["space"]),
        Dialogs,
        Some(24),
        FM
    ),
    shortcut!(
        Confirm,
        "Add repository",
        [DirectoryBrowser],
        BindingSource::Static(&["enter"]),
        Dialogs,
        Some(1),
        FM
    ),
    shortcut!(
        ToggleTerminalCommandMode,
        "Command mode",
        [TerminalPassthrough],
        BindingSource::Setting(SettingBinding::TerminalCommandMode),
        Terminal,
        Some(0),
        F
    ),
    shortcut!(
        ToggleTerminalCommandMode,
        "Return to terminal",
        [TerminalCommand],
        BindingSource::Setting(SettingBinding::TerminalCommandMode),
        Terminal,
        Some(0),
        F
    ),
    shortcut!(
        ScrollTerminalToBottom,
        "Scroll terminal to bottom",
        [
            TerminalCommand,
            TerminalTabs,
            GitLogHeader,
            GitLogFile,
            GitLogDirectory,
            GitLogCommit,
            GitLogCommitFile,
            Branches,
            Diff,
            ReviewFiles,
            ReviewDiff
        ],
        BindingSource::Setting(SettingBinding::ScrollToBottom),
        Terminal,
        Some(50),
        F
    ),
    shortcut!(
        ToggleFullscreen,
        "Toggle terminal fullscreen",
        [
            TerminalCommand,
            TerminalTabs,
            GitLogHeader,
            GitLogFile,
            GitLogDirectory,
            GitLogCommit,
            GitLogCommitFile,
            Branches,
            Diff,
            ReviewFiles,
            ReviewDiff
        ],
        BindingSource::Setting(SettingBinding::Fullscreen),
        Terminal,
        Some(12),
        F
    ),
    shortcut!(
        ToggleYolo,
        "Toggle agent YOLO mode",
        [
            TerminalCommand,
            TerminalTabs,
            GitLogHeader,
            GitLogFile,
            GitLogDirectory,
            GitLogCommit,
            GitLogCommitFile,
            Branches,
            Diff
        ],
        BindingSource::Static(&["Y"]),
        Workspace,
        Some(55),
        F
    ),
    shortcut!(
        NextPane,
        "Next pane",
        [
            TerminalCommand,
            TerminalTabs,
            GitLogHeader,
            GitLogFile,
            GitLogDirectory,
            GitLogCommit,
            GitLogCommitFile,
            Branches,
            Diff
        ],
        BindingSource::Static(&["tab"]),
        Workspace,
        Some(10),
        F
    ),
    shortcut!(
        PreviousPane,
        "Previous pane",
        [
            TerminalCommand,
            TerminalTabs,
            GitLogHeader,
            GitLogFile,
            GitLogDirectory,
            GitLogCommit,
            GitLogCommitFile,
            Branches,
            Diff
        ],
        BindingSource::Static(&["backtab", "shift+tab"]),
        Workspace,
        Some(11),
        F
    ),
    shortcut!(
        OpenWorkspaceCommand,
        "Run workspace command",
        [
            TerminalCommand,
            GitLogHeader,
            GitLogFile,
            GitLogDirectory,
            GitLogCommit,
            GitLogCommitFile,
            Branches,
            Diff
        ],
        BindingSource::Static(&[":"]),
        Workspace,
        Some(25),
        F
    ),
    shortcut!(
        SelectFirstTab,
        "Select first terminal tab",
        [
            TerminalCommand,
            TerminalTabs,
            GitLogHeader,
            GitLogFile,
            GitLogDirectory,
            GitLogCommit,
            GitLogCommitFile,
            Branches,
            Diff
        ],
        BindingSource::Static(&["1"]),
        TerminalTabs,
        Some(65),
        F
    ),
    shortcut!(
        SelectSecondTab,
        "Select second terminal tab",
        [
            TerminalCommand,
            TerminalTabs,
            GitLogHeader,
            GitLogFile,
            GitLogDirectory,
            GitLogCommit,
            GitLogCommitFile,
            Branches,
            Diff
        ],
        BindingSource::Static(&["2"]),
        TerminalTabs,
        Some(66),
        F,
        HasSecondTab
    ),
    shortcut!(
        NextTerminalTab,
        "Next terminal tab",
        [TerminalTabs],
        BindingSource::Static(&["l", "right"]),
        TerminalTabs,
        Some(0),
        F
    ),
    shortcut!(
        PreviousTerminalTab,
        "Previous terminal tab",
        [TerminalTabs],
        BindingSource::Static(&["h", "left"]),
        TerminalTabs,
        Some(0),
        F
    ),
    shortcut!(
        NewShellTab,
        "New shell tab",
        [TerminalTabs],
        BindingSource::Static(&["n"]),
        TerminalTabs,
        Some(2),
        F
    ),
    shortcut!(
        CloseTerminalTab,
        "Close active tab",
        [TerminalTabs],
        BindingSource::Static(&["x"]),
        TerminalTabs,
        Some(3),
        F
    ),
    shortcut!(
        RenameTerminalTab,
        "Rename active tab",
        [TerminalTabs],
        BindingSource::Static(&["r"]),
        TerminalTabs,
        Some(4),
        F
    ),
    shortcut!(
        SwitchAgent,
        "Switch agent",
        [TerminalTabs],
        BindingSource::Static(&["c"]),
        TerminalTabs,
        Some(5),
        F
    ),
    shortcut!(
        StartActiveTerminal,
        "Start active terminal",
        [TerminalTabs],
        BindingSource::Static(&["a", "s"]),
        TerminalTabs,
        Some(6),
        F
    ),
    shortcut!(
        StopActiveTerminal,
        "Stop active terminal",
        [TerminalTabs],
        BindingSource::Static(&["A", "S"]),
        TerminalTabs,
        Some(7),
        F
    ),
    shortcut!(
        ToggleLogItem,
        "Expand / open diff",
        [
            GitLogHeader,
            GitLogFile,
            GitLogDirectory,
            GitLogCommit,
            GitLogCommitFile
        ],
        BindingSource::Static(&["enter"]),
        GitLog,
        Some(1),
        F
    ),
    shortcut!(
        ToggleStage,
        "Stage / unstage",
        [GitLogFile, GitLogDirectory],
        BindingSource::Static(&["space"]),
        GitLog,
        Some(2),
        F,
        LogChangedPath
    ),
    shortcut!(
        StageAll,
        "Stage all",
        [GitLogHeader, GitLogFile, GitLogDirectory],
        BindingSource::Static(&["+"]),
        GitLog,
        Some(3),
        F,
        LogFileContext
    ),
    shortcut!(
        UnstageAll,
        "Unstage all",
        [GitLogHeader, GitLogFile, GitLogDirectory],
        BindingSource::Static(&["-"]),
        GitLog,
        Some(4),
        F,
        LogFileContext
    ),
    shortcut!(
        Commit,
        "Commit staged changes",
        [GitLogHeader, GitLogFile, GitLogDirectory],
        BindingSource::Static(&["c"]),
        GitLog,
        Some(5),
        F,
        LogFileContext
    ),
    shortcut!(
        Discard,
        "Discard selected changes",
        [GitLogFile, GitLogDirectory],
        BindingSource::Static(&["d"]),
        GitLog,
        Some(6),
        F,
        LogChangedPath
    ),
    shortcut!(
        DiscardAll,
        "Discard all changes",
        [GitLogHeader, GitLogFile, GitLogDirectory],
        BindingSource::Static(&["D"]),
        GitLog,
        Some(7),
        F,
        LogHasChanges
    ),
    shortcut!(
        Stash,
        "Stash with message",
        [GitLogHeader, GitLogFile, GitLogDirectory],
        BindingSource::Static(&["s"]),
        GitLog,
        Some(8),
        F,
        LogFileContext
    ),
    shortcut!(
        StashAll,
        "Stash all",
        [GitLogHeader, GitLogFile, GitLogDirectory],
        BindingSource::Static(&["S"]),
        GitLog,
        Some(9),
        F,
        LogHasChanges
    ),
    shortcut!(
        ToggleTags,
        "Toggle tag filter",
        [
            GitLogHeader,
            GitLogFile,
            GitLogDirectory,
            GitLogCommit,
            GitLogCommitFile
        ],
        BindingSource::Static(&["t"]),
        GitLog,
        Some(10),
        F
    ),
    shortcut!(
        SelectLocalBranches,
        "Local branches",
        [Branches],
        BindingSource::Static(&["["]),
        Branches,
        Some(2),
        F
    ),
    shortcut!(
        SelectRemoteBranches,
        "Remote branches",
        [Branches],
        BindingSource::Static(&["]"]),
        Branches,
        Some(2),
        F
    ),
    shortcut!(
        CheckoutBranch,
        "Check out branch",
        [Branches],
        BindingSource::Static(&["space"]),
        Branches,
        Some(1),
        F
    ),
    shortcut!(
        CreateBranch,
        "Create branch",
        [Branches],
        BindingSource::Static(&["c"]),
        Branches,
        Some(3),
        F,
        LocalBranchPane
    ),
    shortcut!(
        DeleteBranch,
        "Delete branch",
        [Branches],
        BindingSource::Static(&["D"]),
        Branches,
        Some(4),
        F
    ),
    shortcut!(
        Pull,
        "Pull",
        [Branches],
        BindingSource::Static(&["p"]),
        Branches,
        Some(5),
        F
    ),
    shortcut!(
        Fetch,
        "Fetch",
        [Branches],
        BindingSource::Static(&["f"]),
        Branches,
        Some(6),
        F
    ),
    shortcut!(
        Push,
        "Push",
        [Branches],
        BindingSource::Static(&["P"]),
        Branches,
        Some(7),
        F
    ),
    shortcut!(
        Push,
        "Push",
        [ReviewFiles, ReviewDiff],
        BindingSource::Static(&["P"]),
        Review,
        Some(7),
        F
    ),
    shortcut!(
        ToggleReviewPane,
        "Switch review pane",
        [ReviewFiles, ReviewDiff],
        BindingSource::Static(&["tab"]),
        Review,
        Some(1),
        F
    ),
    shortcut!(
        OpenReviewFile,
        "Load selected file diff",
        [ReviewFiles],
        BindingSource::Static(&["enter"]),
        Review,
        Some(2),
        F
    ),
    shortcut!(
        ReviewPageDown,
        "Scroll diff down 10",
        [ReviewFiles, ReviewDiff],
        BindingSource::Static(&["J"]),
        Review,
        Some(4),
        F
    ),
    shortcut!(
        ReviewPageUp,
        "Scroll diff up 10",
        [ReviewFiles, ReviewDiff],
        BindingSource::Static(&["K"]),
        Review,
        Some(5),
        F
    ),
    shortcut!(
        OpenPullRequest,
        "Open pull request",
        [ReviewFiles, ReviewDiff],
        BindingSource::Static(&["O"]),
        Review,
        Some(3),
        F
    ),
    shortcut!(
        Back,
        "Close review",
        [ReviewFiles, ReviewDiff],
        BindingSource::Static(&["esc"]),
        Review,
        Some(0),
        F
    ),
    shortcut!(
        RunCommand,
        "Run command",
        [WorkspaceCommandText],
        BindingSource::Static(&["enter"]),
        Dialogs,
        Some(0),
        FM
    ),
    shortcut!(
        OutputUp,
        "Scroll output up",
        [WorkspaceCommandText],
        BindingSource::Static(&["up"]),
        Dialogs,
        Some(12),
        M
    ),
    shortcut!(
        OutputDown,
        "Scroll output down",
        [WorkspaceCommandText],
        BindingSource::Static(&["down"]),
        Dialogs,
        Some(12),
        M
    ),
    shortcut!(
        OutputPageUp,
        "Page output up",
        [WorkspaceCommandText],
        BindingSource::Static(&["pageup"]),
        Dialogs,
        Some(13),
        M
    ),
    shortcut!(
        OutputPageDown,
        "Page output down",
        [WorkspaceCommandText],
        BindingSource::Static(&["pagedown"]),
        Dialogs,
        Some(13),
        M
    ),
    shortcut!(
        Confirm,
        "Resume command",
        [ResurrectConfirm],
        BindingSource::Static(&["enter"]),
        Dialogs,
        Some(0),
        M
    ),
    shortcut!(
        Cancel,
        "Open shell",
        [ResurrectConfirm],
        BindingSource::Static(&["esc"]),
        Dialogs,
        Some(1),
        M
    ),
    shortcut!(
        AttachSession,
        "Attach or revive",
        [ChooserBrowse],
        BindingSource::Static(&["enter"]),
        SessionChooser,
        Some(0),
        F
    ),
    shortcut!(
        NewSession,
        "New session",
        [ChooserBrowse],
        BindingSource::Static(&["n"]),
        SessionChooser,
        Some(2),
        F
    ),
    shortcut!(
        DeleteSession,
        "Delete session",
        [ChooserBrowse],
        BindingSource::Static(&["d", "delete"]),
        SessionChooser,
        Some(3),
        F
    ),
    shortcut!(
        RefreshSessions,
        "Refresh sessions",
        [ChooserBrowse],
        BindingSource::Static(&["r"]),
        SessionChooser,
        Some(4),
        F
    ),
    shortcut!(
        Quit,
        "Quit",
        [ChooserBrowse],
        BindingSource::Static(&["q", "esc"]),
        SessionChooser,
        Some(90),
        F
    ),
    shortcut!(
        Confirm,
        "Delete session",
        [ChooserDeleteConfirm],
        BindingSource::Static(&["y", "Y"]),
        SessionChooser,
        Some(0),
        FM
    ),
    shortcut!(
        Cancel,
        "Cancel",
        [ChooserDeleteConfirm],
        BindingSource::Static(&["n", "N", "esc"]),
        SessionChooser,
        Some(1),
        FM
    ),
    shortcut!(
        Confirm,
        "Create and attach",
        [ChooserNewText],
        BindingSource::Static(&["enter"]),
        SessionChooser,
        Some(0),
        M
    ),
    shortcut!(
        Cancel,
        "Cancel",
        [ChooserNewText],
        BindingSource::Static(&["esc"]),
        SessionChooser,
        Some(1),
        M
    ),
];

/// Selects the contextual or exhaustive help view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpView {
    Current,
    All,
}

/// Stores an open help overlay's captured context, view, and scroll position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpState {
    pub view: HelpView,
    pub context: ShortcutContext,
    pub scroll: u16,
}

impl HelpState {
    pub fn new(context: ShortcutContext) -> Self {
        Self {
            view: HelpView::Current,
            context,
            scroll: 0,
        }
    }

    pub fn toggle_view(&mut self) {
        self.view = match self.view {
            HelpView::Current => HelpView::All,
            HelpView::All => HelpView::Current,
        };
        self.scroll = 0;
    }
}

/// Opens help for an eligible context or applies navigation to an open overlay.
pub fn handle_help_input(
    help: &mut Option<HelpState>,
    settings: &Settings,
    context: ShortcutContext,
    key: KeyEvent,
) -> bool {
    if help.is_none() {
        if context.help_available()
            && match_shortcut_for_context(settings, context, key) == Some(ShortcutId::OpenHelp)
        {
            *help = Some(HelpState::new(context));
            return true;
        }
        return false;
    }

    let action = match_shortcut_for_context(settings, ShortcutContext::Help, key);
    if action == Some(ShortcutId::CloseHelp) {
        *help = None;
        return true;
    }

    let help = help.as_mut().expect("help state checked above");
    match action {
        Some(ShortcutId::ToggleHelpView) => help.toggle_view(),
        Some(ShortcutId::HelpScrollDown) => help.scroll = help.scroll.saturating_add(1),
        Some(ShortcutId::HelpScrollUp) => help.scroll = help.scroll.saturating_sub(1),
        Some(ShortcutId::HelpPageDown) => help.scroll = help.scroll.saturating_add(10),
        Some(ShortcutId::HelpPageUp) => help.scroll = help.scroll.saturating_sub(10),
        Some(ShortcutId::HelpTop) => help.scroll = 0,
        Some(ShortcutId::HelpBottom) => help.scroll = u16::MAX,
        _ => {}
    }
    true
}

/// Contains effective display metadata for one context-resolved shortcut.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedShortcut {
    pub id: ShortcutId,
    pub label: &'static str,
    pub keys: String,
    pub group: HelpGroup,
    pub footer_priority: Option<u8>,
    pub surfaces: &'static [HintSurface],
}

/// Contains one titled section in the Current or All help view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpSection {
    pub title: String,
    pub shortcuts: Vec<ResolvedShortcut>,
}

fn overlay_context(app: &TuiApp) -> Option<ShortcutContext> {
    if app.settings_open {
        if app.confirming_delete_agent {
            return Some(ShortcutContext::SettingsConfirmDeleteAgent);
        }
        if app.new_agent_wizard.is_some() {
            return Some(ShortcutContext::NewAgentText);
        }
        if app.is_editing_keybind() {
            return Some(ShortcutContext::SettingsKeyCapture);
        }
        if app.is_editing_setting() {
            return Some(ShortcutContext::SettingsText);
        }
        return Some(ShortcutContext::Settings);
    }
    if app.is_confirming_delete() {
        return Some(ShortcutContext::DeleteConfirm);
    }
    if app.ssh_history_picker.is_some() {
        return Some(ShortcutContext::SshHistory);
    }
    if app.ssh_workspace_input.is_some() {
        return Some(ShortcutContext::SshText);
    }
    if let Some(browser) = &app.dir_browser {
        return Some(if browser.editing_path {
            ShortcutContext::DirectoryPathText
        } else {
            ShortcutContext::DirectoryBrowser
        });
    }
    if app.rename_workspace_input.is_some() {
        return Some(ShortcutContext::RenameWorkspaceText);
    }
    if app.moving_workspace {
        return Some(ShortcutContext::MovingWorkspace);
    }
    if app.quick_create.is_some() {
        return Some(ShortcutContext::QuickCreateText);
    }
    if app.workspace_command().is_some() {
        return Some(ShortcutContext::WorkspaceCommandText);
    }
    if app.confirm_discard_file.is_some() {
        return Some(ShortcutContext::DiscardConfirm);
    }
    if app.confirm_discard_all.is_some() {
        return Some(ShortcutContext::DiscardAllConfirm);
    }
    if app.confirm_stash_pull_pop.is_some() {
        return Some(ShortcutContext::StashPullPopConfirm);
    }
    if app.confirm_delete_branch.is_some() {
        return Some(ShortcutContext::DeleteBranchConfirm);
    }
    if app.rename_tab_input.is_some() {
        return Some(ShortcutContext::RenameTabText);
    }
    if app.create_branch_input.is_some() {
        return Some(ShortcutContext::CreateBranchText);
    }
    if app.commit_input.is_some() {
        return Some(ShortcutContext::CommitText);
    }
    if app.stash_input.is_some() {
        return Some(ShortcutContext::StashText);
    }
    if let Some(picker) = &app.agent_picker {
        return Some(if picker.custom_input.is_some() {
            ShortcutContext::AgentCommandText
        } else {
            ShortcutContext::AgentPicker
        });
    }
    None
}

/// Resolves the leaf shortcut context in the same precedence order as input handling.
pub fn underlying_context(app: &TuiApp) -> ShortcutContext {
    if let Some(context) = overlay_context(app) {
        return context;
    }
    if matches!(app.focus, Focus::ReviewFiles) {
        return ShortcutContext::ReviewFiles;
    }
    if matches!(app.focus, Focus::ReviewDiff) {
        return ShortcutContext::ReviewDiff;
    }

    match app.route {
        Route::Home => sidebar_context(app, false),
        Route::Repo { .. } => ShortcutContext::RepoSummary,
        Route::Workspace { .. } => match app.focus {
            Focus::Sidebar => sidebar_context(app, true),
            Focus::WsTerminal if !app.terminal_command_mode() => {
                if app.pending_resurrect_command().is_some() {
                    ShortcutContext::ResurrectConfirm
                } else {
                    ShortcutContext::TerminalPassthrough
                }
            }
            Focus::WsTerminal => ShortcutContext::TerminalCommand,
            Focus::WsTerminalTabs => ShortcutContext::TerminalTabs,
            Focus::WsBranches => ShortcutContext::Branches,
            Focus::WsDiff => ShortcutContext::Diff,
            Focus::WsLog => match app.log_item_at(app.ws_selected_commit) {
                LogItem::UncommittedHeader => ShortcutContext::GitLogHeader,
                LogItem::ChangedFile(_) => ShortcutContext::GitLogFile,
                LogItem::ChangedDirectory(_) => ShortcutContext::GitLogDirectory,
                LogItem::Commit(_) => ShortcutContext::GitLogCommit,
                LogItem::CommitFile(_, _) => ShortcutContext::GitLogCommitFile,
            },
            Focus::ReviewFiles => ShortcutContext::ReviewFiles,
            Focus::ReviewDiff => ShortcutContext::ReviewDiff,
        },
    }
}

fn sidebar_context(app: &TuiApp, workspace_route: bool) -> ShortcutContext {
    match (
        workspace_route,
        app.sidebar_mode,
        app.sidebar_popout.is_some(),
    ) {
        (false, SidebarMode::Rail, true) => ShortcutContext::HomeRailPopout,
        (false, SidebarMode::Rail, false) => ShortcutContext::HomeRail,
        (false, _, _) => ShortcutContext::HomeSidebar,
        (true, SidebarMode::Rail, true) => ShortcutContext::WorkspaceRailPopout,
        (true, SidebarMode::Rail, false) => ShortcutContext::WorkspaceRail,
        (true, _, _) => ShortcutContext::WorkspaceSidebar,
    }
}

pub fn active_context(app: &TuiApp) -> ShortcutContext {
    if app.help.is_some() {
        ShortcutContext::Help
    } else {
        underlying_context(app)
    }
}

pub fn resolved_shortcuts(app: &TuiApp, context: ShortcutContext) -> Vec<ResolvedShortcut> {
    resolved_shortcuts_with_settings(Some(app), &app.settings, context)
}

pub fn resolved_shortcuts_for_context(
    settings: &Settings,
    context: ShortcutContext,
) -> Vec<ResolvedShortcut> {
    resolved_shortcuts_with_settings(None, settings, context)
}

fn resolved_shortcuts_with_settings(
    app: Option<&TuiApp>,
    settings: &Settings,
    context: ShortcutContext,
) -> Vec<ResolvedShortcut> {
    CATALOG
        .iter()
        .filter(|spec| spec.contexts.contains(&context))
        .filter(|spec| is_available(spec.availability, context, app))
        .map(|spec| ResolvedShortcut {
            id: spec.id,
            label: spec.label,
            keys: format_bindings(spec.bindings, settings),
            group: spec.help_group,
            footer_priority: spec.footer_priority,
            surfaces: spec.surfaces,
        })
        .collect()
}

fn is_available(
    availability: Availability,
    context: ShortcutContext,
    app: Option<&TuiApp>,
) -> bool {
    let Some(app) = app else {
        return matches!(availability, Availability::Always);
    };
    match availability {
        Availability::Always => true,
        Availability::SidebarSelection => match context {
            ShortcutContext::HomeRailPopout | ShortcutContext::WorkspaceRailPopout => {
                app.selected_popout_workspace_id().is_some()
            }
            ShortcutContext::HomeRail | ShortcutContext::WorkspaceRail => {
                app.selected_rail_repo().is_some()
            }
            ShortcutContext::RepoSummary => app.selected_repo_summary_workspace_id().is_some(),
            ShortcutContext::HomeSidebar | ShortcutContext::WorkspaceSidebar => {
                app.selected_sidebar_row().is_some()
            }
            _ => false,
        },
        Availability::SidebarWorkspace => app.selected_sidebar_workspace_id().is_some(),
        Availability::SidebarRepository => match context {
            ShortcutContext::HomeRail | ShortcutContext::WorkspaceRail => {
                app.selected_rail_repo().is_some()
            }
            ShortcutContext::RepoSummary => app.repo_summary_repo_id().is_some(),
            ShortcutContext::HomeSidebar | ShortcutContext::WorkspaceSidebar => {
                app.sidebar_context_repo().is_some()
            }
            _ => false,
        },
        Availability::SidebarRepositoryRow => matches!(
            app.selected_sidebar_row(),
            Some(crate::app::SidebarRow::Repo(_))
        ),
        Availability::HasSecondTab => app.ws_tabs.len() > 1,
        Availability::LogFileContext => app.log_item_is_file_context(),
        Availability::LogChangedPath => matches!(
            app.log_item_at(app.ws_selected_commit),
            LogItem::ChangedFile(_) | LogItem::ChangedDirectory(_)
        ),
        Availability::LogHasChanges => app
            .active_workspace_id()
            .and_then(|id| app.workspace_git.get(&id))
            .is_some_and(|git| !git.changed.is_empty()),
        Availability::LocalBranchPane => app.ws_branch_sub_pane == BranchSubPane::Local,
        Availability::SettingsAgentRow => app.settings_selected == 0,
    }
}

pub fn match_shortcut(app: &TuiApp, context: ShortcutContext, key: KeyEvent) -> Option<ShortcutId> {
    match_shortcut_with_settings(Some(app), &app.settings, context, key)
}

pub fn match_shortcut_for_context(
    settings: &Settings,
    context: ShortcutContext,
    key: KeyEvent,
) -> Option<ShortcutId> {
    match_shortcut_with_settings(None, settings, context, key)
}

fn match_shortcut_with_settings(
    app: Option<&TuiApp>,
    settings: &Settings,
    context: ShortcutContext,
    key: KeyEvent,
) -> Option<ShortcutId> {
    CATALOG
        .iter()
        .filter(|spec| spec.contexts.contains(&context))
        .filter(|spec| is_available(spec.availability, context, app))
        .find(|spec| {
            resolved_binding_strings(spec.bindings, settings)
                .iter()
                .any(|b| key_matches(key, b))
        })
        .map(|spec| spec.id)
}

pub fn format_bindings(source: BindingSource, settings: &Settings) -> String {
    let mut formatted = Vec::new();
    for binding in resolved_binding_strings(source, settings) {
        let binding = format_binding(binding);
        if !formatted.contains(&binding) {
            formatted.push(binding);
        }
    }
    formatted.join("/")
}

fn resolved_binding_strings(source: BindingSource, settings: &Settings) -> Vec<&str> {
    match source {
        BindingSource::Static(bindings) => bindings.to_vec(),
        BindingSource::Setting(setting) => vec![match setting {
            SettingBinding::PreviousWorkspace => &settings.prev_workspace_key,
            SettingBinding::NextWorkspace => &settings.next_workspace_key,
            SettingBinding::TerminalCommandMode => &settings.passthrough_key,
            SettingBinding::ScrollToBottom => &settings.scroll_to_bottom_key,
            SettingBinding::Fullscreen => &settings.terminal_fullscreen_key,
        }],
    }
}

pub fn format_binding(binding: &str) -> String {
    if binding == "+" {
        return "+".to_string();
    }
    let mut parts = binding.split('+').collect::<Vec<_>>();
    if parts
        .last()
        .is_some_and(|part| part.trim().eq_ignore_ascii_case("backtab"))
    {
        parts.pop();
        if !parts
            .iter()
            .any(|part| part.trim().eq_ignore_ascii_case("shift"))
        {
            parts.push("shift");
        }
        parts.push("tab");
    }
    let has_modifiers = parts.len() > 1;
    parts
        .into_iter()
        .map(|part| match part.trim().to_ascii_lowercase().as_str() {
            "ctrl" | "control" => "Ctrl".to_string(),
            "alt" => "Alt".to_string(),
            "shift" => "Shift".to_string(),
            "enter" | "return" => "Enter".to_string(),
            "esc" | "escape" => "Esc".to_string(),
            "tab" => "Tab".to_string(),
            "backtab" => "Shift+Tab".to_string(),
            "space" => "Space".to_string(),
            "backspace" => "Bksp".to_string(),
            "delete" | "del" => "Del".to_string(),
            "insert" | "ins" => "Ins".to_string(),
            "pageup" | "pgup" => "PgUp".to_string(),
            "pagedown" | "pgdn" => "PgDn".to_string(),
            "up" => "↑".to_string(),
            "down" => "↓".to_string(),
            "left" => "←".to_string(),
            "right" => "→".to_string(),
            "home" => "Home".to_string(),
            "end" => "End".to_string(),
            _ if part.len() == 1 && has_modifiers => part.to_ascii_uppercase(),
            _ if part.len() == 1 => part.to_string(),
            _ => {
                let mut chars = part.chars();
                match chars.next() {
                    Some(first) => first.to_uppercase().chain(chars).collect(),
                    None => String::new(),
                }
            }
        })
        .collect::<Vec<_>>()
        .join("+")
}

fn key_matches(key: KeyEvent, binding: &str) -> bool {
    let Some((expected_code, expected_modifiers)) = parse_binding(binding) else {
        return false;
    };
    let mut actual_modifiers = key.modifiers
        & (KeyModifiers::SHIFT | KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER);
    if matches!(key.code, KeyCode::Char(c) if c.is_ascii_uppercase()) {
        actual_modifiers.insert(KeyModifiers::SHIFT);
    }
    let actual_code = if key.code == KeyCode::BackTab {
        actual_modifiers.insert(KeyModifiers::SHIFT);
        KeyCode::Tab
    } else {
        key.code
    };
    let mut consumed_shift = false;
    let codes_match = match (actual_code, expected_code) {
        (KeyCode::Char(actual), KeyCode::Char(expected)) => {
            if actual.eq_ignore_ascii_case(&expected) {
                true
            } else if actual_modifiers.contains(KeyModifiers::SHIFT)
                && shifted_symbol(actual) == Some(expected)
            {
                consumed_shift = true;
                true
            } else {
                false
            }
        }
        (actual, expected) => actual == expected,
    };
    if !codes_match {
        return false;
    }
    if consumed_shift {
        actual_modifiers.remove(KeyModifiers::SHIFT);
    }
    if actual_modifiers == expected_modifiers {
        return true;
    }
    matches!(key.code, KeyCode::Char(c) if "?+:_{}|<>\"".contains(c))
        && actual_modifiers == (expected_modifiers | KeyModifiers::SHIFT)
}

fn shifted_symbol(c: char) -> Option<char> {
    Some(match c {
        '`' => '~',
        '1' => '!',
        '2' => '@',
        '3' => '#',
        '4' => '$',
        '5' => '%',
        '6' => '^',
        '7' => '&',
        '8' => '*',
        '9' => '(',
        '0' => ')',
        '-' => '_',
        '=' => '+',
        '[' => '{',
        ']' => '}',
        '\\' => '|',
        ';' => ':',
        '\'' => '"',
        ',' => '<',
        '.' => '>',
        '/' => '?',
        _ => return None,
    })
}

fn parse_binding(binding: &str) -> Option<(KeyCode, KeyModifiers)> {
    if binding == "+" {
        return Some((KeyCode::Char('+'), KeyModifiers::empty()));
    }
    let parts = binding.trim().split('+').map(str::trim).collect::<Vec<_>>();
    let (key_name, modifiers) = parts.split_last()?;
    let mut expected_modifiers = KeyModifiers::empty();
    for modifier in modifiers {
        match modifier.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => expected_modifiers.insert(KeyModifiers::CONTROL),
            "alt" => expected_modifiers.insert(KeyModifiers::ALT),
            "shift" => expected_modifiers.insert(KeyModifiers::SHIFT),
            "super" => expected_modifiers.insert(KeyModifiers::SUPER),
            _ => return None,
        }
    }
    let code = if key_name.chars().count() == 1 {
        let c = key_name.chars().next()?;
        if c.is_ascii_uppercase() {
            expected_modifiers.insert(KeyModifiers::SHIFT);
        }
        KeyCode::Char(c.to_ascii_lowercase())
    } else {
        match key_name.to_ascii_lowercase().as_str() {
            "enter" | "return" => KeyCode::Enter,
            "esc" | "escape" => KeyCode::Esc,
            "tab" => KeyCode::Tab,
            "backtab" => {
                expected_modifiers.insert(KeyModifiers::SHIFT);
                KeyCode::Tab
            }
            "space" => KeyCode::Char(' '),
            "backspace" => KeyCode::Backspace,
            "delete" | "del" => KeyCode::Delete,
            "insert" | "ins" => KeyCode::Insert,
            "pageup" | "pgup" => KeyCode::PageUp,
            "pagedown" | "pgdn" => KeyCode::PageDown,
            "up" => KeyCode::Up,
            "down" => KeyCode::Down,
            "left" => KeyCode::Left,
            "right" => KeyCode::Right,
            "home" => KeyCode::Home,
            "end" => KeyCode::End,
            function if function.starts_with('f') => KeyCode::F(function[1..].parse().ok()?),
            _ => return None,
        }
    };
    Some((code, expected_modifiers))
}

pub fn current_help_sections(app: &TuiApp, context: ShortcutContext) -> Vec<HelpSection> {
    vec![HelpSection {
        title: context.title().to_string(),
        shortcuts: resolved_shortcuts(app, context),
    }]
}

pub fn all_help_sections(settings: &Settings) -> Vec<HelpSection> {
    let groups = [
        HelpGroup::Discovery,
        HelpGroup::Global,
        HelpGroup::Home,
        HelpGroup::RepositorySummary,
        HelpGroup::Sidebar,
        HelpGroup::Workspace,
        HelpGroup::Terminal,
        HelpGroup::TerminalTabs,
        HelpGroup::GitLog,
        HelpGroup::Branches,
        HelpGroup::Review,
        HelpGroup::Settings,
        HelpGroup::Dialogs,
        HelpGroup::SessionChooser,
    ];
    groups
        .into_iter()
        .filter_map(|group| {
            let mut shortcuts = Vec::new();
            for spec in CATALOG.iter().filter(|spec| {
                spec.help_group == group && spec.surfaces.contains(&HintSurface::Help)
            }) {
                let keys = format_bindings(spec.bindings, settings);
                if shortcuts.iter().any(|shortcut: &ResolvedShortcut| {
                    shortcut.id == spec.id && shortcut.label == spec.label && shortcut.keys == keys
                }) {
                    continue;
                }
                shortcuts.push(ResolvedShortcut {
                    id: spec.id,
                    label: spec.label,
                    keys,
                    group,
                    footer_priority: spec.footer_priority,
                    surfaces: spec.surfaces,
                });
            }
            (!shortcuts.is_empty()).then(|| HelpSection {
                title: group.title().to_string(),
                shortcuts,
            })
        })
        .collect()
}

pub fn current_help_sections_for_context(
    settings: &Settings,
    context: ShortcutContext,
) -> Vec<HelpSection> {
    vec![HelpSection {
        title: context.title().to_string(),
        shortcuts: resolved_shortcuts_for_context(settings, context),
    }]
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use super::*;

    #[test]
    fn catalog_ids_are_unique_per_semantic_action_and_metadata_is_complete() {
        let mut ids_and_contexts = HashSet::new();
        for spec in CATALOG {
            assert!(!spec.label.trim().is_empty());
            assert!(!spec.contexts.is_empty());
            assert!(!spec.surfaces.is_empty());
            assert!(spec.surfaces.contains(&HintSurface::Help));
            for context in spec.contexts {
                assert!(ids_and_contexts.insert((spec.id, *context)));
            }
            for binding in resolved_binding_strings(spec.bindings, &Settings::default()) {
                assert!(
                    parse_binding(binding).is_some(),
                    "invalid binding: {binding}"
                );
            }
        }
    }

    #[test]
    fn catalog_has_no_ambiguous_bindings_within_a_leaf_context() {
        let settings = Settings::default();
        let mut bindings = HashMap::new();
        for spec in CATALOG {
            for binding in resolved_binding_strings(spec.bindings, &settings) {
                let parsed = parse_binding(binding).expect("catalog bindings are valid");
                for context in spec.contexts {
                    let key = (*context, parsed);
                    if let Some(previous) = bindings.insert(key, spec.id) {
                        assert_eq!(
                            previous, spec.id,
                            "{context:?} binds {binding:?} to both {previous:?} and {:?}",
                            spec.id
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn configured_bindings_are_formatted_for_people() {
        let mut settings = Settings::default();
        settings.prev_workspace_key = "ctrl+shift+h".to_string();
        settings.terminal_fullscreen_key = "alt+enter".to_string();
        assert_eq!(
            format_bindings(
                BindingSource::Setting(SettingBinding::PreviousWorkspace),
                &settings,
            ),
            "Ctrl+Shift+H"
        );
        assert_eq!(
            format_bindings(
                BindingSource::Setting(SettingBinding::Fullscreen),
                &settings
            ),
            "Alt+Enter"
        );
        assert_eq!(format_binding("backtab"), "Shift+Tab");
        assert_eq!(format_binding("shift+backtab"), "Shift+Tab");
        assert_eq!(format_binding("ctrl+backtab"), "Ctrl+Shift+Tab");
        assert_eq!(format_binding("insert"), "Ins");
        assert!(parse_binding("ctrl+insert").is_some());
    }

    #[test]
    fn captured_backtab_binding_matches_backtab_events() {
        let backtab = KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT);
        let captured = crate::keymap::keybind_from_event(backtab).unwrap();
        assert_eq!(captured, "backtab");

        let mut settings = Settings::default();
        settings.prev_workspace_key = captured;
        assert_eq!(
            match_shortcut_for_context(&settings, ShortcutContext::HomeSidebar, backtab),
            Some(ShortcutId::PreviousWorkspace)
        );

        settings.prev_workspace_key = "shift+backtab".to_string();
        assert_eq!(
            match_shortcut_for_context(&settings, ShortcutContext::HomeSidebar, backtab),
            Some(ShortcutId::PreviousWorkspace),
            "bindings captured by older versions remain usable"
        );
    }

    #[test]
    fn aliases_match_the_same_semantic_action() {
        let settings = Settings::default();
        for key in [
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
        ] {
            assert_eq!(
                match_shortcut_for_context(&settings, ShortcutContext::ChooserBrowse, key),
                Some(ShortcutId::MoveDown)
            );
        }
        assert_eq!(
            match_shortcut_for_context(
                &settings,
                ShortcutContext::HomeSidebar,
                KeyEvent::new(KeyCode::Char('/'), KeyModifiers::SHIFT),
            ),
            Some(ShortcutId::OpenHelp)
        );
        assert_eq!(
            match_shortcut_for_context(
                &settings,
                ShortcutContext::Diff,
                KeyEvent::new(KeyCode::Char(';'), KeyModifiers::SHIFT),
            ),
            Some(ShortcutId::OpenWorkspaceCommand)
        );

        assert_eq!(
            match_shortcut_for_context(
                &settings,
                ShortcutContext::QuickCreateText,
                KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
            ),
            None,
            "text-entry letters remain literal"
        );
        assert_eq!(
            match_shortcut_for_context(
                &settings,
                ShortcutContext::QuickCreateText,
                KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            ),
            Some(ShortcutId::MoveDown)
        );
        for key in [
            KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),
            KeyEvent::new(KeyCode::Tab, KeyModifiers::SHIFT),
        ] {
            assert_eq!(
                match_shortcut_for_context(&settings, ShortcutContext::QuickCreateText, key),
                Some(ShortcutId::PreviousField)
            );
        }
    }

    #[test]
    fn configured_control_binding_does_not_match_shift_only() {
        let settings = Settings::default();
        let ctrl_f = KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL);
        let shift_f = KeyEvent::new(KeyCode::Char('F'), KeyModifiers::SHIFT);
        assert_eq!(
            match_shortcut_for_context(&settings, ShortcutContext::Diff, ctrl_f),
            Some(ShortcutId::ToggleFullscreen)
        );
        assert_ne!(
            match_shortcut_for_context(&settings, ShortcutContext::Diff, shift_f),
            Some(ShortcutId::ToggleFullscreen)
        );
    }

    #[test]
    fn all_help_contains_every_catalog_action() {
        let help_ids = all_help_sections(&Settings::default())
            .into_iter()
            .flat_map(|section| section.shortcuts)
            .map(|shortcut| shortcut.id)
            .collect::<HashSet<_>>();
        for spec in CATALOG {
            assert!(help_ids.contains(&spec.id), "missing {:?}", spec.id);
        }
    }

    #[test]
    fn context_resolver_obeys_modal_and_help_precedence() {
        let mut app = TuiApp::default();
        app.begin_quick_create(uuid::Uuid::new_v4());
        assert_eq!(underlying_context(&app), ShortcutContext::QuickCreateText);

        app.pending_delete_workspace = Some(uuid::Uuid::new_v4());
        assert_eq!(underlying_context(&app), ShortcutContext::DeleteConfirm);

        app.help = Some(HelpState::new(ShortcutContext::DeleteConfirm));
        assert_eq!(active_context(&app), ShortcutContext::Help);
        assert_eq!(
            app.help.as_ref().map(|help| help.context),
            Some(ShortcutContext::DeleteConfirm)
        );
    }

    #[test]
    fn context_resolver_covers_sidebar_and_workspace_leaf_modes() {
        let mut app = TuiApp::default();
        assert_eq!(underlying_context(&app), ShortcutContext::HomeSidebar);
        app.sidebar_mode = SidebarMode::Rail;
        assert_eq!(underlying_context(&app), ShortcutContext::HomeRail);
        app.sidebar_popout = Some(uuid::Uuid::new_v4());
        assert_eq!(underlying_context(&app), ShortcutContext::HomeRailPopout);

        app.route = Route::Workspace {
            id: uuid::Uuid::new_v4(),
        };
        app.sidebar_popout = None;
        for (focus, command_mode, expected) in [
            (
                Focus::WsTerminal,
                false,
                ShortcutContext::TerminalPassthrough,
            ),
            (Focus::WsTerminal, true, ShortcutContext::TerminalCommand),
            (Focus::WsTerminalTabs, false, ShortcutContext::TerminalTabs),
            (Focus::WsLog, false, ShortcutContext::GitLogHeader),
            (Focus::WsBranches, false, ShortcutContext::Branches),
            (Focus::WsDiff, false, ShortcutContext::Diff),
            (Focus::ReviewFiles, false, ShortcutContext::ReviewFiles),
            (Focus::ReviewDiff, false, ShortcutContext::ReviewDiff),
        ] {
            app.focus = focus;
            app.terminal_command_mode = command_mode;
            assert_eq!(underlying_context(&app), expected, "{focus:?}");
        }
    }

    #[test]
    fn workspace_sidebar_and_rail_escape_resolve_home() {
        let settings = Settings::default();
        let escape = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        for context in [
            ShortcutContext::WorkspaceSidebar,
            ShortcutContext::WorkspaceRail,
        ] {
            assert_eq!(
                match_shortcut_for_context(&settings, context, escape),
                Some(ShortcutId::Back),
                "{context:?}"
            );
        }
    }

    #[test]
    fn empty_repository_summary_hides_selection_dependent_actions() {
        let mut app = TuiApp::default();
        let repo_id = uuid::Uuid::new_v4();
        app.set_repositories(vec![protocol::RepositorySummary {
            id: repo_id,
            name: "conduit".to_string(),
            path: "/tmp/conduit".to_string(),
            default_branch: Some("main".to_string()),
            worktree_root: None,
            default_agent: None,
            ssh_host: None,
            workspace_count: 0,
            ready_for_review_count: 0,
        }]);
        app.open_repo_summary(repo_id);

        let resolved = resolved_shortcuts(&app, ShortcutContext::RepoSummary);
        assert!(!resolved
            .iter()
            .any(|shortcut| { matches!(shortcut.id, ShortcutId::Open | ShortcutId::Delete) }));
        assert_eq!(
            match_shortcut(
                &app,
                ShortcutContext::RepoSummary,
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            ),
            None
        );
    }

    #[test]
    fn all_help_places_review_push_in_the_review_section() {
        let sections = all_help_sections(&Settings::default());
        let review = sections
            .iter()
            .find(|section| section.title == HelpGroup::Review.title())
            .expect("review help section");
        assert!(review
            .shortcuts
            .iter()
            .any(|shortcut| shortcut.id == ShortcutId::Push));
    }

    #[test]
    fn missed_workspace_and_review_actions_are_discoverable() {
        let settings = Settings::default();
        let all = all_help_sections(&settings)
            .into_iter()
            .flat_map(|section| section.shortcuts)
            .collect::<Vec<_>>();
        for id in [
            ShortcutId::RefreshGit,
            ShortcutId::SelectFirstTab,
            ShortcutId::SelectSecondTab,
            ShortcutId::ToggleYolo,
            ShortcutId::ReviewPageDown,
            ShortcutId::OpenPullRequest,
        ] {
            assert!(all.iter().any(|shortcut| shortcut.id == id), "{id:?}");
        }
    }
}
