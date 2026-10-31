//! Moving and resizing with the mouse, as on Windows and KDE:
//! - a window dragged to the top edge of a screen is maximized, to the
//!   left or right edge it takes that half, to a corner that quarter; a
//!   tinted rectangle shows where it will go before the button is let go
//! - windows with SegLinux title bars get an invisible border to resize
//!   them from, with the matching pointer (arrows)
//! - the pointer's shape where no application decides it (title bars,
//!   borders, while moving)

use std::time::{Duration, Instant};

use smithay::{
    desktop::{Window, layer_map_for_output},
    input::pointer::{CursorIcon, CursorImageStatus},
    reexports::wayland_protocols::xdg::shell::server::xdg_toplevel,
    utils::{Logical, Point, Rectangle},
};

use crate::{grabs::resize_grab::ResizeEdge, state::State};

/// How close to a screen's edge the pointer must be to snap.
const EDGE: f64 = 6.0;
/// How far from a corner along an edge still counts as the corner.
const CORNER: f64 = 80.0;
/// The invisible border around windows with our title bars.
const BORDER: i32 = 8;
/// Two clicks on a title bar within this time: a double click.
pub const DOUBLE_CLICK: Duration = Duration::from_millis(400);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Snap {
    Maximize,
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// Where on `area` a window snapped `snap` goes.
pub fn snap_rect(snap: Snap, area: Rectangle<i32, Logical>) -> Rectangle<i32, Logical> {
    let (x, y, w, h) = (area.loc.x, area.loc.y, area.size.w, area.size.h);
    let (hw, hh) = (w / 2, h / 2);
    let (x, y, w, h) = match snap {
        Snap::Maximize => (x, y, w, h),
        Snap::Left => (x, y, hw, h),
        Snap::Right => (x + hw, y, w - hw, h),
        Snap::TopLeft => (x, y, hw, hh),
        Snap::TopRight => (x + hw, y, w - hw, hh),
        Snap::BottomLeft => (x, y + hh, hw, h - hh),
        Snap::BottomRight => (x + hw, y + hh, w - hw, h - hh),
    };
    Rectangle::new((x, y).into(), (w, h).into())
}

/// Which snap the pointer at `pos` on a screen `screen` asks for.
pub fn snap_for(pos: Point<f64, Logical>, screen: Rectangle<i32, Logical>) -> Option<Snap> {
    let (left, top) = (screen.loc.x as f64, screen.loc.y as f64);
    let (right, bottom) = (left + screen.size.w as f64, top + screen.size.h as f64);
    let at_left = pos.x < left + EDGE;
    let at_right = pos.x >= right - EDGE;
    let at_top = pos.y < top + EDGE;
    let near_top = pos.y < top + CORNER;
    let near_bottom = pos.y >= bottom - CORNER;
    Some(match (at_left, at_right) {
        (true, _) if near_top => Snap::TopLeft,
        (true, _) if near_bottom => Snap::BottomLeft,
        (true, _) => Snap::Left,
        (_, true) if near_top => Snap::TopRight,
        (_, true) if near_bottom => Snap::BottomRight,
        (_, true) => Snap::Right,
        _ if at_top => Snap::Maximize,
        _ => return None,
    })
}

/// Which edges of `rect` (a window with its title bar) the point is on,
/// in the border around it; none inside or beyond.
pub fn edges_at(rect: Rectangle<i32, Logical>, pos: Point<f64, Logical>) -> Option<ResizeEdge> {
    let outer = Rectangle::new(
        (rect.loc.x - BORDER, rect.loc.y - BORDER).into(),
        (rect.size.w + 2 * BORDER, rect.size.h + 2 * BORDER).into(),
    );
    if !outer.to_f64().contains(pos) || rect.to_f64().contains(pos) {
        return None;
    }
    let (l, t) = (rect.loc.x as f64, rect.loc.y as f64);
    let (r, b) = (l + rect.size.w as f64, t + rect.size.h as f64);
    // The sides the point is beyond.
    let mut edges = ResizeEdge::empty();
    if pos.x < l {
        edges |= ResizeEdge::LEFT;
    } else if pos.x >= r {
        edges |= ResizeEdge::RIGHT;
    }
    if pos.y < t {
        edges |= ResizeEdge::TOP;
    } else if pos.y >= b {
        edges |= ResizeEdge::BOTTOM;
    }
    // A little along a side from a corner counts as the corner.
    let reach = 2.0 * BORDER as f64 + 10.0;
    if edges == ResizeEdge::LEFT || edges == ResizeEdge::RIGHT {
        if pos.y < t + reach {
            edges |= ResizeEdge::TOP;
        } else if pos.y >= b - reach {
            edges |= ResizeEdge::BOTTOM;
        }
    } else if edges == ResizeEdge::TOP || edges == ResizeEdge::BOTTOM {
        if pos.x < l + reach {
            edges |= ResizeEdge::LEFT;
        } else if pos.x >= r - reach {
            edges |= ResizeEdge::RIGHT;
        }
    }
    (!edges.is_empty()).then_some(edges)
}

/// The pointer for resizing from these edges.
pub fn edge_icon(edges: ResizeEdge) -> CursorIcon {
    match edges {
        ResizeEdge::TOP => CursorIcon::NResize,
        ResizeEdge::BOTTOM => CursorIcon::SResize,
        ResizeEdge::LEFT => CursorIcon::WResize,
        ResizeEdge::RIGHT => CursorIcon::EResize,
        ResizeEdge::TOP_LEFT => CursorIcon::NwResize,
        ResizeEdge::TOP_RIGHT => CursorIcon::NeResize,
        ResizeEdge::BOTTOM_LEFT => CursorIcon::SwResize,
        ResizeEdge::BOTTOM_RIGHT => CursorIcon::SeResize,
        _ => CursorIcon::Default,
    }
}

impl State {
    /// The snap for a window dragged to `pos`, and where it would go.
    pub fn snap_at(&self, pos: Point<f64, Logical>) -> Option<(Snap, Rectangle<i32, Logical>)> {
        let output = self.space.output_under(pos).next()?;
        let screen = self.space.output_geometry(output)?;
        let snap = snap_for(pos, screen)?;
        let zone = layer_map_for_output(output).non_exclusive_zone();
        let area = Rectangle::new(screen.loc + zone.loc, zone.size);
        Some((snap, snap_rect(snap, area)))
    }

    /// Put `window` where `snap` says.
    pub fn apply_snap(&mut self, window: &Window, snap: Snap, rect: Rectangle<i32, Logical>) {
        if snap == Snap::Maximize {
            if !Self::is_state(window, xdg_toplevel::State::Maximized) {
                self.toggle_maximize(window);
            }
        } else {
            self.place(window, rect, None);
        }
        set_snapped(window, true);
        self.backend.schedule_render();
    }

    /// The window whose resize border is at `pos`, and which edges; none
    /// when a window (or its title bar) is in front there.
    pub fn border_under(&self, pos: Point<f64, Logical>) -> Option<(Window, ResizeEdge)> {
        for window in self.space.elements().rev() {
            let Some(loc) = self.space.element_location(window) else {
                continue;
            };
            let size = window.geometry().size;
            let bar = if crate::deco::has_bar(window) {
                crate::deco::BAR
            } else {
                0
            };
            let rect = Rectangle::new((loc.x, loc.y - bar).into(), (size.w, size.h + bar).into());
            if rect.to_f64().contains(pos) {
                return None;
            }
            let resizable = bar > 0
                && !Self::is_state(window, xdg_toplevel::State::Maximized)
                && !Self::is_state(window, xdg_toplevel::State::Fullscreen);
            if resizable && let Some(edges) = edges_at(rect, pos) {
                return Some((window.clone(), edges));
            }
        }
        None
    }

    /// Where no application has the pointer: the compositor's own shape
    /// (resize arrows on borders, the arrow elsewhere).
    pub fn update_cursor(&mut self, pos: Point<f64, Logical>, on_surface: bool) {
        if on_surface || self.lock.locked {
            return;
        }
        let icon = self
            .border_under(pos)
            .map_or(CursorIcon::Default, |(_, e)| edge_icon(e));
        let status = CursorImageStatus::Named(icon);
        if self.cursor_status != status {
            self.cursor_status = status;
            self.backend.schedule_render();
        }
    }

    /// Give a maximized or snapped window its size back when it is dragged
    /// away, keeping the pointer at the same place along its title bar;
    /// returns where the window now is.
    pub fn unsnap_under(
        &mut self,
        window: &Window,
        pos: Point<f64, Logical>,
    ) -> Option<Point<i32, Logical>> {
        let loc = self.space.element_location(window)?;
        let old_width = window.geometry().size.w.max(1);
        let size = Self::restore_rect(window)?.size;
        self.restore(window);
        set_snapped(window, false);
        let ratio = ((pos.x - loc.x as f64) / old_width as f64).clamp(0.0, 1.0);
        let bar = if crate::deco::has_bar(window) {
            crate::deco::BAR
        } else {
            0
        };
        let x = (pos.x - size.w as f64 * ratio).round() as i32;
        let y = (pos.y + bar as f64 / 2.0).round() as i32;
        let new = Point::from((x, y));
        if let Some(x11) = window.x11_surface() {
            let _ = x11.configure(Rectangle::new(new, size));
        }
        self.space.map_element(window.clone(), new, true);
        Some(new)
    }

    /// The pointer went onto another application's surface: the arrow
    /// until that application sets its own (some never do).
    pub fn entered(
        &mut self,
        before: &Option<smithay::reexports::wayland_server::protocol::wl_surface::WlSurface>,
        now: Option<&smithay::reexports::wayland_server::protocol::wl_surface::WlSurface>,
    ) {
        if now.is_some() && now != before.as_ref() {
            self.cursor_status = CursorImageStatus::default_named();
        }
    }

    /// A second click on the same title bar soon after the first.
    pub fn is_double_click(&mut self, window: &Window) -> bool {
        let now = Instant::now();
        let double = self
            .last_bar_click
            .as_ref()
            .is_some_and(|(w, t)| w == window && now.duration_since(*t) < DOUBLE_CLICK);
        self.last_bar_click = if double {
            None
        } else {
            Some((window.clone(), now))
        };
        double
    }
}

/// A window put on a half or quarter by dragging: dragging it away gives
/// it its size back.
struct Snapped(std::sync::Mutex<bool>);

pub fn set_snapped(window: &Window, on: bool) {
    let slot = window
        .user_data()
        .get_or_insert(|| Snapped(std::sync::Mutex::new(false)));
    *slot.0.lock().unwrap() = on;
}

pub fn is_snapped(window: &Window) -> bool {
    window
        .user_data()
        .get::<Snapped>()
        .is_some_and(|s| *s.0.lock().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    #[test]
    fn edges_and_corners_of_the_screen() {
        let screen = r(0, 0, 1366, 768);
        let at = |x: f64, y: f64| snap_for((x, y).into(), screen);
        assert_eq!(at(600.0, 0.0), Some(Snap::Maximize));
        assert_eq!(at(0.0, 400.0), Some(Snap::Left));
        assert_eq!(at(1365.0, 400.0), Some(Snap::Right));
        assert_eq!(at(0.0, 10.0), Some(Snap::TopLeft));
        assert_eq!(at(1365.0, 767.0), Some(Snap::BottomRight));
        assert_eq!(at(0.0, 760.0), Some(Snap::BottomLeft));
        assert_eq!(at(1365.0, 0.0), Some(Snap::TopRight));
        assert_eq!(at(600.0, 400.0), None);
        // A second screen to the right.
        let second = r(1366, 0, 1920, 1080);
        assert_eq!(snap_for((1366.0, 500.0).into(), second), Some(Snap::Left));
    }

    #[test]
    fn halves_and_quarters_fill_the_work_area() {
        let area = r(0, 0, 1366, 720);
        assert_eq!(snap_rect(Snap::Maximize, area), area);
        assert_eq!(snap_rect(Snap::Left, area), r(0, 0, 683, 720));
        assert_eq!(snap_rect(Snap::Right, area), r(683, 0, 683, 720));
        assert_eq!(snap_rect(Snap::TopLeft, area), r(0, 0, 683, 360));
        assert_eq!(snap_rect(Snap::BottomRight, area), r(683, 360, 683, 360));
        let odd = r(10, 20, 1001, 701);
        let br = snap_rect(Snap::BottomRight, odd);
        assert_eq!(br.loc.x + br.size.w, 1011);
        assert_eq!(br.loc.y + br.size.h, 721);
    }

    #[test]
    fn resize_borders() {
        let w = r(100, 100, 400, 300);
        let at = |x: f64, y: f64| edges_at(w, (x, y).into());
        assert_eq!(at(97.0, 250.0), Some(ResizeEdge::LEFT));
        assert_eq!(at(503.0, 250.0), Some(ResizeEdge::RIGHT));
        assert_eq!(at(300.0, 97.0), Some(ResizeEdge::TOP));
        assert_eq!(at(300.0, 403.0), Some(ResizeEdge::BOTTOM));
        assert_eq!(at(97.0, 97.0), Some(ResizeEdge::TOP_LEFT));
        assert_eq!(at(503.0, 403.0), Some(ResizeEdge::BOTTOM_RIGHT));
        // Near a corner along a side still resizes both ways.
        assert_eq!(at(97.0, 110.0), Some(ResizeEdge::TOP_LEFT));
        assert_eq!(at(300.0, 250.0), None);
        assert_eq!(at(80.0, 250.0), None);
    }
}
