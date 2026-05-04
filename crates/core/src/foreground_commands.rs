use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use protocol::SavedCommand;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ForegroundCommandKey {
    pub workspace_path: String,
    pub tab_id: String,
}

impl ForegroundCommandKey {
    pub fn new(workspace_path: &Path, tab_id: impl Into<String>) -> Self {
        Self {
            workspace_path: workspace_path.to_string_lossy().into_owned(),
            tab_id: tab_id.into(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ForegroundCommandStore {
    entries: HashMap<ForegroundCommandKey, SavedCommand>,
}

impl ForegroundCommandStore {
    pub fn load(path: &Path) -> Self {
        let Ok(raw) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        let Ok(file) = serde_json::from_str::<ForegroundCommandsFile>(&raw) else {
            return Self::default();
        };
        Self::from_file(file)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let raw = serde_json::to_string_pretty(&self.to_file())?;
        std::fs::write(path, raw)?;
        Ok(())
    }

    pub fn keys(&self) -> impl Iterator<Item = ForegroundCommandKey> + '_ {
        self.entries.keys().cloned()
    }

    #[cfg(test)]
    pub fn get(&self, workspace_path: &Path, tab_id: &str) -> Option<&SavedCommand> {
        self.get_key(&ForegroundCommandKey::new(
            workspace_path,
            tab_id.to_string(),
        ))
    }

    pub fn get_key(&self, key: &ForegroundCommandKey) -> Option<&SavedCommand> {
        self.entries.get(key)
    }

    #[cfg(test)]
    pub fn set(&mut self, workspace_path: &Path, tab_id: &str, command: SavedCommand) -> bool {
        self.set_key(
            ForegroundCommandKey::new(workspace_path, tab_id.to_string()),
            command,
        )
    }

    pub fn set_key(&mut self, key: ForegroundCommandKey, command: SavedCommand) -> bool {
        if self.entries.get(&key) == Some(&command) {
            return false;
        }
        self.entries.insert(key, command);
        true
    }

    #[cfg(test)]
    pub fn remove(&mut self, workspace_path: &Path, tab_id: &str) -> bool {
        self.remove_key(&ForegroundCommandKey::new(
            workspace_path,
            tab_id.to_string(),
        ))
    }

    pub fn remove_key(&mut self, key: &ForegroundCommandKey) -> bool {
        self.entries.remove(key).is_some()
    }

    pub fn remove_workspace(&mut self, workspace_path: &Path) -> bool {
        let path = workspace_path.to_string_lossy();
        let before = self.entries.len();
        self.entries.retain(|key, _| key.workspace_path != path);
        self.entries.len() != before
    }

    fn from_file(file: ForegroundCommandsFile) -> Self {
        let mut entries = HashMap::new();
        for (workspace_path, tabs) in file.workspaces {
            for (tab_id, command) in tabs {
                entries.insert(
                    ForegroundCommandKey {
                        workspace_path: workspace_path.clone(),
                        tab_id,
                    },
                    command,
                );
            }
        }
        Self { entries }
    }

    fn to_file(&self) -> ForegroundCommandsFile {
        let mut workspaces: HashMap<String, HashMap<String, SavedCommand>> = HashMap::new();
        for (key, command) in &self.entries {
            workspaces
                .entry(key.workspace_path.clone())
                .or_default()
                .insert(key.tab_id.clone(), command.clone());
        }
        ForegroundCommandsFile { workspaces }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ForegroundCommandsFile {
    #[serde(default)]
    workspaces: HashMap<String, HashMap<String, SavedCommand>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(argv: &[&str], cwd: &str) -> SavedCommand {
        SavedCommand {
            argv: argv.iter().map(|s| (*s).to_string()).collect(),
            cwd: cwd.to_string(),
        }
    }

    #[test]
    fn store_save_load_update_and_remove_round_trip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("foreground_commands.json");
        let workspace = dir.path().join("repo");

        let mut store = ForegroundCommandStore::default();
        assert!(store.set(&workspace, "shell", command(&["sleep", "300"], "/repo")));
        assert!(!store.set(&workspace, "shell", command(&["sleep", "300"], "/repo")));
        store.save(&path).expect("save");

        let mut loaded = ForegroundCommandStore::load(&path);
        assert_eq!(
            loaded.get(&workspace, "shell"),
            Some(&command(&["sleep", "300"], "/repo"))
        );

        assert!(loaded.set(&workspace, "shell", command(&["cargo", "test"], "/repo")));
        assert_eq!(
            loaded.get(&workspace, "shell"),
            Some(&command(&["cargo", "test"], "/repo"))
        );
        assert!(loaded.remove(&workspace, "shell"));
        assert!(loaded.get(&workspace, "shell").is_none());
        assert!(!loaded.remove(&workspace, "shell"));
    }

    #[test]
    fn remove_workspace_removes_all_tabs_for_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ws_a = dir.path().join("a");
        let ws_b = dir.path().join("b");
        let mut store = ForegroundCommandStore::default();
        store.set(&ws_a, "shell", command(&["sleep", "1"], "/a"));
        store.set(&ws_a, "shell-2", command(&["sleep", "2"], "/a"));
        store.set(&ws_b, "shell", command(&["sleep", "3"], "/b"));

        assert!(store.remove_workspace(&ws_a));
        assert!(store.get(&ws_a, "shell").is_none());
        assert!(store.get(&ws_a, "shell-2").is_none());
        assert!(store.get(&ws_b, "shell").is_some());
        assert!(!store.remove_workspace(&ws_a));
    }
}
