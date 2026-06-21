mod app;
mod keymap;
mod resurrect;
mod terminal_core;
mod ui;

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command as OsCommand, Stdio};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use app::TuiApp;
use base64::Engine as _;
use conduit_core::{spawn_core, CoreHandle};
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
use protocol::{CheckoutSource, Command, Event as CoreEvent, Route, TerminalKind, WorkspaceId};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Layout, Rect},
    Terminal,
};
use serde::Deserialize;
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
    WebSetPassword,
    WebServe { session: Option<String> },
    WebShutdown,
    WebStatus,
    /// Open the web UI in a native desktop window (requires the `desktop` feature).
    Desktop,
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

use conduit_core::ipc::{read_frame, write_frame};
use conduit_core::sessions::{
    load_registry, sanitize_session_name, save_registry, socket_alive, SessionEntry,
};

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
}

fn print_help() {
    println!(
        "\
conduit {}

USAGE:
    conduit [OPTIONS]

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

SUBCOMMANDS:
    desktop                        Open the web UI in a native desktop window
    tui                            Launch the terminal UI (non-session mode)
    web serve [--session <name>]   Serve the web UI, attaching to running sessions
    web status                     Show web server status and connected clients
    web shutdown                   Stop the running web server
    web set-password               Set the web UI password (required for remote access)

EXAMPLES:
    conduit                     Open the desktop app (the default)
    conduit tui                 Launch the terminal UI
    conduit -s work             Create or reattach to session 'work'
    conduit -s work -d          Start session 'work' in background
    conduit -a work             Attach to running session 'work'
    conduit -l                  List sessions
    conduit -r work             Remove session 'work'
    conduit web serve           Serve the web UI for all running sessions
    conduit web status          Show web server status and connected clients
    conduit web shutdown        Stop the running web server
    conduit web set-password    Set the web UI password (for remote access)

The web UI listens on https://127.0.0.1:3001 by default (override with
CONDUIT_WEB_PORT / CONDUIT_WEB_BIND; set CONDUIT_WEB_TLS=off for plain HTTP).",
        env!("CARGO_PKG_VERSION")
    );
}

fn parse_cli(args: Vec<String>) -> Result<Cli> {
    // `conduit desktop` — open the web UI in a native window.
    if args.first().map(String::as_str) == Some("desktop") {
        return Ok(Cli {
            mode: LaunchMode::Desktop,
            detach: false,
            version: false,
            help: false,
        });
    }

    // `conduit tui` — launch the terminal UI. Bare `conduit` opens the desktop
    // app by default, so this is how you ask for the TUI explicitly.
    if args.first().map(String::as_str) == Some("tui") {
        return Ok(Cli {
            mode: LaunchMode::Local,
            detach: false,
            version: false,
            help: false,
        });
    }

    // Subcommands: `conduit web set-password` / `conduit web serve [--session <name>]`
    if args.first().map(String::as_str) == Some("web") {
        match args.get(1).map(String::as_str) {
            Some("set-password") => {
                return Ok(Cli {
                    mode: LaunchMode::WebSetPassword,
                    detach: false,
                    version: false,
                    help: false,
                });
            }
            Some("serve") => {
                let mut session = None;
                let mut j = 2;
                while j < args.len() {
                    match args[j].as_str() {
                        "--session" | "-a" => {
                            session = args.get(j + 1).cloned();
                            j += 2;
                        }
                        _ => j += 1,
                    }
                }
                return Ok(Cli {
                    mode: LaunchMode::WebServe { session },
                    detach: false,
                    version: false,
                    help: false,
                });
            }
            Some("shutdown") => {
                return Ok(Cli {
                    mode: LaunchMode::WebShutdown,
                    detach: false,
                    version: false,
                    help: false,
                });
            }
            Some("status") => {
                return Ok(Cli {
                    mode: LaunchMode::WebStatus,
                    detach: false,
                    version: false,
                    help: false,
                });
            }
            _ => {
                return Err(anyhow!(
                    "unknown web subcommand; try: conduit web serve | conduit web status | conduit web shutdown | conduit web set-password"
                ));
            }
        }
    }

    let mut i = 0usize;
    // Bare `conduit` (no subcommand, no flags) defaults to the desktop window.
    // Headless builds (`--no-default-features`) have no desktop UI, so they fall
    // back to the terminal UI instead.
    #[cfg(feature = "desktop")]
    let mut mode = LaunchMode::Desktop;
    #[cfg(not(feature = "desktop"))]
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

fn main() -> Result<()> {
    let mut cli = parse_cli(std::env::args().skip(1).collect::<Vec<_>>())?;
    if cli.help {
        print_help();
        return Ok(());
    }
    if cli.version {
        println!("conduit {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    // GUI launchers (the macOS `.app`, the Linux `.desktop`) set CONDUIT_DESKTOP=1
    // and run the bare binary — with no explicit mode, open the desktop window.
    if matches!(cli.mode, LaunchMode::Local) && std::env::var_os("CONDUIT_DESKTOP").is_some() {
        cli.mode = LaunchMode::Desktop;
    }

    // The desktop app builds its own tokio runtime and owns the main thread for
    // the GUI event loop (required by macOS), so it must run *before* we enter a
    // runtime here — not from inside one.
    #[cfg(feature = "desktop")]
    if matches!(cli.mode, LaunchMode::Desktop) {
        return conduit_desktop::run();
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(run_cli(cli))
}

async fn run_cli(cli: Cli) -> Result<()> {
    match cli.mode {
        LaunchMode::Update => self_update(false),
        LaunchMode::Reinstall => self_update(true),
        LaunchMode::WebSetPassword => set_web_password(),
        LaunchMode::WebServe { session } => {
            let Some(cfg) = conduit_server::WebConfig::from_env(session) else {
                return Err(anyhow!(
                    "web server not started (a non-localhost bind needs a password and TLS)"
                ));
            };
            conduit_server::serve(cfg).await
        }
        LaunchMode::WebShutdown => {
            match conduit_server::control::shutdown().await {
                Ok(()) => println!("web server stopped"),
                Err(e) => println!("{e}"),
            }
            Ok(())
        }
        LaunchMode::WebStatus => {
            match conduit_server::control::status().await {
                Ok(report) => print_web_status(&report),
                Err(e) => println!("{e}"),
            }
            Ok(())
        }
        LaunchMode::RunDaemon { name } => conduit_core::daemon::run_session_daemon(&name).await,
        LaunchMode::RemoveSession { name } => delete_session(&name),
        LaunchMode::ListSessions => list_sessions(),
        LaunchMode::CreateSession { name } => {
            let entry = conduit_core::daemon::ensure_session_running(&name).await?;
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
                conduit_core::daemon::ensure_session_running(&name).await?
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
        LaunchMode::Desktop => Err(anyhow!(
            "this build of conduit has no desktop UI; rebuild with `--features desktop`"
        )),
    }
}

fn print_web_status(r: &conduit_server::control::StatusReport) {
    println!("web server: running (pid {})", r.pid);
    println!("  url:     {}", r.url);
    println!("  tls:     {}", if r.tls { "on" } else { "off" });
    println!("  auth:    {}", if r.auth_enabled { "on" } else { "off" });
    println!("  uptime:  {}s", r.uptime_secs);
    if r.clients.is_empty() {
        println!("  clients: none");
    } else {
        println!("  clients: {}", r.clients.len());
        for c in &r.clients {
            println!(
                "    - {}  (session {}, connected {}s ago)",
                c.addr, c.session, c.connected_secs
            );
        }
    }
}

fn set_web_password() -> Result<()> {
    let pw = rpassword::prompt_password("New web password: ")?;
    if pw.is_empty() {
        return Err(anyhow!("password cannot be empty"));
    }
    let confirm = rpassword::prompt_password("Confirm password: ")?;
    if pw != confirm {
        return Err(anyhow!("passwords don't match"));
    }
    let path = conduit_server::web_auth_path();
    conduit_server::auth::set_password(&path, &pw)?;
    println!(
        "Web password set ({}). Non-localhost access now requires it over TLS. \
         A running `conduit web serve` picks this up immediately — no restart needed.",
        path.display()
    );
    Ok(())
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

    let url = format!(
        "https://github.com/inhesrom/conduit/releases/download/{tag}/conduit-{target}.tar.gz"
    );

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
        return Err(anyhow!(
            "extracted archive does not contain 'conduit' binary"
        ));
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

// The session daemon (`run_session_daemon`) lives in `conduit_core::daemon` so
// embedders (e.g. the desktop app's in-process server) can reuse it.

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
    if let Some(path) = session_repositories_persist_path(name) {
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

fn session_repositories_persist_path(name: &str) -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let safe = sanitize_session_name(name);
    Some(
        PathBuf::from(home)
            .join(".config")
            .join("conduit")
            .join(format!("repositories.{safe}.json")),
    )
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

    // Read crossterm events on a dedicated OS thread so that synchronous
    // poll/read can never block the tokio runtime (and thus the render loop).
    let (ct_tx, mut ct_rx) = mpsc::channel::<Event>(256);
    std::thread::spawn(move || loop {
        match event::poll(Duration::from_millis(100)) {
            Ok(true) => match event::read() {
                Ok(evt) => {
                    if ct_tx.blocking_send(evt).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            },
            Ok(false) => {}
            Err(_) => break,
        }
    });

    'main: loop {
        // Drive the loop from async sources — the dedicated thread above
        // feeds crossterm events into ct_rx without blocking this runtime.
        let mut pending_ct: Option<Event> = None;
        tokio::select! {
            _ = frame_interval.tick() => {}
            evt = backend.evt_rx.recv() => {
                if let Some(evt) = evt {
                    apply_event(&mut app, evt);
                }
            }
            ct_evt = ct_rx.recv() => {
                pending_ct = ct_evt;
            }
        }

        for _ in 0..128 {
            match backend.evt_rx.try_recv() {
                Ok(evt) => apply_event(&mut app, evt),
                Err(_) => break,
            }
        }

        // Open + start terminals for a freshly created workspace (auto-agent),
        // and deliver any queued initial prompt once the agent has spun up.
        if let Some(id) = app.pending_open_created.take() {
            activate_workspace(&mut app, &backend, id).await;
        }
        if let Some((id, prompt)) = app.pending_agent_fallback.take() {
            let agent_choice = app.workspace_agent(id);
            let cmd = agent_vanilla_cmd_for(&app.settings, agent_choice.as_deref());
            app.queue_agent_startup(id, true);
            let _ = backend
                .cmd_tx
                .send(Command::StartTerminal {
                    id,
                    kind: TerminalKind::Agent,
                    tab_id: Some("agent".to_string()),
                    cmd,
                    cols: app.terminal_content_size.0,
                    rows: app.terminal_content_size.1,
                })
                .await;
            if let Some(prompt) = prompt {
                app.pending_agent_prompt = Some((id, prompt));
            }
        }
        if let Some((id, prompt)) = app.pending_prompt_send.take() {
            app.record_agent_prompt_sent(id, &prompt);
            let mut data = prompt.into_bytes();
            data.push(b'\r');
            let _ = backend
                .cmd_tx
                .send(Command::SendTerminalInput {
                    id,
                    kind: TerminalKind::Agent,
                    tab_id: None,
                    data_b64: base64::engine::general_purpose::STANDARD.encode(data),
                })
                .await;
        }
        if let Some((id, file)) = app.pending_review_diff.take() {
            let _ = backend
                .cmd_tx
                .send(Command::LoadBranchFileDiff { id, file })
                .await;
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
            let _ = backend.cmd_tx.try_send(Command::StartTerminal {
                id,
                kind: protocol::TerminalKind::Shell,
                tab_id,
                cmd: Vec::new(),
                cols: app.terminal_content_size.0,
                rows: app.terminal_content_size.1,
            });
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
                // Size against the detail area (sidebar carved off), matching
                // what `terminal.draw` actually renders the workspace into. Using
                // the full terminal width here would tell the PTY it's wider than
                // the visible pane and the child's output runs off the right edge.
                let full = ratatui::layout::Rect::new(0, 0, size.width, size.height);
                let area = detail_area(full, app.sidebar_mode);
                let inner = ui::screens::workspace::terminal_content_rect(
                    area,
                    app.focus,
                    app.terminal_fullscreen(),
                );
                let cols = inner.width.max(1);
                let rows = inner.height.max(1);
                // Resize every tab of the open workspace, not just the active
                // one: all tabs render into the same pane geometry, so a
                // background tab left at its (possibly wider) birth size would
                // spill off the right edge the instant it is shown. Snapshot the
                // tab list first so the immutable borrow of `app` is released
                // before the mutable resize-parser/latch calls below.
                let tabs: Vec<(String, TerminalKind)> =
                    app.ws_tabs.iter().map(|t| (t.id.clone(), t.kind)).collect();
                for (tid, kind) in tabs {
                    if app.has_terminal_tab(id, &tid) && app.needs_resize(id, &tid, cols, rows) {
                        // Keep what we render (the emulator) and what the child
                        // sees (the PTY) in lockstep. Only latch the new size and
                        // rebuild the emulator once the resize command is actually
                        // enqueued. If the channel is momentarily full we leave
                        // the size unlatched so the next frame retries — otherwise
                        // the PTY can stay wider than the pane we draw, and the
                        // child (e.g. the Claude TUI) wraps to the stale width,
                        // spilling text off the right edge until it is restarted.
                        let sent = backend.cmd_tx.try_send(Command::ResizeTerminal {
                            id,
                            kind,
                            tab_id: Some(tid.clone()),
                            cols,
                            rows,
                        });
                        if sent.is_ok() {
                            app.resize_terminal_parser(id, &tid, cols, rows);
                            app.mark_resize_sent(id, &tid, cols, rows);
                        }
                    }
                }
            }
        }

        // Update cached grid height for scroll calculations.
        if let Ok(size) = terminal.size() {
            let full = Rect::new(0, 0, size.width, size.height);
            app.last_grid_height = ui::screens::home::grid_rect(full).height;
            // Track the terminal content size every frame (even on Home) so a
            // newly-opened workspace can spawn its PTYs at the right size. Carve
            // the sidebar off and use the focused-terminal layout (not the current
            // focus) since that's the size the pane will have once the terminal is
            // shown — so the child is born at its final width with no reflow.
            let area = detail_area(full, app.sidebar_mode);
            let content = ui::screens::workspace::terminal_content_rect(
                area,
                app::Focus::WsTerminal,
                app.terminal_fullscreen(),
            );
            app.terminal_content_size = (content.width.max(1), content.height.max(1));
        }

        app.debug_fps_frame_count += 1;
        let fps_reset = app.debug_fps_last_reset.get_or_insert_with(Instant::now);
        if fps_reset.elapsed() >= Duration::from_secs(1) {
            app.debug_fps = app.debug_fps_frame_count as u16;
            app.debug_fps_frame_count = 0;
            *fps_reset = Instant::now();
        }
        let mut pending_clipboard_text: Option<String> = None;
        terminal.draw(|frame| {
            let full = frame.area();
            // Persistent sidebar on the left + detail pane on the right. The
            // sidebar has three display modes (Expanded tree / vertical Rail /
            // Hidden); Rail mode can also float a workspace pop-out.
            let sidebar_w = ui::widgets::sidebar::width(app.sidebar_mode);
            let (sidebar_rect, detail_area) = if sidebar_w > 0 {
                let chunks =
                    Layout::horizontal([Constraint::Length(sidebar_w), Constraint::Min(0)])
                        .split(full);
                (Some(chunks[0]), chunks[1])
            } else {
                (None, full)
            };
            if let Some(sr) = sidebar_rect {
                if app.sidebar_mode == app::SidebarMode::Rail {
                    ui::widgets::sidebar::render_rail(frame, sr, &app);
                } else {
                    ui::widgets::sidebar::render(frame, sr, &app);
                }
            }
            match app.route {
                Route::Home => ui::screens::home::render(frame, detail_area, &app),
                Route::Repo { .. } => {
                    ui::screens::repo_summary::render(frame, detail_area, &app)
                }
                Route::Workspace { .. } => ui::screens::workspace::render(frame, detail_area, &app),
            }
            // The Rail's workspace pop-out floats over the detail pane, so it is
            // drawn after the route content.
            if app.sidebar_mode == app::SidebarMode::Rail {
                if let Some(sr) = sidebar_rect {
                    ui::widgets::sidebar::render_popout(frame, sr, full, &app);
                }
            }
            // The quick-create and delete-confirm modals float over everything
            // (they can be raised from the sidebar regardless of route), so they
            // are drawn last.
            ui::screens::home::render_quick_create_modal(frame, detail_area, &app);
            if app.is_confirming_delete() {
                ui::screens::home::render_delete_modal(frame, detail_area, &app);
            }
            // Extract selected text from the rendered buffer before applying highlights.
            if let Some(sel) = &app.pending_copy_selection {
                let borders = match app.route {
                    Route::Home => ui::screens::home::border_rects(detail_area),
                    Route::Repo { .. } => ui::screens::repo_summary::border_rects(detail_area),
                    Route::Workspace { .. } => {
                        ui::screens::workspace::border_rects(detail_area, &app)
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

        for evt in pending_ct
            .into_iter()
            .chain(std::iter::from_fn(|| ct_rx.try_recv().ok()))
        {
            match evt {
                Event::Key(key) => {
                    if matches!(key.kind, KeyEventKind::Release) {
                        continue;
                    }

                    if keymap::is_quit(key)
                        && !app.is_adding_workspace()
                        && !app.is_adding_ssh_workspace()
                        && app.ssh_history_picker.is_none()
                        && app.agent_picker.is_none()
                        && !app.is_confirming_delete()
                        && !app.is_renaming_workspace()
                        && !app.is_renaming_tab()
                        && !app.is_committing()
                        && !app.is_creating_branch()
                        && !app.is_workspace_command_open()
                        && !app.is_confirming_discard()
                        && !app.is_confirming_stash_pull_pop()
                        && !app.is_confirming_delete_branch()
                        && !app.is_stashing()
                        && !app.is_settings_open()
                        && !app.is_quick_creating()
                        && !matches!(app.focus, app::Focus::WsTerminal)
                    {
                        break 'main;
                    }

                    if handle_global_workspace_hotkey(&mut app, &backend, key).await {
                        continue;
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
                                            if let Some((_, _, buf)) = &mut app.new_agent_wizard {
                                                buf.pop();
                                            }
                                        }
                                        KeyCode::Char(c) => {
                                            if let Some((_, _, buf)) = &mut app.new_agent_wizard {
                                                buf.push(c);
                                            }
                                        }
                                        _ => {}
                                    }
                                } else if app.is_editing_keybind() {
                                    // Keybinding rows capture the next key press
                                    // directly (Esc cancels, any other press is
                                    // recorded as the new binding).
                                    if key.code == KeyCode::Esc {
                                        app.cancel_setting_edit();
                                    } else if let Some(binding) = keymap::keybind_from_event(key) {
                                        app.apply_captured_keybind(binding);
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
                                        KeyCode::Esc | KeyCode::Char('S') => app.close_settings(),
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
                                        confirm_pending_delete(&mut app, &backend).await;
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
                                                .send(Command::RegisterRepository {
                                                    name,
                                                    path,
                                                    ssh: Some(target),
                                                    default_agent: Some(
                                                        app.settings.default_agent.clone(),
                                                    ),
                                                    worktree_root: None,
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
                                                    .send(Command::RegisterRepository {
                                                        name,
                                                        path,
                                                        ssh: None,
                                                        default_agent: Some(
                                                            app.settings.default_agent.clone(),
                                                        ),
                                                        worktree_root: None,
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
                                                        .send(Command::RegisterRepository {
                                                            name,
                                                            path,
                                                            ssh: None,
                                                            default_agent: Some(
                                                                app.settings.default_agent.clone(),
                                                            ),
                                                            worktree_root: None,
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
                                    KeyCode::Esc | KeyCode::Enter | KeyCode::Char('M') => {
                                        app.end_move_workspace();
                                    }
                                    _ => {}
                                }
                            } else if app.is_quick_creating() {
                                handle_quick_create_key(&mut app, &backend, key).await;
                            } else {
                                handle_sidebar_key(&mut app, &backend, key, true).await;
                            }
                        }
                        Route::Repo { .. } => {
                            // A delete confirmation or quick-create can be raised
                            // from the summary, so service those modals first.
                            if app.is_confirming_delete() {
                                match key.code {
                                    KeyCode::Char('y') | KeyCode::Char('Y') => {
                                        confirm_pending_delete(&mut app, &backend).await;
                                    }
                                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                                        app.cancel_delete_workspace();
                                    }
                                    _ => {}
                                }
                                continue;
                            }
                            if handle_quick_create_key(&mut app, &backend, key).await {
                                continue;
                            }
                            handle_repo_summary_key(&mut app, &backend, key).await;
                        }
                        Route::Workspace { id } => {
                            if handle_workspace_command_key(&mut app, &backend, id, key).await {
                                continue;
                            }

                            // A delete confirmation can be raised from the sidebar while a
                            // workspace is open, so it must be answerable from this route too.
                            if app.is_confirming_delete() {
                                match key.code {
                                    KeyCode::Char('y') | KeyCode::Char('Y') => {
                                        confirm_pending_delete(&mut app, &backend).await;
                                    }
                                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                                        app.cancel_delete_workspace();
                                    }
                                    _ => {}
                                }
                                continue;
                            }

                            // A new workspace can be created from the sidebar while another
                            // workspace is open, so service the quick-create modal here too.
                            if handle_quick_create_key(&mut app, &backend, key).await {
                                continue;
                            }

                            // When the sidebar is focused (e.g. clicked), it owns navigation
                            // and actions like delete/new. Fall through only for keys it
                            // doesn't consume so workspace shortcuts (Esc, F, …) still work.
                            if app.focus == app::Focus::Sidebar
                                && handle_sidebar_key(&mut app, &backend, key, false).await
                            {
                                continue;
                            }

                            if matches!(app.focus, app::Focus::ReviewFiles | app::Focus::ReviewDiff)
                            {
                                let file_count =
                                    app.review_files.get(&id).map(|f| f.len()).unwrap_or(0);
                                let load_selected =
                                    |app: &mut app::TuiApp| -> Option<(protocol::WorkspaceId, String)> {
                                        app.review_files
                                            .get(&id)
                                            .and_then(|f| f.get(app.review_file_selected))
                                            .cloned()
                                            .map(|file| (id, file))
                                    };
                                match key.code {
                                    KeyCode::Esc => app.exit_review_mode(),
                                    KeyCode::Tab => {
                                        app.focus = if app.focus == app::Focus::ReviewFiles {
                                            app::Focus::ReviewDiff
                                        } else {
                                            app::Focus::ReviewFiles
                                        };
                                    }
                                    KeyCode::Char('P') => {
                                        let _ = backend.cmd_tx.send(Command::GitPush { id }).await;
                                    }
                                    KeyCode::Char('O') => {
                                        let _ = backend
                                            .cmd_tx
                                            .send(Command::OpenPullRequest { id })
                                            .await;
                                    }
                                    KeyCode::Char('J') => {
                                        app.review_diff_scroll =
                                            app.review_diff_scroll.saturating_add(10);
                                    }
                                    KeyCode::Char('K') => {
                                        app.review_diff_scroll =
                                            app.review_diff_scroll.saturating_sub(10);
                                    }
                                    KeyCode::Down | KeyCode::Char('j') => {
                                        if app.focus == app::Focus::ReviewDiff {
                                            app.review_diff_scroll =
                                                app.review_diff_scroll.saturating_add(1);
                                        } else if file_count > 0 {
                                            app.review_file_selected =
                                                (app.review_file_selected + 1).min(file_count - 1);
                                            app.review_diff_scroll = 0;
                                            if let Some((id, file)) = load_selected(&mut app) {
                                                let _ = backend
                                                    .cmd_tx
                                                    .send(Command::LoadBranchFileDiff { id, file })
                                                    .await;
                                            }
                                        }
                                    }
                                    KeyCode::Up | KeyCode::Char('k') => {
                                        if app.focus == app::Focus::ReviewDiff {
                                            app.review_diff_scroll =
                                                app.review_diff_scroll.saturating_sub(1);
                                        } else if app.review_file_selected > 0 {
                                            app.review_file_selected -= 1;
                                            app.review_diff_scroll = 0;
                                            if let Some((id, file)) = load_selected(&mut app) {
                                                let _ = backend
                                                    .cmd_tx
                                                    .send(Command::LoadBranchFileDiff { id, file })
                                                    .await;
                                            }
                                        }
                                    }
                                    KeyCode::Enter => {
                                        if let Some((id, file)) = load_selected(&mut app) {
                                            app.review_diff_scroll = 0;
                                            let _ = backend
                                                .cmd_tx
                                                .send(Command::LoadBranchFileDiff { id, file })
                                                .await;
                                        }
                                    }
                                    _ => {}
                                }
                                continue;
                            }
                            if app.agent_picker.is_some() {
                                let in_custom = app
                                    .agent_picker
                                    .as_ref()
                                    .is_some_and(|p| p.custom_input.is_some());
                                if in_custom {
                                    match key.code {
                                        KeyCode::Esc => {
                                            if let Some(picker) = app.agent_picker.as_mut() {
                                                picker.custom_input = None;
                                            }
                                        }
                                        KeyCode::Enter => {
                                            let entry = app.agent_picker.as_ref().and_then(|p| {
                                                p.custom_input
                                                    .as_deref()
                                                    .map(str::trim)
                                                    .filter(|c| !c.is_empty())
                                                    .map(|c| (p.id, c.to_string()))
                                            });
                                            if let Some((target, command)) = entry {
                                                apply_agent_switch(
                                                    &mut app,
                                                    &backend,
                                                    target,
                                                    Some(command),
                                                )
                                                .await;
                                            }
                                        }
                                        KeyCode::Backspace => {
                                            if let Some(input) = app
                                                .agent_picker
                                                .as_mut()
                                                .and_then(|p| p.custom_input.as_mut())
                                            {
                                                input.pop();
                                            }
                                        }
                                        KeyCode::Char(c) => {
                                            if let Some(input) = app
                                                .agent_picker
                                                .as_mut()
                                                .and_then(|p| p.custom_input.as_mut())
                                            {
                                                input.push(c);
                                            }
                                        }
                                        _ => {}
                                    }
                                } else {
                                    match key.code {
                                        KeyCode::Char('j') | KeyCode::Down => {
                                            app.agent_picker_move(1)
                                        }
                                        KeyCode::Char('k') | KeyCode::Up => {
                                            app.agent_picker_move(-1)
                                        }
                                        KeyCode::Esc => app.cancel_agent_picker(),
                                        KeyCode::Enter => {
                                            let selection =
                                                app.agent_picker.as_ref().map(|p| (p.id, p.selected));
                                            if let Some((target, selected)) = selection {
                                                if let Some(profile) =
                                                    app.settings.agents.get(selected)
                                                {
                                                    let name = profile.name.clone();
                                                    apply_agent_switch(
                                                        &mut app,
                                                        &backend,
                                                        target,
                                                        Some(name),
                                                    )
                                                    .await;
                                                } else {
                                                    app.begin_agent_picker_custom();
                                                }
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                                continue;
                            }

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

                            if app.is_confirming_discard_all() {
                                match key.code {
                                    KeyCode::Char('y') | KeyCode::Enter => {
                                        if let Some(ws_id) = app.take_discard_all() {
                                            app.begin_git_op(ws_id);
                                            let _ = backend
                                                .cmd_tx
                                                .send(Command::GitDiscardAll { id: ws_id })
                                                .await;
                                        }
                                    }
                                    KeyCode::Char('n') | KeyCode::Esc => {
                                        app.cancel_discard_all();
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

                            // Configurable hotkey for terminal command mode.
                            if keymap::matches_keybinding(key, &app.settings.passthrough_key)
                                && matches!(app.focus, app::Focus::WsTerminal)
                            {
                                app.toggle_terminal_command_mode();
                                continue;
                            }

                            // Resurrect-command overlay intercepts Enter/Esc when shown.
                            // Any other keystroke also auto-dismisses the overlay (the user
                            // is clearly already interacting with the shell), then falls
                            // through to deliver the keystroke to the PTY normally.
                            if matches!(app.focus, app::Focus::WsTerminal)
                                && !app.terminal_command_mode()
                                && app.pending_resurrect_command().is_some()
                            {
                                match key.code {
                                    KeyCode::Enter => {
                                        let tab_id = app.active_tab_id();
                                        if let Some(cmd) = app.take_resurrect_command() {
                                            let payload = resurrect::resurrect_input(&cmd);
                                            let _ = backend
                                                .cmd_tx
                                                .send(Command::ClearShellResurrection {
                                                    id,
                                                    tab_id: tab_id.clone(),
                                                })
                                                .await;
                                            let _ = backend
                                                .cmd_tx
                                                .send(Command::SendTerminalInput {
                                                    id,
                                                    kind: app.active_tab_kind(),
                                                    tab_id: Some(tab_id),
                                                    data_b64:
                                                        base64::engine::general_purpose::STANDARD
                                                            .encode(payload),
                                                })
                                                .await;
                                        }
                                        continue;
                                    }
                                    KeyCode::Esc => {
                                        let tab_id = app.active_tab_id();
                                        app.dismiss_resurrect_overlay();
                                        let _ = backend
                                            .cmd_tx
                                            .send(Command::ClearShellResurrection { id, tab_id })
                                            .await;
                                        continue;
                                    }
                                    _ => {
                                        let tab_id = app.active_tab_id();
                                        app.dismiss_resurrect_overlay();
                                        let _ = backend
                                            .cmd_tx
                                            .send(Command::ClearShellResurrection { id, tab_id })
                                            .await;
                                        // fall through — let the keystroke reach the PTY
                                    }
                                }
                            }

                            // In normal terminal mode, the focused terminal owns keys.
                            if matches!(app.focus, app::Focus::WsTerminal)
                                && !app.terminal_command_mode()
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

                            // Configurable "scroll terminal to bottom" hotkey
                            // (resets scrollback). Works from any workspace pane.
                            if keymap::matches_keybinding(key, &app.settings.scroll_to_bottom_key) {
                                let tab_id = app.active_tab_id();
                                app.reset_terminal_scrollback(id, &tab_id);
                                continue;
                            }

                            // Configurable fullscreen hotkey. Works from any
                            // workspace pane once terminal passthrough has
                            // yielded command mode or focus is elsewhere.
                            if keymap::matches_keybinding(
                                key,
                                &app.settings.terminal_fullscreen_key,
                            ) {
                                app.toggle_terminal_fullscreen();
                                continue;
                            }

                            // Shift+Y toggles YOLO mode from any workspace pane.
                            if key.code == KeyCode::Char('Y') {
                                app.toggle_yolo_mode();
                                if app.workspace_agent_running(id) {
                                    app.suppress_next_agent_exit(id);
                                }
                                let _ = backend
                                    .cmd_tx
                                    .send(Command::StopTerminal {
                                        id,
                                        kind: TerminalKind::Agent,
                                        tab_id: Some("agent".to_string()),
                                    })
                                    .await;
                                let agent_choice = app.workspace_agent(id);
                                let cmd =
                                    agent_cmd_continue_for(&app.settings, agent_choice.as_deref());
                                app.queue_agent_startup(id, false);
                                let _ = backend
                                    .cmd_tx
                                    .send(Command::StartTerminal {
                                        id,
                                        kind: TerminalKind::Agent,
                                        tab_id: Some("agent".to_string()),
                                        cmd,
                                        cols: app.terminal_content_size.0,
                                        rows: app.terminal_content_size.1,
                                    })
                                    .await;
                                continue;
                            }

                            if key.code == KeyCode::Esc {
                                if matches!(app.focus, app::Focus::WsTerminal) {
                                    app.focus = app::Focus::WsTerminalTabs;
                                } else {
                                    app.go_home();
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
                                            app::LogItem::ChangedDirectory(_) => {
                                                app.toggle_selected_uncommitted_directory();
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
                                KeyCode::Char(':') if workspace_command_shortcut_enabled(&app) => {
                                    app.begin_workspace_command();
                                }
                                KeyCode::Char(';')
                                    if key.modifiers.contains(KeyModifiers::SHIFT)
                                        && workspace_command_shortcut_enabled(&app) =>
                                {
                                    app.begin_workspace_command();
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
                                                | app::LogItem::ChangedDirectory(_)
                                        ) =>
                                {
                                    // Toggle stage/unstage selected uncommitted path.
                                    if let Some((file, index_status, _)) =
                                        app.selected_uncommitted_status()
                                    {
                                        let is_staged = index_status != ' ' && index_status != '?';
                                        let cmd = if is_staged {
                                            Command::GitUnstageFile { id, file }
                                        } else {
                                            Command::GitStageFile { id, file }
                                        };
                                        let _ = backend.cmd_tx.send(cmd).await;
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
                                                | app::LogItem::ChangedDirectory(_)
                                        ) =>
                                {
                                    app.begin_discard();
                                }
                                KeyCode::Char('D')
                                    if matches!(app.focus, app::Focus::WsLog)
                                        && app.log_item_is_file_context() =>
                                {
                                    let has_changes = app
                                        .workspace_git
                                        .get(&id)
                                        .map(|g| !g.changed.is_empty())
                                        .unwrap_or(false);
                                    if has_changes {
                                        app.begin_discard_all(id);
                                    }
                                }
                                KeyCode::Char('s')
                                    if matches!(app.focus, app::Focus::WsLog)
                                        && app.log_item_is_file_context() =>
                                {
                                    app.stash_input = Some(String::new());
                                }
                                KeyCode::Char('S')
                                    if matches!(app.focus, app::Focus::WsLog)
                                        && app.log_item_is_file_context() =>
                                {
                                    let has_changes = app
                                        .workspace_git
                                        .get(&id)
                                        .map(|g| !g.changed.is_empty())
                                        .unwrap_or(false);
                                    if has_changes {
                                        app.begin_git_op(id);
                                        let _ =
                                            backend.cmd_tx.send(Command::GitStashAll { id }).await;
                                    }
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
                                            cols: app.terminal_content_size.0,
                                            rows: app.terminal_content_size.1,
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
                                KeyCode::Char('c')
                                    if matches!(app.focus, app::Focus::WsTerminalTabs) =>
                                {
                                    app.begin_agent_picker(id);
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
                                            cols: app.terminal_content_size.0,
                                            rows: app.terminal_content_size.1,
                                        })
                                        .await;
                                    app.focus = app::Focus::WsTerminal;
                                }
                                KeyCode::Char('A')
                                    if matches!(app.focus, app::Focus::WsTerminalTabs) =>
                                {
                                    let kind = app.active_tab_kind();
                                    if kind == TerminalKind::Agent
                                        && app.workspace_agent_running(id)
                                    {
                                        app.suppress_next_agent_exit(id);
                                    }
                                    let _ = backend
                                        .cmd_tx
                                        .send(Command::StopTerminal {
                                            id,
                                            kind,
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
                                            cols: app.terminal_content_size.0,
                                            rows: app.terminal_content_size.1,
                                        })
                                        .await;
                                }
                                KeyCode::Char('S')
                                    if matches!(app.focus, app::Focus::WsTerminalTabs) =>
                                {
                                    let kind = app.active_tab_kind();
                                    if kind == TerminalKind::Agent
                                        && app.workspace_agent_running(id)
                                    {
                                        app.suppress_next_agent_exit(id);
                                    }
                                    let _ = backend
                                        .cmd_tx
                                        .send(Command::StopTerminal {
                                            id,
                                            kind,
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
                    if let Some(state) = app.workspace_command_mut() {
                        if !state.running {
                            paste_into_workspace_command(state, &text);
                        }
                    } else if matches!(app.focus, app::Focus::WsTerminal)
                        && !app.terminal_command_mode()
                    {
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

/// Sends the appropriate removal command for whatever delete is currently
/// pending (a workspace or a repository) and clears the confirmation state.
async fn confirm_pending_delete(app: &mut TuiApp, backend: &Backend) {
    if let Some(id) = app.take_delete_workspace() {
        let _ = backend.cmd_tx.send(Command::RemoveWorkspace { id }).await;
    } else if let Some(repo_id) = app.take_delete_repo() {
        let _ = backend
            .cmd_tx
            .send(Command::RemoveRepository { repo_id })
            .await;
    }
}

/// Handles a key while the quick-create ("New Workspace") modal is open.
/// Returns `true` if the key was consumed; `false` if the modal isn't open.
/// Shared between routes so a workspace can be created from the sidebar while
/// another workspace is already open.
async fn handle_quick_create_key(app: &mut TuiApp, backend: &Backend, key: KeyEvent) -> bool {
    if !app.is_quick_creating() {
        return false;
    }
    match key.code {
        KeyCode::Esc => app.cancel_quick_create(),
        KeyCode::Tab | KeyCode::BackTab => {
            if let Some(qc) = app.quick_create.as_mut() {
                if !qc.expanded {
                    qc.expanded = true;
                    qc.field = app::QuickCreateField::Mode;
                } else {
                    // Leaving the branch picker commits the highlighted
                    // suggestion into the filter text so it's what gets created.
                    if qc.field == app::QuickCreateField::Branch {
                        if let Some(b) = qc.selected_branch() {
                            qc.branch_filter = b.display.clone();
                        }
                    }
                    if key.code == KeyCode::BackTab {
                        qc.prev_field();
                    } else {
                        qc.next_field();
                    }
                }
            }
        }
        KeyCode::Left | KeyCode::Right => {
            // ←/→ toggle the mode field, and cycle configured agents while the
            // agent field is focused in selection mode. Once expanded into
            // command-edit mode (or on other fields) the arrows do nothing — so
            // they can't clobber an edited command.
            if let Some(qc) = app.quick_create.as_mut() {
                if qc.field == app::QuickCreateField::Mode {
                    qc.cycle_mode();
                } else if qc.field == app::QuickCreateField::Agent && !qc.agent_command_edit {
                    let delta = if key.code == KeyCode::Left { -1 } else { 1 };
                    qc.cycle_agent(&app.settings.agents, delta);
                }
            }
        }
        KeyCode::Up | KeyCode::Down => {
            // ↑/↓ move the branch-picker highlight.
            if let Some(qc) = app.quick_create.as_mut() {
                if qc.field == app::QuickCreateField::Branch {
                    let len = qc.filtered_branches().len();
                    if len > 0 {
                        let cur = qc.branch_selected.min(len - 1);
                        qc.branch_selected = if key.code == KeyCode::Down {
                            (cur + 1).min(len - 1)
                        } else {
                            cur.saturating_sub(1)
                        };
                    }
                }
            }
        }
        KeyCode::Enter => {
            // First Enter on the agent selector expands the chosen agent into
            // its full launch command for editing, instead of creating. Read the
            // name under an immutable borrow, then mutate (so `&app.settings` and
            // the `&mut qc` borrow don't overlap).
            let expand_agent = app
                .quick_create
                .as_ref()
                .map(|qc| qc.field == app::QuickCreateField::Agent && !qc.agent_command_edit)
                .unwrap_or(false);
            if expand_agent {
                let name = app
                    .quick_create
                    .as_ref()
                    .map(|qc| qc.agent.trim().to_string())
                    .unwrap_or_default();
                let resolved = agent_cmd_for(&app.settings, Some(&name)).join(" ");
                if let Some(qc) = app.quick_create.as_mut() {
                    qc.agent = resolved;
                    qc.agent_command_edit = true;
                }
                return true;
            }
            // In existing-branch mode a branch must be resolvable; keep the
            // modal open with a status message otherwise.
            let unresolved_branch = app
                .quick_create
                .as_ref()
                .map(|qc| {
                    qc.mode == app::QuickCreateMode::ExistingBranch
                        && qc.selected_branch().is_none()
                })
                .unwrap_or(false);
            if unresolved_branch {
                app.git_action_message = Some((
                    "No matching branch to check out".to_string(),
                    Instant::now(),
                ));
                return true;
            }
            if let Some(qc) = app.quick_create.take() {
                let existing = if qc.mode == app::QuickCreateMode::ExistingBranch {
                    qc.selected_branch().map(|b| {
                        if b.is_remote {
                            CheckoutSource::RemoteBranch {
                                remote_ref: b.display.clone(),
                            }
                        } else {
                            CheckoutSource::LocalBranch {
                                name: b.display.clone(),
                            }
                        }
                    })
                } else {
                    None
                };
                let base_branch = if existing.is_some() || qc.base_branch.trim().is_empty() {
                    None
                } else {
                    Some(qc.base_branch.trim().to_string())
                };
                let agent = if qc.agent.trim().is_empty() {
                    None
                } else {
                    Some(qc.agent.trim().to_string())
                };
                let prompt = qc.initial_prompt.trim().to_string();
                if !prompt.is_empty() {
                    app.pending_create_prompt = Some(prompt);
                }
                let _ = backend
                    .cmd_tx
                    .send(Command::CreateWorkspace {
                        repo_id: qc.repo_id,
                        name: qc.name.clone(),
                        base_branch,
                        agent,
                        existing,
                    })
                    .await;
            }
        }
        KeyCode::Backspace => {
            if let Some(qc) = app.quick_create.as_mut() {
                // Selector fields (mode, agent profile cycling) have no text
                // buffer; `active_input_mut` returns None for them.
                if let Some(input) = qc.active_input_mut() {
                    input.pop();
                    if qc.field == app::QuickCreateField::Branch {
                        qc.branch_selected = 0;
                    }
                }
            }
        }
        KeyCode::Char(c) => {
            if let Some(qc) = app.quick_create.as_mut() {
                if let Some(input) = qc.active_input_mut() {
                    input.push(c);
                    if qc.field == app::QuickCreateField::Branch {
                        qc.branch_selected = 0;
                    }
                }
            }
        }
        _ => {}
    }
    true
}

fn workspace_hotkeys_allowed(app: &TuiApp) -> bool {
    !app.is_settings_open()
        && !app.is_adding_workspace()
        && !app.is_adding_ssh_workspace()
        && app.ssh_history_picker.is_none()
        && app.agent_picker.is_none()
        && !app.is_confirming_delete()
        && !app.is_renaming_workspace()
        && !app.is_renaming_tab()
        && !app.is_committing()
        && !app.is_creating_branch()
        && !app.is_workspace_command_open()
        && !app.is_confirming_discard()
        && !app.is_confirming_discard_all()
        && !app.is_confirming_stash_pull_pop()
        && !app.is_confirming_delete_branch()
        && !app.is_stashing()
        && !app.is_quick_creating()
}

async fn handle_global_workspace_hotkey(
    app: &mut TuiApp,
    backend: &Backend,
    key: KeyEvent,
) -> bool {
    if !workspace_hotkeys_allowed(app) {
        return false;
    }

    let switch_delta = if keymap::matches_keybinding(key, &app.settings.prev_workspace_key) {
        Some(-1)
    } else if keymap::matches_keybinding(key, &app.settings.next_workspace_key) {
        Some(1)
    } else {
        None
    };

    if let Some(delta) = switch_delta {
        if let Some(id) = app.adjacent_workspace_target_id(delta) {
            if Some(id) == app.active_workspace_id() {
                app.focus = app::Focus::WsTerminal;
            } else {
                activate_workspace(app, backend, id).await;
            }
        }
        return true;
    }

    false
}

async fn activate_workspace(app: &mut TuiApp, backend: &Backend, id: WorkspaceId) {
    app.open_workspace(id);
    let size = app.terminal_content_size;
    start_workspace_tab_terminals(&backend.cmd_tx, app, id, size).await;
    let _ = backend.cmd_tx.send(Command::RefreshGit { id }).await;
    let _ = backend.cmd_tx.send(Command::ClearAttention { id }).await;
}

/// Handles a key while the left sidebar is focused (`Focus::Sidebar`). Returns
/// `true` if the key was consumed by the sidebar; `false` lets the caller fall
/// through to its own handling. Shared between the Home and Workspace routes so
/// sidebar actions (navigate, open, delete, review) work consistently from
/// either — including deleting the selected workspace or repository.
///
/// `allow_modals` gates the keys that open Home-only modals (add repository
/// `N`/`a`, add SSH workspace `A`, settings `S`). The Workspace route passes
/// `false` because it cannot service those modals, which would otherwise trap
/// input. New-workspace (`n`) is always allowed — its modal is serviced by both
/// routes via [`handle_quick_create_key`].
async fn handle_sidebar_key(
    app: &mut TuiApp,
    backend: &Backend,
    key: KeyEvent,
    allow_modals: bool,
) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    if app.sidebar_mode == app::SidebarMode::Rail && app.sidebar_popout.is_some() {
        // Pop-out open: navigate/open the repo's workspaces.
        match key.code {
            KeyCode::Char('b') if ctrl => app.cycle_sidebar_mode(),
            KeyCode::Esc | KeyCode::Char('h') | KeyCode::Left => app.close_sidebar_popout(),
            KeyCode::Down | KeyCode::Char('j') => app.move_popout_selection(1),
            KeyCode::Up | KeyCode::Char('k') => app.move_popout_selection(-1),
            KeyCode::Enter | KeyCode::Char('l') => {
                if let Some(id) = app.selected_popout_workspace_id() {
                    app.close_sidebar_popout();
                    activate_workspace(app, backend, id).await;
                }
            }
            KeyCode::Char('D') => {
                if let Some(id) = app.selected_popout_workspace_id() {
                    app.pending_delete_workspace = Some(id);
                }
            }
            _ => return false,
        }
    } else if app.sidebar_mode == app::SidebarMode::Rail {
        // Rail, no pop-out: navigate repos; Enter opens the pop-out.
        match key.code {
            KeyCode::Char('b') if ctrl => app.cycle_sidebar_mode(),
            KeyCode::Down | KeyCode::Char('j') => app.move_rail_selection(1),
            KeyCode::Up | KeyCode::Char('k') => app.move_rail_selection(-1),
            KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => app.toggle_sidebar_popout(),
            KeyCode::Char('n') => match app.selected_rail_repo() {
                Some(repo_id) => {
                    app.begin_quick_create(repo_id);
                    // Populate the existing-branch picker in the background.
                    let _ = backend
                        .cmd_tx
                        .send(Command::ListRepoBranches { repo_id })
                        .await;
                }
                None => {
                    app.git_action_message = Some((
                        "No repositories — press N to add one first".to_string(),
                        Instant::now(),
                    ));
                }
            },
            KeyCode::Char('N') if allow_modals => {
                let cwd = std::env::current_dir()
                    .unwrap_or_else(|_| PathBuf::from("."))
                    .display()
                    .to_string();
                app.begin_add_workspace(cwd);
            }
            // `D` — remove the selected repository.
            KeyCode::Char('D') => {
                if let Some(repo_id) = app.selected_rail_repo() {
                    app.pending_delete_repo = Some(repo_id);
                }
            }
            KeyCode::Char('f') => {
                app.sidebar_review_filter = !app.sidebar_review_filter;
            }
            KeyCode::Char('S') if allow_modals => app.open_settings(),
            _ => return false,
        }
    } else {
        // Sidebar (Repository -> Workspace tree) is focused.
        match key.code {
            KeyCode::Char('b') if ctrl => app.cycle_sidebar_mode(),
            // `n` — new workspace under the selected repo.
            KeyCode::Char('n') => match app.sidebar_context_repo() {
                Some(repo_id) => {
                    app.begin_quick_create(repo_id);
                    // Populate the existing-branch picker in the background.
                    let _ = backend
                        .cmd_tx
                        .send(Command::ListRepoBranches { repo_id })
                        .await;
                }
                None => {
                    app.git_action_message = Some((
                        "No repositories — press N to add one first".to_string(),
                        Instant::now(),
                    ));
                }
            },
            // `N` — add (register) a new repository.
            KeyCode::Char('N') if allow_modals => {
                let cwd = std::env::current_dir()
                    .unwrap_or_else(|_| PathBuf::from("."))
                    .display()
                    .to_string();
                app.begin_add_workspace(cwd);
            }
            KeyCode::Down | KeyCode::Char('j') => app.move_sidebar_selection(1),
            KeyCode::Up | KeyCode::Char('k') => app.move_sidebar_selection(-1),
            KeyCode::Left | KeyCode::Char('h') => {
                if let Some(app::SidebarRow::Repo(id)) = app.selected_sidebar_row() {
                    app.collapsed_repos.insert(id);
                }
            }
            KeyCode::Right => {
                if let Some(app::SidebarRow::Repo(id)) = app.selected_sidebar_row() {
                    app.collapsed_repos.remove(&id);
                }
            }
            // `Enter` — open the repo's status summary, or open a workspace.
            KeyCode::Enter => match app.selected_sidebar_row() {
                Some(app::SidebarRow::Repo(id)) => app.open_repo_summary(id),
                Some(app::SidebarRow::Workspace(id)) => {
                    activate_workspace(app, backend, id).await;
                }
                None => {}
            },
            // `l` — expand the repo (collapse/expand stays on h/l/arrows), or
            // open a workspace.
            KeyCode::Char('l') => match app.selected_sidebar_row() {
                Some(app::SidebarRow::Repo(_)) => app.toggle_collapse_selected(),
                Some(app::SidebarRow::Workspace(id)) => {
                    activate_workspace(app, backend, id).await;
                }
                None => {}
            },
            KeyCode::Char('a') if allow_modals => {
                let cwd = std::env::current_dir()
                    .unwrap_or_else(|_| PathBuf::from("."))
                    .display()
                    .to_string();
                app.begin_add_workspace(cwd);
            }
            KeyCode::Char('A') if allow_modals => app.begin_add_ssh_workspace(),
            // `D` — delete the selected workspace, or remove the selected repository.
            KeyCode::Char('D') => match app.selected_sidebar_row() {
                Some(app::SidebarRow::Workspace(id)) => {
                    app.pending_delete_workspace = Some(id);
                }
                Some(app::SidebarRow::Repo(repo_id)) => {
                    app.pending_delete_repo = Some(repo_id);
                }
                None => {}
            },
            KeyCode::Char('R') => {
                if let Some(id) = app.selected_sidebar_workspace_id() {
                    activate_workspace(app, backend, id).await;
                    app.enter_review_mode();
                    let _ = backend.cmd_tx.send(Command::LoadBranchDiff { id }).await;
                }
            }
            KeyCode::Char(' ') => {
                if let Some(id) = app.selected_sidebar_workspace_id() {
                    let ready = app
                        .workspaces
                        .iter()
                        .find(|w| w.id == id)
                        .map(|w| w.ready_for_review)
                        .unwrap_or(false);
                    let _ = backend
                        .cmd_tx
                        .send(Command::SetReadyForReview { id, ready: !ready })
                        .await;
                }
            }
            KeyCode::Char('f') => {
                app.sidebar_review_filter = !app.sidebar_review_filter;
            }
            KeyCode::Char('g') => {
                if let Some(id) = app.selected_sidebar_workspace_id() {
                    let _ = backend.cmd_tx.send(Command::RefreshGit { id }).await;
                }
            }
            KeyCode::Char('S') if allow_modals => app.open_settings(),
            _ => return false,
        }
    }
    true
}

/// Keys for the repo status summary (`Route::Repo`): navigate the workspace
/// list, open the selected one, or step back to Home. Returns `true` when the
/// key was consumed.
async fn handle_repo_summary_key(app: &mut TuiApp, backend: &Backend, key: KeyEvent) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char('b') if ctrl => app.cycle_sidebar_mode(),
        KeyCode::Esc | KeyCode::Char('h') | KeyCode::Left => app.go_home(),
        KeyCode::Down | KeyCode::Char('j') => app.move_repo_summary_selection(1),
        KeyCode::Up | KeyCode::Char('k') => app.move_repo_summary_selection(-1),
        KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => {
            if let Some(id) = app.selected_repo_summary_workspace_id() {
                activate_workspace(app, backend, id).await;
            }
        }
        // `n` — new workspace under this repo (mirrors the empty-state hint).
        KeyCode::Char('n') => {
            if let Some(repo_id) = app.repo_summary_repo_id() {
                app.begin_quick_create(repo_id);
                let _ = backend
                    .cmd_tx
                    .send(Command::ListRepoBranches { repo_id })
                    .await;
            }
        }
        // `D` — delete the selected workspace.
        KeyCode::Char('D') => {
            if let Some(id) = app.selected_repo_summary_workspace_id() {
                app.pending_delete_workspace = Some(id);
            }
        }
        KeyCode::Char('g') => {
            if let Some(id) = app.selected_repo_summary_workspace_id() {
                let _ = backend.cmd_tx.send(Command::RefreshGit { id }).await;
            }
        }
        _ => return false,
    }
    true
}

async fn handle_workspace_command_key(
    app: &mut TuiApp,
    backend: &Backend,
    id: WorkspaceId,
    key: KeyEvent,
) -> bool {
    if !app.is_workspace_command_open() {
        return false;
    }

    match key.code {
        KeyCode::Esc => app.close_workspace_command(),
        KeyCode::Enter => {
            if let Some(command) = app.take_workspace_command_request() {
                let _ = backend
                    .cmd_tx
                    .send(Command::RunWorkspaceCommand { id, command })
                    .await;
            }
        }
        KeyCode::Left => {
            if let Some(state) = app.workspace_command_mut() {
                state.input.move_left();
            }
        }
        KeyCode::Right => {
            if let Some(state) = app.workspace_command_mut() {
                state.input.move_right();
            }
        }
        KeyCode::Home => {
            if let Some(state) = app.workspace_command_mut() {
                state.input.move_home();
            }
        }
        KeyCode::End => {
            if let Some(state) = app.workspace_command_mut() {
                state.input.move_end();
            }
        }
        KeyCode::Backspace => {
            if let Some(state) = app.workspace_command_mut() {
                if !state.running {
                    state.input.backspace();
                }
            }
        }
        KeyCode::Delete => {
            if let Some(state) = app.workspace_command_mut() {
                if !state.running {
                    state.input.delete();
                }
            }
        }
        KeyCode::Up => app.scroll_workspace_command_output(-1),
        KeyCode::Down => app.scroll_workspace_command_output(1),
        KeyCode::PageUp => app.scroll_workspace_command_output(-10),
        KeyCode::PageDown => app.scroll_workspace_command_output(10),
        KeyCode::Char(c) => {
            if let Some(state) = app.workspace_command_mut() {
                if !state.running {
                    state.input.insert_char(c);
                }
            }
        }
        _ => {}
    }

    true
}

fn paste_into_workspace_command(state: &mut app::WorkspaceCommandState, text: &str) {
    for c in text.chars() {
        match c {
            '\r' | '\n' | '\t' => state.input.insert_char(' '),
            c if !c.is_control() => state.input.insert_char(c),
            _ => {}
        }
    }
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
            // Deliver a queued initial prompt once the agent has produced output.
            if kind == protocol::TerminalKind::Agent {
                if matches!(&app.pending_agent_prompt, Some((pid, _)) if *pid == id) {
                    app.pending_prompt_send = app.pending_agent_prompt.take();
                }
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

            if kind == protocol::TerminalKind::Agent {
                match app.handle_agent_exit(id, code) {
                    app::AgentExitAction::Fallback { prompt } => {
                        app.pending_agent_fallback = Some((id, prompt));
                    }
                    app::AgentExitAction::RespawnShell => {
                        // Keep existing behavior once fallback is not applicable.
                        app.pending_agent_respawn = Some((id, tab_id));
                    }
                    app::AgentExitAction::None => {}
                }
            }
        }
        CoreEvent::TerminalStarted {
            id, kind, tab_id, ..
        } => {
            if kind == protocol::TerminalKind::Agent {
                app.record_agent_started(id);
            }
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
        CoreEvent::WorkspaceCommandResult {
            id,
            cwd,
            command,
            exit_code,
        } => app.apply_workspace_command_result(id, cwd, command, exit_code),
        CoreEvent::WorkspaceCommandOutput {
            id,
            cwd,
            stream,
            data,
        } => app.append_workspace_command_output(id, cwd, stream, data),
        CoreEvent::WorkspaceAttentionChanged { id, level } => {
            if let Some(ws) = app.workspaces.iter_mut().find(|w| w.id == id) {
                ws.attention = level;
            }
        }
        CoreEvent::ShellForegroundChanged {
            id,
            tab_id,
            command,
        } => {
            app.apply_foreground_change(id, tab_id, command);
        }
        CoreEvent::ShellResurrectionChanged {
            id,
            tab_id,
            command,
        } => {
            app.apply_shell_resurrection_change(id, tab_id, command);
        }
        CoreEvent::Error { message } => {
            app.git_action_message = Some((message, Instant::now()));
        }
        // --- Repository registry + review ---
        CoreEvent::RepositoryList { items } => app.set_repositories(items),
        CoreEvent::WorkspaceCreated { id, .. } => {
            // Defer the open + agent-start until after WorkspaceList is applied
            // (it arrives in the same drain), so the workspace is in the list.
            app.pending_open_created = Some(id);
            if let Some(prompt) = app.pending_create_prompt.take() {
                if !prompt.trim().is_empty() {
                    app.pending_agent_prompt = Some((id, prompt));
                }
            }
        }
        CoreEvent::WorktreeCreateProgress { stage, .. } => {
            app.git_action_message = Some((format!("creating worktree: {stage}"), Instant::now()));
        }
        CoreEvent::RepoBranches {
            repo_id,
            local,
            remote,
        } => {
            if let Some(qc) = app.quick_create.as_mut() {
                if qc.repo_id == repo_id {
                    qc.set_branches(local, remote);
                }
            }
        }
        CoreEvent::WorkspaceReviewChanged { id, ready } => {
            if let Some(ws) = app.workspaces.iter_mut().find(|w| w.id == id) {
                ws.ready_for_review = ready;
            }
        }
        CoreEvent::BranchDiffFilesLoaded { id, files, .. } => {
            let paths: Vec<String> = files.into_iter().map(|f| f.path).collect();
            if let Some(first) = paths.first() {
                app.pending_review_diff = Some((id, first.clone()));
            }
            app.review_files.insert(id, paths);
            app.review_file_selected = 0;
        }
    }
}

fn cycle_workspace_focus(focus: app::Focus) -> app::Focus {
    match focus {
        app::Focus::WsTerminalTabs => app::Focus::WsTerminal,
        app::Focus::WsTerminal => app::Focus::WsLog,
        app::Focus::WsLog => app::Focus::WsBranches,
        app::Focus::WsBranches => app::Focus::WsDiff,
        app::Focus::WsDiff => app::Focus::WsTerminalTabs,
        _ => app::Focus::WsTerminalTabs,
    }
}

fn workspace_command_shortcut_enabled(app: &TuiApp) -> bool {
    matches!(
        app.focus,
        app::Focus::WsLog | app::Focus::WsBranches | app::Focus::WsDiff
    ) || (matches!(app.focus, app::Focus::WsTerminal) && app.terminal_command_mode())
}

fn cycle_workspace_focus_reverse(focus: app::Focus) -> app::Focus {
    match focus {
        app::Focus::WsTerminalTabs => app::Focus::WsDiff,
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
    let full = match terminal.size() {
        Ok(s) => ratatui::layout::Rect::new(0, 0, s.width, s.height),
        Err(_) => return,
    };
    // Mirror the draw split: `area` is the detail pane (offset right by the
    // sidebar), `sidebar_rect` is the tree on the left. Hit-tests below run
    // against `area`, so detail clicks line up with what's rendered.
    let (sidebar_rect, area) = {
        let w = ui::widgets::sidebar::width(app.sidebar_mode);
        if w > 0 {
            let chunks =
                Layout::horizontal([Constraint::Length(w), Constraint::Min(0)]).split(full);
            (Some(chunks[0]), chunks[1])
        } else {
            (None, full)
        }
    };

    // The Rail's workspace pop-out floats over the detail pane, so it gets first
    // crack at clicks: inside it selects+opens a workspace; a click elsewhere
    // (outside the rail) dismisses it.
    if app.sidebar_mode == app::SidebarMode::Rail && app.sidebar_popout.is_some() {
        if let (Some(sr), MouseEventKind::Down(MouseButton::Left)) = (sidebar_rect, mouse.kind) {
            if let Some(idx) =
                ui::widgets::sidebar::popout_row_index_at(sr, full, app, mouse.column, mouse.row)
            {
                app.popout_selected = idx;
                if let Some(id) = app.selected_popout_workspace_id() {
                    app.close_sidebar_popout();
                    if Some(id) != app.active_workspace_id() {
                        app.open_workspace(id);
                        let size = app.terminal_content_size;
                        start_workspace_tab_terminals(cmd_tx, app, id, size).await;
                        let _ = cmd_tx.send(Command::RefreshGit { id }).await;
                        let _ = cmd_tx.send(Command::ClearAttention { id }).await;
                    }
                }
                return;
            }
            if !point_in_rect(sr, mouse.column, mouse.row) {
                app.close_sidebar_popout();
                return;
            }
        }
    }

    // The sidebar is clickable from any route and regardless of focus (including
    // while the terminal is focused or in passthrough). Handle it before
    // anything else so a click on the sidebar always lands here.
    if let Some(sr) = sidebar_rect {
        if point_in_rect(sr, mouse.column, mouse.row) {
            match mouse.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    if app.sidebar_mode == app::SidebarMode::Rail {
                        if let Some(ri) = ui::widgets::sidebar::rail_repo_index_at(
                            sr,
                            app,
                            mouse.column,
                            mouse.row,
                        ) {
                            app.rail_selected = ri;
                            app.focus = app::Focus::Sidebar;
                            app.open_sidebar_popout();
                        }
                    } else if let Some(idx) =
                        ui::widgets::sidebar::row_index_at(sr, app, mouse.column, mouse.row)
                    {
                        app.sidebar_selected = idx;
                        app.focus = app::Focus::Sidebar;
                        match app.sidebar_rows()[idx] {
                            app::SidebarRow::Repo(_) => app.toggle_collapse_selected(),
                            app::SidebarRow::Workspace(id) => {
                                if Some(id) != app.active_workspace_id() {
                                    app.open_workspace(id);
                                    let size = app.terminal_content_size;
                                    start_workspace_tab_terminals(cmd_tx, app, id, size).await;
                                    let _ = cmd_tx.send(Command::RefreshGit { id }).await;
                                    let _ = cmd_tx.send(Command::ClearAttention { id }).await;
                                }
                            }
                        }
                    }
                    return;
                }
                MouseEventKind::ScrollUp => {
                    if app.sidebar_mode == app::SidebarMode::Rail {
                        app.move_rail_selection(-1);
                    } else {
                        app.move_sidebar_selection(-1);
                    }
                    return;
                }
                MouseEventKind::ScrollDown => {
                    if app.sidebar_mode == app::SidebarMode::Rail {
                        app.move_rail_selection(1);
                    } else {
                        app.move_sidebar_selection(1);
                    }
                    return;
                }
                // Drag / button-up fall through to the global selection handlers
                // so a drag that began in a detail pane can still release here.
                _ => {}
            }
        }
    }

    if matches!(app.route, Route::Workspace { .. }) && app.is_workspace_command_open() {
        match mouse.kind {
            MouseEventKind::ScrollUp => app.scroll_workspace_command_output(-3),
            MouseEventKind::ScrollDown => app.scroll_workspace_command_output(3),
            _ => {}
        }
        return;
    }

    if forward_mouse_to_terminal(app, cmd_tx, area, mouse).await {
        return;
    }

    if terminal_pane_owns_mouse(app, area, mouse.column, mouse.row)
        && matches!(
            mouse.kind,
            MouseEventKind::Down(MouseButton::Left)
                | MouseEventKind::Drag(MouseButton::Left)
                | MouseEventKind::Up(MouseButton::Left)
        )
    {
        if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
            app.focus = app::Focus::WsTerminal;
        }
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

    // The delete-confirm modal floats over the detail pane regardless of route
    // (it can be raised from the sidebar while a workspace is open), so handle
    // clicks on it before the per-route handlers.
    if app.is_confirming_delete() {
        if let MouseEventKind::Down(MouseButton::Left) = mouse.kind {
            let rect = ui::screens::home::delete_modal_rect(area);
            if point_in_rect(rect, mouse.column, mouse.row) {
                let mid = rect.x + rect.width / 2;
                if mouse.column < mid {
                    if let Some(id) = app.take_delete_workspace() {
                        let _ = cmd_tx.send(Command::RemoveWorkspace { id }).await;
                    } else if let Some(repo_id) = app.take_delete_repo() {
                        let _ = cmd_tx.send(Command::RemoveRepository { repo_id }).await;
                    }
                } else {
                    app.cancel_delete_workspace();
                }
            } else {
                app.cancel_delete_workspace();
            }
            return;
        }
    }

    match app.route {
        Route::Home => match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if app.is_adding_workspace() {
                    let rect = ui::screens::home::add_modal_rect(area);
                    if !point_in_rect(rect, mouse.column, mouse.row) {
                        app.cancel_add_workspace();
                    }
                    return;
                }
                // Otherwise start a text selection confined to the clicked element.
                let rect = selection_confine_rect(area, app, mouse.column, mouse.row);
                app.mouse_selection = Some(app::MouseSelection::at_confined(
                    mouse.column,
                    mouse.row,
                    rect,
                ));
            }
            _ => {}
        },
        Route::Workspace { id } => match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let rect = selection_confine_rect(area, app, mouse.column, mouse.row);
                app.mouse_selection = Some(app::MouseSelection::at_confined(
                    mouse.column,
                    mouse.row,
                    rect,
                ));
                if let Some(hit) =
                    ui::screens::workspace::hit_test(area, app, mouse.column, mouse.row)
                {
                    match hit {
                        ui::screens::workspace::WorkspaceHit::TerminalTab(idx) => {
                            app.focus = app::Focus::WsTerminalTabs;
                            app.set_active_tab_index(idx);
                        }
                        ui::screens::workspace::WorkspaceHit::TerminalPane => {
                            app.focus = app::Focus::WsTerminal;
                        }
                        ui::screens::workspace::WorkspaceHit::ScrollToBottom => {
                            let tab_id = app.active_tab_id();
                            app.reset_terminal_scrollback(id, &tab_id);
                        }
                        ui::screens::workspace::WorkspaceHit::LogList(idx) => {
                            app.focus = app::Focus::WsLog;
                            app.ws_selected_commit = idx;
                            match app.log_item_at(idx) {
                                app::LogItem::UncommittedHeader => {
                                    app.ws_uncommitted_expanded = !app.ws_uncommitted_expanded;
                                }
                                app::LogItem::ChangedFile(_) => {
                                    if let Some(file) = app.selected_log_file() {
                                        let _ = cmd_tx.send(Command::LoadDiff { id, file }).await;
                                    }
                                }
                                app::LogItem::ChangedDirectory(_) => {
                                    app.toggle_selected_uncommitted_directory();
                                }
                                app::LogItem::Commit(ci) => {
                                    if app.ws_expanded_commit == Some(ci) {
                                        app.ws_expanded_commit = None;
                                    } else {
                                        app.ws_expanded_commit = Some(ci);
                                        if let Some(hash) = app.selected_commit_hash() {
                                            if !app.commit_files_cache.contains_key(&hash) {
                                                let _ = cmd_tx
                                                    .send(Command::LoadCommitFiles { id, hash })
                                                    .await;
                                            }
                                        }
                                    }
                                }
                                app::LogItem::CommitFile(_, _) => {
                                    if let Some((hash, file)) = app.selected_commit_file() {
                                        let _ = cmd_tx
                                            .send(Command::LoadCommitFileDiff { id, hash, file })
                                            .await;
                                    }
                                }
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
        Route::Repo { .. } => {
            if let MouseEventKind::Down(MouseButton::Left) = mouse.kind {
                // Start a text selection confined to the clicked element.
                let rect = selection_confine_rect(area, app, mouse.column, mouse.row);
                app.mouse_selection =
                    Some(app::MouseSelection::at_confined(mouse.column, mouse.row, rect));
            }
        }
    }
}

fn point_in_rect(r: ratatui::layout::Rect, x: u16, y: u16) -> bool {
    x >= r.x && y >= r.y && x < r.right() && y < r.bottom()
}

fn terminal_pane_owns_mouse(app: &TuiApp, area: ratatui::layout::Rect, x: u16, y: u16) -> bool {
    if app.terminal_command_mode() {
        return false;
    }
    if !matches!(app.route, Route::Workspace { .. }) {
        return false;
    }
    matches!(
        ui::screens::workspace::hit_test(area, app, x, y),
        Some(ui::screens::workspace::WorkspaceHit::TerminalPane)
    )
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

    if app.terminal_command_mode() {
        return false;
    }

    // Ctrl+Click opens URLs in the default browser.
    if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
        && mouse.modifiers.contains(KeyModifiers::CONTROL)
    {
        let tab_id = app.active_tab_id();
        let col = mouse.column.saturating_sub(content.x);
        let row = mouse.row.saturating_sub(content.y);
        if let Some(url) = app.url_at_terminal_position(id, &tab_id, row, col) {
            open_url(&url);
            return true;
        }
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
    fn detail_area_matches_sidebar_width_per_mode() {
        use app::SidebarMode::*;
        let full = Rect::new(0, 0, 120, 40);

        // Hidden: terminal sizing uses the whole width.
        assert_eq!(detail_area(full, Hidden), full);

        // Expanded / Rail: the detail area is offset and narrowed by exactly the
        // sidebar width the renderer draws, so the PTY is sized to the visible
        // pane and never told it is wider than what's on screen.
        for mode in [Expanded, Rail] {
            let w = ui::widgets::sidebar::width(mode);
            let detail = detail_area(full, mode);
            assert_eq!(detail.x, w, "mode {mode:?} x");
            assert_eq!(detail.width, 120 - w, "mode {mode:?} width");
            assert_eq!(detail.height, 40);
        }
    }

    #[test]
    fn terminal_sizing_stays_within_drawn_pane_in_every_mode() {
        use app::SidebarMode::*;
        let full = Rect::new(0, 0, 200, 50);
        for mode in [Expanded, Rail, Hidden] {
            // Sizing path: the geometry the resize loop sends to the PTY.
            let area = detail_area(full, mode);
            let content =
                ui::screens::workspace::terminal_content_rect(area, app::Focus::WsTerminal, false);
            // Draw path: the carve `terminal.draw` actually renders the workspace
            // into. These must be identical so the PTY can never be told it is
            // wider than the pane on screen.
            let sidebar_w = ui::widgets::sidebar::width(mode);
            let drawn = if sidebar_w > 0 {
                Layout::horizontal([Constraint::Length(sidebar_w), Constraint::Min(0)]).split(full)
                    [1]
            } else {
                full
            };
            assert_eq!(
                area, drawn,
                "sizing carve must match drawn carve in {mode:?}"
            );
            assert!(
                content.right() <= drawn.right(),
                "{mode:?} overflows right edge"
            );
            assert!(content.x >= drawn.x, "{mode:?} starts left of the pane");
        }
    }

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
    fn agent_cmd_for_falls_back_to_default_when_unset() {
        let settings = app::Settings::default();
        // None / empty → the global default agent (claude).
        assert_eq!(agent_cmd_for(&settings, None), vec!["claude".to_string()]);
        assert_eq!(
            agent_cmd_for(&settings, Some("  ")),
            vec!["claude".to_string()]
        );
    }

    #[test]
    fn agent_cmd_for_uses_named_profile_and_yolo_flags() {
        let mut settings = app::Settings::default();
        // A configured profile name resolves to its command (no yolo by default).
        assert_eq!(
            agent_cmd_for(&settings, Some("codex")),
            vec!["codex".to_string()]
        );
        // With yolo_mode on, the profile's yolo flags are appended.
        settings.yolo_mode = true;
        assert_eq!(
            agent_cmd_for(&settings, Some("codex")),
            vec!["codex".to_string(), "--full-auto".to_string()],
        );
    }

    #[test]
    fn agent_cmd_for_treats_unknown_value_as_custom_command() {
        let settings = app::Settings::default();
        // Not a known profile → split the raw command on whitespace into argv.
        assert_eq!(
            agent_cmd_for(&settings, Some("aider --model gpt-4")),
            vec![
                "aider".to_string(),
                "--model".to_string(),
                "gpt-4".to_string()
            ],
        );
    }

    #[test]
    fn expanded_then_edited_command_launches_verbatim() {
        let settings = app::Settings::default();
        // Expanding `codex` (yolo on) yields its full command; the user edits it
        // and the edited string launches verbatim as argv (custom-command path).
        let mut settings_yolo = settings.clone();
        settings_yolo.yolo_mode = true;
        let expanded = agent_cmd_for(&settings_yolo, Some("codex")).join(" ");
        assert_eq!(expanded, "codex --full-auto");
        let edited = format!("{expanded} --model o3");
        assert_eq!(
            agent_cmd_for(&settings, Some(&edited)),
            vec![
                "codex".to_string(),
                "--full-auto".to_string(),
                "--model".to_string(),
                "o3".to_string(),
            ],
        );
    }

    #[test]
    fn agent_vanilla_cmd_for_ignores_claude_yolo_continue_flag() {
        let mut settings = app::Settings::default();
        settings.yolo_mode = true;
        let claude = settings
            .agents
            .iter_mut()
            .find(|agent| agent.name == "claude")
            .expect("claude profile");
        claude.yolo_flags = vec!["-c".to_string()];

        assert_eq!(
            agent_vanilla_cmd_for(&settings, Some("claude")),
            vec!["claude".to_string()],
        );
    }

    #[test]
    fn agent_vanilla_cmd_for_codex_profile_ignores_yolo_flags() {
        let mut settings = app::Settings::default();
        settings.yolo_mode = true;

        assert_eq!(
            agent_vanilla_cmd_for(&settings, Some("codex")),
            vec!["codex".to_string()],
        );
    }

    #[test]
    fn agent_vanilla_cmd_for_custom_command_uses_first_argv_token() {
        let settings = app::Settings::default();

        assert_eq!(
            agent_vanilla_cmd_for(&settings, Some("claude -c")),
            vec!["claude".to_string()],
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
        assert_eq!(
            parse_conduit_version_output("conduit 0.3.21\n"),
            Some("0.3.21")
        );
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

    // --- confine-aware extraction (selection stays within one UI element) ---

    fn put_char(buf: &mut ratatui::buffer::Buffer, col: u16, row: u16, ch: char) {
        buf.cell_mut(ratatui::layout::Position::new(col, row))
            .unwrap()
            .set_symbol(&ch.to_string());
    }

    #[test]
    fn extract_confined_multirow_clamps_intermediate_rows() {
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;

        // 20-wide buffer: in-band content in cols 2..=7, junk from an adjacent pane
        // in cols 12..=17. The selection is confined to the left band only.
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 4));
        for row in 0..4u16 {
            for (i, ch) in "ABCDEF".chars().enumerate() {
                put_char(&mut buf, 2 + i as u16, row, ch);
            }
            for (i, ch) in "XXXXXX".chars().enumerate() {
                put_char(&mut buf, 12 + i as u16, row, ch);
            }
        }

        let confine = Rect::new(2, 0, 6, 4); // cols 2..=7, all 4 rows
        let sel = app::MouseSelection {
            anchor_col: 4,
            anchor_row: 0,
            end_col: 5,
            end_row: 3,
            confine: Some(confine),
        };
        let text = extract_selected_text_from_buf(&buf, &sel, &[]);

        // Intermediate rows span only the confine band, never the adjacent pane.
        assert!(text.contains("ABCDEF"), "in-band content missing: {text:?}");
        assert!(!text.contains('X'), "adjacent-pane junk leaked: {text:?}");
    }

    #[test]
    fn extract_confined_singlerow_uses_endpoints_not_band() {
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;

        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 1));
        for (i, ch) in "0123456789".chars().enumerate() {
            put_char(&mut buf, i as u16, 0, ch);
        }
        // Wide confine, but a single-row selection only spans its own endpoints.
        let sel = app::MouseSelection {
            anchor_col: 3,
            anchor_row: 0,
            end_col: 6,
            end_row: 0,
            confine: Some(Rect::new(0, 0, 20, 1)),
        };
        assert_eq!(extract_selected_text_from_buf(&buf, &sel, &[]), "3456");
    }

    #[test]
    fn extract_unconfined_spans_full_width() {
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;

        // With no confine, intermediate rows still span the full buffer width
        // (legacy terminal-style flow selection is preserved).
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 3));
        for row in 0..3u16 {
            for (i, ch) in "abcdefghij".chars().enumerate() {
                put_char(&mut buf, i as u16, row, ch);
            }
        }
        let sel = app::MouseSelection {
            anchor_col: 8,
            anchor_row: 0,
            end_col: 1,
            end_row: 2,
            confine: None,
        };
        let text = extract_selected_text_from_buf(&buf, &sel, &[]);
        assert!(
            text.contains("abcdefghij"),
            "middle row not full width: {text:?}"
        );
    }

    // --- confine-rect resolver ---

    #[test]
    fn inset_for_border_insets_tall_keeps_short() {
        use ratatui::layout::Rect;

        // A tall bordered pane is inset one cell on each side.
        assert_eq!(
            inset_for_border(Rect::new(10, 5, 40, 20)),
            Rect::new(11, 6, 38, 18)
        );
        // A 2-row footer is left unchanged (insetting would zero its height).
        let footer = Rect::new(0, 38, 80, 2);
        assert_eq!(inset_for_border(footer), footer);
    }

    #[test]
    fn selection_confine_rect_home_returns_section_else_area() {
        use ratatui::layout::Rect;

        let app = TuiApp::default(); // defaults to Route::Home
        let area = Rect::new(0, 0, 80, 24);

        // A point inside resolves to a home section: a strict sub-rect of the detail
        // area, never the whole terminal.
        let r = selection_confine_rect(area, &app, 10, 10);
        assert!(r.height < area.height && r.width <= area.width);
        assert!(r.y >= area.y && r.bottom() <= area.bottom());

        // A point outside the area falls back to the detail area.
        assert_eq!(selection_confine_rect(area, &app, 200, 200), area);
    }
}

/// xterm-256 colour 39 — a medium sky-blue used for mouse selection highlighting.
const SELECTION_BG: ratatui::style::Color = ratatui::style::Color::Indexed(39);

fn apply_selection_highlight(frame: &mut ratatui::Frame, sel: &app::MouseSelection) {
    let ((start_col, start_row), (end_col, end_row)) = sel.ordered();
    let buf = frame.buffer_mut();
    let width = buf.area.width;
    // Intermediate rows of a multi-row selection span the confine band (a single UI
    // element) rather than the full terminal width, so the highlight stays inside the
    // clicked element instead of bleeding across panes.
    let (left_bound, right_bound) = match sel.confine {
        Some(r) => (r.x, r.right().saturating_sub(1)),
        None => (0, width.saturating_sub(1)),
    };
    for row in start_row..=end_row {
        let row_start = if row == start_row {
            start_col
        } else {
            left_bound
        };
        let row_end = if row == end_row { end_col } else { right_bound };
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
    // Confine intermediate rows to the selection's element band (mirrors
    // `apply_selection_highlight`) so copied text doesn't pull in adjacent panes.
    let (left_bound, right_bound) = match sel.confine {
        Some(r) => (r.x, r.right().saturating_sub(1)),
        None => (0, width.saturating_sub(1)),
    };
    let mut lines: Vec<String> = Vec::new();
    for row in start_row..=end_row {
        let row_start = if row == start_row {
            start_col
        } else {
            left_bound
        };
        let row_end = if row == end_row { end_col } else { right_bound };
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
    agent_profile_cmd(settings.active_agent(), settings.yolo_mode)
}

/// Resolves the command to launch in a Workspace's agent terminal, honoring a
/// per-Workspace `agent` choice. The choice is either a configured profile name
/// (→ that profile's command + yolo flags) or a raw custom command (→ split on
/// whitespace into argv). `None`/empty falls back to the global default agent.
/// Switch a workspace's agent: persist the choice, stop any running agent
/// process, and start the new agent fresh in the workspace's agent tab.
async fn apply_agent_switch(
    app: &mut TuiApp,
    backend: &Backend,
    id: protocol::WorkspaceId,
    agent: Option<String>,
) {
    app.agent_picker = None;
    // Optimistic update so agent resolution sees the new choice before the
    // core's WorkspaceList round-trip lands.
    if let Some(ws) = app.workspaces.iter_mut().find(|w| w.id == id) {
        ws.agent = agent.clone();
    }
    let _ = backend
        .cmd_tx
        .send(Command::SetWorkspaceAgent {
            id,
            agent: agent.clone(),
        })
        .await;
    if app.workspace_agent_running(id) {
        app.suppress_next_agent_exit(id);
    }
    let _ = backend
        .cmd_tx
        .send(Command::StopTerminal {
            id,
            kind: TerminalKind::Agent,
            tab_id: Some("agent".to_string()),
        })
        .await;
    let cmd = agent_cmd_for(&app.settings, agent.as_deref());
    app.queue_agent_startup(id, false);
    let _ = backend
        .cmd_tx
        .send(Command::StartTerminal {
            id,
            kind: TerminalKind::Agent,
            tab_id: Some("agent".to_string()),
            cmd,
            cols: app.terminal_content_size.0,
            rows: app.terminal_content_size.1,
        })
        .await;
    if matches!(app.route, Route::Workspace { id: rid } if rid == id) {
        if let Some(idx) = app
            .ws_tabs
            .iter()
            .position(|t| t.kind == TerminalKind::Agent)
        {
            app.set_active_tab_index(idx);
        }
        app.focus = app::Focus::WsTerminal;
    }
}

fn agent_cmd_for(settings: &app::Settings, agent_choice: Option<&str>) -> Vec<String> {
    let Some(choice) = agent_choice.map(str::trim).filter(|c| !c.is_empty()) else {
        return agent_cmd(settings);
    };
    if let Some(profile) = settings.agents.iter().find(|a| a.name == choice) {
        agent_profile_cmd(Some(profile), settings.yolo_mode)
    } else {
        split_raw_agent_cmd(choice)
    }
}

fn agent_vanilla_cmd_for(settings: &app::Settings, agent_choice: Option<&str>) -> Vec<String> {
    let Some(choice) = agent_choice.map(str::trim).filter(|c| !c.is_empty()) else {
        return agent_vanilla_profile_cmd(settings.active_agent());
    };
    if let Some(profile) = settings.agents.iter().find(|a| a.name == choice) {
        agent_vanilla_profile_cmd(Some(profile))
    } else {
        choice
            .split_whitespace()
            .next()
            .map(|program| vec![program.to_string()])
            .unwrap_or_else(|| agent_vanilla_profile_cmd(settings.active_agent()))
    }
}

fn agent_cmd_continue_for(settings: &app::Settings, agent_choice: Option<&str>) -> Vec<String> {
    let Some(choice) = agent_choice.map(str::trim).filter(|c| !c.is_empty()) else {
        return agent_profile_continue_cmd(settings.active_agent(), settings.yolo_mode);
    };
    if let Some(profile) = settings.agents.iter().find(|a| a.name == choice) {
        agent_profile_continue_cmd(Some(profile), settings.yolo_mode)
    } else {
        split_raw_agent_cmd(choice)
    }
}

fn agent_profile_cmd(agent: Option<&app::AgentProfile>, yolo_mode: bool) -> Vec<String> {
    let Some(agent) = agent else {
        return vec!["claude".to_string()];
    };
    let mut cmd = vec![agent.command.clone()];
    if yolo_mode {
        cmd.extend(agent.yolo_flags.iter().cloned());
    }
    cmd
}

fn agent_profile_continue_cmd(agent: Option<&app::AgentProfile>, yolo_mode: bool) -> Vec<String> {
    let Some(agent) = agent else {
        return vec!["claude".to_string()];
    };
    let mut cmd = vec![agent.command.clone()];
    cmd.extend(agent.continue_flags.iter().cloned());
    if yolo_mode {
        cmd.extend(agent.yolo_flags.iter().cloned());
    }
    cmd
}

fn agent_vanilla_profile_cmd(agent: Option<&app::AgentProfile>) -> Vec<String> {
    agent
        .map(|agent| vec![agent.command.clone()])
        .unwrap_or_else(|| vec!["claude".to_string()])
}

fn split_raw_agent_cmd(choice: &str) -> Vec<String> {
    choice.split_whitespace().map(str::to_string).collect()
}

fn open_url(url: &str) {
    #[cfg(target_os = "linux")]
    let cmd = "xdg-open";
    #[cfg(target_os = "macos")]
    let cmd = "open";
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let cmd = "xdg-open";

    let _ = OsCommand::new(cmd)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

/// Carves the persistent left sidebar (when visible) off the full terminal area,
/// returning the detail area the Home/Workspace screen is actually rendered into.
///
/// Terminal sizing must be derived from this — not the full terminal width — or
/// the embedded PTY is told it's wider than the pane we draw and the child's
/// output (e.g. the Claude TUI) wraps past the right edge. This mirrors the carve
/// in `terminal.draw` and the mouse handler.
fn detail_area(full: Rect, sidebar_mode: app::SidebarMode) -> Rect {
    let w = ui::widgets::sidebar::width(sidebar_mode);
    if w > 0 {
        Layout::horizontal([Constraint::Length(w), Constraint::Min(0)]).split(full)[1]
    } else {
        full
    }
}

/// Resolves the rect a mouse selection beginning at `(x, y)` should be confined to:
/// the UI element under the cursor. Falls back to `area` (the detail pane) so a
/// selection never spans the whole terminal or bleeds into the sidebar.
fn selection_confine_rect(area: Rect, app: &TuiApp, x: u16, y: u16) -> Rect {
    match app.route {
        Route::Workspace { .. } => match ui::screens::workspace::pane_rect_at(area, app, x, y) {
            Some(r) => inset_for_border(r),
            None => area,
        },
        Route::Home => ui::screens::home::chunk_at(area, x, y).unwrap_or(area),
        Route::Repo { .. } => ui::screens::repo_summary::chunk_at(area, x, y).unwrap_or(area),
    }
}

/// Insets a bordered pane rect by one cell on each side so the selection band sits
/// inside the frame (no highlight over border glyphs, no leading/trailing border
/// spaces in copied text). Left unchanged when too small to keep a non-empty
/// interior, e.g. the 2-row footer.
fn inset_for_border(r: Rect) -> Rect {
    if r.width > 2 && r.height > 2 {
        Rect::new(r.x + 1, r.y + 1, r.width - 2, r.height - 2)
    } else {
        r
    }
}

async fn start_workspace_tab_terminals(
    cmd_tx: &tokio::sync::mpsc::Sender<Command>,
    app: &mut TuiApp,
    id: protocol::WorkspaceId,
    size: (u16, u16),
) {
    let tabs = app.ws_tabs.clone();
    let settings = app.settings.clone();
    let agent_choice = app.workspace_agent(id);
    for tab in &tabs {
        let cmd = if tab.kind == protocol::TerminalKind::Agent {
            agent_cmd_for(&settings, agent_choice.as_deref())
        } else {
            Vec::new()
        };
        if tab.kind == protocol::TerminalKind::Agent && !cmd.is_empty() {
            app.queue_agent_startup(id, false);
        }
        let _ = cmd_tx
            .send(Command::StartTerminal {
                id,
                kind: tab.kind,
                tab_id: Some(tab.id.clone()),
                cmd,
                cols: size.0,
                rows: size.1,
            })
            .await;
    }
}
