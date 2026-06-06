//! Iced front-end for the Conduit GUI spike.
//!
//! Reuses Conduit's real `core` through the shared bridge (workspace list +
//! embedded terminal), renders the vt100 grid from `termgrid` with iced
//! widgets, and feeds keyboard input back as `SendTerminalInput`. To make the
//! whole loop verifiable headlessly, it auto-selects the demo workspace and
//! fires a scripted `echo SPIKE_INPUT_OK` shortly after the shell starts.

use std::sync::OnceLock;
use std::time::Duration;

use iced::font::{Style as FontStyle, Weight};
use iced::widget::{button, container, text, Column, Row};
use iced::{Background, Color as IColor, Element, Font, Length, Subscription, Task, Theme};

use gui_spike_bridge::protocol::{TerminalKind, WorkspaceId, WorkspaceSummary};
use gui_spike_bridge::{broadcast, decode_b64, encode_b64, Command, CoreHandle, Event};
use termgrid::{idx_to_rgb, key_to_bytes, CellSnap, Color as TColor, Key, Term};

/// Cloneable core handle, stashed so subscriptions/update can reach it without the Backend.
static CORE: OnceLock<CoreHandle> = OnceLock::new();

const ROWS: u16 = 24;
const COLS: u16 = 80;
const CELL_SIZE: u16 = 14;

fn send(cmd: Command) {
    if let Some(h) = CORE.get() {
        let _ = h.cmd_tx.try_send(cmd);
    }
}

fn main() -> iced::Result {
    let backend = gui_spike_bridge::Backend::start();
    let _ = CORE.set(backend.core_handle());
    let res = iced::application("Conduit GUI Spike — Iced", App::update, App::view)
        .subscription(App::subscription)
        .theme(|_| Theme::Dark)
        .default_font(Font::MONOSPACE)
        .run_with(App::new);
    drop(backend); // keep the runtime alive for the whole app run
    res
}

struct App {
    workspaces: Vec<WorkspaceSummary>,
    active: Option<(WorkspaceId, TerminalKind)>,
    term: Term,
    scripted_input_sent: bool,
}

#[derive(Debug, Clone)]
enum Message {
    Core(Event),
    Boot,
    Select(WorkspaceId),
    Key(Key, bool, bool),
    ScriptTick,
}

impl App {
    fn new() -> (Self, Task<Message>) {
        let app = App {
            workspaces: Vec::new(),
            active: None,
            term: Term::new(ROWS, COLS),
            scripted_input_sent: false,
        };
        (app, Task::done(Message::Boot))
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Boot => {
                let cwd = std::env::current_dir()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|_| ".".into());
                println!("[iced] boot: AddWorkspace gui-spike-demo at {cwd}");
                send(Command::AddWorkspace {
                    name: "gui-spike-demo".into(),
                    path: cwd,
                    ssh: None,
                });
            }
            Message::Core(ev) => self.on_core(ev),
            Message::Select(id) => self.select(id),
            Message::Key(k, ctrl, alt) => {
                if let Some((id, kind)) = self.active {
                    if let Some(bytes) = key_to_bytes(k, ctrl, alt) {
                        send(Command::SendTerminalInput {
                            id,
                            kind,
                            tab_id: None,
                            data_b64: encode_b64(&bytes),
                        });
                    }
                }
            }
            Message::ScriptTick => {
                if let Some((id, kind)) = self.active {
                    if !self.scripted_input_sent {
                        self.scripted_input_sent = true;
                        println!("[iced] scripted input: echo SPIKE_INPUT_OK");
                        send(Command::SendTerminalInput {
                            id,
                            kind,
                            tab_id: None,
                            data_b64: encode_b64(b"echo SPIKE_INPUT_OK\r"),
                        });
                    }
                }
            }
        }
        Task::none()
    }

    fn on_core(&mut self, ev: Event) {
        match ev {
            Event::WorkspaceList { items } => {
                println!("[iced] WorkspaceList: {} workspace(s)", items.len());
                self.workspaces = items;
                if self.active.is_none() {
                    if let Some(ws) = self.workspaces.iter().find(|w| w.name == "gui-spike-demo") {
                        let id = ws.id;
                        self.select(id);
                    }
                }
            }
            Event::TerminalOutput {
                id, kind, data_b64, ..
            } => {
                if self.active == Some((id, kind)) {
                    let bytes = decode_b64(&data_b64);
                    if String::from_utf8_lossy(&bytes).contains("SPIKE_INPUT_OK") {
                        println!("[iced] terminal echoed SPIKE_INPUT_OK \u{2713}");
                    }
                    self.term.feed(&bytes);
                }
            }
            Event::TerminalStarted { id, kind, .. } => {
                println!("[iced] TerminalStarted {id} {kind:?}")
            }
            Event::TerminalExited { code, .. } => println!("[iced] TerminalExited {code:?}"),
            Event::Error { message } => eprintln!("[iced] core error: {message}"),
            _ => {}
        }
    }

    fn select(&mut self, id: WorkspaceId) {
        println!("[iced] select {id} -> StartTerminal(shell)");
        self.active = Some((id, TerminalKind::Shell));
        self.term = Term::new(ROWS, COLS);
        send(Command::StartTerminal {
            id,
            kind: TerminalKind::Shell,
            tab_id: None,
            cmd: vec![],
        });
        let (rows, cols) = self.term.size();
        send(Command::ResizeTerminal {
            id,
            kind: TerminalKind::Shell,
            tab_id: None,
            cols,
            rows,
        });
    }

    fn view(&self) -> Element<'_, Message> {
        // Sidebar: workspace list.
        let mut list = Column::new()
            .spacing(4)
            .padding(8)
            .width(Length::Fixed(240.0));
        list = list.push(text("Workspaces").size(18));
        for ws in &self.workspaces {
            let marker = if self.active.map(|(id, _)| id) == Some(ws.id) {
                "\u{25B8} "
            } else {
                "  "
            };
            list = list.push(
                button(text(format!("{marker}{}", ws.name)).size(14))
                    .width(Length::Fill)
                    .on_press(Message::Select(ws.id)),
            );
        }

        let pane = container(self.term_view())
            .padding(8)
            .width(Length::Fill)
            .height(Length::Fill);

        Row::new()
            .push(container(list).height(Length::Fill))
            .push(pane)
            .into()
    }

    fn term_view(&self) -> Element<'_, Message> {
        let snap = self.term.snapshot();
        let mut col = Column::new();
        for cells in &snap {
            col = col.push(row_view(cells));
        }
        container(col)
            .padding(6)
            .style(|_| container::Style {
                background: Some(Background::Color(IColor::from_rgb8(0x10, 0x12, 0x16))),
                ..Default::default()
            })
            .into()
    }

    fn subscription(&self) -> Subscription<Message> {
        let core = Subscription::run(core_event_stream);
        let keys = iced::keyboard::on_key_press(map_key);
        let script = iced::time::every(Duration::from_millis(800)).map(|_| Message::ScriptTick);
        Subscription::batch([core, keys, script])
    }
}

/// Render one terminal row, coalescing consecutive cells with identical style.
fn row_view(cells: &[CellSnap]) -> Element<'static, Message> {
    let mut r = Row::new();
    let mut i = 0;
    while i < cells.len() {
        let key = style_key(&cells[i]);
        let mut run = String::new();
        while i < cells.len() && style_key(&cells[i]) == key {
            run.push_str(&cells[i].text);
            i += 1;
        }
        // NOTE: plain iced `Text` has no underline in 0.13 (only rich-text spans);
        // underline is kept in the style key for run-splitting but not rendered here.
        let (fg, bg, bold, italic, _underline) = key;
        let t = text(run)
            .size(CELL_SIZE)
            .font(cell_font(bold, italic))
            .color(resolve(fg, true));
        r = r.push(
            container(t).style(move |_| container::Style {
                background: Some(Background::Color(resolve(bg, false))),
                ..Default::default()
            }),
        );
    }
    r.into()
}

type StyleKey = (TColor, TColor, bool, bool, bool);

fn style_key(c: &CellSnap) -> StyleKey {
    (c.fg, c.bg, c.bold, c.italic, c.underline)
}

fn cell_font(bold: bool, italic: bool) -> Font {
    let mut f = Font::MONOSPACE;
    if bold {
        f.weight = Weight::Bold;
    }
    if italic {
        f.style = FontStyle::Italic;
    }
    f
}

/// Resolve a terminal color to an iced color; `Default` differs for fg vs bg.
fn resolve(c: TColor, is_fg: bool) -> IColor {
    match c {
        TColor::Default => {
            if is_fg {
                IColor::from_rgb8(0xcc, 0xcc, 0xcc)
            } else {
                IColor::from_rgb8(0x10, 0x12, 0x16)
            }
        }
        TColor::Idx(i) => {
            let (r, g, b) = idx_to_rgb(i);
            IColor::from_rgb8(r, g, b)
        }
        TColor::Rgb(r, g, b) => IColor::from_rgb8(r, g, b),
    }
}

fn core_event_stream() -> impl iced::futures::Stream<Item = Message> {
    use iced::futures::stream;
    let rx = CORE.get().expect("core handle set").evt_tx.subscribe();
    stream::unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(ev) => return Some((Message::Core(ev), rx)),
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    })
}

fn map_key(key: iced::keyboard::Key, mods: iced::keyboard::Modifiers) -> Option<Message> {
    use iced::keyboard::key::Named;
    use iced::keyboard::Key as K;
    let ctrl = mods.control();
    let alt = mods.alt();
    let tk = match key {
        K::Character(s) => Key::Char(s.chars().next()?),
        K::Named(named) => match named {
            Named::Enter => Key::Enter,
            Named::Backspace => Key::Backspace,
            Named::Tab => Key::Tab,
            Named::Escape => Key::Escape,
            Named::ArrowUp => Key::Up,
            Named::ArrowDown => Key::Down,
            Named::ArrowLeft => Key::Left,
            Named::ArrowRight => Key::Right,
            Named::Home => Key::Home,
            Named::End => Key::End,
            Named::PageUp => Key::PageUp,
            Named::PageDown => Key::PageDown,
            Named::Delete => Key::Delete,
            Named::Space => Key::Char(' '),
            _ => return None,
        },
        _ => return None,
    };
    Some(Message::Key(tk, ctrl, alt))
}
