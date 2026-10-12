//! The interactive installer: a text-mode Setup in the tradition of the
//! blue Windows XP setup screens. ENTER continues, ESC goes back, F3
//! quits and F8 confirms erasing the disk.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use anyhow::Result;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Gauge, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{DefaultTerminal, Frame};

use crate::answer::{self, Answer, Filesystem, Finish, UserSpec};
use crate::disk::Disk;
use crate::exec::{Executor, LOG_DIR, Log, Progress};
use crate::facts::Facts;
use crate::ids::{IdSource, Random};
use crate::plan::{self, Plan};

const BG: Color = Color::Blue;
const FG: Color = Color::White;

fn base() -> Style {
    Style::new().fg(FG).bg(BG)
}

fn selected() -> Style {
    Style::new().fg(BG).bg(FG)
}

fn bar() -> Style {
    Style::new().fg(Color::Black).bg(Color::Gray)
}

/// A filterable pick list: type to filter, arrows to choose.
struct Picker {
    items: Vec<String>,
    /// Whether values outside `items` are allowed (no list was available).
    free_text: bool,
    query: String,
    selected: usize,
    value: String,
}

impl Picker {
    fn new(known: Option<&BTreeSet<String>>, value: &str, fallback: &[&str]) -> Picker {
        let (items, free_text) = match known {
            Some(set) if !set.is_empty() => (set.iter().cloned().collect(), false),
            _ => (fallback.iter().map(|s| s.to_string()).collect(), true),
        };
        Picker {
            items,
            free_text,
            query: String::new(),
            selected: 0,
            value: value.to_string(),
        }
    }

    fn matches(&self) -> Vec<&String> {
        let q = self.query.to_lowercase();
        self.items
            .iter()
            .filter(|i| i.to_lowercase().contains(&q))
            .collect()
    }

    fn key(&mut self, key: KeyEvent) {
        let count = self.matches().len();
        match key.code {
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.query.push(c);
                self.selected = 0;
            }
            KeyCode::Backspace => {
                self.query.pop();
                self.selected = 0;
            }
            KeyCode::Up => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down if count > 0 => self.selected = (self.selected + 1).min(count - 1),
            KeyCode::PageUp => self.selected = self.selected.saturating_sub(10),
            KeyCode::PageDown if count > 0 => self.selected = (self.selected + 10).min(count - 1),
            _ => {}
        }
    }

    /// Take the highlighted entry (or the typed text when free text is
    /// allowed) as the value.
    fn commit(&mut self) {
        let chosen = self.matches().get(self.selected).map(|s| s.to_string());
        if let Some(v) = chosen {
            self.value = v;
        } else if self.free_text && !self.query.trim().is_empty() {
            self.value = self.query.trim().to_string();
        }
        self.query.clear();
        self.selected = self
            .matches()
            .iter()
            .position(|i| **i == self.value)
            .unwrap_or(0);
    }
}

struct Field {
    label: &'static str,
    value: String,
    secret: bool,
}

impl Field {
    fn new(label: &'static str, value: &str, secret: bool) -> Field {
        Field {
            label,
            value: value.to_string(),
            secret,
        }
    }

    fn shown(&self) -> String {
        if self.secret {
            "*".repeat(self.value.chars().count())
        } else {
            self.value.clone()
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Welcome,
    Region,
    Disk,
    Account,
    Summary,
    Installing,
    Done,
}

enum Msg {
    Progress(Progress),
    Done(Result<(), String>),
}

struct App {
    screen: Screen,
    facts: Facts,
    target: PathBuf,
    region: [Picker; 3],
    region_focus: usize,
    disks: Vec<Disk>,
    disk_state: ListState,
    filesystem: Filesystem,
    fields: [Field; 5],
    field_focus: usize,
    error: Option<String>,
    plan: Option<Plan>,
    progress: Option<Progress>,
    rx: Option<Receiver<Msg>>,
    outcome: Option<Result<(), String>>,
    quit: Option<Option<Finish>>,
}

const ACCOUNT_FULL_NAME: usize = 0;
const ACCOUNT_USER: usize = 1;
const ACCOUNT_PASSWORD: usize = 2;
const ACCOUNT_CONFIRM: usize = 3;
const ACCOUNT_COMPUTER: usize = 4;

pub fn run(facts: Facts, target: &Path) -> Result<Option<Finish>> {
    let mut app = App::new(facts, target);
    let mut terminal = ratatui::init();
    let result = app.run(&mut terminal);
    ratatui::restore();
    result
}

impl App {
    fn new(facts: Facts, target: &Path) -> App {
        let defaults = answer::RegionSection::default();
        let region = [
            Picker::new(facts.locales.as_ref(), &defaults.locale, &["en_US.UTF-8"]),
            Picker::new(facts.keyboards.as_ref(), &defaults.keyboard, &["us"]),
            Picker::new(facts.timezones.as_ref(), &defaults.timezone, &["UTC"]),
        ];
        let disks: Vec<Disk> = facts.candidate_disks().into_iter().cloned().collect();
        let mut disk_state = ListState::default();
        let preferred = facts
            .auto_disk()
            .and_then(|d| disks.iter().position(|c| c.path == d.path))
            .unwrap_or(0);
        if !disks.is_empty() {
            disk_state.select(Some(preferred));
        }
        let mut region = region;
        for p in &mut region {
            p.selected = p.matches().iter().position(|i| **i == p.value).unwrap_or(0);
        }
        App {
            screen: Screen::Welcome,
            facts,
            target: target.to_path_buf(),
            region,
            region_focus: 0,
            disks,
            disk_state,
            filesystem: Filesystem::Ext4,
            fields: [
                Field::new("Full name", "", false),
                Field::new("User name", "", false),
                Field::new("Password", "", true),
                Field::new("Confirm password", "", true),
                Field::new("Computer name", &Random.computer_name(), false),
            ],
            field_focus: 0,
            error: None,
            plan: None,
            progress: None,
            rx: None,
            outcome: None,
            quit: None,
        }
    }

    fn run(&mut self, terminal: &mut DefaultTerminal) -> Result<Option<Finish>> {
        loop {
            terminal.draw(|f| self.draw(f))?;
            if let Some(result) = self.quit.take() {
                return Ok(result);
            }
            self.poll_install();
            if event::poll(Duration::from_millis(100))?
                && let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
            {
                self.key(key);
            }
        }
    }

    fn poll_install(&mut self) {
        let Some(rx) = &self.rx else { return };
        while let Ok(msg) = rx.try_recv() {
            match msg {
                Msg::Progress(p) => self.progress = Some(p),
                Msg::Done(result) => {
                    self.outcome = Some(result);
                    self.screen = Screen::Done;
                }
            }
        }
    }

    // ----- input ---------------------------------------------------------

    fn key(&mut self, key: KeyEvent) {
        let installing = self.screen == Screen::Installing;
        if !installing
            && (key.code == KeyCode::F(3)
                || (key.code == KeyCode::Char('c')
                    && key.modifiers.contains(KeyModifiers::CONTROL)))
        {
            self.quit = Some(None);
            return;
        }
        match self.screen {
            Screen::Welcome => {
                if key.code == KeyCode::Enter {
                    self.screen = Screen::Region;
                }
            }
            Screen::Region => self.key_region(key),
            Screen::Disk => self.key_disk(key),
            Screen::Account => self.key_account(key),
            Screen::Summary => match key.code {
                KeyCode::F(8) => self.start_install(),
                KeyCode::Esc => self.screen = Screen::Account,
                _ => {}
            },
            Screen::Installing => {}
            Screen::Done => match (&self.outcome, key.code) {
                (Some(Ok(())), KeyCode::Enter) => self.quit = Some(Some(Finish::Reboot)),
                (Some(Ok(())), KeyCode::Char('q') | KeyCode::Esc) => self.quit = Some(None),
                (Some(Err(_)), KeyCode::Enter | KeyCode::Esc) => self.quit = Some(None),
                _ => {}
            },
        }
    }

    fn key_region(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.region[self.region_focus].query.clear();
                self.screen = Screen::Welcome;
            }
            KeyCode::Tab | KeyCode::Enter => {
                self.region[self.region_focus].commit();
                if self.region_focus < 2 {
                    self.region_focus += 1;
                } else if key.code == KeyCode::Enter {
                    self.region_focus = 0;
                    self.screen = Screen::Disk;
                } else {
                    self.region_focus = 0;
                }
            }
            KeyCode::BackTab => {
                self.region[self.region_focus].commit();
                self.region_focus = (self.region_focus + 2) % 3;
            }
            _ => self.region[self.region_focus].key(key),
        }
    }

    fn key_disk(&mut self, key: KeyEvent) {
        let n = self.disks.len();
        match key.code {
            KeyCode::Esc => self.screen = Screen::Region,
            KeyCode::Up if n > 0 => {
                let i = self.disk_state.selected().unwrap_or(0);
                self.disk_state.select(Some(i.saturating_sub(1)));
            }
            KeyCode::Down if n > 0 => {
                let i = self.disk_state.selected().unwrap_or(0);
                self.disk_state.select(Some((i + 1).min(n - 1)));
            }
            KeyCode::Char('f' | 'F') => {
                self.filesystem = match self.filesystem {
                    Filesystem::Ext4 => Filesystem::Btrfs,
                    Filesystem::Btrfs => Filesystem::Xfs,
                    Filesystem::Xfs => Filesystem::Ext4,
                }
            }
            KeyCode::Enter if n > 0 => {
                self.error = None;
                self.screen = Screen::Account;
            }
            _ => {}
        }
    }

    fn key_account(&mut self, key: KeyEvent) {
        let last = self.fields.len() - 1;
        match key.code {
            KeyCode::Esc => {
                self.error = None;
                self.screen = Screen::Disk;
            }
            KeyCode::Up | KeyCode::BackTab => self.field_focus = self.field_focus.saturating_sub(1),
            KeyCode::Down | KeyCode::Tab => self.leave_field(),
            KeyCode::Enter if self.field_focus < last => self.leave_field(),
            KeyCode::Enter => self.prepare_summary(),
            KeyCode::Backspace => {
                self.fields[self.field_focus].value.pop();
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.fields[self.field_focus].value.push(c);
            }
            _ => {}
        }
    }

    /// Move to the next field, suggesting a user name from the full name
    /// like Windows' out-of-box experience does.
    fn leave_field(&mut self) {
        if self.field_focus == ACCOUNT_FULL_NAME && self.fields[ACCOUNT_USER].value.is_empty() {
            let suggestion: String = self.fields[ACCOUNT_FULL_NAME]
                .value
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_lowercase()
                .chars()
                .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
                .collect();
            if answer::valid_username(&suggestion) {
                self.fields[ACCOUNT_USER].value = suggestion;
            }
        }
        self.field_focus = (self.field_focus + 1).min(self.fields.len() - 1);
    }

    fn answer(&self) -> Answer {
        let disk = self
            .disk_state
            .selected()
            .and_then(|i| self.disks.get(i))
            .map(|d| d.path.clone())
            .unwrap_or_default();
        let mut a = Answer::default();
        a.disk.device = disk;
        a.disk.wipe = true;
        a.disk.filesystem = self.filesystem;
        a.region.locale = self.region[0].value.clone();
        a.region.keyboard = self.region[1].value.clone();
        a.region.timezone = self.region[2].value.clone();
        a.computer.name = self.fields[ACCOUNT_COMPUTER].value.trim().to_string();
        a.users = vec![UserSpec {
            name: self.fields[ACCOUNT_USER].value.trim().to_string(),
            full_name: self.fields[ACCOUNT_FULL_NAME].value.trim().to_string(),
            password: Some(self.fields[ACCOUNT_PASSWORD].value.clone()),
            admin: true,
            ..Default::default()
        }];
        a
    }

    fn prepare_summary(&mut self) {
        if self.fields[ACCOUNT_PASSWORD].value != self.fields[ACCOUNT_CONFIRM].value {
            self.error = Some("The passwords do not match.".into());
            self.field_focus = ACCOUNT_CONFIRM;
            return;
        }
        let answer = self.answer();
        let problems = answer.validate();
        if let Some(first) = problems.first() {
            self.error = Some(first.clone());
            return;
        }
        match plan::build(&answer, &self.facts, &mut Random, &self.target) {
            Ok(plan) => {
                self.plan = Some(plan);
                self.error = None;
                self.screen = Screen::Summary;
            }
            Err(e) => self.error = Some(format!("{e:#}")),
        }
    }

    fn start_install(&mut self) {
        let Some(plan) = self.plan.clone() else {
            return;
        };
        let (tx, rx) = mpsc::channel();
        let target = self.target.clone();
        let init = self.facts.init;
        thread::spawn(move || {
            let result = Log::open(Path::new(LOG_DIR)).and_then(|mut log| {
                Executor::new(&target, init, &mut log).run(&plan, &mut |p| {
                    let _ = tx.send(Msg::Progress(p));
                })
            });
            let _ = tx.send(Msg::Done(result.map_err(|e| format!("{e:#}"))));
        });
        self.rx = Some(rx);
        self.screen = Screen::Installing;
    }

    // ----- drawing -------------------------------------------------------

    fn draw(&mut self, f: &mut Frame) {
        let [header, body, footer] = Layout::vertical([
            Constraint::Length(2),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .areas(f.area());
        f.render_widget(Block::new().style(base()), f.area());
        f.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    " Wincraft Setup",
                    base().add_modifier(Modifier::BOLD),
                )),
                Line::from("═".repeat(header.width as usize)),
            ])
            .style(base()),
            header,
        );
        let body = body.inner(ratatui::layout::Margin::new(3, 1));
        let hints = match self.screen {
            Screen::Welcome => "ENTER=Continue  F3=Quit",
            Screen::Region => {
                "Type to search  ↑↓=Choose  TAB=Next field  ENTER=Continue  ESC=Back  F3=Quit"
            }
            Screen::Disk => "↑↓=Choose disk  F=File system  ENTER=Continue  ESC=Back  F3=Quit",
            Screen::Account => "TAB/↓=Next field  ENTER=Continue  ESC=Back  F3=Quit",
            Screen::Summary => "F8=Erase disk and install  ESC=Back  F3=Quit",
            Screen::Installing => "Please wait...",
            Screen::Done => match &self.outcome {
                Some(Ok(())) => "ENTER=Restart computer  Q=Return to the live desktop",
                _ => "ENTER=Quit",
            },
        };
        f.render_widget(Paragraph::new(format!("  {hints}")).style(bar()), footer);

        match self.screen {
            Screen::Welcome => self.draw_welcome(f, body),
            Screen::Region => self.draw_region(f, body),
            Screen::Disk => self.draw_disk(f, body),
            Screen::Account => self.draw_account(f, body),
            Screen::Summary => self.draw_summary(f, body),
            Screen::Installing => self.draw_installing(f, body),
            Screen::Done => self.draw_done(f, body),
        }
    }

    fn draw_welcome(&self, f: &mut Frame, area: Rect) {
        let text = Text::from(vec![
            Line::from(Span::styled(
                "Welcome to Setup.",
                base().add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(
                "This portion of the Setup program prepares Wincraft to run on your computer.",
            ),
            Line::from(""),
            Line::from("   •  To set up Wincraft now, press ENTER."),
            Line::from(""),
            Line::from("   •  To quit Setup without installing Wincraft, press F3."),
            Line::from(""),
            Line::from(""),
            Line::from(
                "Wincraft is a lightweight gaming system based on Artix Linux. It looks and",
            ),
            Line::from(
                "works like Windows and runs Windows games and programs through Wine, DXVK,",
            ),
            Line::from("VKD3D-Proton and Steam's Proton."),
            Line::from(""),
            Line::from(format!(
                "Detected: {} firmware, {} init system, {} disk(s).",
                self.facts.firmware,
                self.facts.init,
                self.disks.len()
            )),
        ]);
        f.render_widget(
            Paragraph::new(text)
                .style(base())
                .wrap(Wrap { trim: false }),
            area,
        );
    }

    fn draw_region(&mut self, f: &mut Frame, area: Rect) {
        let [left, right] =
            Layout::horizontal([Constraint::Length(40), Constraint::Min(20)]).areas(area);
        let labels = ["Language", "Keyboard layout", "Time zone"];
        let mut lines = vec![Line::from("Choose your region settings."), Line::from("")];
        for (i, label) in labels.iter().enumerate() {
            let style = if i == self.region_focus {
                selected()
            } else {
                base()
            };
            lines.push(Line::from(vec![
                Span::styled(format!("{label:<17}"), base()),
                Span::styled(format!(" {} ", self.region[i].value), style),
            ]));
            lines.push(Line::from(""));
        }
        f.render_widget(Paragraph::new(lines).style(base()), left);

        let picker = &self.region[self.region_focus];
        let matches = picker.matches();
        let items: Vec<ListItem> = matches.iter().map(|m| ListItem::new(m.as_str())).collect();
        let title = format!(
            " {} - search: {}_ ",
            labels[self.region_focus], picker.query
        );
        let mut state = ListState::default();
        if !matches.is_empty() {
            state.select(Some(picker.selected.min(matches.len() - 1)));
        }
        let list = List::new(items)
            .block(
                Block::new()
                    .borders(Borders::ALL)
                    .title(title)
                    .style(base()),
            )
            .highlight_style(selected())
            .style(base());
        f.render_stateful_widget(list, right, &mut state);
    }

    fn draw_disk(&mut self, f: &mut Frame, area: Rect) {
        let [intro, list_area, info] = Layout::vertical([
            Constraint::Length(4),
            Constraint::Min(3),
            Constraint::Length(4),
        ])
        .areas(area);
        f.render_widget(
            Paragraph::new(vec![
                Line::from("Where do you want to install Wincraft?"),
                Line::from(""),
                Line::from(
                    "The selected disk will be erased completely and partitioned automatically.",
                ),
            ])
            .style(base()),
            intro,
        );
        if self.disks.is_empty() {
            f.render_widget(
                Paragraph::new("No disk was found that Wincraft can be installed to.")
                    .style(base()),
                list_area,
            );
        } else {
            let items: Vec<ListItem> = self
                .disks
                .iter()
                .map(|d| ListItem::new(d.label()))
                .collect();
            let list = List::new(items)
                .block(
                    Block::new()
                        .borders(Borders::ALL)
                        .title(" Disks ")
                        .style(base()),
                )
                .highlight_style(selected())
                .style(base());
            f.render_stateful_widget(list, list_area, &mut self.disk_state);
        }
        f.render_widget(
            Paragraph::new(vec![
                Line::from(format!("File system: {}   (press F to change)", self.filesystem)),
                Line::from("Partitions: EFI system (512 MiB), BIOS boot, Wincraft. Swap: compressed in RAM (zram)."),
            ])
            .style(base()),
            info,
        );
    }

    fn draw_account(&self, f: &mut Frame, area: Rect) {
        let mut lines = vec![
            Line::from("Who's going to use this PC?"),
            Line::from(
                "This account is an administrator: it can install software and change settings.",
            ),
            Line::from(""),
        ];
        for (i, field) in self.fields.iter().enumerate() {
            let style = if i == self.field_focus {
                selected()
            } else {
                base()
            };
            let cursor = if i == self.field_focus { "_" } else { " " };
            lines.push(Line::from(vec![
                Span::styled(format!("{:<18}", field.label), base()),
                Span::styled(format!(" {:<32}{cursor}", field.shown()), style),
            ]));
            lines.push(Line::from(""));
        }
        if let Some(err) = &self.error {
            lines.push(Line::from(Span::styled(
                format!("! {err}"),
                Style::new()
                    .fg(Color::Yellow)
                    .bg(BG)
                    .add_modifier(Modifier::BOLD),
            )));
        }
        f.render_widget(
            Paragraph::new(lines)
                .style(base())
                .wrap(Wrap { trim: false }),
            area,
        );
    }

    fn draw_summary(&self, f: &mut Frame, area: Rect) {
        let Some(plan) = &self.plan else { return };
        let mut lines: Vec<Line> = plan
            .to_string()
            .lines()
            .take_while(|l| !l.starts_with("=="))
            .map(|l| Line::from(l.to_string()))
            .collect();
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!(
                "WARNING: all data on {} will be permanently erased.",
                plan.summary.disk.path
            ),
            Style::new()
                .fg(Color::Yellow)
                .bg(BG)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(
            "To erase the disk and install Wincraft, press F8.",
        ));
        lines.push(Line::from("To change your choices, press ESC."));
        f.render_widget(
            Paragraph::new(lines)
                .style(base())
                .wrap(Wrap { trim: false }),
            area,
        );
    }

    fn draw_installing(&self, f: &mut Frame, area: Rect) {
        let [text, gauge] =
            Layout::vertical([Constraint::Length(8), Constraint::Length(3)]).areas(area);
        let (percent, pass, step) = match &self.progress {
            Some(p) => (p.percent(), p.pass.title(), p.description.clone()),
            None => (0, "Starting", String::new()),
        };
        f.render_widget(
            Paragraph::new(vec![
                Line::from(
                    "Please wait while Setup installs Wincraft. This might take several minutes.",
                ),
                Line::from(""),
                Line::from(Span::styled(pass, base().add_modifier(Modifier::BOLD))),
                Line::from(format!("  {step}...")),
                Line::from(""),
                Line::from(format!("Progress is logged to {LOG_DIR}/setupact.log")),
            ])
            .style(base()),
            text,
        );
        f.render_widget(
            Gauge::default()
                .block(Block::new().borders(Borders::ALL).style(base()))
                .gauge_style(Style::new().fg(Color::Yellow).bg(Color::Black))
                .percent(percent),
            gauge,
        );
    }

    fn draw_done(&self, f: &mut Frame, area: Rect) {
        let lines = match &self.outcome {
            Some(Ok(())) => vec![
                Line::from(Span::styled(
                    "Wincraft has been installed.",
                    base().add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(
                    "Remove the installation media, then press ENTER to restart your computer.",
                ),
                Line::from(""),
                Line::from("After the restart, sign in with the account you created."),
            ],
            Some(Err(e)) => {
                let mut v = vec![
                    Line::from(Span::styled(
                        "Setup could not install Wincraft.",
                        Style::new()
                            .fg(Color::Yellow)
                            .bg(BG)
                            .add_modifier(Modifier::BOLD),
                    )),
                    Line::from(""),
                ];
                v.extend(e.lines().map(|l| Line::from(l.to_string())));
                v.push(Line::from(""));
                v.push(Line::from(format!(
                    "Details: {LOG_DIR}/setuperr.log and setupact.log"
                )));
                v
            }
            None => vec![],
        };
        f.render_widget(
            Paragraph::new(lines)
                .style(base())
                .wrap(Wrap { trim: false }),
            area,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::answer::GIB;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use wincraft_core::init::InitSystem;

    fn press(app: &mut App, code: KeyCode) {
        app.key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn type_text(app: &mut App, text: &str) {
        for c in text.chars() {
            press(app, KeyCode::Char(c));
        }
    }

    fn facts() -> Facts {
        Facts {
            firmware: crate::facts::Firmware::Uefi,
            init: InitSystem::OpenRc,
            disks: vec![Disk {
                path: "/dev/vda".into(),
                size: 64 * GIB,
                model: "Virtio".into(),
                removable: false,
                read_only: false,
                transport: String::new(),
            }],
            boot_disk: None,
            image: Some("/run/artix/bootmnt/LiveOS/rootfs.img".into()),
            oem_dir: None,
            offline_packages: vec![],
            memory: 8 * GIB,
            gpus: vec![],
            timezones: Some(
                ["UTC", "Europe/Paris", "Europe/Berlin"]
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
            ),
            locales: Some(
                ["en_US.UTF-8", "fr_FR.UTF-8"]
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
            ),
            keyboards: Some(["us", "fr", "de"].iter().map(|s| s.to_string()).collect()),
        }
    }

    fn render(app: &mut App) -> String {
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal.draw(|f| app.draw(f)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn walk_through_to_the_summary() {
        let mut app = App::new(facts(), Path::new("/mnt/wincraft"));
        assert!(render(&mut app).contains("Welcome to Setup."));
        press(&mut app, KeyCode::Enter);

        // Region: pick French, French keyboard, Paris by typing.
        type_text(&mut app, "fr_");
        press(&mut app, KeyCode::Enter);
        type_text(&mut app, "fr");
        press(&mut app, KeyCode::Enter);
        type_text(&mut app, "paris");
        assert!(render(&mut app).contains("Europe/Paris"));
        press(&mut app, KeyCode::Enter);
        assert!(app.screen == Screen::Disk);
        assert_eq!(app.region[0].value, "fr_FR.UTF-8");
        assert_eq!(app.region[1].value, "fr");
        assert_eq!(app.region[2].value, "Europe/Paris");

        press(&mut app, KeyCode::Char('f'));
        assert_eq!(app.filesystem, Filesystem::Btrfs);
        assert!(render(&mut app).contains("/dev/vda"));
        press(&mut app, KeyCode::Enter);

        // Account: the user name is suggested from the full name.
        type_text(&mut app, "Ada Lovelace");
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.fields[ACCOUNT_USER].value, "ada");
        press(&mut app, KeyCode::Tab);
        type_text(&mut app, "pw1");
        press(&mut app, KeyCode::Tab);
        type_text(&mut app, "pw2");
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Enter);
        assert!(app.error.as_deref().unwrap().contains("do not match"));
        assert!(!render(&mut app).contains("pw1"));

        app.fields[ACCOUNT_CONFIRM].value = "pw1".into();
        app.field_focus = ACCOUNT_COMPUTER;
        press(&mut app, KeyCode::Enter);
        assert!(app.screen == Screen::Summary, "error: {:?}", app.error);
        let screen = render(&mut app);
        assert!(screen.contains("will be permanently erased"));
        assert!(screen.contains("F8=Erase disk and install"));
        let plan = app.plan.as_ref().unwrap();
        assert_eq!(plan.summary.users, vec!["ada"]);
        assert_eq!(plan.summary.layout.filesystem, Filesystem::Btrfs);

        press(&mut app, KeyCode::Esc);
        assert!(app.screen == Screen::Account);
        press(&mut app, KeyCode::F(3));
        assert!(matches!(app.quit, Some(None)));
    }

    /// `cargo test -p wincraft-setup print_screens -- --ignored --nocapture`
    /// prints every screen, e.g. for documentation.
    #[test]
    #[ignore]
    fn print_screens() {
        let mut app = App::new(facts(), Path::new("/mnt/wincraft"));
        app.fields[ACCOUNT_COMPUTER].value = "DESKTOP-4K2Q9ZT".into();
        println!("{}", render(&mut app));
        press(&mut app, KeyCode::Enter);
        type_text(&mut app, "en");
        println!("{}", render(&mut app));
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter);
        println!("{}", render(&mut app));
        press(&mut app, KeyCode::Enter);
        type_text(&mut app, "Ada Lovelace");
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Tab);
        type_text(&mut app, "secret");
        press(&mut app, KeyCode::Tab);
        type_text(&mut app, "secret");
        println!("{}", render(&mut app));
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Enter);
        println!("{}", render(&mut app));
        app.screen = Screen::Installing;
        app.progress = Some(Progress {
            pass: crate::plan::Pass::WindowsPe,
            done: 6,
            total: 25,
            description: "Copying Wincraft files (this takes a few minutes)".into(),
        });
        println!("{}", render(&mut app));
    }

    #[test]
    fn picker_accepts_free_text_without_a_list() {
        let mut p = Picker::new(None, "UTC", &["UTC"]);
        for c in "Asia/Tokyo".chars() {
            p.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        p.commit();
        assert_eq!(p.value, "Asia/Tokyo");
    }

    #[test]
    fn picker_keeps_value_when_nothing_typed() {
        let known: BTreeSet<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
        let mut p = Picker::new(Some(&known), "b", &[]);
        p.selected = 1;
        p.commit();
        assert_eq!(p.value, "b");
        p.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        p.commit();
        assert_eq!(p.value, "c");
    }
}
