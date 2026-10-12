//! Wincraft Setup, graphical: a GTK 4 wizard for the live session.
//!
//! It runs as the live user: it inspects the computer, collects the
//! choices, shows exactly what Setup will do (the summary of the same plan
//! `wincraft-setup plan` prints), writes the answer file and runs
//! `sudo -n wincraft-setup install --unattend <file> --progress json`,
//! following its progress. The installation itself is the tested
//! unattended code path; nothing here touches a disk.
//!
//! `wincraft-installer --demo` uses an invented computer and only simulates
//! the installation, for screenshots and development; `--demo-page=NAME`
//! also fills in sample answers and opens that page (welcome, region, disk,
//! account, summary, installing, finished).

mod model;

use std::cell::{Cell, RefCell};
use std::ffi::{OsStr, OsString};
use std::fs::{self, OpenOptions, Permissions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use anyhow::{Context, Result};
use gtk::prelude::*;
use gtk::{gio, glib};

use wincraft_setup::answer::Answer;
use wincraft_setup::disk::{MIN_DISK_SIZE, human_size};
use wincraft_setup::exec::{Event, LOG_DIR};
use wincraft_setup::facts::{Facts, has_default_route};
use wincraft_setup::ids::{IdSource, Random};
use wincraft_setup::{is_root, plan};

use model::{Choices, FILESYSTEMS, SWAP};

const APP_ID: &str = "org.wincraft.Installer";
const CSS: &str = include_str!("style.css");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Page {
    Welcome,
    Region,
    Disk,
    Account,
    Summary,
    Installing,
    Finished,
}

impl Page {
    const ALL: [Page; 7] = [
        Page::Welcome,
        Page::Region,
        Page::Disk,
        Page::Account,
        Page::Summary,
        Page::Installing,
        Page::Finished,
    ];

    fn name(self) -> &'static str {
        match self {
            Page::Welcome => "welcome",
            Page::Region => "region",
            Page::Disk => "disk",
            Page::Account => "account",
            Page::Summary => "summary",
            Page::Installing => "installing",
            Page::Finished => "finished",
        }
    }

    /// The step's name in the sidebar.
    fn step(self) -> &'static str {
        match self {
            Page::Welcome => "Welcome",
            Page::Region => "Region and language",
            Page::Disk => "Where to install",
            Page::Account => "Your account",
            Page::Summary => "Ready to install",
            Page::Installing => "Installing Wincraft",
            Page::Finished => "Finished",
        }
    }

    fn index(self) -> usize {
        Page::ALL.iter().position(|p| *p == self).unwrap_or(0)
    }

    fn from_name(name: &str) -> Option<Page> {
        Page::ALL.into_iter().find(|p| p.name() == name)
    }
}

fn main() -> glib::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!(
            "Usage: wincraft-installer [--demo | --demo-page=PAGE]\n\n\
             Graphical Wincraft Setup for the live session. It runs\n\
             `sudo -n wincraft-setup install --unattend FILE --progress json`.\n\
             --demo            use an invented computer and only simulate the installation\n\
             --demo-page=PAGE  the same, with sample answers, opened at PAGE (welcome,\n\
             \x20                 region, disk, account, summary, installing, finished)"
        );
        return glib::ExitCode::SUCCESS;
    }
    let demo_page = args
        .iter()
        .find_map(|a| a.strip_prefix("--demo-page="))
        .and_then(Page::from_name);
    let demo = demo_page.is_some() || args.iter().any(|a| a == "--demo");
    let app = gtk::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.connect_activate(move |app| activate(app, demo, demo_page));
    app.run_with_args(&["wincraft-installer"])
}

fn activate(app: &gtk::Application, demo: bool, demo_page: Option<Page>) {
    if let Some(display) = gtk::gdk::Display::default() {
        let css = gtk::CssProvider::new();
        css.load_from_data(CSS);
        gtk::style_context_add_provider_for_display(
            &display,
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title(if demo {
            "Wincraft Setup (demo)"
        } else {
            "Wincraft Setup"
        })
        .default_width(960)
        .default_height(660)
        .icon_name("system-software-install")
        .build();
    let facts = if demo {
        Ok(model::demo_facts())
    } else {
        Facts::probe()
    };
    match facts {
        Ok(facts) => {
            let ui = Ui::build(&window, facts, demo);
            if let Some(page) = demo_page {
                ui.open_with_sample_answers(page);
            }
        }
        Err(e) => {
            let page = page_box(
                "Setup cannot start",
                &format!("Setup could not inspect this computer: {e:#}"),
            );
            window.set_child(Some(&page));
        }
    }
    window.present();
}

/// A page: a title, an introduction and room for its contents.
fn page_box(title: &str, intro: &str) -> gtk::Box {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 14);
    page.add_css_class("page");
    if !title.is_empty() {
        let heading = gtk::Label::new(Some(title));
        heading.add_css_class("page-title");
        heading.set_xalign(0.0);
        page.append(&heading);
    }
    if !intro.is_empty() {
        page.append(&text(intro));
    }
    page
}

/// A left-aligned, wrapping label.
fn text(s: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(s));
    label.set_wrap(true);
    label.set_xalign(0.0);
    label
}

/// A form: labels on the left, inputs on the right.
fn form(rows: &[(&str, &gtk::Widget)]) -> gtk::Grid {
    let grid = gtk::Grid::new();
    grid.set_row_spacing(10);
    grid.set_column_spacing(16);
    grid.set_margin_top(8);
    for (i, (label, widget)) in rows.iter().enumerate() {
        let l = gtk::Label::new(Some(label));
        l.set_xalign(0.0);
        l.add_css_class("field-label");
        grid.attach(&l, 0, i as i32, 1, 1);
        widget.set_hexpand(true);
        grid.attach(*widget, 1, i as i32, 1, 1);
    }
    grid
}

/// A drop-down list that can be searched by typing.
fn dropdown(items: &[String], selected: &str) -> gtk::DropDown {
    let refs: Vec<&str> = items.iter().map(String::as_str).collect();
    let model = gtk::StringList::new(&refs);
    let expression = gtk::PropertyExpression::new(
        gtk::StringObject::static_type(),
        None::<gtk::Expression>,
        "string",
    );
    let dd = gtk::DropDown::new(Some(model), Some(expression));
    dd.set_enable_search(true);
    if let Some(i) = items.iter().position(|s| s == selected) {
        dd.set_selected(i as u32);
    }
    dd
}

fn dropdown_value(dd: &gtk::DropDown) -> Option<String> {
    dd.selected_item()
        .and_downcast::<gtk::StringObject>()
        .map(|s| s.string().to_string())
}

fn labels_dropdown(labels: &[&str]) -> gtk::DropDown {
    gtk::DropDown::from_strings(labels)
}

fn monospace_view(buffer: &gtk::TextBuffer, height: i32) -> gtk::ScrolledWindow {
    let view = gtk::TextView::with_buffer(buffer);
    view.set_editable(false);
    view.set_cursor_visible(false);
    view.set_monospace(true);
    view.set_wrap_mode(gtk::WrapMode::WordChar);
    view.set_left_margin(10);
    view.set_top_margin(8);
    let scroll = gtk::ScrolledWindow::builder()
        .child(&view)
        .min_content_height(height)
        .vexpand(true)
        .build();
    scroll.add_css_class("output");
    scroll
}

struct Ui {
    facts: Facts,
    demo: bool,
    window: gtk::ApplicationWindow,
    stack: gtk::Stack,
    steps: Vec<gtk::Label>,
    back: gtk::Button,
    next: gtk::Button,
    quit: gtk::Button,
    message: gtk::Label,
    page: Cell<Page>,
    // Region
    locale: gtk::DropDown,
    keyboard: gtk::DropDown,
    timezone: gtk::DropDown,
    // Disk
    disks: Vec<(String, gtk::CheckButton)>,
    filesystem: gtk::DropDown,
    swap: gtk::DropDown,
    // Account
    full_name: gtk::Entry,
    user: gtk::Entry,
    password: gtk::PasswordEntry,
    confirm: gtk::PasswordEntry,
    computer: gtk::Entry,
    autologin: gtk::CheckButton,
    suggested_user: RefCell<String>,
    // Summary
    summary: gtk::TextBuffer,
    erase_note: gtk::Label,
    network_note: gtk::Label,
    consent: gtk::CheckButton,
    // Installing and finished
    pass: gtk::Label,
    progress: gtk::ProgressBar,
    current: gtk::Label,
    details: gtk::TextBuffer,
    finish_title: gtk::Label,
    finish_text: gtk::Label,
    choices: RefCell<Option<Choices>>,
    installing: Cell<bool>,
    succeeded: Cell<bool>,
}

impl Ui {
    fn build(window: &gtk::ApplicationWindow, facts: Facts, demo: bool) -> Rc<Ui> {
        let defaults = Choices::default();
        let stack = gtk::Stack::new();
        stack.set_transition_type(gtk::StackTransitionType::SlideLeftRight);
        stack.set_hexpand(true);
        stack.set_vexpand(true);

        // --- Welcome ---------------------------------------------------------
        let welcome = page_box(
            "Welcome to Wincraft Setup",
            "Setup installs Wincraft on this computer. It asks a few questions, shows you \
             everything it will do, and only then starts. Installing takes 10 to 30 minutes.",
        );
        for line in [
            "1.  Choose your language, keyboard and time zone.",
            "2.  Choose the disk for Wincraft. Everything on it is erased.",
            "3.  Create your account.",
        ] {
            let l = text(line);
            l.add_css_class("bullet");
            welcome.append(&l);
        }
        if !facts.online_packages.is_empty() {
            let lite = text(&format!(
                "This is Wincraft Lite: Setup downloads Wincraft's apps, drivers and games \
                 ({} packages) while it installs, so this computer needs an internet connection.",
                facts.online_packages.len()
            ));
            lite.add_css_class("note");
            welcome.append(&lite);
        }
        if demo {
            let note = text("Demo mode: this computer is invented and nothing is installed.");
            note.add_css_class("warning");
            welcome.append(&note);
        }

        // --- Region ----------------------------------------------------------
        let region = page_box(
            "Region and language",
            "Wincraft uses these for menus, the keyboard and the clock. Type in a list to search it.",
        );
        let locale = dropdown(
            &model::locale_options(facts.locales.as_ref(), &defaults.locale),
            &defaults.locale,
        );
        let keyboard = dropdown(
            &model::options(facts.keyboards.as_ref(), &defaults.keyboard),
            &defaults.keyboard,
        );
        let timezone = dropdown(
            &model::options(facts.timezones.as_ref(), &defaults.timezone),
            &defaults.timezone,
        );
        region.append(&form(&[
            ("Language", locale.upcast_ref()),
            ("Keyboard layout", keyboard.upcast_ref()),
            ("Time zone", timezone.upcast_ref()),
        ]));

        // --- Disk ------------------------------------------------------------
        let disk_page = page_box(
            "Where do you want to install Wincraft?",
            "Setup uses the whole disk you choose.",
        );
        let disk_list = gtk::Box::new(gtk::Orientation::Vertical, 6);
        disk_list.add_css_class("disk-list");
        let mut disks = Vec::new();
        let preferred = facts.auto_disk().map(|d| d.path.clone());
        let mut group: Option<gtk::CheckButton> = None;
        for d in facts.candidate_disks() {
            let small = d.size < MIN_DISK_SIZE;
            let mut label = d.label();
            if small {
                label.push_str(&format!(
                    "  (too small: Wincraft needs {})",
                    human_size(MIN_DISK_SIZE)
                ));
            }
            let radio = gtk::CheckButton::with_label(&label);
            radio.set_sensitive(!small);
            if let Some(first) = &group {
                radio.set_group(Some(first));
            } else {
                group = Some(radio.clone());
            }
            radio.set_active(preferred.as_deref() == Some(d.path.as_str()));
            disk_list.append(&radio);
            disks.push((d.path.clone(), radio));
        }
        if disks.is_empty() {
            let none = text("No disk was found that Wincraft can be installed to.");
            none.add_css_class("error");
            disk_list.append(&none);
        }
        disk_page.append(&disk_list);
        let fs_labels: Vec<&str> = FILESYSTEMS.iter().map(|(_, l)| *l).collect();
        let swap_labels: Vec<&str> = SWAP.iter().map(|(_, l)| *l).collect();
        let filesystem = labels_dropdown(&fs_labels);
        let swap = labels_dropdown(&swap_labels);
        disk_page.append(&form(&[
            ("File system", filesystem.upcast_ref()),
            ("Swap", swap.upcast_ref()),
        ]));
        let erase = text(
            "Everything on the disk you choose will be erased. Copy your files to another drive first.",
        );
        erase.add_css_class("warning");
        disk_page.append(&erase);

        // --- Account ---------------------------------------------------------
        let account = page_box(
            "Your account",
            "This account is an administrator: it can install programs and change settings.",
        );
        let full_name = gtk::Entry::builder()
            .placeholder_text("Ada Lovelace")
            .build();
        let user = gtk::Entry::builder().placeholder_text("ada").build();
        let password = gtk::PasswordEntry::builder().show_peek_icon(true).build();
        let confirm = gtk::PasswordEntry::builder().show_peek_icon(true).build();
        let computer = gtk::Entry::builder().text(Random.computer_name()).build();
        let autologin =
            gtk::CheckButton::with_label("Sign in automatically when the computer starts");
        account.append(&form(&[
            ("Your name", full_name.upcast_ref()),
            ("User name", user.upcast_ref()),
            ("Password", password.upcast_ref()),
            ("Confirm password", confirm.upcast_ref()),
            ("Computer name", computer.upcast_ref()),
        ]));
        account.append(&autologin);

        // --- Summary ---------------------------------------------------------
        let summary_page = page_box("Ready to install", "Setup will make these changes:");
        let summary = gtk::TextBuffer::new(None);
        summary_page.append(&monospace_view(&summary, 220));
        let network_note = text("");
        network_note.add_css_class("warning");
        network_note.set_visible(false);
        summary_page.append(&network_note);
        let erase_note = text("");
        erase_note.add_css_class("danger");
        summary_page.append(&erase_note);
        let consent =
            gtk::CheckButton::with_label("I understand: erase this disk and install Wincraft");
        summary_page.append(&consent);

        // --- Installing ------------------------------------------------------
        let installing = page_box(
            "Installing Wincraft",
            "This takes a while. Keep the computer on; you can use the live session meanwhile.",
        );
        let pass = text("Starting Setup");
        pass.add_css_class("pass");
        let progress = gtk::ProgressBar::new();
        progress.set_show_text(true);
        let current = text("");
        current.add_css_class("dim");
        installing.append(&pass);
        installing.append(&progress);
        installing.append(&current);
        let details = gtk::TextBuffer::new(None);
        let expander = gtk::Expander::new(Some("Details"));
        expander.set_child(Some(&monospace_view(&details, 200)));
        installing.append(&expander);

        // --- Finished --------------------------------------------------------
        let finished = page_box("", "");
        let finish_title = text("");
        finish_title.add_css_class("page-title");
        let finish_text = text("");
        finished.append(&finish_title);
        finished.append(&finish_text);
        let finish_details = gtk::Expander::new(Some("Details"));
        finish_details.set_child(Some(&monospace_view(&details, 200)));
        finished.append(&finish_details);

        for (page, widget) in [
            (Page::Welcome, &welcome),
            (Page::Region, &region),
            (Page::Disk, &disk_page),
            (Page::Account, &account),
            (Page::Summary, &summary_page),
            (Page::Installing, &installing),
            (Page::Finished, &finished),
        ] {
            stack.add_named(widget, Some(page.name()));
        }

        // --- Sidebar and buttons -----------------------------------------------
        let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 4);
        sidebar.add_css_class("sidebar");
        let brand = gtk::Label::new(Some("Wincraft Setup"));
        brand.add_css_class("brand");
        brand.set_xalign(0.0);
        sidebar.append(&brand);
        let steps: Vec<gtk::Label> = Page::ALL
            .iter()
            .map(|p| {
                let l = gtk::Label::new(Some(p.step()));
                l.set_xalign(0.0);
                l.add_css_class("step");
                sidebar.append(&l);
                l
            })
            .collect();
        let spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        spacer.set_vexpand(true);
        sidebar.append(&spacer);
        let hint = text(if facts.online_packages.is_empty() {
            "Setup takes about 10 to 20 minutes."
        } else {
            "Setup takes about 15 to 40 minutes, depending on your internet connection."
        });
        hint.add_css_class("hint");
        sidebar.append(&hint);

        let message = text("");
        message.set_visible(false);
        let quit = gtk::Button::with_label("Quit");
        let back = gtk::Button::with_label("Back");
        let next = gtk::Button::with_label("Next");
        next.add_css_class("suggested-action");
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        bar.add_css_class("buttonbar");
        bar.append(&quit);
        let fill = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        fill.set_hexpand(true);
        bar.append(&fill);
        bar.append(&back);
        bar.append(&next);

        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.append(&stack);
        message.add_css_class("message");
        content.append(&message);
        content.append(&bar);
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        root.append(&sidebar);
        root.append(&content);
        window.set_child(Some(&root));

        let ui = Rc::new(Ui {
            facts,
            demo,
            window: window.clone(),
            stack,
            steps,
            back,
            next,
            quit,
            message,
            page: Cell::new(Page::Welcome),
            locale,
            keyboard,
            timezone,
            disks,
            filesystem,
            swap,
            full_name,
            user,
            password,
            confirm,
            computer,
            autologin,
            suggested_user: RefCell::new(String::new()),
            summary,
            erase_note,
            network_note,
            consent,
            pass,
            progress,
            current,
            details,
            finish_title,
            finish_text,
            choices: RefCell::new(None),
            installing: Cell::new(false),
            succeeded: Cell::new(false),
        });
        ui.connect();
        ui.go(Page::Welcome);
        ui
    }

    /// --demo-page: sample answers, then the requested page.
    fn open_with_sample_answers(self: &Rc<Self>, page: Page) {
        self.full_name.set_text("Ada Lovelace");
        self.password.set_text("wincraft");
        self.confirm.set_text("wincraft");
        match page {
            Page::Summary => self.prepare_summary(),
            Page::Installing | Page::Finished => {
                self.prepare_summary();
                self.start();
            }
            other => self.go(other),
        }
    }

    fn connect(self: &Rc<Self>) {
        let ui = self.clone();
        self.next.connect_clicked(move |_| ui.on_next());
        let ui = self.clone();
        self.back.connect_clicked(move |_| ui.on_back());
        let ui = self.clone();
        self.quit.connect_clicked(move |_| ui.window.close());
        let ui = self.clone();
        self.consent
            .connect_toggled(move |c| ui.next.set_sensitive(c.is_active()));
        // Suggest a user name from the full name until one is typed.
        let ui = self.clone();
        self.full_name.connect_changed(move |e| {
            let current = ui.user.text();
            if current.is_empty() || current == ui.suggested_user.borrow().as_str() {
                let suggestion = model::suggest_user(&e.text());
                ui.user.set_text(&suggestion);
                *ui.suggested_user.borrow_mut() = suggestion;
            }
        });
        // Setup cannot be interrupted safely half-way.
        let ui = self.clone();
        self.window.connect_close_request(move |_| {
            if ui.installing.get() {
                ui.say(
                    "Setup is still installing. Wait until it has finished.",
                    "warning",
                );
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
    }

    fn say(&self, text: &str, class: &str) {
        for c in ["error", "warning"] {
            self.message.remove_css_class(c);
        }
        self.message.add_css_class(class);
        self.message.set_text(text);
        self.message.set_visible(!text.is_empty());
    }

    fn go(&self, page: Page) {
        self.page.set(page);
        self.stack.set_visible_child_name(page.name());
        let here = page.index();
        for (i, (label, p)) in self.steps.iter().zip(Page::ALL).enumerate() {
            label.remove_css_class("current");
            label.remove_css_class("done");
            let mark = if i < here {
                label.add_css_class("done");
                "✓"
            } else if i == here {
                label.add_css_class("current");
                "▸"
            } else {
                "•"
            };
            label.set_text(&format!("{mark}   {}", p.step()));
        }
        self.say("", "error");
        self.next.remove_css_class("destructive-action");
        self.next.add_css_class("suggested-action");
        self.next.set_sensitive(true);
        self.next.set_visible(true);
        self.back.set_visible(true);
        self.quit.set_visible(true);
        self.quit.set_label("Quit");
        match page {
            Page::Welcome => {
                self.back.set_visible(false);
                self.next.set_label("Install Wincraft");
            }
            Page::Disk => {
                self.next.set_label("Next");
                self.next
                    .set_sensitive(self.disks.iter().any(|(_, r)| r.is_sensitive()));
            }
            Page::Summary => {
                self.next.set_label("Install");
                self.next.remove_css_class("suggested-action");
                self.next.add_css_class("destructive-action");
                self.next.set_sensitive(self.consent.is_active());
            }
            Page::Installing => {
                self.next.set_visible(false);
                self.back.set_visible(false);
                self.quit.set_visible(false);
            }
            Page::Finished => {
                self.back.set_visible(false);
                self.quit.set_label("Close");
                self.next.set_label("Restart now");
                self.next.set_visible(self.succeeded.get());
            }
            Page::Region | Page::Account => self.next.set_label("Next"),
        }
    }

    fn on_back(&self) {
        let back = match self.page.get() {
            Page::Region => Page::Welcome,
            Page::Disk => Page::Region,
            Page::Account => Page::Disk,
            Page::Summary => Page::Account,
            other => other,
        };
        self.go(back);
    }

    fn on_next(self: &Rc<Self>) {
        match self.page.get() {
            Page::Welcome => self.go(Page::Region),
            Page::Region => self.go(Page::Disk),
            Page::Disk => {
                if self.selected_disk().is_some() {
                    self.go(Page::Account);
                } else {
                    self.say("Choose the disk to install Wincraft to.", "error");
                }
            }
            Page::Account => self.prepare_summary(),
            Page::Summary => self.start(),
            Page::Finished => self.restart(),
            Page::Installing => {}
        }
    }

    fn selected_disk(&self) -> Option<String> {
        self.disks
            .iter()
            .find(|(_, r)| r.is_active() && r.is_sensitive())
            .map(|(p, _)| p.clone())
    }

    fn collect(&self) -> Choices {
        let defaults = Choices::default();
        Choices {
            locale: dropdown_value(&self.locale).unwrap_or(defaults.locale),
            keyboard: dropdown_value(&self.keyboard).unwrap_or(defaults.keyboard),
            timezone: dropdown_value(&self.timezone).unwrap_or(defaults.timezone),
            disk: self.selected_disk(),
            filesystem: FILESYSTEMS
                .get(self.filesystem.selected() as usize)
                .map_or(defaults.filesystem, |(f, _)| *f),
            swap: SWAP
                .get(self.swap.selected() as usize)
                .map_or(defaults.swap, |(s, _)| s.to_string()),
            full_name: self.full_name.text().to_string(),
            user: self.user.text().to_string(),
            password: self.password.text().to_string(),
            confirm: self.confirm.text().to_string(),
            computer: self.computer.text().to_string(),
            autologin: self.autologin.is_active(),
        }
    }

    fn prepare_summary(&self) {
        let choices = self.collect();
        if let Some(problem) = choices.account_problem() {
            self.say(problem, "error");
            return;
        }
        let answer = choices.answer();
        if let Some(problem) = answer.validate().into_iter().next() {
            self.say(&problem, "error");
            return;
        }
        let plan = match plan::build(
            &answer,
            &self.facts,
            &mut Random,
            Path::new(plan::DEFAULT_TARGET),
        ) {
            Ok(plan) => plan,
            Err(e) => {
                self.say(&format!("{e:#}"), "error");
                return;
            }
        };
        self.summary.set_text(&model::summary(&plan));
        let disk = &plan.summary.disk;
        let model = if disk.model.is_empty() {
            "disk"
        } else {
            disk.model.as_str()
        };
        self.erase_note.set_text(&format!(
            "Everything on {} ({model}, {}) will be erased.",
            disk.path,
            human_size(disk.size)
        ));
        let offline = !self.facts.online_packages.is_empty() && !self.demo && !has_default_route();
        self.network_note.set_text(
            "This computer is not connected to the internet, so Setup cannot download Wincraft's \
             apps, drivers and games. Connect with Network in the taskbar first; without a \
             connection you get a minimal system.",
        );
        self.network_note.set_visible(offline);
        self.consent.set_active(false);
        *self.choices.borrow_mut() = Some(choices);
        self.go(Page::Summary);
    }

    fn start(self: &Rc<Self>) {
        let Some(choices) = self.choices.borrow().clone() else {
            return;
        };
        self.details.set_text("");
        self.progress.set_fraction(0.0);
        self.progress.set_text(Some("0%"));
        self.installing.set(true);
        self.go(Page::Installing);
        if self.demo {
            self.simulate(&choices);
        } else {
            self.run_setup(&choices);
        }
    }

    fn log(&self, line: &str) {
        let mut end = self.details.end_iter();
        self.details.insert(&mut end, line.trim_end());
        let mut end = self.details.end_iter();
        self.details.insert(&mut end, "\n");
    }

    fn show_progress(&self, title: &str, pass: &str, done: usize, total: usize, step: &str) {
        let fraction = model::fraction(done, total);
        self.pass.set_text(&format!("{title}  ({pass})"));
        self.progress.set_fraction(fraction);
        self.progress
            .set_text(Some(&format!("{:.0}%", fraction * 100.0)));
        self.current.set_text(step);
        self.log(&format!("[{pass}] {step}"));
    }

    /// Run the real installation: the unattended Setup with JSON progress.
    fn run_setup(self: &Rc<Self>, choices: &Choices) {
        let path = match write_answer_file(&choices.answer()) {
            Ok(path) => path,
            Err(e) => return self.finish(Err(format!("{e:#}"))),
        };
        let mut argv: Vec<OsString> = Vec::new();
        if !is_root() {
            argv.extend(["sudo", "-n"].map(OsString::from));
        }
        argv.extend(["wincraft-setup", "install", "--unattend"].map(OsString::from));
        argv.push(path.clone().into());
        argv.extend(["--progress", "json"].map(OsString::from));
        let refs: Vec<&OsStr> = argv.iter().map(OsString::as_os_str).collect();
        let process = match gio::Subprocess::newv(
            &refs,
            gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_MERGE,
        ) {
            Ok(p) => p,
            Err(e) => {
                let _ = fs::remove_file(&path);
                return self.finish(Err(format!("Cannot start wincraft-setup: {e}")));
            }
        };
        self.log(&format!(
            "$ {}",
            argv.iter()
                .map(|a| a.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" ")
        ));
        let ui = self.clone();
        glib::spawn_future_local(async move {
            let mut outcome: Option<Result<(), String>> = None;
            let mut recent: Vec<String> = Vec::new();
            if let Some(stdout) = process.stdout_pipe() {
                let lines = gio::DataInputStream::new(&stdout);
                loop {
                    match lines.read_line_utf8_future(glib::Priority::DEFAULT).await {
                        Ok(Some(line)) => match Event::parse(&line) {
                            Some(Event::Progress {
                                pass,
                                title,
                                done,
                                total,
                                description,
                            }) => ui.show_progress(&title, &pass, done, total, &description),
                            Some(Event::Done) => outcome = Some(Ok(())),
                            Some(Event::Error { message }) => outcome = Some(Err(message)),
                            None => {
                                ui.log(&line);
                                recent.push(line.to_string());
                                if recent.len() > 12 {
                                    recent.remove(0);
                                }
                            }
                        },
                        Ok(None) => break,
                        Err(e) => {
                            ui.log(&format!("(cannot read Setup's output: {e})"));
                            break;
                        }
                    }
                }
            }
            let _ = process.wait_future().await;
            // The answer file holds the password.
            let _ = fs::remove_file(&path);
            let result = match outcome {
                Some(result) => result,
                None if process.is_successful() => Ok(()),
                None => Err(format!(
                    "Setup stopped unexpectedly.\n{}",
                    recent.join("\n")
                )),
            };
            ui.finish(result);
        });
    }

    /// --demo: walk through the real plan's steps without running them.
    fn simulate(self: &Rc<Self>, choices: &Choices) {
        let plan = match plan::build(
            &choices.answer(),
            &self.facts,
            &mut Random,
            Path::new(plan::DEFAULT_TARGET),
        ) {
            Ok(plan) => plan,
            Err(e) => return self.finish(Err(format!("{e:#}"))),
        };
        let steps: Vec<(&'static str, &'static str, String)> = plan
            .phases
            .iter()
            .flat_map(|phase| {
                phase.steps.iter().map(move |s| {
                    (
                        phase.pass.title(),
                        phase.pass.windows_name(),
                        s.description.clone(),
                    )
                })
            })
            .collect();
        let total = steps.len();
        let next = Cell::new(0usize);
        let ui = self.clone();
        glib::timeout_add_local(Duration::from_millis(140), move || {
            let i = next.get();
            match steps.get(i) {
                Some((title, pass, step)) => {
                    ui.show_progress(title, pass, i, total, step);
                    next.set(i + 1);
                    glib::ControlFlow::Continue
                }
                None => {
                    ui.finish(Ok(()));
                    glib::ControlFlow::Break
                }
            }
        });
    }

    fn finish(&self, result: Result<(), String>) {
        self.installing.set(false);
        match result {
            Ok(()) => {
                self.succeeded.set(true);
                self.progress.set_fraction(1.0);
                self.progress.set_text(Some("100%"));
                let user = self
                    .choices
                    .borrow()
                    .as_ref()
                    .map(|c| c.user.trim().to_string())
                    .unwrap_or_default();
                self.finish_title.set_text("Wincraft is installed");
                self.finish_text.set_text(&format!(
                    "Remove the installation media, then restart the computer to start Wincraft. \
                     Sign in as {user}.{}",
                    if self.demo {
                        "\n\n(Demo mode: nothing was installed.)"
                    } else {
                        ""
                    }
                ));
                self.log("Wincraft is installed.");
            }
            Err(message) => {
                self.succeeded.set(false);
                self.finish_title.set_text("Setup could not finish");
                self.finish_text.set_text(&format!(
                    "{message}\n\nWhat Setup did is in {LOG_DIR}/setupact.log, the errors in \
                     {LOG_DIR}/setuperr.log. The disk may be partly written: run Setup again \
                     to start over."
                ));
                self.log(&format!("Error: {message}"));
            }
        }
        self.go(Page::Finished);
    }

    fn restart(&self) {
        if self.demo {
            self.window.close();
            return;
        }
        let mut reboot = std::process::Command::new("sudo");
        reboot.args(["-n", "reboot"]);
        if is_root() {
            reboot = std::process::Command::new("reboot");
        }
        let succeeded = |mut c: std::process::Command| c.status().is_ok_and(|s| s.success());
        let mut loginctl = std::process::Command::new("loginctl");
        loginctl.arg("reboot");
        if !succeeded(reboot) && !succeeded(loginctl) {
            self.say("Cannot restart: restart the computer yourself.", "error");
        }
    }
}

/// Writes the answer file where only this user (and root) can read it: it
/// holds the password until Setup has finished.
fn write_answer_file(answer: &Answer) -> Result<PathBuf> {
    let dir = glib::user_runtime_dir().join("wincraft-installer");
    fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
    fs::set_permissions(&dir, Permissions::from_mode(0o700))?;
    let path = dir.join("autounattend.toml");
    let text = toml::to_string(answer).context("cannot write the answer file")?;
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&path)
        .with_context(|| format!("cannot write {}", path.display()))?;
    fs::set_permissions(&path, Permissions::from_mode(0o600))?;
    file.write_all(text.as_bytes())?;
    Ok(path)
}
