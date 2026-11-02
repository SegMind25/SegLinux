//! Alt+Tab, as on Windows: a list of the windows in the middle of the
//! screen, the last used first. Tab (Shift+Tab: back) moves through it while
//! Alt is held; letting go of Alt brings the chosen window forward, Escape
//! keeps the current one. A quick Alt+Tab goes back to the previous window,
//! however many are open. Minimized windows come last and are restored when
//! chosen.

use smithay::{
    backend::renderer::element::{memory::MemoryRenderBuffer, solid::SolidColorBuffer},
    desktop::Window,
    output::Output,
    utils::{IsAlive, Logical, Point, Rectangle, Size},
};

use crate::state::State;

const ROW: i32 = 36;
const PAD: i32 = 10;
/// Rows shown at once; more scroll with the choice.
const ROWS: usize = 12;
const PANEL: [f32; 4] = [0.07, 0.09, 0.13, 0.94];
/// Breeze's highlight blue, as the snap preview.
const CHOSEN: [f32; 4] = [0.215, 0.614, 0.822, 0.9];
const TEXT: [f32; 4] = [0.95, 0.96, 0.98, 1.0];

/// Each title and where it goes.
pub type Titles<'a> = Vec<(Point<i32, Logical>, &'a MemoryRenderBuffer)>;

pub struct Switcher {
    pub list: Vec<Window>,
    pub index: usize,
    pub output: Output,
    /// The panel, on its output.
    pub rect: Rectangle<i32, Logical>,
    pub panel: SolidColorBuffer,
    pub chosen: SolidColorBuffer,
    pub titles: Vec<Option<(Size<i32, Logical>, MemoryRenderBuffer)>>,
}

impl Switcher {
    /// The rows on screen: the first one's index and how many.
    pub fn visible(&self) -> (usize, usize) {
        let n = self.list.len().min(ROWS);
        let first = self.index.saturating_sub(n - 1).min(self.list.len() - n);
        (first, n)
    }

    /// What to draw, relative to the output: the panel, the chosen row,
    /// and each title with its place.
    pub fn layout(&self) -> (Rectangle<i32, Logical>, Point<i32, Logical>, Titles<'_>) {
        let (first, n) = self.visible();
        let row = |i: usize| self.rect.loc + Point::from((PAD, PAD + (i - first) as i32 * ROW));
        let chosen = row(self.index);
        let titles = (first..first + n)
            .filter_map(|i| {
                let (size, buffer) = self.titles.get(i)?.as_ref()?;
                Some((row(i) + Point::from((12, (ROW - size.h) / 2)), buffer))
            })
            .collect();
        (self.rect, chosen, titles)
    }
}

impl State {
    /// Alt+Tab (`back`: Alt+Shift+Tab): open the list, or move in it.
    pub fn switch_windows(&mut self, back: bool) {
        if let Some(s) = &mut self.switcher {
            let n = s.list.len();
            s.index = if back {
                (s.index + n - 1) % n
            } else {
                (s.index + 1) % n
            };
            self.backend.schedule_render();
            return;
        }
        let mut list: Vec<Window> = self
            .space
            .elements()
            .rev()
            .filter(|w| !w.x11_surface().is_some_and(|x| x.is_override_redirect()))
            .cloned()
            .collect();
        list.extend(self.minimized.iter().rev().cloned());
        if list.len() < 2 {
            // One window: just bring it back if it is minimized.
            if let Some(w) = list.pop() {
                self.choose(w);
            }
            return;
        }
        let Some(output) = self
            .space
            .output_under(self.pointer_location())
            .next()
            .or_else(|| self.space.outputs().next())
            .cloned()
        else {
            return;
        };
        let Some(geo) = self.space.output_geometry(&output) else {
            return;
        };
        let width = (geo.size.w - 80).clamp(200, 640);
        let shown = list.len().min(ROWS) as i32;
        let size = Size::from((width, shown * ROW + 2 * PAD));
        let rect = Rectangle::new(
            ((geo.size.w - size.w) / 2, (geo.size.h - size.h) / 2).into(),
            size,
        );
        let titles = list
            .iter()
            .map(|w| crate::deco::text(&crate::render::window_title(w), width - 2 * PAD - 24, TEXT))
            .collect();
        let index = if back { list.len() - 1 } else { 1 };
        self.switcher = Some(Switcher {
            list,
            index,
            output,
            rect,
            panel: SolidColorBuffer::new(size, PANEL),
            chosen: SolidColorBuffer::new((width - 2 * PAD, ROW), CHOSEN),
            titles,
        });
        self.backend.schedule_render();
    }

    /// Alt let go: the chosen window comes forward.
    pub fn finish_switcher(&mut self) {
        let Some(s) = self.switcher.take() else {
            return;
        };
        if let Some(w) = s.list.get(s.index).filter(|w| w.alive()).cloned() {
            self.choose(w);
        }
        self.backend.schedule_render();
    }

    /// Escape: the list closes, nothing changes.
    pub fn cancel_switcher(&mut self) {
        if self.switcher.take().is_some() {
            self.backend.schedule_render();
        }
    }

    fn choose(&mut self, window: Window) {
        if self.minimized.contains(&window) {
            self.unminimize(&window);
        }
        self.focus(Some(window));
    }

    fn pointer_location(&self) -> Point<f64, Logical> {
        self.seat
            .get_pointer()
            .map(|p| p.current_location())
            .unwrap_or_default()
    }
}
