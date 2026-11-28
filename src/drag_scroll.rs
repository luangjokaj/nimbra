//! Click and drag to scroll a row sideways (the signature chips in Settings, the accounts in
//! the search panel), for a mouse without a sideways wheel.
//!
//! A press is a click until the pointer has moved a few points; after that it drags the
//! row, and letting go picks nothing (`moved`), so a drag never picks a chip by accident.

use gpui::{Pixels, ScrollHandle, point, px};

/// How far the pointer moves before a press drags rather than clicks.
const SLOP: Pixels = px(4.);

/// A press on a row that may become a drag.
pub struct DragScroll {
    /// Where the pointer went down, and how far the row was scrolled then.
    from: Pixels,
    offset: Pixels,
    /// Whether it has moved past `SLOP`: then it is a drag, and not a click.
    pub moved: bool,
}

impl DragScroll {
    pub fn start(x: Pixels, handle: &ScrollHandle) -> Self {
        DragScroll {
            from: x,
            offset: handle.offset().x,
            moved: false,
        }
    }

    /// The pointer at `x`: the row follows it, once it has moved far enough. True when the
    /// row moved, to draw again.
    pub fn follow(&mut self, x: Pixels, handle: &ScrollHandle) -> bool {
        let dx = x - self.from;
        if !self.moved && dx.abs() <= SLOP {
            return false;
        }
        self.moved = true;
        let max = handle.max_offset().width;
        let to = (self.offset + dx).clamp(-max, px(0.));
        let now = handle.offset();
        if now.x == to {
            return false;
        }
        handle.set_offset(point(to, now.y));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_press_is_a_click_until_it_moves_a_few_points() {
        let handle = ScrollHandle::new();
        let mut drag = DragScroll::start(px(100.), &handle);
        // A hand that shakes a little still clicks.
        drag.follow(px(103.), &handle);
        drag.follow(px(97.), &handle);
        assert!(!drag.moved);
        // Past the slop it drags, and stays a drag even back where it started.
        drag.follow(px(110.), &handle);
        assert!(drag.moved);
        drag.follow(px(100.), &handle);
        assert!(drag.moved);
        // Never past the row's start (nothing to scroll here, so it stays at 0).
        assert_eq!(handle.offset().x, px(0.));
    }
}
