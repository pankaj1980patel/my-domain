//! Screens, the pointer kept on them, and pointer barriers on their outer
//! edges (InputCapture).

/// A screen (a portal stream or an InputCapture zone) in the desktop's
/// logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

/// A side of the desktop.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

/// Where the pointer ends up moving from `from` by (`dx`, `dy`): it stays
/// on the screens. Past every screen, it stops at the border of the one
/// it was on (as a real pointer does). No screens known: just moved.
pub fn moved(screens: &[Rect], from: (f64, f64), dx: f64, dy: f64) -> (f64, f64) {
    let to = (from.0 + dx, from.1 + dy);
    if screens.is_empty() || screens.iter().any(|s| s.contains(to.0, to.1)) {
        return to;
    }
    let on = screens
        .iter()
        .find(|s| s.contains(from.0, from.1))
        .unwrap_or(&screens[0]);
    (
        to.0.clamp(on.x, on.x + on.w - 1.0),
        to.1.clamp(on.y, on.y + on.h - 1.0),
    )
}

/// A pointer barrier: a line on a screen's outer edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Barrier {
    /// Its id (never 0).
    pub id: u32,
    pub edge: Edge,
    pub x1: i32,
    pub y1: i32,
    pub x2: i32,
    pub y2: i32,
}

/// Barriers for `edges` on the screens (`zones`, whole pixels): every
/// screen's side that's on the desktop's outer edge there gets one, as the
/// portal wants them (on the zone's border: the left edge at x, the right
/// at x + width; inclusive ends).
pub fn barriers(zones: &[Rect], edges: &[Edge]) -> Vec<Barrier> {
    let mut out = Vec::new();
    if zones.is_empty() {
        return out;
    }
    let min_x = zones.iter().map(|z| z.x).fold(f64::INFINITY, f64::min);
    let min_y = zones.iter().map(|z| z.y).fold(f64::INFINITY, f64::min);
    let max_x = zones
        .iter()
        .map(|z| z.x + z.w)
        .fold(f64::NEG_INFINITY, f64::max);
    let max_y = zones
        .iter()
        .map(|z| z.y + z.h)
        .fold(f64::NEG_INFINITY, f64::max);
    let mut id = 0;
    for edge in edges {
        for z in zones {
            let (x, y, w, h) = (z.x as i32, z.y as i32, z.w as i32, z.h as i32);
            let line = match edge {
                Edge::Left if z.x <= min_x => (x, y, x, y + h - 1),
                Edge::Right if z.x + z.w >= max_x => (x + w, y, x + w, y + h - 1),
                Edge::Top if z.y <= min_y => (x, y, x + w - 1, y),
                Edge::Bottom if z.y + z.h >= max_y => (x, y + h, x + w - 1, y + h),
                _ => continue,
            };
            id += 1;
            out.push(Barrier {
                id,
                edge: *edge,
                x1: line.0,
                y1: line.1,
                x2: line.2,
                y2: line.3,
            });
        }
    }
    out
}

/// A point just inside the desktop from `at` on `edge` (where the pointer
/// is let go after a capture: off the barrier, so it doesn't fire again).
pub fn inside(edge: Edge, at: (f64, f64), by: f64) -> (f64, f64) {
    match edge {
        Edge::Left => (at.0 + by, at.1),
        Edge::Right => (at.0 - by, at.1),
        Edge::Top => (at.0, at.1 + by),
        Edge::Bottom => (at.0, at.1 - by),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn the_pointer_stays_on_the_screens() {
        let s = [r(0.0, 0.0, 1920.0, 1080.0), r(1920.0, 0.0, 1280.0, 1024.0)];
        assert_eq!(moved(&s, (100.0, 100.0), 10.0, -5.0), (110.0, 95.0));
        // Across into the second screen.
        assert_eq!(moved(&s, (1910.0, 10.0), 20.0, 0.0), (1930.0, 10.0));
        // Off the left: stops at the border.
        assert_eq!(moved(&s, (5.0, 10.0), -50.0, 0.0), (0.0, 10.0));
        // Below the shorter second screen: stops at its bottom.
        assert_eq!(moved(&s, (2000.0, 1000.0), 0.0, 60.0), (2000.0, 1023.0));
        assert_eq!(moved(&[], (1.0, 1.0), -5.0, 0.0), (-4.0, 1.0));
    }

    #[test]
    fn barriers_sit_on_the_outer_edges_only() {
        let s = [r(0.0, 0.0, 1920.0, 1080.0), r(1920.0, 0.0, 1280.0, 1024.0)];
        let b = barriers(&s, &[Edge::Right, Edge::Left]);
        assert_eq!(b.len(), 2);
        assert_eq!((b[0].id, b[0].edge), (1, Edge::Right));
        assert_eq!((b[0].x1, b[0].y1, b[0].x2, b[0].y2), (3200, 0, 3200, 1023));
        assert_eq!((b[1].x1, b[1].y1, b[1].x2, b[1].y2), (0, 0, 0, 1079));
        // Top: both screens touch it.
        let top = barriers(&s, &[Edge::Top]);
        assert_eq!(top.len(), 2);
        assert_eq!((top[1].x1, top[1].x2, top[1].y1), (1920, 3199, 0));
        // Bottom: only the taller screen is at the bottom.
        let bottom = barriers(&s, &[Edge::Bottom]);
        assert_eq!(bottom.len(), 1);
        assert_eq!((bottom[0].y1, bottom[0].x2), (1080, 1919));
        assert!(barriers(&[], &[Edge::Left]).is_empty());
    }

    #[test]
    fn let_go_just_inside() {
        assert_eq!(inside(Edge::Right, (3200.0, 50.0), 3.0), (3197.0, 50.0));
        assert_eq!(inside(Edge::Top, (10.0, 0.0), 3.0), (10.0, 3.0));
    }
}
