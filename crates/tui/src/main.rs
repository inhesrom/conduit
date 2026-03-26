mod app;
mod keymap;
mod ui;

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command as OsCommand, Stdio};
use std::time::{Duration, Instant};

use conduit_core::{spawn_core, CoreHandle};
use anyhow::{anyhow, Context, Result};
use app::TuiApp;
use base64::Engine as _;
use crossterm::{
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags,
        MouseButton, MouseEvent, MouseEventKind, PopKeyboardEnhancementFlags,
        PushKeyboardEnhancementFlags,
    },
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use protocol::{AttentionLevel, Command, Event as CoreEvent, Route, TerminalKind};
use ratatui::{backend::CrosstermBackend, layout::Rect, Terminal};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;
use vt100::{MouseProtocolEncoding, MouseProtocolMode};

#[derive(Debug)]
enum LaunchMode {
    Local,
    CreateSession { name: String },
    AttachSession { name: String },
    RemoveSession { name: String },
    ListSessions,
    RunDaemon { name: String },
    Update,
    Reinstall,
}

#[derive(Debug)]
struct Cli {
    mode: LaunchMode,
    detach: bool,
    version: bool,
    help: bool,
}

struct Backend {
    cmd_tx: mpsc::Sender<Command>,
    evt_rx: mpsc::Receiver<CoreEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SessionEntry {
    name: String,
    socket_path: String,
    pid: u32,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct SessionRegistry {
    sessions: Vec<SessionEntry>,
}

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
}

fn print_help() {
    println!(
        "\
conduit {}

USAGE:
    conduit[OPTIONS]

OPTIONS:
    -s, --session <name>   Create (or reattach to) a named session
    -a <name>              Attach to an existing session
    -r, --remove <name>    Remove a session (stops its daemon)
    -l, --list             List active sessions
    -d, --detach           Start session in background only (with -s or -a)
    -u, --update           Update to the latest release from GitHub
        --reinstall        Reinstall the latest release (even if already up to date)
    -V, --version          Print version
    -h, --help             Print this help

EXAMPLES:
    conduit                  Launch in local (non-session) mode
    conduit-s work           Create or reattach to session 'work'
    conduit-s work -d        Start session 'work' in background
    conduit-a work           Attach to running session 'work'
    conduit-l                List sessions
    conduit-r work           Remove session 'work'",
        env!("CARGO_PKG_VERSION")
    );
}

fn parse_cli(args: Vec<String>) -> Result<Cli> {
    let mut i = 0usize;
    let mut mode = LaunchMode::Local;
    let mut detach = false;
    let mut version = false;
    let mut help = false;
    let mut daemon_name: Option<String> = None;

    while i < args.len() {
        match args[i].as_str() {
            "-s" | "--session" => {
                let Some(name) = args.get(i + 1).cloned() else {
                    return Err(anyhow!("missing session name for {}", args[i]));
                };
                mode = LaunchMode::CreateSession { name };
                i += 2;
            }
            "-a" => {
                let Some(name) = args.get(i + 1).cloned() else {
                    return Err(anyhow!("missing session name for -a"));
                };
                mode = LaunchMode::AttachSession { name };
                i += 2;
            }
            "-r" | "--remove" => {
                let Some(name) = args.get(i + 1).cloned() else {
                    return Err(anyhow!("missing session name for {}", args[i]));
                };
                mode = LaunchMode::RemoveSession { name };
                i += 2;
            }
            "-V" | "--version" => {
                version = true;
                i += 1;
            }
            "-d" | "--detach" => {
                detach = true;
                i += 1;
            }
            "-l" | "--list" => {
                mode = LaunchMode::ListSessions;
                i += 1;
            }
            "-u" | "--update" => {
                mode = LaunchMode::Update;
                i += 1;
            }
            "--reinstall" => {
                mode = LaunchMode::Reinstall;
                i += 1;
            }
            "-h" | "--help" => {
                help = true;
                i += 1;
            }
            "--run-daemon" => {
                mode = LaunchMode::RunDaemon {
                    name: String::new(),
                };
                i += 1;
            }
            "--session-name" => {
                let Some(name) = args.get(i + 1).cloned() else {
                    return Err(anyhow!("missing name for --session-name"));
                };
                daemon_name = Some(name);
                i += 2;
            }
            other => {
                return Err(anyhow!("unknown argument: {other}"));
            }
        }
    }

    if matches!(mode, LaunchMode::RunDaemon { .. }) {
        let name = daemon_name.unwrap_or_default();
        return Ok(Cli {
            mode: LaunchMode::RunDaemon { name },
            detach,
            version,
            help,
        });
    }

    if detach
        && matches!(
            mode,
            LaunchMode::RemoveSession { .. } | LaunchMode::ListSessions
        )
    {
        return Err(anyhow!(
            "--detach is only valid with session create/attach (-s or -a)"
        ));
    }

    Ok(Cli {
        mode,
        detach,
        version,
        help,
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = parse_cli(std::env::args().skip(1).collect::<Vec<_>>())?;
    if cli.help {
        print_help();
        return Ok(());
    }
    if cli.version {
        println!("conduit {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    match cli.mode {
        LaunchMode::Update => self_update(false),
        LaunchMode::Reinstall => self_update(true),
        LaunchMode::RunDaemon { name } => run_daemon(&name).await,
        LaunchMode::RemoveSession { name } => delete_session(&name),
        LaunchMode::ListSessions => list_sessions(),
        LaunchMode::CreateSession { name } => {
            let entry = ensure_session_running(&name).await?;
            if cli.detach {
                println!("session '{}' running in background (detached)", entry.name);
                return Ok(());
            }
            let backend = build_remote_backend(&entry.socket_path).await?;
            run_tui(backend).await
        }
        LaunchMode::AttachSession { name } => {
            let entry = get_session(&name)?.ok_or_else(|| {
                anyhow!(
                    "session '{}' not found. create it with: conduit -s {}",
                    name,
                    name
                )
            })?;
            let entry = if !socket_alive(&entry.socket_path) {
                eprintln!("session '{}' is stale, restarting…", name);
                ensure_session_running(&name).await?
            } else {
                entry
            };
            if cli.detach {
                println!("session '{}' is running (detached)", entry.name);
                return Ok(());
            }
            let backend = build_remote_backend(&entry.socket_path).await?;
            run_tui(backend).await
        }
        LaunchMode::Local => {
            if cli.detach {
                return Err(anyhow!(
                    "--detach requires a named session: use `conduit -s <name> -d` or `conduit -a <name> -d`"
                ));
            }
            let (backend, _core) = build_local_backend();
            run_tui(backend).await
        }
    }
}

fn self_update(force: bool) -> Result<()> {
    let current_version = env!("CARGO_PKG_VERSION");

    // Fetch latest release info from GitHub
    let api_output = OsCommand::new("curl")
        .args([
            "-fsSL",
            "https://api.github.com/repos/inhesrom/conduit/releases/latest",
        ])
        .output()
        .context("failed to run curl — is it installed?")?;
    if !api_output.status.success() {
        return Err(anyhow!(
            "failed to fetch latest release info from GitHub (curl exit {})",
            api_output.status
        ));
    }
    let api_body = String::from_utf8_lossy(&api_output.stdout);

    let tag = parse_latest_release_tag(&api_body)?;

    let latest_version = tag.strip_prefix('v').unwrap_or(&tag);

    if latest_version == current_version && !force {
        println!("conduit is already up to date (v{current_version})");
        return Ok(());
    }

    if latest_version == current_version {
        println!("reinstalling conduit v{current_version}...");
    } else {
        println!("updating conduit v{current_version} -> v{latest_version}...");
    }

    // Detect platform
    let os_output = OsCommand::new("uname").arg("-s").output()?;
    let os_name = String::from_utf8_lossy(&os_output.stdout)
        .trim()
        .to_lowercase();

    let arch_output = OsCommand::new("uname").arg("-m").output()?;
    let arch_name = String::from_utf8_lossy(&arch_output.stdout)
        .trim()
        .to_string();

    let target = detect_release_target(&os_name, &arch_name)?;

    let url =
        format!("https://github.com/inhesrom/conduit/releases/download/{tag}/conduit-{target}.tar.gz");

    // Download to a temp directory
    let tmp_dir = std::env::temp_dir().join(format!("conduit-update-{}", std::process::id()));
    std::fs::create_dir_all(&tmp_dir)?;
    let _cleanup = TempDirGuard(tmp_dir.clone());

    let tarball = tmp_dir.join("conduit.tar.gz");
    let dl_status = OsCommand::new("curl")
        .args(["-fsSL", &url, "-o"])
        .arg(&tarball)
        .status()
        .context("failed to run curl for download")?;
    if !dl_status.success() {
        return Err(anyhow!("failed to download release tarball from {url}"));
    }

    // Extract
    let extract_status = OsCommand::new("tar")
        .arg("xzf")
        .arg(&tarball)
        .arg("-C")
        .arg(&tmp_dir)
        .status()
        .context("failed to run tar")?;
    if !extract_status.success() {
        return Err(anyhow!("failed to extract release tarball"));
    }

    // Replace the current binary
    let current_exe =
        std::env::current_exe().context("cannot determine current executable path")?;
    let new_binary = tmp_dir.join("conduit");
    if !new_binary.exists() {
        return Err(anyhow!("extracted archive does not contain 'conduit' binary"));
    }
    let downloaded_version = read_conduit_version(&new_binary).with_context(|| {
        format!(
            "downloaded release asset at {} is not a valid conduit binary",
            new_binary.display()
        )
    })?;
    if downloaded_version != latest_version {
        return Err(anyhow!(
            "downloaded release asset reports v{downloaded_version}, expected v{latest_version}. The GitHub release may contain a stale binary."
        ));
    }

    // Remove the running binary first — Linux allows unlinking an in-use
    // executable but blocks writing to it (ETXTBSY / "Text file busy").
    std::fs::remove_file(&current_exe).with_context(|| {
        format!(
            "failed to remove old binary at {}. You may need to run with sudo.",
            current_exe.display()
        )
    })?;
    std::fs::copy(&new_binary, &current_exe)
        .with_context(|| format!("failed to install new binary at {}.", current_exe.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&current_exe, std::fs::Permissions::from_mode(0o755))?;
    }

    let installed_version = read_conduit_version(&current_exe).with_context(|| {
        format!(
            "installed binary at {} could not be verified after update",
            current_exe.display()
        )
    })?;
    if installed_version != latest_version {
        return Err(anyhow!(
            "updated binary at {} still reports v{}, expected v{}",
            current_exe.display(),
            installed_version,
            latest_version
        ));
    }

    if let Some(path_binary) = find_binary_on_path("conduit") {
        if !same_executable(&path_binary, &current_exe) {
            let path_version = read_conduit_version(&path_binary).with_context(|| {
                format!(
                    "`conduit` on PATH resolves to {}, which is different from the updated binary at {}",
                    path_binary.display(),
                    current_exe.display()
                )
            })?;
            if path_version != latest_version {
                return Err(anyhow!(
                    "updated {} to v{}, but `conduit` on PATH resolves to {} and reports v{}. Adjust PATH or update that install.",
                    current_exe.display(),
                    latest_version,
                    path_binary.display(),
                    path_version
                ));
            }
        }
    }

    println!(
        "conduit updated to v{latest_version} at {}",
        current_exe.display()
    );
    Ok(())
}

fn parse_latest_release_tag(api_body: &str) -> Result<String> {
    let release: GitHubRelease =
        serde_json::from_str(api_body).context("failed to parse GitHub release response")?;
    let tag = release.tag_name.trim();
    if tag.is_empty() {
        return Err(anyhow!("GitHub release response did not include tag_name"));
    }
    Ok(tag.to_string())
}

fn detect_release_target(os_name: &str, arch_name: &str) -> Result<&'static str> {
    match (os_name, arch_name) {
        ("darwin", "arm64" | "aarch64") => Ok("aarch64-apple-darwin"),
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-gnu"),
        _ => Err(anyhow!("unsupported platform: {os_name} {arch_name}")),
    }
}

fn parse_conduit_version_output(output: &str) -> Option<&str> {
    let line = output.lines().find(|line| !line.trim().is_empty())?.trim();
    let version = line.strip_prefix("conduit ")?;
    Some(version.strip_prefix('v').unwrap_or(version))
}

fn read_conduit_version(path: &Path) -> Result<String> {
    let output = OsCommand::new(path)
        .arg("--version")
        .output()
        .with_context(|| format!("failed to run {} --version", path.display()))?;
    if !output.status.success() {
        return Err(anyhow!(
            "{} --version exited with {}",
            path.display(),
            output.status
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_conduit_version_output(&stdout)
        .map(ToOwned::to_owned)
        .ok_or_else(|| {
            anyhow!(
                "unexpected version output from {}: {}",
                path.display(),
                stdout.trim()
            )
        })
}

fn find_binary_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

fn same_executable(lhs: &Path, rhs: &Path) -> bool {
    if lhs == rhs {
        return true;
    }
    match (lhs.canonicalize(), rhs.canonicalize()) {
        (Ok(lhs), Ok(rhs)) => lhs == rhs,
        _ => false,
    }
}

struct TempDirGuard(PathBuf);
impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn build_local_backend() -> (Backend, CoreHandle) {
    let core = spawn_core();
    let cmd_tx = core.cmd_tx.clone();

    let (evt_tx, evt_rx) = mpsc::channel::<CoreEvent>(1024);
    let mut broadcast_rx = core.evt_tx.subscribe();
    tokio::spawn(async move {
        loop {
            match broadcast_rx.recv().await {
                Ok(evt) => {
                    if evt_tx.send(evt).await.is_err() {
                        break;
                    }
                }
                Err(RecvError::Closed) => break,
                Err(RecvError::Lagged(_)) => continue,
            }
        }
    });

    (Backend { cmd_tx, evt_rx }, core)
}

// ---------------------------------------------------------------------------
// Unix-domain-socket session infrastructure
// ---------------------------------------------------------------------------

fn session_socket_dir() -> Result<PathBuf> {
    let base = if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        PathBuf::from(xdg)
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".config")
    } else {
        return Err(anyhow!("cannot determine config directory"));
    };
    Ok(base.join("conduit").join("sessions"))
}

fn session_socket_path(name: &str) -> Result<PathBuf> {
    let safe = sanitize_session_name(name);
    Ok(session_socket_dir()?.join(format!("{safe}.sock")))
}

/// 4-byte big-endian length prefix + JSON payload.
async fn read_frame<R: tokio::io::AsyncRead + Unpin>(r: &mut R) -> Result<Option<Vec<u8>>> {
    let mut len_buf = [0u8; 4];
    match r.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > 16 * 1024 * 1024 {
        return Err(anyhow!("frame too large: {} bytes", len));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).await?;
    Ok(Some(buf))
}

async fn write_frame<W: tokio::io::AsyncWrite + Unpin>(w: &mut W, data: &[u8]) -> Result<()> {
    let len = (data.len() as u32).to_be_bytes();
    w.write_all(&len).await?;
    w.write_all(data).await?;
    w.flush().await?;
    Ok(())
}

use std::collections::HashMap;
use uuid::Uuid;

/// Per-workspace keyed state store — keeps only the latest of each state event type.
struct WorkspaceState {
    git: Option<Vec<u8>>,
    attention: Option<Vec<u8>>,
}

/// Keyed state store for non-terminal events. No eviction needed — each workspace
/// stores only the latest of each event type.
struct EventHistory {
    workspace_list: Option<Vec<u8>>,
    per_workspace: HashMap<Uuid, WorkspaceState>,
}

impl EventHistory {
    fn new() -> Self {
        Self {
            workspace_list: None,
            per_workspace: HashMap::new(),
        }
    }

    fn update(&mut self, evt: &CoreEvent, payload: Vec<u8>) {
        match evt {
            CoreEvent::WorkspaceList { .. } => {
                self.workspace_list = Some(payload);
            }
            CoreEvent::WorkspaceGitUpdated { id, .. } => {
                self.per_workspace
                    .entry(*id)
                    .or_insert_with(|| WorkspaceState {
                        git: None,
                        attention: None,
                    })
                    .git = Some(payload);
            }
            CoreEvent::WorkspaceAttentionChanged { id, .. } => {
                self.per_workspace
                    .entry(*id)
                    .or_insert_with(|| WorkspaceState {
                        git: None,
                        attention: None,
                    })
                    .attention = Some(payload);
            }
            _ => {}
        }
    }

    fn snapshot(&self) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        if let Some(ref frame) = self.workspace_list {
            out.push(frame.clone());
        }
        for ws in self.per_workspace.values() {
            if let Some(ref frame) = ws.git {
                out.push(frame.clone());
            }
            if let Some(ref frame) = ws.attention {
                out.push(frame.clone());
            }
        }
        out
    }
}

/// Per-terminal ring buffer — stores raw terminal bytes (base64-decoded) per tab.
struct TerminalBuffer {
    kind: protocol::TerminalKind,
    data: Vec<u8>,
}

struct TerminalHistory {
    buffers: HashMap<(Uuid, String), TerminalBuffer>,
}

const TERMINAL_HISTORY_MAX_BYTES: usize = 512 * 1024; // 512 KB per terminal tab

impl TerminalHistory {
    fn new() -> Self {
        Self {
            buffers: HashMap::new(),
        }
    }

    fn append(&mut self, id: Uuid, kind: protocol::TerminalKind, tab_id: String, raw_bytes: &[u8]) {
        let entry = self
            .buffers
            .entry((id, tab_id))
            .or_insert_with(|| TerminalBuffer {
                kind,
                data: Vec::new(),
            });
        entry.kind = kind;
        entry.data.extend_from_slice(raw_bytes);
        if entry.data.len() > TERMINAL_HISTORY_MAX_BYTES {
            let excess = entry.data.len() - TERMINAL_HISTORY_MAX_BYTES;
            entry.data.drain(..excess);
        }
    }

    fn reset(&mut self, id: Uuid, tab_id: String) {
        self.buffers.remove(&(id, tab_id));
    }

    /// Emit a TerminalStarted + single TerminalOutput per buffer for replay.
    fn snapshot(&self) -> Vec<CoreEvent> {
        let mut out = Vec::new();
        for ((id, tab_id), entry) in &self.buffers {
            if entry.data.is_empty() {
                continue;
            }
            out.push(CoreEvent::TerminalStarted {
                id: *id,
                kind: entry.kind,
                tab_id: Some(tab_id.clone()),
            });
            let data_b64 = base64::engine::general_purpose::STANDARD.encode(&entry.data);
            out.push(CoreEvent::TerminalOutput {
                id: *id,
                kind: entry.kind,
                tab_id: Some(tab_id.clone()),
                data_b64,
            });
        }
        out
    }
}

/// Combined history shared under a single mutex for atomic snapshots.
struct CombinedHistory {
    state: EventHistory,
    terminals: TerminalHistory,
}

impl CombinedHistory {
    fn new() -> Self {
        Self {
            state: EventHistory::new(),
            terminals: TerminalHistory::new(),
        }
    }
}

async fn run_daemon(name: &str) -> Result<()> {
    let sock_path = session_socket_path(name)?;
    if let Some(parent) = sock_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Remove stale socket file if it exists
    let _ = std::fs::remove_file(&sock_path);

    let core = spawn_core();
    let listener = tokio::net::UnixListener::bind(&sock_path)
        .with_context(|| format!("failed to bind unix socket: {}", sock_path.display()))?;

    // Clean up socket on exit
    struct CleanupGuard(PathBuf);
    impl Drop for CleanupGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let _guard = CleanupGuard(sock_path.clone());

    // Shared history buffer for replaying events to reconnecting clients
    let history = std::sync::Arc::new(tokio::sync::Mutex::new(CombinedHistory::new()));

    // Background task: record replayable events into history
    {
        let history = history.clone();
        let mut evt_rx = core.evt_tx.subscribe();
        tokio::spawn(async move {
            loop {
                match evt_rx.recv().await {
                    Ok(ref evt) => {
                        match evt {
                            CoreEvent::TerminalOutput {
                                id,
                                kind,
                                tab_id,
                                data_b64,
                            } => {
                                if let Ok(raw) =
                                    base64::engine::general_purpose::STANDARD.decode(data_b64)
                                {
                                    let tab =
                                        tab_id.clone().unwrap_or_else(|| "default".to_string());
                                    history.lock().await.terminals.append(*id, *kind, tab, &raw);
                                }
                            }
                            CoreEvent::TerminalStarted { id, tab_id, .. } => {
                                let tab = tab_id.clone().unwrap_or_else(|| "default".to_string());
                                history.lock().await.terminals.reset(*id, tab);
                            }
                            CoreEvent::WorkspaceList { .. }
                            | CoreEvent::WorkspaceGitUpdated { .. }
                            | CoreEvent::WorkspaceAttentionChanged { .. } => {
                                if let Ok(payload) = serde_json::to_vec(evt) {
                                    history.lock().await.state.update(evt, payload);
                                }
                            }
                            // TerminalExited: leave buffer intact (shows last output)
                            _ => {}
                        }
                    }
                    Err(RecvError::Closed) => break,
                    Err(RecvError::Lagged(n)) => {
                        eprintln!("[conduit] event recorder lagged by {n} events");
                        continue;
                    }
                }
            }
        });
    }

    loop {
        let (stream, _) = listener.accept().await?;
        let (mut reader, mut writer) = stream.into_split();
        let cmd_tx = core.cmd_tx.clone();
        let history = history.clone();
        let core_evt_tx = core.evt_tx.clone();

        // Bridge: read Commands from socket, send Events back
        tokio::spawn(async move {
            // Write events to socket
            let (write_tx, mut write_rx) = mpsc::channel::<Vec<u8>>(2048);
            tokio::spawn(async move {
                while let Some(data) = write_rx.recv().await {
                    if write_frame(&mut writer, &data).await.is_err() {
                        break;
                    }
                }
            });

            // Lock history, take snapshot, subscribe to broadcast — all under lock
            // to guarantee no gap or overlap between snapshot and live events.
            let mut evt_rx = {
                let combined = history.lock().await;

                // Send state events first
                for frame in combined.state.snapshot() {
                    if write_tx.send(frame).await.is_err() {
                        return;
                    }
                }
                // Then terminal history (TerminalStarted + TerminalOutput per tab)
                for evt in combined.terminals.snapshot() {
                    if let Ok(payload) = serde_json::to_vec(&evt) {
                        if write_tx.send(payload).await.is_err() {
                            return;
                        }
                    }
                }

                // Subscribe while still holding lock — no events can be missed
                let rx = core_evt_tx.subscribe();
                // Lock is dropped when `combined` goes out of scope
                rx
            };

            // Forward live broadcast events directly to socket writer
            let write_tx2 = write_tx.clone();
            tokio::spawn(async move {
                loop {
                    match evt_rx.recv().await {
                        Ok(evt) => {
                            if let Ok(payload) = serde_json::to_vec(&evt) {
                                if write_tx2.send(payload).await.is_err() {
                                    break;
                                }
                            }
                        }
                        Err(RecvError::Closed) => break,
                        Err(RecvError::Lagged(n)) => {
                            eprintln!("[conduit] client event forwarder lagged by {n} events");
                            continue;
                        }
                    }
                }
            });

            // Read commands from socket
            loop {
                match read_frame(&mut reader).await {
                    Ok(Some(data)) => {
                        if let Ok(cmd) = serde_json::from_slice::<Command>(&data) {
                            if cmd_tx.send(cmd).await.is_err() {
                                break;
                            }
                        }
                    }
                    Ok(None) => break,
                    Err(_) => break,
                }
            }
        });
    }
}

async fn build_remote_backend(socket_path: &str) -> Result<Backend> {
    let stream = tokio::net::UnixStream::connect(socket_path)
        .await
        .with_context(|| format!("failed to connect to daemon socket: {socket_path}"))?;
    let (mut reader, mut writer) = stream.into_split();

    let (cmd_tx, mut cmd_rx) = mpsc::channel::<Command>(1024);
    let (evt_tx, evt_rx) = mpsc::channel::<CoreEvent>(1024);

    // Write commands to socket
    tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            if let Ok(payload) = serde_json::to_vec(&cmd) {
                if write_frame(&mut writer, &payload).await.is_err() {
                    break;
                }
            }
        }
    });

    // Read events from socket
    tokio::spawn(async move {
        loop {
            match read_frame(&mut reader).await {
                Ok(Some(data)) => {
                    if let Ok(evt) = serde_json::from_slice::<CoreEvent>(&data) {
                        if evt_tx.send(evt).await.is_err() {
                            break;
                        }
                    }
                }
                Ok(None) => break,
                Err(_) => break,
            }
        }
    });

    Ok(Backend { cmd_tx, evt_rx })
}

// ---------------------------------------------------------------------------
// Session management
// ---------------------------------------------------------------------------

async fn ensure_session_running(name: &str) -> Result<SessionEntry> {
    let mut registry = load_registry()?;
    if let Some(existing) = registry.sessions.iter().find(|s| s.name == name).cloned() {
        if socket_alive(&existing.socket_path) {
            return Ok(existing);
        }
        registry.sessions.retain(|s| s.name != name);
    }

    let pid = spawn_daemon_process(name)?;
    let sock_path = session_socket_path(name)?;
    let sock_str = sock_path.display().to_string();

    wait_for_socket(&sock_str, Duration::from_secs(8)).await?;

    let entry = SessionEntry {
        name: name.to_string(),
        socket_path: sock_str,
        pid,
    };
    registry.sessions.retain(|s| s.name != name);
    registry.sessions.push(entry.clone());
    save_registry(&registry)?;
    Ok(entry)
}

fn get_session(name: &str) -> Result<Option<SessionEntry>> {
    let registry = load_registry()?;
    Ok(registry.sessions.into_iter().find(|s| s.name == name))
}

fn delete_session(name: &str) -> Result<()> {
    let mut registry = load_registry()?;
    let Some(entry) = registry.sessions.iter().find(|s| s.name == name).cloned() else {
        println!("session '{}' not found", name);
        return Ok(());
    };

    print!(
        "Delete session '{}'? This will stop running terminals. [y/N]: ",
        entry.name
    );
    std::io::stdout().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let confirm = matches!(input.trim().to_lowercase().as_str(), "y" | "yes");
    if !confirm {
        println!("aborted");
        return Ok(());
    }

    if is_expected_daemon_process(&entry) {
        let _ = OsCommand::new("kill").arg(entry.pid.to_string()).status();
    } else {
        println!(
            "warning: pid {} does not look like session daemon '{}'; skipping kill and removing registry entry only",
            entry.pid, entry.name
        );
    }

    // Clean up socket file
    let _ = std::fs::remove_file(&entry.socket_path);

    registry.sessions.retain(|s| s.name != name);
    save_registry(&registry)?;
    if let Some(path) = session_workspaces_persist_path(name) {
        let _ = std::fs::remove_file(path);
    }
    println!("deleted session '{}'", name);
    Ok(())
}

fn list_sessions() -> Result<()> {
    let registry = load_registry()?;
    if registry.sessions.is_empty() {
        println!("no sessions");
        return Ok(());
    }

    println!("sessions:");
    for s in registry.sessions {
        let state = if socket_alive(&s.socket_path) {
            "running"
        } else {
            "stale"
        };
        println!("- {}  (pid {} {})", s.name, s.pid, state);
    }
    Ok(())
}

fn spawn_daemon_process(name: &str) -> Result<u32> {
    let exe = std::env::current_exe()?;
    let child = OsCommand::new(exe)
        .env("CONDUIT_SESSION_NAME", name)
        .arg("--run-daemon")
        .arg("--session-name")
        .arg(name)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("failed to spawn daemon for session '{}'", name))?;
    Ok(child.id())
}

async fn wait_for_socket(path: &str, timeout: Duration) -> Result<()> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if socket_alive(path) {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(120)).await;
    }
    Err(anyhow!("daemon did not become ready at {}", path))
}

fn socket_alive(path: &str) -> bool {
    std::os::unix::net::UnixStream::connect(path).is_ok()
}

fn is_expected_daemon_process(entry: &SessionEntry) -> bool {
    let output = match OsCommand::new("ps")
        .arg("-p")
        .arg(entry.pid.to_string())
        .arg("-o")
        .arg("command=")
        .output()
    {
        Ok(out) => out,
        Err(_) => return false,
    };
    if !output.status.success() {
        return false;
    }
    let cmdline = String::from_utf8_lossy(&output.stdout);
    cmdline.contains("--run-daemon") && cmdline.contains(&format!("--session-name {}", entry.name))
}

fn session_registry_path() -> Option<PathBuf> {
    let base = if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        PathBuf::from(xdg)
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".config")
    } else {
        return None;
    };
    Some(base.join("conduit").join("sessions.json"))
}

fn session_workspaces_persist_path(name: &str) -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let safe = sanitize_session_name(name);
    Some(
        PathBuf::from(home)
            .join(".config")
            .join("conduit")
            .join(format!("workspaces.{safe}.json")),
    )
}

fn sanitize_session_name(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return "default".to_string();
    }
    let mut out = String::with_capacity(trimmed.len());
    for c in trimmed.chars() {
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    out
}

fn load_registry() -> Result<SessionRegistry> {
    let Some(path) = session_registry_path() else {
        return Ok(SessionRegistry::default());
    };
    if !path.exists() {
        return Ok(SessionRegistry::default());
    }
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read session registry: {}", path.display()))?;
    let registry = serde_json::from_str::<SessionRegistry>(&raw).unwrap_or_default();
    Ok(registry)
}

fn save_registry(registry: &SessionRegistry) -> Result<()> {
    let Some(path) = session_registry_path() else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let raw = serde_json::to_string_pretty(registry)?;
    std::fs::write(&path, raw)
        .with_context(|| format!("failed to write session registry: {}", path.display()))?;
    Ok(())
}

async fn run_tui(mut backend: Backend) -> Result<()> {
    // Install a panic hook that restores the terminal before printing the panic.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let mut stdout = std::io::stdout();
        let _ = stdout.execute(PopKeyboardEnhancementFlags);
        let _ = stdout.execute(DisableBracketedPaste);
        let _ = stdout.execute(DisableMouseCapture);
        let _ = stdout.execute(crossterm::cursor::Show);
        let _ = stdout.execute(LeaveAlternateScreen);
        default_hook(info);
    }));

    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    stdout.execute(EnterAlternateScreen)?;
    stdout.execute(EnableMouseCapture)?;
    stdout.execute(EnableBracketedPaste)?;
    let keyboard_enhancement_active =
        crossterm::terminal::supports_keyboard_enhancement().unwrap_or(false);
    if keyboard_enhancement_active {
        let _ = stdout.execute(PushKeyboardEnhancementFlags(
            KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES,
        ));
    }

    let backend_term = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend_term)?;
    let mut app = TuiApp::default();
    let mut last_flash_toggle = Instant::now();
    let mut frame_interval = tokio::time::interval(Duration::from_millis(16));
    frame_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    'main: loop {
        // Drive the loop from async sources — never rely on crossterm's
        // synchronous poll timeout which can block the thread indefinitely.
        tokio::select! {
            _ = frame_interval.tick() => {}
            evt = backend.evt_rx.recv() => {
                if let Some(evt) = evt {
                    apply_event(&mut app, evt);
                }
            }
        }

        for _ in 0..128 {
            match backend.evt_rx.try_recv() {
                Ok(evt) => apply_event(&mut app, evt),
                Err(_) => break,
            }
        }

        // Send any pending CPR (Cursor Position Report) responses back to the
        // PTY so programs like fzf that query the cursor position don't hang.
        for (id, tab_id, kind, response) in app.pending_cpr_responses.drain(..) {
            let _ = backend.cmd_tx.try_send(Command::SendTerminalInput {
                id,
                kind,
                tab_id: Some(tab_id),
                data_b64: base64::engine::general_purpose::STANDARD.encode(response),
            });
        }

        // Respawn agent tab as shell after agent exits.
        if let Some((id, tab_id)) = app.pending_agent_respawn.take() {
            let _ = backend
                .cmd_tx
                .send(Command::StartTerminal {
                    id,
                    kind: protocol::TerminalKind::Shell,
                    tab_id,
                    cmd: Vec::new(),
                })
                .await;
        }

        // Check if deferred git result can now be shown (spinner min duration met).
        if let Some((id, msg)) = app.deferred_git_result.take() {
            if app.finish_git_op(id) {
                app.git_action_message = Some((msg, std::time::Instant::now()));
            } else {
                app.deferred_git_result = Some((id, msg));
            }
        }

        if let Route::Workspace { id } = app.route {
            if let Ok(size) = terminal.size() {
                let area = ratatui::layout::Rect::new(0, 0, size.width, size.height);
                let inner = ui::screens::workspace::terminal_content_rect(
                    area,
                    app.focus,
                    app.terminal_fullscreen(),
                );
                let cols = inner.width.max(1);
                let rows = inner.height.max(1);
                let tid = app.active_tab_id();
                let kind = app.active_tab_kind();
                if app.has_terminal_tab(id, &tid) && app.should_send_resize(id, &tid, cols, rows) {
                    app.resize_terminal_parser(id, &tid, cols, rows);
                    let _ = backend
                        .cmd_tx
                        .send(Command::ResizeTerminal {
                            id,
                            kind,
                            tab_id: Some(tid),
                            cols,
                            rows,
                        })
                        .await;
                }
            }
        }

        // Update cached grid height for scroll calculations.
        if let Ok(size) = terminal.size() {
            let area = Rect::new(0, 0, size.width, size.height);
            app.last_grid_height = ui::screens::home::grid_rect(area).height;
        }

        app.debug_frame = app.debug_frame.wrapping_add(1);
        let mut pending_clipboard_text: Option<String> = None;
        terminal.draw(|frame| {
            match app.route {
                Route::Home => ui::screens::home::render(frame, frame.area(), &app),
                Route::Workspace { .. } => {
                    ui::screens::workspace::render(frame, frame.area(), &app)
                }
            }
            // Extract selected text from the rendered buffer before applying highlights.
            if let Some(sel) = &app.pending_copy_selection {
                let borders = match app.route {
                    Route::Home => ui::screens::home::border_rects(frame.area(), &app),
                    Route::Workspace { .. } => {
                        ui::screens::workspace::border_rects(frame.area(), &app)
                    }
                };
                pending_clipboard_text = Some(extract_selected_text_from_buf(
                    frame.buffer_mut(),
                    sel,
                    &borders,
                ));
            }
            if let Some(sel) = &app.mouse_selection {
                if !sel.is_empty() {
                    apply_selection_highlight(frame, sel);
                }
            }
        })?;
        if let Some(text) = pending_clipboard_text {
            app.pending_copy_selection = None;
            if !text.is_empty() {
                let copied = if cfg!(target_os = "linux") {
                    // On Wayland, arboard's clipboard doesn't persist after drop.
                    // Use wl-copy which forks a background process to serve paste requests.
                    std::process::Command::new("wl-copy")
                        .stdin(std::process::Stdio::piped())
                        .spawn()
                        .and_then(|mut child| {
                            use std::io::Write;
                            if let Some(stdin) = child.stdin.as_mut() {
                                stdin.write_all(text.as_bytes())?;
                            }
                            child.wait()
                        })
                        .is_ok()
                } else {
                    arboard::Clipboard::new()
                        .and_then(|mut clipboard| clipboard.set_text(text))
                        .is_ok()
                };
                if copied {
                    app.git_action_message =
                        Some(("Copied to clipboard".to_string(), Instant::now()));
                }
            }
        }

        while event::poll(Duration::ZERO)? {
            match event::read()? {
                Event::Key(key) => {
                    if matches!(key.kind, KeyEventKind::Release) {
                        continue;
                    }

                    if keymap::is_quit(key)
                        && !app.is_adding_workspace()
                        && !app.is_adding_ssh_workspace()
                        && app.ssh_history_picker.is_none()
                        && !app.is_confirming_delete()
                        && !app.is_renaming_workspace()
                        && !app.is_renaming_tab()
                        && !app.is_committing()
                        && !app.is_creating_branch()
                        && !app.is_confirming_discard()
                        && !app.is_confirming_stash_pull_pop()
                        && !app.is_confirming_delete_branch()
                        && !app.is_stashing()
                        && !app.is_settings_open()
                        && !matches!(app.focus, app::Focus::WsTerminal)
                    {
                        break 'main;
                    }

                    match app.route {
                        Route::Home => {
                            if app.is_settings_open() {
                                if app.confirming_delete_agent {
                                    match key.code {
                                        KeyCode::Char('y') | KeyCode::Char('Y') => {
                                            app.confirm_delete_agent()
                                        }
                                        _ => app.cancel_delete_agent(),
                                    }
                                } else if app.is_adding_agent() {
                                    // New agent wizard text input
                                    match key.code {
                                        KeyCode::Enter => app.new_agent_advance(),
                                        KeyCode::Esc => app.cancel_new_agent(),
                                        KeyCode::Backspace => {
                                            if let Some((_, _, buf)) =
                                                &mut app.new_agent_wizard
                                            {
                                                buf.pop();
                                            }
                                        }
                                        KeyCode::Char(c) => {
                                            if let Some((_, _, buf)) =
                                                &mut app.new_agent_wizard
                                            {
                                                buf.push(c);
                                            }
                                        }
                                        _ => {}
                                    }
                                } else if app.is_editing_setting() {
                                    match key.code {
                                        KeyCode::Enter => app.confirm_setting_edit(),
                                        KeyCode::Esc => app.cancel_setting_edit(),
                                        KeyCode::Backspace => {
                                            if let Some(buf) = &mut app.settings_edit_buffer {
                                                buf.pop();
                                            }
                                        }
                                        KeyCode::Char(c) => {
                                            if let Some(buf) = &mut app.settings_edit_buffer {
                                                buf.push(c);
                                            }
                                        }
                                        _ => {}
                                    }
                                } else {
                                    match key.code {
                                        KeyCode::Esc | KeyCode::Char('S') => {
                                            app.close_settings()
                                        }
                                        KeyCode::Down | KeyCode::Char('j') => {
                                            app.settings_selected = (app.settings_selected + 1)
                                                .min(app.settings_count() - 1);
                                        }
                                        KeyCode::Up | KeyCode::Char('k') => {
                                            app.settings_selected =
                                                app.settings_selected.saturating_sub(1);
                                        }
                                        KeyCode::Enter | KeyCode::Char(' ') => {
                                            app.toggle_selected_setting()
                                        }
                                        KeyCode::Left | KeyCode::Char('h') => {
                                            app.adjust_selected_setting(-1)
                                        }
                                        KeyCode::Right | KeyCode::Char('l') => {
                                            app.adjust_selected_setting(1)
                                        }
                                        KeyCode::Char('n') if app.settings_selected == 0 => {
                                            app.begin_new_agent()
                                        }
                                        KeyCode::Char('d') if app.settings_selected == 0 => {
                                            app.begin_delete_agent()
                                        }
                                        _ => {}
                                    }
                                }
                            } else if app.is_confirming_delete() {
                                match key.code {
                                    KeyCode::Char('y') | KeyCode::Char('Y') => {
                                        if let Some(id) = app.take_delete_workspace() {
                                            let _ = backend
                                                .cmd_tx
                                                .send(Command::RemoveWorkspace { id })
                                                .await;
                                        }
                                    }
                                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                                        app.cancel_delete_workspace()
                                    }
                                    _ => {}
                                }
                            } else if app.ssh_history_picker.is_some() {
                                match key.code {
                                    KeyCode::Char('j') | KeyCode::Down => {
                                        if let Some(ref mut picker) = app.ssh_history_picker {
                                            let len = app.ssh_history.len();
                                            if len > 0 {
                                                picker.selected = (picker.selected + 1) % len;
                                            }
                                        }
                                    }
                                    KeyCode::Char('k') | KeyCode::Up => {
                                        if let Some(ref mut picker) = app.ssh_history_picker {
                                            let len = app.ssh_history.len();
                                            if len > 0 {
                                                picker.selected = (picker.selected + len - 1) % len;
                                            }
                                        }
                                    }
                                    KeyCode::Enter => app.select_ssh_history_entry(),
                                    KeyCode::Char('n') => app.begin_new_ssh_from_picker(),
                                    KeyCode::Esc => app.cancel_ssh_history_picker(),
                                    _ => {}
                                }
                                continue;
                            } else if app.is_adding_ssh_workspace() {
                                match key.code {
                                    KeyCode::Esc => app.cancel_ssh_workspace(),
                                    KeyCode::Tab | KeyCode::BackTab => {
                                        if let Some(ref mut input) = app.ssh_workspace_input {
                                            input.cycle_field();
                                        }
                                    }
                                    KeyCode::Enter => {
                                        // Record history before taking the request
                                        if let Some(ref input) = app.ssh_workspace_input {
                                            let host = input.host.trim().to_string();
                                            let path = input.path.trim().to_string();
                                            if !host.is_empty() && !path.is_empty() {
                                                let user = if input.user.trim().is_empty() {
                                                    None
                                                } else {
                                                    Some(input.user.trim().to_string())
                                                };
                                                app.record_ssh_history(app::SshHistoryEntry {
                                                    host,
                                                    user,
                                                    path,
                                                });
                                            }
                                        }
                                        if let Some((name, path, target)) =
                                            app.take_ssh_workspace_request()
                                        {
                                            let _ = backend
                                                .cmd_tx
                                                .send(Command::AddWorkspace {
                                                    name,
                                                    path,
                                                    ssh: Some(target),
                                                })
                                                .await;
                                        }
                                    }
                                    KeyCode::Char(c) => {
                                        if let Some(ref mut input) = app.ssh_workspace_input {
                                            input.active_input_mut().push(c);
                                        }
                                    }
                                    KeyCode::Backspace => {
                                        if let Some(ref mut input) = app.ssh_workspace_input {
                                            input.active_input_mut().pop();
                                        }
                                    }
                                    _ => {}
                                }
                                continue;
                            } else if app.is_adding_workspace() {
                                let editing =
                                    app.dir_browser.as_ref().map_or(false, |b| b.editing_path);
                                if editing {
                                    match key.code {
                                        KeyCode::Esc => app.cancel_add_workspace(),
                                        KeyCode::Enter => {
                                            if let Some(browser) = app.dir_browser_mut() {
                                                browser.confirm_path_edit();
                                            }
                                        }
                                        KeyCode::Backspace => {
                                            if let Some(browser) = app.dir_browser_mut() {
                                                browser.path_input.pop();
                                            }
                                        }
                                        KeyCode::Tab => {
                                            if let Some(browser) = app.dir_browser_mut() {
                                                apply_path_autocomplete(&mut browser.path_input);
                                                browser.confirm_path_edit();
                                            }
                                        }
                                        KeyCode::Char(c) => {
                                            if let Some(browser) = app.dir_browser_mut() {
                                                browser.path_input.push(c);
                                            }
                                        }
                                        _ => {}
                                    }
                                } else {
                                    match key.code {
                                        KeyCode::Esc => app.cancel_add_workspace(),
                                        KeyCode::Char('j') | KeyCode::Down => {
                                            if let Some(browser) = app.dir_browser_mut() {
                                                browser.move_selection(1);
                                            }
                                        }
                                        KeyCode::Char('k') | KeyCode::Up => {
                                            if let Some(browser) = app.dir_browser_mut() {
                                                browser.move_selection(-1);
                                            }
                                        }
                                        KeyCode::Enter => {
                                            if let Some((name, path)) =
                                                app.take_add_workspace_request()
                                            {
                                                let _ = backend
                                                    .cmd_tx
                                                    .send(Command::AddWorkspace {
                                                        name,
                                                        path,
                                                        ssh: None,
                                                    })
                                                    .await;
                                            }
                                        }
                                        KeyCode::Backspace => {
                                            if let Some(browser) = app.dir_browser_mut() {
                                                browser.go_up();
                                            }
                                        }
                                        KeyCode::Char('.') => {
                                            if let Some(browser) = app.dir_browser_mut() {
                                                browser.toggle_hidden();
                                            }
                                        }
                                        KeyCode::Char('/') => {
                                            if let Some(browser) = app.dir_browser_mut() {
                                                browser.begin_path_edit();
                                            }
                                        }
                                        KeyCode::Tab => {
                                            if let Some(browser) = app.dir_browser_mut() {
                                                browser.enter_selected();
                                            }
                                        }
                                        KeyCode::Char(' ') => {
                                            let child_path = app
                                                .dir_browser
                                                .as_ref()
                                                .and_then(|b| b.selected_child_path());
                                            if let Some(path) = child_path {
                                                if let Some((name, path)) =
                                                    app.take_add_workspace_request_with_path(path)
                                                {
                                                    let _ = backend
                                                        .cmd_tx
                                                        .send(Command::AddWorkspace {
                                                            name,
                                                            path,
                                                            ssh: None,
                                                        })
                                                        .await;
                                                }
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            } else if app.is_renaming_workspace() {
                                match key.code {
                                    KeyCode::Esc => app.cancel_rename_workspace(),
                                    KeyCode::Enter => {
                                        if let Some((id, name)) = app.take_rename_request_home() {
                                            let _ = backend
                                                .cmd_tx
                                                .send(Command::RenameWorkspace { id, name })
                                                .await;
                                        }
                                    }
                                    KeyCode::Backspace => {
                                        if let Some(input) = app.rename_input_mut() {
                                            input.pop();
                                        }
                                    }
                                    KeyCode::Char(c) => {
                                        if let Some(input) = app.rename_input_mut() {
                                            input.push(c);
                                        }
                                    }
                                    _ => {}
                                }
                            } else if app.moving_workspace {
                                match key.code {
                                    KeyCode::Down | KeyCode::Char('j') => {
                                        if let Some((id, delta)) = app.swap_workspace(1) {
                                            let _ = backend
                                                .cmd_tx
                                                .send(Command::MoveWorkspace { id, delta })
                                                .await;
                                        }
                                    }
                                    KeyCode::Up | KeyCode::Char('k') => {
                                        if let Some((id, delta)) = app.swap_workspace(-1) {
                                            let _ = backend
                                                .cmd_tx
                                                .send(Command::MoveWorkspace { id, delta })
                                                .await;
                                        }
                                    }
                                    KeyCode::Esc
                                    | KeyCode::Enter
                                    | KeyCode::Char('M') => {
                                        app.end_move_workspace();
                                    }
                                    _ => {}
                                }
                            } else {
                                // Shift+Enter: send Enter to the agent terminal
                                // without leaving the home screen.
                                if key.code == KeyCode::Enter
                                    && key.modifiers.contains(KeyModifiers::SHIFT)
                                {
                                    if let Some(id) = app.selected_workspace_id() {
                                        let _ = backend
                                            .cmd_tx
                                            .send(Command::SendTerminalInput {
                                                id,
                                                kind: TerminalKind::Agent,
                                                tab_id: None,
                                                data_b64:
                                                    base64::engine::general_purpose::STANDARD
                                                        .encode(b"\r"),
                                            })
                                            .await;
                                    }
                                    continue;
                                }
                                match key.code {
                                    KeyCode::Esc => {
                                        app.go_home();
                                    }
                                    KeyCode::Enter => {
                                        if let Some(id) = app.selected_workspace_id() {
                                            app.open_workspace(id);
                                            start_workspace_tab_terminals(
                                                &backend.cmd_tx,
                                                id,
                                                &app.ws_tabs,
                                                &app.settings,
                                            )
                                            .await;
                                            let _ = backend
                                                .cmd_tx
                                                .send(Command::RefreshGit { id })
                                                .await;
                                            let _ = backend
                                                .cmd_tx
                                                .send(Command::ClearAttention { id })
                                                .await;
                                        }
                                    }
                                    KeyCode::Down | KeyCode::Char('j') => {
                                        app.move_home_selection(1)
                                    }
                                    KeyCode::Up | KeyCode::Char('k') => {
                                        app.move_home_selection(-1)
                                    }
                                    KeyCode::Left | KeyCode::Char('h') => {
                                        if let Some(id) = app.selected_workspace_id() {
                                            let _ = backend
                                                .cmd_tx
                                                .send(Command::SendTerminalInput {
                                                    id,
                                                    kind: TerminalKind::Agent,
                                                    tab_id: None,
                                                    data_b64:
                                                        base64::engine::general_purpose::STANDARD
                                                            .encode(b"\x1b[A"),
                                                })
                                                .await;
                                        }
                                    }
                                    KeyCode::Right | KeyCode::Char('l') => {
                                        if let Some(id) = app.selected_workspace_id() {
                                            let _ = backend
                                                .cmd_tx
                                                .send(Command::SendTerminalInput {
                                                    id,
                                                    kind: TerminalKind::Agent,
                                                    tab_id: None,
                                                    data_b64:
                                                        base64::engine::general_purpose::STANDARD
                                                            .encode(b"\x1b[B"),
                                                })
                                                .await;
                                        }
                                    }
                                    KeyCode::Char(' ') => {
                                        app.toggle_home_expanded_tile()
                                    }
                                    KeyCode::Char('n') => {
                                        let cwd = std::env::current_dir()
                                            .unwrap_or_else(|_| PathBuf::from("."))
                                            .display()
                                            .to_string();
                                        app.begin_add_workspace(cwd);
                                    }
                                    KeyCode::Char('M') => app.begin_move_workspace(),
                                    KeyCode::Char('R') => app.begin_add_ssh_workspace(),
                                    KeyCode::Char('D') => app.begin_delete_workspace(),
                                    KeyCode::Char('e') => app.begin_rename_workspace_home(),
                                    KeyCode::Char('S') => app.open_settings(),
                                    KeyCode::Char('!') => {
                                        if let Some(id) = app.selected_workspace_id() {
                                            let level = app
                                                .workspaces
                                                .get(app.home_selected)
                                                .map(|w| w.attention)
                                                .unwrap_or(AttentionLevel::None);
                                            let cmd = if matches!(
                                                level,
                                                AttentionLevel::NeedsInput | AttentionLevel::Error
                                            ) {
                                                Command::ClearAttention { id }
                                            } else {
                                                Command::SetAttention {
                                                    id,
                                                    level: AttentionLevel::NeedsInput,
                                                }
                                            };
                                            let _ = backend.cmd_tx.send(cmd).await;
                                        }
                                    }
                                    KeyCode::Char('g') => {
                                        if let Some(id) = app.selected_workspace_id() {
                                            let _ = backend
                                                .cmd_tx
                                                .send(Command::RefreshGit { id })
                                                .await;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                        Route::Workspace { id } => {
                            if app.is_renaming_tab() {
                                match key.code {
                                    KeyCode::Esc => app.cancel_rename_tab(),
                                    KeyCode::Enter => app.apply_rename_tab(),
                                    KeyCode::Backspace => {
                                        if let Some(input) = app.rename_tab_input_mut() {
                                            input.pop();
                                        }
                                    }
                                    KeyCode::Char(c) => {
                                        if let Some(input) = app.rename_tab_input_mut() {
                                            input.push(c);
                                        }
                                    }
                                    _ => {}
                                }
                                continue;
                            }

                            if app.is_renaming_workspace() {
                                match key.code {
                                    KeyCode::Esc => app.cancel_rename_workspace(),
                                    KeyCode::Enter => {
                                        if let Some((id, name)) = app.take_rename_request() {
                                            let _ = backend
                                                .cmd_tx
                                                .send(Command::RenameWorkspace { id, name })
                                                .await;
                                        }
                                    }
                                    KeyCode::Backspace => {
                                        if let Some(input) = app.rename_input_mut() {
                                            input.pop();
                                        }
                                    }
                                    KeyCode::Char(c) => {
                                        if let Some(input) = app.rename_input_mut() {
                                            input.push(c);
                                        }
                                    }
                                    _ => {}
                                }
                                continue;
                            }

                            if app.is_creating_branch() {
                                match key.code {
                                    KeyCode::Esc => {
                                        app.cancel_create_branch();
                                    }
                                    KeyCode::Enter => {
                                        if let Some(name) = app.create_branch_input.take() {
                                            let trimmed = name.trim().to_string();
                                            if !trimmed.is_empty() {
                                                app.ws_pending_select_head_branch = true;
                                                let _ = backend
                                                    .cmd_tx
                                                    .send(Command::GitCreateBranch {
                                                        id,
                                                        branch: trimmed,
                                                    })
                                                    .await;
                                            }
                                        }
                                    }
                                    KeyCode::Backspace => {
                                        if let Some(input) = app.create_branch_input.as_mut() {
                                            input.pop();
                                        }
                                    }
                                    KeyCode::Char(c) => {
                                        if let Some(input) = app.create_branch_input.as_mut() {
                                            input.push(c);
                                        }
                                    }
                                    _ => {}
                                }
                                continue;
                            }

                            if app.is_committing() {
                                match key.code {
                                    KeyCode::Esc => {
                                        app.commit_input = None;
                                    }
                                    KeyCode::Enter => {
                                        if let Some(msg) = app.commit_input.take() {
                                            let trimmed = msg.trim().to_string();
                                            if !trimmed.is_empty() {
                                                let _ = backend
                                                    .cmd_tx
                                                    .send(Command::GitCommit {
                                                        id,
                                                        message: trimmed,
                                                    })
                                                    .await;
                                            }
                                        }
                                    }
                                    KeyCode::Backspace => {
                                        if let Some(input) = app.commit_input.as_mut() {
                                            input.pop();
                                        }
                                    }
                                    KeyCode::Char(c) => {
                                        if let Some(input) = app.commit_input.as_mut() {
                                            input.push(c);
                                        }
                                    }
                                    _ => {}
                                }
                                continue;
                            }

                            if app.is_confirming_discard() {
                                match key.code {
                                    KeyCode::Char('y') | KeyCode::Enter => {
                                        if let Some(file) = app.take_discard_file() {
                                            let _ = backend
                                                .cmd_tx
                                                .send(Command::GitDiscardFile { id, file })
                                                .await;
                                        }
                                    }
                                    KeyCode::Char('n') | KeyCode::Esc => {
                                        app.cancel_discard();
                                    }
                                    _ => {}
                                }
                                continue;
                            }

                            if app.is_confirming_stash_pull_pop() {
                                match key.code {
                                    KeyCode::Char('y') | KeyCode::Enter => {
                                        if let Some(ws_id) = app.take_stash_pull_pop() {
                                            app.begin_git_op(ws_id);
                                            let _ = backend
                                                .cmd_tx
                                                .send(Command::GitStashPullPop { id: ws_id })
                                                .await;
                                        }
                                    }
                                    KeyCode::Char('n') | KeyCode::Esc => {
                                        app.cancel_stash_pull_pop();
                                    }
                                    _ => {}
                                }
                                continue;
                            }

                            if app.is_confirming_delete_branch() {
                                match key.code {
                                    KeyCode::Char('y') => {
                                        if let Some(target) = app.take_delete_branch() {
                                            match target {
                                                app::DeleteBranchTarget::Local { branch } => {
                                                    let _ = backend
                                                        .cmd_tx
                                                        .send(Command::GitDeleteLocalBranch {
                                                            id,
                                                            branch,
                                                        })
                                                        .await;
                                                }
                                                app::DeleteBranchTarget::Remote {
                                                    remote,
                                                    branch,
                                                    ..
                                                } => {
                                                    app.begin_git_op(id);
                                                    let _ = backend
                                                        .cmd_tx
                                                        .send(Command::GitDeleteRemoteBranch {
                                                            id,
                                                            remote,
                                                            branch,
                                                        })
                                                        .await;
                                                }
                                            }
                                        }
                                    }
                                    KeyCode::Char('n') | KeyCode::Esc | KeyCode::Enter => {
                                        app.cancel_delete_branch();
                                    }
                                    _ => {}
                                }
                                continue;
                            }

                            if app.is_stashing() {
                                match key.code {
                                    KeyCode::Esc => {
                                        app.stash_input = None;
                                    }
                                    KeyCode::Enter => {
                                        if let Some(msg) = app.stash_input.take() {
                                            let message = if msg.trim().is_empty() {
                                                None
                                            } else {
                                                Some(msg)
                                            };
                                            let _ = backend
                                                .cmd_tx
                                                .send(Command::GitStash { id, message })
                                                .await;
                                        }
                                    }
                                    KeyCode::Backspace => {
                                        if let Some(input) = app.stash_input.as_mut() {
                                            input.pop();
                                        }
                                    }
                                    KeyCode::Char(c) => {
                                        if let Some(input) = app.stash_input.as_mut() {
                                            input.push(c);
                                        }
                                    }
                                    _ => {}
                                }
                                continue;
                            }

                            // Ctrl+G toggles terminal passthrough mode.
                            if key.code == KeyCode::Char('g')
                                && key.modifiers.contains(KeyModifiers::CONTROL)
                                && matches!(app.focus, app::Focus::WsTerminal)
                            {
                                app.toggle_active_tab_passthrough();
                                continue;
                            }

                            // In passthrough mode, forward everything (including Esc/Tab)
                            // to the terminal.
                            if app.active_tab_passthrough()
                                && matches!(app.focus, app::Focus::WsTerminal)
                            {
                                if let Some(bytes) = key_to_terminal_bytes(key) {
                                    let _ = backend
                                        .cmd_tx
                                        .send(Command::SendTerminalInput {
                                            id,
                                            kind: app.active_tab_kind(),
                                            tab_id: Some(app.active_tab_id()),
                                            data_b64: base64::engine::general_purpose::STANDARD
                                                .encode(bytes),
                                        })
                                        .await;
                                }
                                continue;
                            }

                            // Shift+F toggles terminal fullscreen from any workspace pane.
                            if key.code == KeyCode::Char('F') {
                                app.toggle_terminal_fullscreen();
                                continue;
                            }

                            // Shift+Y toggles YOLO mode from any workspace pane.
                            if key.code == KeyCode::Char('Y') {
                                app.toggle_yolo_mode();
                                let _ = backend
                                    .cmd_tx
                                    .send(Command::StopTerminal {
                                        id,
                                        kind: TerminalKind::Agent,
                                        tab_id: Some("agent".to_string()),
                                    })
                                    .await;
                                let _ = backend
                                    .cmd_tx
                                    .send(Command::StartTerminal {
                                        id,
                                        kind: TerminalKind::Agent,
                                        tab_id: Some("agent".to_string()),
                                        cmd: agent_cmd_continue(&app.settings),
                                    })
                                    .await;
                                continue;
                            }

                            if key.code == KeyCode::Esc {
                                if matches!(app.focus, app::Focus::WsTerminal) {
                                    app.focus = app::Focus::WsTerminalTabs;
                                } else if matches!(app.focus, app::Focus::WsBar) {
                                    app.focus = app::Focus::WsTerminal;
                                } else {
                                    app.go_home();
                                }
                                continue;
                            }

                            // Workspace bar navigation
                            if matches!(app.focus, app::Focus::WsBar) {
                                match key.code {
                                    KeyCode::Left | KeyCode::Char('h') => {
                                        app.ws_bar_selected =
                                            app.ws_bar_selected.saturating_sub(1);
                                    }
                                    KeyCode::Right | KeyCode::Char('l') => {
                                        app.ws_bar_selected = (app.ws_bar_selected + 1)
                                            .min(app.workspaces.len().saturating_sub(1));
                                    }
                                    KeyCode::Enter => {
                                        if let Some(target) =
                                            app.workspaces.get(app.ws_bar_selected)
                                        {
                                            let target_id = target.id;
                                            if Some(target_id)
                                                != app.active_workspace_id()
                                            {
                                                app.open_workspace(target_id);
                                                start_workspace_tab_terminals(
                                                    &backend.cmd_tx,
                                                    target_id,
                                                    &app.ws_tabs,
                                                    &app.settings,
                                                )
                                                .await;
                                                let _ = backend
                                                    .cmd_tx
                                                    .send(Command::RefreshGit {
                                                        id: target_id,
                                                    })
                                                    .await;
                                                let _ = backend
                                                    .cmd_tx
                                                    .send(Command::ClearAttention {
                                                        id: target_id,
                                                    })
                                                    .await;
                                            } else {
                                                app.focus = app::Focus::WsTerminal;
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                                continue;
                            }

                            if matches!(app.focus, app::Focus::WsTerminal)
                                && key.code != KeyCode::Tab
                                && key.code != KeyCode::BackTab
                            {
                                if let Some(bytes) = key_to_terminal_bytes(key) {
                                    let _ = backend
                                        .cmd_tx
                                        .send(Command::SendTerminalInput {
                                            id,
                                            kind: app.active_tab_kind(),
                                            tab_id: Some(app.active_tab_id()),
                                            data_b64: base64::engine::general_purpose::STANDARD
                                                .encode(bytes),
                                        })
                                        .await;
                                    continue;
                                }
                            }

                            match key.code {
                                KeyCode::Enter => {
                                    if matches!(app.focus, app::Focus::WsLog) {
                                        match app.log_item_at(app.ws_selected_commit) {
                                            app::LogItem::UncommittedHeader => {
                                                app.ws_uncommitted_expanded =
                                                    !app.ws_uncommitted_expanded;
                                            }
                                            app::LogItem::ChangedFile(_) => {
                                                if let Some(file) = app.selected_log_file() {
                                                    let _ = backend
                                                        .cmd_tx
                                                        .send(Command::LoadDiff { id, file })
                                                        .await;
                                                }
                                            }
                                            app::LogItem::Commit(ci) => {
                                                if app.ws_expanded_commit == Some(ci) {
                                                    app.ws_expanded_commit = None;
                                                } else {
                                                    app.ws_expanded_commit = Some(ci);
                                                    if let Some(hash) = app.selected_commit_hash() {
                                                        if !app
                                                            .commit_files_cache
                                                            .contains_key(&hash)
                                                        {
                                                            let _ = backend
                                                                .cmd_tx
                                                                .send(Command::LoadCommitFiles {
                                                                    id,
                                                                    hash,
                                                                })
                                                                .await;
                                                        }
                                                    }
                                                }
                                            }
                                            app::LogItem::CommitFile(_, _) => {
                                                if let Some((hash, file)) =
                                                    app.selected_commit_file()
                                                {
                                                    let _ = backend
                                                        .cmd_tx
                                                        .send(Command::LoadCommitFileDiff {
                                                            id,
                                                            hash,
                                                            file,
                                                        })
                                                        .await;
                                                }
                                            }
                                        }
                                    }
                                }
                                KeyCode::Tab => {
                                    if key.modifiers.contains(KeyModifiers::SHIFT) {
                                        app.focus = cycle_workspace_focus_reverse(app.focus);
                                    } else {
                                        app.focus = cycle_workspace_focus(app.focus);
                                    }
                                }
                                KeyCode::BackTab => {
                                    app.focus = cycle_workspace_focus_reverse(app.focus)
                                }
                                KeyCode::Char('g') => {
                                    let _ = backend.cmd_tx.send(Command::RefreshGit { id }).await;
                                }
                                KeyCode::Down | KeyCode::Char('j') => match app.focus {
                                    app::Focus::WsLog => {
                                        app.move_workspace_commit_selection(1);
                                        if let Some(file) = app.selected_log_file() {
                                            let _ = backend
                                                .cmd_tx
                                                .send(Command::LoadDiff { id, file })
                                                .await;
                                        } else if let Some((hash, file)) =
                                            app.selected_commit_file()
                                        {
                                            let _ = backend
                                                .cmd_tx
                                                .send(Command::LoadCommitFileDiff {
                                                    id,
                                                    hash,
                                                    file,
                                                })
                                                .await;
                                        }
                                    }
                                    app::Focus::WsBranches => {
                                        app.move_branch_selection(1);
                                    }
                                    app::Focus::WsDiff => {
                                        app.ws_diff_scroll = app.ws_diff_scroll.saturating_add(1)
                                    }
                                    _ => {}
                                },
                                KeyCode::Up | KeyCode::Char('k') => match app.focus {
                                    app::Focus::WsLog => {
                                        app.move_workspace_commit_selection(-1);
                                        if let Some(file) = app.selected_log_file() {
                                            let _ = backend
                                                .cmd_tx
                                                .send(Command::LoadDiff { id, file })
                                                .await;
                                        } else if let Some((hash, file)) =
                                            app.selected_commit_file()
                                        {
                                            let _ = backend
                                                .cmd_tx
                                                .send(Command::LoadCommitFileDiff {
                                                    id,
                                                    hash,
                                                    file,
                                                })
                                                .await;
                                        }
                                    }
                                    app::Focus::WsBranches => {
                                        app.move_branch_selection(-1);
                                    }
                                    app::Focus::WsDiff => {
                                        app.ws_diff_scroll = app.ws_diff_scroll.saturating_sub(1)
                                    }
                                    _ => {}
                                },
                                KeyCode::Char(' ')
                                    if matches!(app.focus, app::Focus::WsLog)
                                        && matches!(
                                            app.log_item_at(app.ws_selected_commit),
                                            app::LogItem::ChangedFile(_)
                                        ) =>
                                {
                                    // Toggle stage/unstage selected file
                                    if let app::LogItem::ChangedFile(fi) =
                                        app.log_item_at(app.ws_selected_commit)
                                    {
                                        if let Some(git) = app.workspace_git.get(&id) {
                                            if let Some(f) = git.changed.get(fi) {
                                                let file = f.path.clone();
                                                let is_staged =
                                                    f.index_status != ' ' && f.index_status != '?';
                                                let cmd = if is_staged {
                                                    Command::GitUnstageFile { id, file }
                                                } else {
                                                    Command::GitStageFile { id, file }
                                                };
                                                let _ = backend.cmd_tx.send(cmd).await;
                                            }
                                        }
                                    }
                                }
                                KeyCode::Char('+')
                                    if matches!(app.focus, app::Focus::WsLog)
                                        && app.log_item_is_file_context() =>
                                {
                                    let _ = backend.cmd_tx.send(Command::GitStageAll { id }).await;
                                }
                                KeyCode::Char('-')
                                    if matches!(app.focus, app::Focus::WsLog)
                                        && app.log_item_is_file_context() =>
                                {
                                    let _ =
                                        backend.cmd_tx.send(Command::GitUnstageAll { id }).await;
                                }
                                KeyCode::Char('c')
                                    if matches!(app.focus, app::Focus::WsLog)
                                        && app.log_item_is_file_context() =>
                                {
                                    app.commit_input = Some(String::new());
                                }
                                KeyCode::Char('d')
                                    if matches!(app.focus, app::Focus::WsLog)
                                        && matches!(
                                            app.log_item_at(app.ws_selected_commit),
                                            app::LogItem::ChangedFile(_)
                                        ) =>
                                {
                                    app.begin_discard();
                                }
                                KeyCode::Char('s')
                                    if matches!(app.focus, app::Focus::WsLog)
                                        && app.log_item_is_file_context() =>
                                {
                                    app.stash_input = Some(String::new());
                                }
                                KeyCode::Char('t') if matches!(app.focus, app::Focus::WsLog) => {
                                    app.ws_tag_filter = !app.ws_tag_filter;
                                    app.ws_selected_commit = app
                                        .ws_selected_commit
                                        .min(app.total_log_items().saturating_sub(1));
                                }
                                KeyCode::Char('c')
                                    if matches!(app.focus, app::Focus::WsBranches) =>
                                {
                                    if matches!(app.ws_branch_sub_pane, app::BranchSubPane::Local) {
                                        app.begin_create_branch();
                                    }
                                }
                                KeyCode::Char('D')
                                    if matches!(app.focus, app::Focus::WsBranches) =>
                                {
                                    app.begin_delete_branch();
                                }
                                KeyCode::Char('[')
                                    if matches!(app.focus, app::Focus::WsBranches) =>
                                {
                                    app.toggle_branch_sub_pane(app::BranchSubPane::Local);
                                }
                                KeyCode::Char(']')
                                    if matches!(app.focus, app::Focus::WsBranches) =>
                                {
                                    app.toggle_branch_sub_pane(app::BranchSubPane::Remote);
                                }
                                KeyCode::Char(' ')
                                    if matches!(app.focus, app::Focus::WsBranches) =>
                                {
                                    match app.ws_branch_sub_pane {
                                        app::BranchSubPane::Local => {
                                            if let Some(branch) = app.selected_local_branch() {
                                                if !branch.is_head {
                                                    let branch_name = branch.name.clone();
                                                    let _ = backend
                                                        .cmd_tx
                                                        .send(Command::GitCheckoutBranch {
                                                            id,
                                                            branch: branch_name,
                                                        })
                                                        .await;
                                                }
                                            }
                                        }
                                        app::BranchSubPane::Remote => {
                                            if let Some(rb) = app.selected_remote_branch() {
                                                let full = rb.full_name.clone();
                                                if let Some(local_name) = full.splitn(2, '/').nth(1)
                                                {
                                                    let local_name = local_name.to_string();
                                                    app.ws_pending_select_head_branch = true;
                                                    app.ws_branch_sub_pane =
                                                        app::BranchSubPane::Local;
                                                    let _ = backend
                                                        .cmd_tx
                                                        .send(Command::GitCheckoutRemoteBranch {
                                                            id,
                                                            remote_branch: full,
                                                            local_name,
                                                        })
                                                        .await;
                                                }
                                            }
                                        }
                                    }
                                }
                                KeyCode::Char('p')
                                    if matches!(app.focus, app::Focus::WsBranches) =>
                                {
                                    let _ = backend.cmd_tx.send(Command::GitPull { id }).await;
                                    app.begin_git_op(id);
                                }
                                KeyCode::Char('f')
                                    if matches!(app.focus, app::Focus::WsBranches) =>
                                {
                                    let _ = backend.cmd_tx.send(Command::GitFetch { id }).await;
                                    app.begin_git_op(id);
                                }
                                KeyCode::Char('P')
                                    if matches!(app.focus, app::Focus::WsBranches) =>
                                {
                                    let _ = backend.cmd_tx.send(Command::GitPush { id }).await;
                                    app.begin_git_op(id);
                                }
                                KeyCode::Char('1') => app.set_active_tab_index(0),
                                KeyCode::Char('2') => app.set_active_tab_index(1),
                                KeyCode::Right | KeyCode::Char('l')
                                    if matches!(app.focus, app::Focus::WsTerminalTabs) =>
                                {
                                    app.move_terminal_tab(1);
                                }
                                KeyCode::Left | KeyCode::Char('h')
                                    if matches!(app.focus, app::Focus::WsTerminalTabs) =>
                                {
                                    app.move_terminal_tab(-1);
                                }
                                KeyCode::Char('n')
                                    if matches!(app.focus, app::Focus::WsTerminalTabs) =>
                                {
                                    app.add_shell_tab();
                                    let _ = backend
                                        .cmd_tx
                                        .send(Command::StartTerminal {
                                            id,
                                            kind: TerminalKind::Shell,
                                            tab_id: Some(app.active_tab_id()),
                                            cmd: Vec::new(),
                                        })
                                        .await;
                                }
                                KeyCode::Char('x')
                                    if matches!(app.focus, app::Focus::WsTerminalTabs) =>
                                {
                                    if let Some(closed) = app.close_active_tab() {
                                        let _ = backend
                                            .cmd_tx
                                            .send(Command::StopTerminal {
                                                id,
                                                kind: closed.kind,
                                                tab_id: Some(closed.id),
                                            })
                                            .await;
                                    }
                                }
                                KeyCode::Char('r')
                                    if matches!(app.focus, app::Focus::WsTerminalTabs) =>
                                {
                                    app.begin_rename_tab();
                                }
                                KeyCode::Char('a')
                                    if matches!(app.focus, app::Focus::WsTerminalTabs) =>
                                {
                                    let _ = backend
                                        .cmd_tx
                                        .send(Command::StartTerminal {
                                            id,
                                            kind: app.active_tab_kind(),
                                            tab_id: Some(app.active_tab_id()),
                                            cmd: Vec::new(),
                                        })
                                        .await;
                                    app.focus = app::Focus::WsTerminal;
                                }
                                KeyCode::Char('A')
                                    if matches!(app.focus, app::Focus::WsTerminalTabs) =>
                                {
                                    let _ = backend
                                        .cmd_tx
                                        .send(Command::StopTerminal {
                                            id,
                                            kind: app.active_tab_kind(),
                                            tab_id: Some(app.active_tab_id()),
                                        })
                                        .await;
                                }
                                KeyCode::Char('s')
                                    if matches!(app.focus, app::Focus::WsTerminalTabs) =>
                                {
                                    let _ = backend
                                        .cmd_tx
                                        .send(Command::StartTerminal {
                                            id,
                                            kind: app.active_tab_kind(),
                                            tab_id: Some(app.active_tab_id()),
                                            cmd: Vec::new(),
                                        })
                                        .await;
                                }
                                KeyCode::Char('S')
                                    if matches!(app.focus, app::Focus::WsTerminalTabs) =>
                                {
                                    let _ = backend
                                        .cmd_tx
                                        .send(Command::StopTerminal {
                                            id,
                                            kind: app.active_tab_kind(),
                                            tab_id: Some(app.active_tab_id()),
                                        })
                                        .await;
                                }
                                _ => {}
                            }
                        }
                    };
                }
                Event::Paste(text) => {
                    if matches!(app.focus, app::Focus::WsTerminal) {
                        if let Route::Workspace { id } = app.route {
                            // Wrap pasted text in bracketed paste sequences so the
                            // inner shell/editor knows it is a paste (prevents
                            // unintended interpretation of special characters).
                            let mut payload = Vec::new();
                            payload.extend_from_slice(b"\x1b[200~");
                            payload.extend_from_slice(text.as_bytes());
                            payload.extend_from_slice(b"\x1b[201~");
                            let _ = backend
                                .cmd_tx
                                .send(Command::SendTerminalInput {
                                    id,
                                    kind: app.active_tab_kind(),
                                    tab_id: Some(app.active_tab_id()),
                                    data_b64: base64::engine::general_purpose::STANDARD
                                        .encode(payload),
                                })
                                .await;
                        }
                    }
                }
                Event::Mouse(mouse) => {
                    handle_mouse(&mut app, &backend.cmd_tx, &mut terminal, mouse).await;
                }
                _ => {}
            }
        }

        if last_flash_toggle.elapsed() >= Duration::from_millis(250) {
            app.spinner_tick = app.spinner_tick.wrapping_add(1);
            last_flash_toggle = Instant::now();
        }
    }

    disable_raw_mode()?;
    let _ = std::io::stdout().execute(PopKeyboardEnhancementFlags);
    std::io::stdout().execute(DisableBracketedPaste)?;
    std::io::stdout().execute(DisableMouseCapture)?;
    std::io::stdout().execute(LeaveAlternateScreen)?;
    Ok(())
}

fn apply_event(app: &mut TuiApp, evt: CoreEvent) {
    match evt {
        CoreEvent::WorkspaceList { items } => app.set_workspaces(items),
        CoreEvent::WorkspaceGitUpdated { id, git } => app.set_workspace_git(id, git),
        CoreEvent::WorkspaceDiffUpdated { id, file, diff } => {
            app.set_workspace_diff(id, file, diff)
        }
        CoreEvent::CommitFilesLoaded { id: _, hash, files } => {
            app.commit_files_cache.insert(hash, files);
        }
        CoreEvent::TerminalOutput {
            id,
            kind,
            data_b64,
            tab_id,
            ..
        } => {
            if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(data_b64) {
                let tid = tab_id.unwrap_or_else(|| "shell".to_string());
                app.append_terminal_bytes(id, &tid, kind, &bytes);
            }
        }
        CoreEvent::TerminalExited {
            id,
            kind,
            code,
            tab_id,
            ..
        } => {
            let msg = format!("\r\n[terminal exited: {:?}]\r\n", code);
            let tid = tab_id.clone().unwrap_or_else(|| "shell".to_string());
            app.append_terminal_bytes(id, &tid, kind, msg.as_bytes());

            // Queue agent tab respawn as shell so user can manually run another agent
            if kind == protocol::TerminalKind::Agent {
                app.pending_agent_respawn = Some((id, tab_id));
            }
        }
        CoreEvent::TerminalStarted {
            id,
            kind,
            tab_id,
            ..
        } => {
            let tid = tab_id.unwrap_or_else(|| "shell".to_string());
            app.reset_terminal(id, &tid);
            app.append_terminal_bytes(id, &tid, kind, b"[terminal started]\r\n");
        }
        CoreEvent::GitActionResult {
            id,
            ref action,
            success,
            ref message,
        } => {
            if action == "pull_dirty_tree" && !success {
                // Cancel the spinner and show confirmation modal instead of toast
                let _ = app.finish_git_op(id);
                app.begin_stash_pull_pop(id);
            } else if app.finish_git_op(id) {
                app.git_action_message = Some((message.clone(), std::time::Instant::now()));
            } else {
                // Spinner minimum duration not met; defer the toast.
                app.deferred_git_result = Some((id, message.clone()));
            }
        }
        CoreEvent::WorkspaceAttentionChanged { id, level } => {
            if let Some(ws) = app.workspaces.iter_mut().find(|w| w.id == id) {
                ws.attention = level;
            }
        }
        CoreEvent::Error { message } => {
            app.git_action_message = Some((message, Instant::now()));
        }
    }
}

fn cycle_workspace_focus(focus: app::Focus) -> app::Focus {
    match focus {
        app::Focus::WsBar => app::Focus::WsTerminalTabs,
        app::Focus::WsTerminalTabs => app::Focus::WsTerminal,
        app::Focus::WsTerminal => app::Focus::WsLog,
        app::Focus::WsLog => app::Focus::WsBranches,
        app::Focus::WsBranches => app::Focus::WsDiff,
        app::Focus::WsDiff => app::Focus::WsBar,
        _ => app::Focus::WsTerminalTabs,
    }
}

fn cycle_workspace_focus_reverse(focus: app::Focus) -> app::Focus {
    match focus {
        app::Focus::WsBar => app::Focus::WsDiff,
        app::Focus::WsTerminalTabs => app::Focus::WsBar,
        app::Focus::WsTerminal => app::Focus::WsTerminalTabs,
        app::Focus::WsLog => app::Focus::WsTerminal,
        app::Focus::WsBranches => app::Focus::WsLog,
        app::Focus::WsDiff => app::Focus::WsBranches,
        _ => app::Focus::WsTerminalTabs,
    }
}

/// Computes the xterm modifier parameter for key modifiers.
/// xterm uses 1 + bitmask where Shift=1, Alt=2, Ctrl=4.
fn xterm_modifier(mods: KeyModifiers) -> u8 {
    let mut m: u8 = 0;
    if mods.contains(KeyModifiers::SHIFT) {
        m |= 1;
    }
    if mods.contains(KeyModifiers::ALT) {
        m |= 2;
    }
    if mods.contains(KeyModifiers::CONTROL) {
        m |= 4;
    }
    1 + m
}

/// Returns true when modifiers beyond SHIFT are present (ALT, CTRL, or combos).
/// SHIFT alone on special keys is handled by BackTab or is implicit in the char.
fn has_extra_modifiers(mods: KeyModifiers) -> bool {
    mods.intersects(
        KeyModifiers::ALT
            .union(KeyModifiers::CONTROL)
            .union(KeyModifiers::SHIFT),
    )
}

/// Encodes a letter-style special key: plain = `\x1b[{letter}`,
/// modified = `\x1b[1;{mod}{letter}`.
fn encode_letter_key(letter: u8, mods: KeyModifiers) -> Vec<u8> {
    if has_extra_modifiers(mods) {
        format!("\x1b[1;{}{}", xterm_modifier(mods), letter as char).into_bytes()
    } else {
        vec![0x1b, b'[', letter]
    }
}

/// Encodes a tilde-style special key: plain = `\x1b[{code}~`,
/// modified = `\x1b[{code};{mod}~`.
fn encode_tilde_key(code: u16, mods: KeyModifiers) -> Vec<u8> {
    if has_extra_modifiers(mods) {
        format!("\x1b[{};{}~", code, xterm_modifier(mods)).into_bytes()
    } else {
        format!("\x1b[{}~", code).into_bytes()
    }
}

fn key_to_terminal_bytes(key: KeyEvent) -> Option<Vec<u8>> {
    let mods = key.modifiers;
    let has_alt = mods.contains(KeyModifiers::ALT);

    match key.code {
        KeyCode::Char(c) => {
            if mods.contains(KeyModifiers::CONTROL) {
                let b = (c as u8) & 0x1f;
                if has_alt {
                    Some(vec![0x1b, b])
                } else {
                    Some(vec![b])
                }
            } else if has_alt {
                let mut bytes = vec![0x1b];
                bytes.extend(c.to_string().into_bytes());
                Some(bytes)
            } else {
                Some(c.to_string().into_bytes())
            }
        }
        KeyCode::Null => Some(vec![0x00]),
        KeyCode::Esc => Some(vec![0x1b]),
        KeyCode::Enter => {
            let base = if mods.contains(KeyModifiers::SHIFT) {
                b'\n'
            } else {
                b'\r'
            };
            if has_alt {
                Some(vec![0x1b, base])
            } else {
                Some(vec![base])
            }
        }
        KeyCode::Backspace => {
            if has_alt {
                Some(vec![0x1b, 0x7f])
            } else {
                Some(vec![0x7f])
            }
        }
        KeyCode::Tab => {
            if has_alt {
                Some(vec![0x1b, b'\t'])
            } else {
                Some(vec![b'\t'])
            }
        }
        KeyCode::BackTab => Some(b"\x1b[Z".to_vec()),
        KeyCode::Left => Some(encode_letter_key(b'D', mods)),
        KeyCode::Right => Some(encode_letter_key(b'C', mods)),
        KeyCode::Up => Some(encode_letter_key(b'A', mods)),
        KeyCode::Down => Some(encode_letter_key(b'B', mods)),
        KeyCode::Home => Some(encode_letter_key(b'H', mods)),
        KeyCode::End => Some(encode_letter_key(b'F', mods)),
        KeyCode::PageUp => Some(encode_tilde_key(5, mods)),
        KeyCode::PageDown => Some(encode_tilde_key(6, mods)),
        KeyCode::Insert => Some(encode_tilde_key(2, mods)),
        KeyCode::Delete => Some(encode_tilde_key(3, mods)),
        KeyCode::F(n) => {
            // F1-F4 use SS3 encoding (plain) or CSI 1;mod (modified)
            // F5-F12 use tilde encoding
            match n {
                1 => {
                    if has_extra_modifiers(mods) {
                        Some(format!("\x1b[1;{}P", xterm_modifier(mods)).into_bytes())
                    } else {
                        Some(b"\x1bOP".to_vec())
                    }
                }
                2 => {
                    if has_extra_modifiers(mods) {
                        Some(format!("\x1b[1;{}Q", xterm_modifier(mods)).into_bytes())
                    } else {
                        Some(b"\x1bOQ".to_vec())
                    }
                }
                3 => {
                    if has_extra_modifiers(mods) {
                        Some(format!("\x1b[1;{}R", xterm_modifier(mods)).into_bytes())
                    } else {
                        Some(b"\x1bOR".to_vec())
                    }
                }
                4 => {
                    if has_extra_modifiers(mods) {
                        Some(format!("\x1b[1;{}S", xterm_modifier(mods)).into_bytes())
                    } else {
                        Some(b"\x1bOS".to_vec())
                    }
                }
                5 => Some(encode_tilde_key(15, mods)),
                6 => Some(encode_tilde_key(17, mods)),
                7 => Some(encode_tilde_key(18, mods)),
                8 => Some(encode_tilde_key(19, mods)),
                9 => Some(encode_tilde_key(20, mods)),
                10 => Some(encode_tilde_key(21, mods)),
                11 => Some(encode_tilde_key(23, mods)),
                12 => Some(encode_tilde_key(24, mods)),
                _ => None,
            }
        }
        _ => None,
    }
}

fn apply_path_autocomplete(input: &mut String) {
    let current = input.trim();
    let (dir, prefix) = split_dir_and_prefix(current);
    let entries = match std::fs::read_dir(&dir) {
        Ok(rd) => rd,
        Err(_) => return,
    };

    let mut matches = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with(&prefix) {
            matches.push((name, entry.path().is_dir()));
        }
    }
    if matches.is_empty() {
        return;
    }
    matches.sort_by(|a, b| a.0.cmp(&b.0));

    let common = longest_common_prefix(
        &matches
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
    );
    let replacement = if common.len() > prefix.len() {
        common
    } else {
        matches[0].0.clone()
    };

    let mut completed = if dir.as_os_str().is_empty() || dir == Path::new(".") {
        replacement
    } else {
        format!("{}/{}", dir.display(), replacement)
    };

    if matches.len() == 1 && matches[0].1 {
        completed.push('/');
    }
    *input = completed;
}

fn split_dir_and_prefix(input: &str) -> (PathBuf, String) {
    if input.is_empty() {
        return (PathBuf::from("."), String::new());
    }
    if input.ends_with('/') {
        return (PathBuf::from(input), String::new());
    }
    let path = Path::new(input);
    let dir = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let prefix = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    (dir, prefix)
}

fn longest_common_prefix(parts: &[&str]) -> String {
    let Some(first) = parts.first() else {
        return String::new();
    };
    let mut end = first.len();
    for part in parts.iter().skip(1) {
        while end > 0 && !part.starts_with(&first[..end]) {
            end -= 1;
        }
        if end == 0 {
            break;
        }
    }
    first[..end].to_string()
}

async fn handle_mouse(
    app: &mut TuiApp,
    cmd_tx: &tokio::sync::mpsc::Sender<Command>,
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    mouse: MouseEvent,
) {
    let area = match terminal.size() {
        Ok(s) => ratatui::layout::Rect::new(0, 0, s.width, s.height),
        Err(_) => return,
    };

    if forward_mouse_to_terminal(app, cmd_tx, area, mouse).await {
        return;
    }

    // Handle drag selection (works across all routes/panes)
    match mouse.kind {
        MouseEventKind::Drag(MouseButton::Left) => {
            if let Some(sel) = &mut app.mouse_selection {
                sel.end_col = mouse.column;
                sel.end_row = mouse.row;
                sel.clamp_to_confine();
            }
            return;
        }
        MouseEventKind::Up(MouseButton::Left) => {
            if let Some(sel) = app.mouse_selection.take() {
                if !sel.is_empty() {
                    app.pending_copy_selection = Some(sel);
                }
            }
            return;
        }
        _ => {}
    }

    match app.route {
        Route::Home => match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                app.mouse_selection =
                    if let Some(r) = ui::screens::home::pane_rect_at(area, app, mouse.column, mouse.row) {
                        Some(app::MouseSelection::at_confined(mouse.column, mouse.row, r))
                    } else {
                        Some(app::MouseSelection::at(mouse.column, mouse.row))
                    };
                if app.is_confirming_delete() {
                    let rect = ui::screens::home::delete_modal_rect(area);
                    if point_in_rect(rect, mouse.column, mouse.row) {
                        let mid = rect.x + rect.width / 2;
                        if mouse.column < mid {
                            if let Some(id) = app.take_delete_workspace() {
                                let _ = cmd_tx.send(Command::RemoveWorkspace { id }).await;
                            }
                        } else {
                            app.cancel_delete_workspace();
                        }
                    } else {
                        app.cancel_delete_workspace();
                    }
                    return;
                }
                if app.is_adding_workspace() {
                    let rect = ui::screens::home::add_modal_rect(area);
                    if !point_in_rect(rect, mouse.column, mouse.row) {
                        app.cancel_add_workspace();
                    }
                    return;
                }

                let grid = ui::screens::home::grid_rect(area);
                let expanded_h = ui::widgets::tile_grid::tile_h_expanded(app.settings.preview_lines);
                if let Some(idx) = ui::widgets::tile_grid::index_at(
                    grid,
                    mouse.column,
                    mouse.row,
                    app.workspaces.len(),
                    &app.home_expanded_tiles,
                    expanded_h,
                    app.home_scroll_offset,
                ) {
                    app.set_home_selection(idx);
                    if let Some(id) = app.selected_workspace_id() {
                        app.open_workspace(id);
                        start_workspace_tab_terminals(cmd_tx, id, &app.ws_tabs, &app.settings).await;
                        let _ = cmd_tx.send(Command::RefreshGit { id }).await;
                        let _ = cmd_tx.send(Command::ClearAttention { id }).await;
                    }
                }
            }
            MouseEventKind::ScrollUp => {
                app.scroll_home(-3);
            }
            MouseEventKind::ScrollDown => {
                app.scroll_home(3);
            }
            _ => {}
        },
        Route::Workspace { id } => match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                app.mouse_selection =
                    if let Some(r) = ui::screens::workspace::pane_rect_at(area, app, mouse.column, mouse.row) {
                        Some(app::MouseSelection::at_confined(mouse.column, mouse.row, r))
                    } else {
                        Some(app::MouseSelection::at(mouse.column, mouse.row))
                    };
                if let Some(hit) =
                    ui::screens::workspace::hit_test(area, app, mouse.column, mouse.row)
                {
                    match hit {
                        ui::screens::workspace::WorkspaceHit::WorkspaceBarPill(idx) => {
                            if let Some(target) = app.workspaces.get(idx) {
                                let target_id = target.id;
                                if Some(target_id) != app.active_workspace_id() {
                                    app.open_workspace(target_id);
                                    start_workspace_tab_terminals(
                                        cmd_tx, target_id, &app.ws_tabs, &app.settings,
                                    )
                                    .await;
                                    let _ = cmd_tx.send(Command::RefreshGit { id: target_id }).await;
                                    let _ = cmd_tx.send(Command::ClearAttention { id: target_id }).await;
                                }
                            }
                        }
                        ui::screens::workspace::WorkspaceHit::TerminalTab(idx) => {
                            app.focus = app::Focus::WsTerminalTabs;
                            app.set_active_tab_index(idx);
                        }
                        ui::screens::workspace::WorkspaceHit::TerminalPane => {
                            app.focus = app::Focus::WsTerminal;
                        }
                        ui::screens::workspace::WorkspaceHit::LogList(idx) => {
                            app.focus = app::Focus::WsLog;
                            app.ws_selected_commit = idx;
                            if let Some(file) = app.selected_log_file() {
                                let _ = cmd_tx.send(Command::LoadDiff { id, file }).await;
                            } else if let Some((hash, file)) = app.selected_commit_file() {
                                let _ = cmd_tx
                                    .send(Command::LoadCommitFileDiff { id, hash, file })
                                    .await;
                            }
                        }
                        ui::screens::workspace::WorkspaceHit::BranchesPane(idx) => {
                            app.focus = app::Focus::WsBranches;
                            match app.ws_branch_sub_pane {
                                app::BranchSubPane::Local => {
                                    app.ws_selected_local_branch = idx;
                                }
                                app::BranchSubPane::Remote => {
                                    app.ws_selected_remote_branch = idx;
                                }
                            }
                        }
                        ui::screens::workspace::WorkspaceHit::DiffPane => {
                            app.focus = app::Focus::WsDiff;
                        }
                    }
                }
            }
            MouseEventKind::ScrollUp => {
                let hit = ui::screens::workspace::hit_test(area, app, mouse.column, mouse.row);
                if matches!(app.focus, app::Focus::WsDiff)
                    || matches!(hit, Some(ui::screens::workspace::WorkspaceHit::DiffPane))
                {
                    app.ws_diff_scroll = app.ws_diff_scroll.saturating_sub(3);
                } else if matches!(app.focus, app::Focus::WsTerminal)
                    || matches!(
                        hit,
                        Some(ui::screens::workspace::WorkspaceHit::TerminalPane)
                    )
                {
                    let tab_id = app.active_tab_id();
                    app.scroll_terminal_scrollback(id, &tab_id, 3);
                }
            }
            MouseEventKind::ScrollDown => {
                let hit = ui::screens::workspace::hit_test(area, app, mouse.column, mouse.row);
                if matches!(app.focus, app::Focus::WsDiff)
                    || matches!(hit, Some(ui::screens::workspace::WorkspaceHit::DiffPane))
                {
                    app.ws_diff_scroll = app.ws_diff_scroll.saturating_add(3);
                } else if matches!(app.focus, app::Focus::WsTerminal)
                    || matches!(
                        hit,
                        Some(ui::screens::workspace::WorkspaceHit::TerminalPane)
                    )
                {
                    let tab_id = app.active_tab_id();
                    app.scroll_terminal_scrollback(id, &tab_id, -3);
                }
            }
            _ => {}
        },
    }
}

fn point_in_rect(r: ratatui::layout::Rect, x: u16, y: u16) -> bool {
    x >= r.x && y >= r.y && x < r.right() && y < r.bottom()
}

async fn forward_mouse_to_terminal(
    app: &mut TuiApp,
    cmd_tx: &tokio::sync::mpsc::Sender<Command>,
    area: ratatui::layout::Rect,
    mouse: MouseEvent,
) -> bool {
    let Route::Workspace { id } = app.route else {
        return false;
    };

    let hit = ui::screens::workspace::hit_test(area, app, mouse.column, mouse.row);
    if !matches!(
        hit,
        Some(ui::screens::workspace::WorkspaceHit::TerminalPane)
    ) {
        return false;
    }

    let content =
        ui::screens::workspace::terminal_content_rect(area, app.focus, app.terminal_fullscreen());
    if !point_in_rect(content, mouse.column, mouse.row) {
        return false;
    }

    let tab_id = app.active_tab_id();
    let kind = app.active_tab_kind();
    let Some((mode, encoding, alternate_screen)) = app.terminal_mouse_state(id, &tab_id) else {
        return false;
    };
    if !should_forward_mouse_event_to_terminal(mouse.kind, mode, alternate_screen) {
        return false;
    }

    if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
        app.focus = app::Focus::WsTerminal;
    }

    let col = mouse.column.saturating_sub(content.x).saturating_add(1);
    let row = mouse.row.saturating_sub(content.y).saturating_add(1);
    let Some(bytes) = encode_terminal_mouse(mouse, col, row, mode, encoding) else {
        return false;
    };

    let _ = cmd_tx
        .send(Command::SendTerminalInput {
            id,
            kind,
            tab_id: Some(tab_id),
            data_b64: base64::engine::general_purpose::STANDARD.encode(bytes),
        })
        .await;
    true
}

fn should_forward_mouse_event_to_terminal(
    kind: MouseEventKind,
    mode: MouseProtocolMode,
    alternate_screen: bool,
) -> bool {
    if matches!(mode, MouseProtocolMode::None) {
        return false;
    }

    if matches!(kind, MouseEventKind::ScrollUp | MouseEventKind::ScrollDown) {
        return alternate_screen;
    }

    true
}

fn encode_terminal_mouse(
    mouse: MouseEvent,
    col: u16,
    row: u16,
    mode: MouseProtocolMode,
    encoding: MouseProtocolEncoding,
) -> Option<Vec<u8>> {
    let modifiers = encode_mouse_modifiers(mouse.modifiers);
    let report = match mouse.kind {
        MouseEventKind::Down(button) => {
            if matches!(mode, MouseProtocolMode::None) {
                return None;
            }
            MouseReport {
                cb: encode_mouse_button(button)? | modifiers,
                release: false,
            }
        }
        MouseEventKind::Up(button) => {
            if matches!(mode, MouseProtocolMode::Press | MouseProtocolMode::None) {
                return None;
            }
            let cb = if matches!(encoding, MouseProtocolEncoding::Sgr) {
                encode_mouse_button(button)?
            } else {
                3
            };
            MouseReport {
                cb: cb | modifiers,
                release: matches!(encoding, MouseProtocolEncoding::Sgr),
            }
        }
        MouseEventKind::Drag(button) => {
            if !matches!(
                mode,
                MouseProtocolMode::ButtonMotion | MouseProtocolMode::AnyMotion
            ) {
                return None;
            }
            MouseReport {
                cb: encode_mouse_button(button)? | modifiers | 32,
                release: false,
            }
        }
        MouseEventKind::Moved => {
            if !matches!(mode, MouseProtocolMode::AnyMotion) {
                return None;
            }
            MouseReport {
                cb: 3 | modifiers | 32,
                release: false,
            }
        }
        MouseEventKind::ScrollUp => MouseReport {
            cb: 64 | modifiers,
            release: false,
        },
        MouseEventKind::ScrollDown => MouseReport {
            cb: 65 | modifiers,
            release: false,
        },
        MouseEventKind::ScrollLeft => MouseReport {
            cb: 66 | modifiers,
            release: false,
        },
        MouseEventKind::ScrollRight => MouseReport {
            cb: 67 | modifiers,
            release: false,
        },
    };

    encode_mouse_report(report, col, row, encoding)
}

struct MouseReport {
    cb: u16,
    release: bool,
}

fn encode_mouse_button(button: MouseButton) -> Option<u16> {
    match button {
        MouseButton::Left => Some(0),
        MouseButton::Middle => Some(1),
        MouseButton::Right => Some(2),
    }
}

fn encode_mouse_modifiers(modifiers: KeyModifiers) -> u16 {
    let mut bits = 0;
    if modifiers.contains(KeyModifiers::SHIFT) {
        bits |= 4;
    }
    if modifiers.contains(KeyModifiers::ALT) {
        bits |= 8;
    }
    if modifiers.contains(KeyModifiers::CONTROL) {
        bits |= 16;
    }
    bits
}

fn encode_mouse_report(
    report: MouseReport,
    col: u16,
    row: u16,
    encoding: MouseProtocolEncoding,
) -> Option<Vec<u8>> {
    match encoding {
        MouseProtocolEncoding::Sgr => Some(
            format!(
                "\x1b[<{};{};{}{}",
                report.cb,
                col,
                row,
                if report.release { 'm' } else { 'M' }
            )
            .into_bytes(),
        ),
        MouseProtocolEncoding::Default => {
            let cb = u8::try_from(report.cb + 32).ok()?;
            let cx = u8::try_from(col + 32).ok()?;
            let cy = u8::try_from(row + 32).ok()?;
            Some(vec![b'\x1b', b'[', b'M', cb, cx, cy])
        }
        MouseProtocolEncoding::Utf8 => {
            let mut out = vec![b'\x1b', b'[', b'M'];
            push_utf8_mouse_value(&mut out, report.cb + 32)?;
            push_utf8_mouse_value(&mut out, col + 32)?;
            push_utf8_mouse_value(&mut out, row + 32)?;
            Some(out)
        }
    }
}

fn push_utf8_mouse_value(out: &mut Vec<u8>, value: u16) -> Option<()> {
    let ch = char::from_u32(u32::from(value))?;
    let mut buf = [0; 4];
    let encoded = ch.encode_utf8(&mut buf);
    out.extend_from_slice(encoded.as_bytes());
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_screen_terminals_keep_wheel_events_local() {
        assert!(!should_forward_mouse_event_to_terminal(
            MouseEventKind::ScrollUp,
            MouseProtocolMode::Press,
            false,
        ));
        assert!(!should_forward_mouse_event_to_terminal(
            MouseEventKind::ScrollDown,
            MouseProtocolMode::AnyMotion,
            false,
        ));
    }

    #[test]
    fn alternate_screen_terminals_receive_wheel_events() {
        assert!(should_forward_mouse_event_to_terminal(
            MouseEventKind::ScrollUp,
            MouseProtocolMode::Press,
            true,
        ));
    }

    #[test]
    fn mouse_clicks_still_forward_when_mouse_mode_is_enabled() {
        assert!(should_forward_mouse_event_to_terminal(
            MouseEventKind::Down(MouseButton::Left),
            MouseProtocolMode::ButtonMotion,
            false,
        ));
    }

    #[test]
    fn mouse_events_do_not_forward_without_mouse_mode() {
        assert!(!should_forward_mouse_event_to_terminal(
            MouseEventKind::Down(MouseButton::Left),
            MouseProtocolMode::None,
            true,
        ));
    }

    #[test]
    fn alternate_screen_without_mouse_mode_keeps_wheel_events_local() {
        assert!(!should_forward_mouse_event_to_terminal(
            MouseEventKind::ScrollUp,
            MouseProtocolMode::None,
            true,
        ));
        assert!(!should_forward_mouse_event_to_terminal(
            MouseEventKind::ScrollDown,
            MouseProtocolMode::None,
            true,
        ));
    }

    #[test]
    fn page_navigation_keys_are_forwarded_to_terminals() {
        assert_eq!(
            key_to_terminal_bytes(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE)),
            Some(b"\x1b[5~".to_vec()),
        );
        assert_eq!(
            key_to_terminal_bytes(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE)),
            Some(b"\x1b[6~".to_vec()),
        );
    }

    #[test]
    fn parses_latest_release_tag_from_json() {
        let body = r#"{"tag_name":"v0.3.21"}"#;
        assert_eq!(parse_latest_release_tag(body).unwrap(), "v0.3.21");
    }

    #[test]
    fn rejects_release_json_without_tag_name() {
        let body = r#"{"name":"v0.3.21"}"#;
        assert!(parse_latest_release_tag(body).is_err());
    }

    #[test]
    fn parses_version_output() {
        assert_eq!(parse_conduit_version_output("conduit 0.3.21\n"), Some("0.3.21"));
        assert_eq!(
            parse_conduit_version_output("\nconduit v0.3.21\n"),
            Some("0.3.21")
        );
    }

    #[test]
    fn rejects_unexpected_version_output() {
        assert_eq!(parse_conduit_version_output("0.3.21\n"), None);
    }

    #[test]
    fn detects_supported_release_targets() {
        assert_eq!(
            detect_release_target("darwin", "arm64").unwrap(),
            "aarch64-apple-darwin"
        );
        assert_eq!(
            detect_release_target("linux", "x86_64").unwrap(),
            "x86_64-unknown-linux-gnu"
        );
    }

    #[test]
    fn rejects_unsupported_release_targets() {
        assert!(detect_release_target("darwin", "x86_64").is_err());
    }

    // --- is_border_cell tests ---

    #[test]
    fn border_cell_on_outer_ring() {
        let r = ratatui::layout::Rect::new(5, 5, 10, 8);
        let rects = vec![r];
        // Top edge
        assert!(is_border_cell(5, 5, &rects));
        assert!(is_border_cell(10, 5, &rects));
        // Bottom edge
        assert!(is_border_cell(5, 12, &rects));
        assert!(is_border_cell(14, 12, &rects));
        // Left edge
        assert!(is_border_cell(5, 8, &rects));
        // Right edge
        assert!(is_border_cell(14, 8, &rects));
    }

    #[test]
    fn border_cell_interior_not_detected() {
        let r = ratatui::layout::Rect::new(5, 5, 10, 8);
        let rects = vec![r];
        assert!(!is_border_cell(6, 6, &rects));
        assert!(!is_border_cell(10, 10, &rects));
    }

    #[test]
    fn border_cell_outside_not_detected() {
        let r = ratatui::layout::Rect::new(5, 5, 10, 8);
        let rects = vec![r];
        assert!(!is_border_cell(4, 5, &rects));
        assert!(!is_border_cell(15, 5, &rects));
        assert!(!is_border_cell(5, 4, &rects));
        assert!(!is_border_cell(5, 13, &rects));
    }

    #[test]
    fn border_cell_empty_rects() {
        assert!(!is_border_cell(5, 5, &[]));
    }

    // --- extract_selected_text_from_buf with border rects ---

    #[test]
    fn extract_text_replaces_border_cells_with_spaces() {
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;

        // Create a 10x3 buffer simulating a small bordered pane
        let area = Rect::new(0, 0, 10, 3);
        let mut buf = Buffer::empty(area);

        // Fill border ring with box-drawing chars
        for col in 0..10u16 {
            buf.cell_mut(ratatui::layout::Position::new(col, 0))
                .unwrap()
                .set_symbol("─");
            buf.cell_mut(ratatui::layout::Position::new(col, 2))
                .unwrap()
                .set_symbol("─");
        }
        for row in 0..3u16 {
            buf.cell_mut(ratatui::layout::Position::new(0, row))
                .unwrap()
                .set_symbol("│");
            buf.cell_mut(ratatui::layout::Position::new(9, row))
                .unwrap()
                .set_symbol("│");
        }
        // Interior content
        for (i, ch) in "hello   ".chars().enumerate() {
            buf.cell_mut(ratatui::layout::Position::new(1 + i as u16, 1))
                .unwrap()
                .set_symbol(&ch.to_string());
        }

        let sel = app::MouseSelection {
            anchor_col: 0,
            anchor_row: 0,
            end_col: 9,
            end_row: 2,
            confine: None,
        };
        let border_rects = vec![area];
        let text = extract_selected_text_from_buf(&buf, &sel, &border_rects);
        // Border chars should be replaced; only content remains
        assert!(!text.contains('─'));
        assert!(!text.contains('│'));
        assert!(text.contains("hello"));
    }

    #[test]
    fn extract_text_preserves_interior_box_drawing() {
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;

        // 12x3 buffer with a border rect on [0..10], but box-drawing in interior
        let area = Rect::new(0, 0, 12, 3);
        let mut buf = Buffer::empty(area);

        // Fill everything with spaces first
        for row in 0..3u16 {
            for col in 0..12u16 {
                buf.cell_mut(ratatui::layout::Position::new(col, row))
                    .unwrap()
                    .set_symbol(" ");
            }
        }

        // Put a box-drawing char in the interior (simulating `tree` output)
        buf.cell_mut(ratatui::layout::Position::new(5, 1))
            .unwrap()
            .set_symbol("├");

        let sel = app::MouseSelection {
            anchor_col: 0,
            anchor_row: 0,
            end_col: 11,
            end_row: 2,
            confine: None,
        };
        // Border rect covers only columns 0..10
        let border_rects = vec![Rect::new(0, 0, 10, 3)];
        let text = extract_selected_text_from_buf(&buf, &sel, &border_rects);
        // The ├ at (5,1) is interior to the border rect, so it IS preserved
        assert!(text.contains('├'));
    }

    #[test]
    fn extract_text_collapses_blank_lines() {
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;

        let area = Rect::new(0, 0, 10, 5);
        let mut buf = Buffer::empty(area);

        // Fill with spaces
        for row in 0..5u16 {
            for col in 0..10u16 {
                buf.cell_mut(ratatui::layout::Position::new(col, row))
                    .unwrap()
                    .set_symbol(" ");
            }
        }
        // Put text on row 0 and row 4, leaving rows 1-3 blank
        for (i, ch) in "top".chars().enumerate() {
            buf.cell_mut(ratatui::layout::Position::new(i as u16, 0))
                .unwrap()
                .set_symbol(&ch.to_string());
        }
        for (i, ch) in "bot".chars().enumerate() {
            buf.cell_mut(ratatui::layout::Position::new(i as u16, 4))
                .unwrap()
                .set_symbol(&ch.to_string());
        }

        let sel = app::MouseSelection {
            anchor_col: 0,
            anchor_row: 0,
            end_col: 9,
            end_row: 4,
            confine: None,
        };
        let text = extract_selected_text_from_buf(&buf, &sel, &[]);
        // Should have at most one blank line between "top" and "bot"
        assert!(text.contains("top"));
        assert!(text.contains("bot"));
        assert!(!text.contains("\n\n\n"));
    }
}

/// xterm-256 colour 39 — a medium sky-blue used for mouse selection highlighting.
const SELECTION_BG: ratatui::style::Color = ratatui::style::Color::Indexed(39);

fn apply_selection_highlight(frame: &mut ratatui::Frame, sel: &app::MouseSelection) {
    let ((start_col, start_row), (end_col, end_row)) = sel.ordered();
    let buf = frame.buffer_mut();
    let width = buf.area.width;
    for row in start_row..=end_row {
        let row_start = if row == start_row { start_col } else { 0 };
        let row_end = if row == end_row {
            end_col
        } else {
            width.saturating_sub(1)
        };
        for col in row_start..=row_end {
            if let Some(cell) = buf.cell_mut(ratatui::layout::Position::new(col, row)) {
                cell.set_style(
                    ratatui::style::Style::default()
                        .bg(SELECTION_BG)
                        .fg(ratatui::style::Color::Black),
                );
            }
        }
    }
}

/// Returns `true` if `(col, row)` falls on the outer 1-cell border ring of any rect.
fn is_border_cell(col: u16, row: u16, border_rects: &[ratatui::layout::Rect]) -> bool {
    for r in border_rects {
        if col >= r.x && col < r.right() && row >= r.y && row < r.bottom() {
            if col == r.x || col == r.right() - 1 || row == r.y || row == r.bottom() - 1 {
                return true;
            }
        }
    }
    false
}

fn extract_selected_text_from_buf(
    buf: &ratatui::buffer::Buffer,
    sel: &app::MouseSelection,
    border_rects: &[ratatui::layout::Rect],
) -> String {
    let ((start_col, start_row), (end_col, end_row)) = sel.ordered();
    let width = buf.area.width;
    let mut lines: Vec<String> = Vec::new();
    for row in start_row..=end_row {
        let row_start = if row == start_row { start_col } else { 0 };
        let row_end = if row == end_row {
            end_col
        } else {
            width.saturating_sub(1)
        };
        let mut line = String::new();
        for col in row_start..=row_end {
            if is_border_cell(col, row, border_rects) {
                line.push(' ');
            } else if let Some(cell) = buf.cell(ratatui::layout::Position::new(col, row)) {
                line.push_str(cell.symbol());
            }
        }
        lines.push(line.trim_end().to_string());
    }

    // Collapse consecutive blank lines to at most one
    let mut result = String::new();
    let mut prev_blank = false;
    for line in &lines {
        let is_blank = line.trim().is_empty();
        if is_blank && prev_blank {
            continue;
        }
        if !result.is_empty() {
            result.push('\n');
        }
        result.push_str(line);
        prev_blank = is_blank;
    }

    // Strip leading/trailing blank lines
    let trimmed = result.trim_matches('\n');
    trimmed.to_string()
}

fn agent_cmd(settings: &app::Settings) -> Vec<String> {
    let Some(agent) = settings.active_agent() else {
        return vec!["claude".to_string()];
    };
    let mut cmd = vec![agent.command.clone()];
    if settings.yolo_mode {
        cmd.extend(agent.yolo_flags.iter().cloned());
    }
    cmd
}

fn agent_cmd_continue(settings: &app::Settings) -> Vec<String> {
    let Some(agent) = settings.active_agent() else {
        return vec!["claude".to_string()];
    };
    let mut cmd = vec![agent.command.clone()];
    cmd.extend(agent.continue_flags.iter().cloned());
    if settings.yolo_mode {
        cmd.extend(agent.yolo_flags.iter().cloned());
    }
    cmd
}

async fn start_workspace_tab_terminals(
    cmd_tx: &tokio::sync::mpsc::Sender<Command>,
    id: protocol::WorkspaceId,
    tabs: &[app::TerminalTab],
    settings: &app::Settings,
) {
    for tab in tabs {
        let cmd = if tab.kind == protocol::TerminalKind::Agent {
            agent_cmd(settings)
        } else {
            Vec::new()
        };
        let _ = cmd_tx
            .send(Command::StartTerminal {
                id,
                kind: tab.kind,
                tab_id: Some(tab.id.clone()),
                cmd,
            })
            .await;
    }
}
