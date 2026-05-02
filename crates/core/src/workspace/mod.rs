pub mod attention;
pub mod git;
pub mod process_info;
pub mod ssh;
pub mod terminal;

pub use protocol::{AttentionLevel, ChangedFile, GitState, TerminalKind};
pub use terminal::WorkspaceTerminals;
