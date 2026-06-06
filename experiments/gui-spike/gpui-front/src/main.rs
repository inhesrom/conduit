//! GPUI front-end for the Conduit GUI spike.
//!
//! Reuses Conduit's real `core` through the same `bridge` + `termgrid` as the
//! Iced front. GPUI-specific parts: render the vt100 grid as nested `div`s,
//! bridge the core `Event` stream into the view with `cx.spawn`, and forward
//! `observe_keystrokes` into `SendTerminalInput`. Like the Iced front, it
//! auto-selects the demo workspace and fires a scripted `echo SPIKE_INPUT_OK`
//! so the loop is verifiable from logs.

use std::sync::OnceLock;

use gpui::prelude::*;
use gpui::{
    div, px, rgb, App, AsyncApp, Context, FontWeight, Keystroke, MouseButton, MouseDownEvent,
    Render, Window, WindowOptions,
};
use gpui_platform::application;

use gui_spike_bridge::protocol::{TerminalKind, WorkspaceId, WorkspaceSummary};
use gui_spike_bridge::{broadcast, decode_b64, encode_b64, Command, CoreHandle, Event};
use termgrid::{idx_to_rgb, key_to_bytes, CellSnap, Color as TColor, Key, Term};

static CORE: OnceLock<CoreHandle> = OnceLock::new();

const ROWS: u16 = 24;
const COLS: u16 = 80;

fn send(cmd: Command) {
    if let Some(h) = CORE.get() {
        let _ = h.cmd_tx.try_send(cmd);
    }
}

fn subscribe() -> broadcast::Receiver<Event> {
    CORE.get().expect("core handle set").evt_tx.subscribe()
}

struct TermApp {
    workspaces: Vec<WorkspaceSummary>,
    active: Option<(WorkspaceId, TerminalKind)>,
    term: Term,
    scripted_input_sent: bool,
    got_output: bool,
    auto_selected: bool,
}

impl TermApp {
    fn new() -> Self {
        Self {
            workspaces: Vec::new(),
            active: None,
            term: Term::new(ROWS, COLS),
            scripted_input_sent: false,
            got_output: false,
            auto_selected: false,
        }
    }

    fn on_core(&mut self, ev: Event, cx: &mut Context<Self>) {
        match ev {
            Event::WorkspaceList { items } => {
                println!("[gpui] WorkspaceList: {} workspace(s)", items.len());
                self.workspaces = items;
                if !self.auto_selected {
                    if let Some(ws) = self.workspaces.iter().find(|w| w.name == "gui-spike-demo") {
                        self.auto_selected = true;
                        let id = ws.id;
                        self.select(id, cx);
                    }
                }
            }
            Event::TerminalOutput {
                id, kind, data_b64, ..
            } => {
                if self.active == Some((id, kind)) {
                    let bytes = decode_b64(&data_b64);
                    if !self.got_output {
                        self.got_output = true;
                        println!("[gpui] first TerminalOutput: {} bytes", bytes.len());
                    }
                    if String::from_utf8_lossy(&bytes).contains("SPIKE_INPUT_OK") {
                        println!("[gpui] terminal echoed SPIKE_INPUT_OK \u{2713}");
                    }
                    self.term.feed(&bytes);
                    // Once the shell is alive and producing output, fire the scripted input.
                    self.send_scripted_input();
                }
            }
            Event::TerminalStarted { id, kind, .. } => {
                println!("[gpui] TerminalStarted {id} {kind:?}")
            }
            Event::TerminalExited { code, .. } => println!("[gpui] TerminalExited {code:?}"),
            Event::Error { message } => eprintln!("[gpui] core error: {message}"),
            _ => {}
        }
        cx.notify();
    }

    fn select(&mut self, id: WorkspaceId, cx: &mut Context<Self>) {
        println!("[gpui] select {id} -> StartTerminal(shell)");
        self.active = Some((id, TerminalKind::Shell));
        self.term = Term::new(ROWS, COLS);
        send(Command::StartTerminal {
            id,
            kind: TerminalKind::Shell,
            tab_id: None,
            cmd: vec![],
        });
        send(Command::ResizeTerminal {
            id,
            kind: TerminalKind::Shell,
            tab_id: None,
            cols: COLS,
            rows: ROWS,
        });
        cx.notify();
    }

    fn send_scripted_input(&mut self) {
        if let Some((id, kind)) = self.active {
            if !self.scripted_input_sent {
                self.scripted_input_sent = true;
                println!("[gpui] scripted input: echo SPIKE_INPUT_OK");
                send(Command::SendTerminalInput {
                    id,
                    kind,
                    tab_id: None,
                    data_b64: encode_b64(b"echo SPIKE_INPUT_OK\r"),
                });
            }
        }
    }

    fn on_keystroke(&mut self, ks: &Keystroke) {
        let Some((id, kind)) = self.active else {
            return;
        };
        let ctrl = ks.modifiers.control;
        let alt = ks.modifiers.alt;
        let tk = match ks.key.as_str() {
            "enter" => Key::Enter,
            "backspace" => Key::Backspace,
            "tab" => Key::Tab,
            "escape" => Key::Escape,
            "up" => Key::Up,
            "down" => Key::Down,
            "left" => Key::Left,
            "right" => Key::Right,
            "home" => Key::Home,
            "end" => Key::End,
            "pageup" => Key::PageUp,
            "pagedown" => Key::PageDown,
            "delete" => Key::Delete,
            "space" => Key::Char(' '),
            _ => {
                let ch = if ctrl {
                    ks.key.chars().next()
                } else {
                    ks.key_char
                        .as_ref()
                        .and_then(|s| s.chars().next())
                        .or_else(|| ks.key.chars().next())
                };
                match ch {
                    Some(c) => Key::Char(c),
                    None => return,
                }
            }
        };
        if let Some(bytes) = key_to_bytes(tk, ctrl, alt) {
            send(Command::SendTerminalInput {
                id,
                kind,
                tab_id: None,
                data_b64: encode_b64(&bytes),
            });
        }
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.active.map(|(id, _)| id);
        let mut col = div()
            .w(px(240.0))
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .bg(rgb(0x15181d))
            .text_color(rgb(0xdddddd))
            .child(div().child("Workspaces").text_size(px(15.0)));
        for ws in &self.workspaces {
            let id = ws.id;
            let selected = active == Some(id);
            let label = format!("{}{}", if selected { "\u{25B8} " } else { "  " }, ws.name);
            col = col.child(
                div()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .bg(rgb(if selected { 0x2a2f37 } else { 0x15181d }))
                    .child(label)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _ev: &MouseDownEvent, _window, cx| {
                            this.select(id, cx)
                        }),
                    ),
            );
        }
        col
    }

    fn term_pane(&self) -> impl IntoElement {
        let snap = self.term.snapshot();
        let rows = snap.into_iter().map(|cells| {
            div()
                .flex()
                .flex_row()
                .children(coalesce(&cells).into_iter().map(|run| {
                    let mut d = div().bg(rgb(run.bg)).text_color(rgb(run.fg)).child(run.text);
                    if run.bold {
                        d = d.font_weight(FontWeight::BOLD);
                    }
                    d
                }))
        });
        div()
            .flex()
            .flex_col()
            .flex_grow(1.0)
            .h_full()
            .bg(rgb(0x101216))
            .font_family("monospace")
            .text_size(px(13.0))
            .p_2()
            .children(rows)
    }
}

impl Render for TermApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_row()
            .size_full()
            .bg(rgb(0x0b0d10))
            .child(self.sidebar(cx))
            .child(self.term_pane())
    }
}

struct Run {
    text: String,
    fg: u32,
    bg: u32,
    bold: bool,
}

/// Coalesce consecutive cells with identical resolved style into runs.
fn coalesce(cells: &[CellSnap]) -> Vec<Run> {
    let mut runs = Vec::new();
    let mut i = 0;
    while i < cells.len() {
        let fg = resolve(cells[i].fg, true);
        let bg = resolve(cells[i].bg, false);
        let bold = cells[i].bold;
        let mut text = String::new();
        while i < cells.len()
            && resolve(cells[i].fg, true) == fg
            && resolve(cells[i].bg, false) == bg
            && cells[i].bold == bold
        {
            text.push_str(&cells[i].text);
            i += 1;
        }
        runs.push(Run { text, fg, bg, bold });
    }
    runs
}

fn resolve(c: TColor, is_fg: bool) -> u32 {
    match c {
        TColor::Default => {
            if is_fg {
                0x00cc_cccc
            } else {
                0x0010_1216
            }
        }
        TColor::Idx(i) => {
            let (r, g, b) = idx_to_rgb(i);
            rgb_u32(r, g, b)
        }
        TColor::Rgb(r, g, b) => rgb_u32(r, g, b),
    }
}

fn rgb_u32(r: u8, g: u8, b: u8) -> u32 {
    ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}

fn main() {
    let backend = gui_spike_bridge::Backend::start();
    let _ = CORE.set(backend.core_handle());

    application().run(|cx: &mut App| {
        let window = cx
            .open_window(WindowOptions::default(), |_window, cx| {
                cx.new(|_cx| TermApp::new())
            })
            .unwrap();
        let view = window.update(cx, |_, _, cx| cx.entity()).unwrap();

        let cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| ".".into());
        println!("[gpui] boot: AddWorkspace gui-spike-demo at {cwd}");
        send(Command::AddWorkspace {
            name: "gui-spike-demo".into(),
            path: cwd,
            ssh: None,
        });

        // Bridge core events into the view.
        let weak = view.downgrade();
        cx.spawn(async move |acx: &mut AsyncApp| {
            let mut rx = subscribe();
            loop {
                match rx.recv().await {
                    Ok(ev) => {
                        if weak.update(acx, |app, cx| app.on_core(ev, cx)).is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        })
        .detach();

        // Forward keystrokes to the active terminal.
        let weak3 = view.downgrade();
        cx.observe_keystrokes(move |ev, _window, cx| {
            let ks = ev.keystroke.clone();
            let _ = weak3.update(cx, |app, _cx| app.on_keystroke(&ks));
        })
        .detach();

        window.update(cx, |_, _, cx| cx.activate(true)).unwrap();
    });

    drop(backend);
}
