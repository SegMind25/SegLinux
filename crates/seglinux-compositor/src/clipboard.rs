//! The clipboard outlives the application it was copied from. On Wayland
//! the application that copied keeps the data and hands it over on each
//! paste, so closing it empties the clipboard. The compositor keeps a copy
//! of what was copied (text, links, HTML, PNG images: up to 32 MiB) and,
//! once the application is gone, offers that copy itself, to Wayland and
//! X11 applications alike.

use std::{
    io::{Read, Write},
    os::fd::OwnedFd,
    sync::{Arc, Mutex},
};

use smithay::{
    reexports::rustix::pipe::{PipeFlags, pipe_with},
    wayland::selection::data_device::{
        current_data_device_selection_userdata, request_data_device_client_selection,
        set_data_device_selection,
    },
};

use crate::state::State;

/// Who the compositor serves the clipboard for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Owner {
    /// An X11 application copied it (XWayland).
    X11,
    /// The kept copy, its application closed.
    Kept,
}

/// The kinds of data worth keeping: what people paste.
const KEPT_TYPES: &[&str] = &[
    "text/plain;charset=utf-8",
    "text/plain",
    "UTF8_STRING",
    "STRING",
    "TEXT",
    "text/html",
    "text/uri-list",
    "image/png",
];
/// Larger copies are not kept (a few photos at most).
const LIMIT: usize = 32 << 20;

#[derive(Default)]
struct Kept {
    /// Bumped on every copy: late readers of an older one are dropped.
    generation: u64,
    items: Vec<(String, Arc<Vec<u8>>)>,
}

#[derive(Default)]
pub struct Clipboard {
    kept: Arc<Mutex<Kept>>,
    /// What the copying application offers, watched until it is gone.
    source: Option<smithay::wayland::selection::SelectionSource>,
}

impl State {
    /// The copying application closed (checked every loop turn): its
    /// kept copy takes over.
    pub fn check_clipboard_source(&mut self) {
        use smithay::utils::IsAlive;
        if self.clipboard.source.as_ref().is_some_and(|s| !s.alive()) {
            self.clipboard.source = None;
            self.offer_kept_clipboard();
        }
    }

    /// A Wayland application copied: read and keep what it offers.
    pub fn keep_clipboard(&mut self, source: &smithay::wayland::selection::SelectionSource) {
        self.clipboard.source = Some(source.clone());
        let mime_types = source.mime_types();
        let generation = {
            let mut kept = self.clipboard.kept.lock().unwrap();
            kept.generation += 1;
            kept.items.clear();
            kept.generation
        };
        for mime in mime_types
            .iter()
            .filter(|m| KEPT_TYPES.contains(&m.as_str()))
        {
            let Ok((read, write)) = pipe_with(PipeFlags::CLOEXEC) else {
                return;
            };
            if request_data_device_client_selection(&self.seat, mime.clone(), write).is_err() {
                return;
            }
            let kept = self.clipboard.kept.clone();
            let mime = mime.clone();
            std::thread::spawn(move || {
                let mut data = Vec::new();
                let mut file = std::fs::File::from(read).take(LIMIT as u64 + 1);
                if file.read_to_end(&mut data).is_err() || data.len() > LIMIT {
                    return;
                }
                let mut kept = kept.lock().unwrap();
                let total: usize = kept.items.iter().map(|(_, d)| d.len()).sum();
                if kept.generation == generation && total + data.len() <= LIMIT {
                    kept.items.push((mime, Arc::new(data)));
                }
            });
        }
    }

    /// An application cleared the clipboard: the kept copy goes too.
    pub fn clipboard_cleared(&mut self) {
        self.clipboard.source = None;
        let mut kept = self.clipboard.kept.lock().unwrap();
        kept.generation += 1;
        kept.items.clear();
    }

    /// The application that copied is gone: offer the kept copy instead.
    pub fn offer_kept_clipboard(&mut self) {
        let mimes: Vec<String> = {
            let kept = self.clipboard.kept.lock().unwrap();
            kept.items.iter().map(|(m, _)| m.clone()).collect()
        };
        if mimes.is_empty() {
            return;
        }
        set_data_device_selection(&self.display_handle, &self.seat, mimes.clone(), Owner::Kept);
        // X11 applications can paste it too.
        if let Some(xwm) = self.xwm.as_mut()
            && let Err(e) = xwm.new_selection(
                smithay::wayland::selection::SelectionTarget::Clipboard,
                Some(mimes),
            )
        {
            tracing::warn!("kept clipboard to X11: {e}");
        }
    }

    /// The kept copy is what is offered now.
    pub fn serving_kept_clipboard(&self) -> bool {
        current_data_device_selection_userdata::<State>(&self.seat)
            .is_some_and(|o| *o == Owner::Kept)
    }

    /// Paste the kept copy into `fd` (in the background: big images).
    pub fn send_kept_clipboard(&self, mime_type: &str, fd: OwnedFd) {
        let data = self
            .clipboard
            .kept
            .lock()
            .unwrap()
            .items
            .iter()
            .find(|(m, _)| m == mime_type)
            .map(|(_, d)| d.clone());
        if let Some(data) = data {
            std::thread::spawn(move || {
                let _ = std::fs::File::from(fd).write_all(&data);
            });
        }
    }
}
