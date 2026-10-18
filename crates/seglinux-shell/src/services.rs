//! What the desktop offers other applications, once per session (the docks
//! are rebuilt when settings or monitors change; these are not): the
//! notification service, the system tray, the on-screen display and the
//! badges applications put on their dock icons.
use crate::{badges, notify, osd, sni};

#[derive(Clone)]
pub struct Services {
    pub notifications: notify::Center,
    pub tray: sni::Host,
    pub osd: osd::Osd,
    pub badges: badges::Badges,
}

impl Services {
    /// `layer`: pop-ups as layer-shell surfaces (else windows, in the
    /// preview).
    pub fn start(app: &gtk::Application, layer: bool) -> Services {
        Services {
            notifications: notify::Center::start(app, layer),
            tray: sni::Host::start(),
            osd: osd::Osd::new(app, layer),
            badges: badges::Badges::start(),
        }
    }
}
