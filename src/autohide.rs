//! The hidden sidebar's auto-hide: at the window's left edge it floats in over the content,
//! and it leaves again with the pointer.
//!
//! The pointer is asked from AppKit on a short timer rather than taken from mouse events:
//! those never arrive over the email's native web view (which reaches the edge when the
//! list is hidden), nor once a fast move has carried the pointer out of the window.

use std::time::Instant;

/// The strip along the window's left edge that brings the panel in.
const EDGE: f32 = 6.;
/// How far past that edge a move that was too fast to stop on it still counts.
const OVERSHOOT: f32 = 48.;
/// How far right of the panel the pointer may go before it leaves.
const SLACK: f32 = 10.;
const IN_SECONDS: f32 = 0.18;
const OUT_SECONDS: f32 = 0.12;

/// The pointer, in window points from the left edge.
#[derive(Clone, Copy, Debug)]
pub struct Pointer {
    pub x: f32,
    /// Between the window's top and bottom.
    pub level: bool,
    /// A mouse button is down (a drag, a resize).
    pub pressed: bool,
}

pub struct Peek {
    shown: bool,
    from: f32,
    since: Option<Instant>,
    /// Where the pointer was at the last look.
    last: Option<f32>,
}

impl Peek {
    pub fn new() -> Self {
        Peek {
            shown: false,
            from: 0.,
            since: None,
            last: None,
        }
    }

    /// Away at once, until the pointer comes to the edge again.
    pub fn reset(&mut self) {
        *self = Peek::new();
    }

    /// In place or away at once, without the slide (Reduce Motion).
    pub fn settle(&mut self) {
        self.since = None;
    }

    /// How far in the panel is `ahead` seconds from now: 0 away, 1 in place, a little over
    /// as it springs in.
    pub fn progress(&self, ahead: f32) -> f32 {
        let to = if self.shown { 1. } else { 0. };
        let Some(since) = self.since else {
            return to;
        };
        let elapsed = since.elapsed().as_secs_f32() + ahead;
        let eased = if self.shown {
            spring(elapsed / IN_SECONDS)
        } else {
            let t = (elapsed / OUT_SECONDS).min(1.);
            t * t
        };
        self.from + (to - self.from) * eased
    }

    pub fn moving(&self) -> bool {
        let seconds = if self.shown { IN_SECONDS } else { OUT_SECONDS };
        self.since
            .is_some_and(|since| since.elapsed().as_secs_f32() < seconds)
    }

    /// Takes a look at the pointer. `None`: it is not this window's to follow (inactive,
    /// covered). `right` is the panel's right edge, `hold` keeps a panel that is in.
    /// Returns whether the panel started to move.
    pub fn look(&mut self, pointer: Option<Pointer>, right: f32, hold: bool) -> bool {
        let last = self.last;
        self.last = pointer.map(|p| p.x);
        let shown = if self.shown {
            hold || pointer.is_some_and(|p| {
                p.pressed || (p.level && (-OVERSHOOT..=right + SLACK).contains(&p.x))
            })
        } else {
            // Only arriving at the edge counts: resting there when it was hidden does not.
            match (pointer, last) {
                (Some(p), Some(last)) if p.level && !p.pressed => {
                    let arrived = (0. ..=EDGE).contains(&p.x) && !(0. ..=EDGE).contains(&last);
                    let flew_past = (-OVERSHOOT..0.).contains(&p.x) && last > EDGE;
                    arrived || flew_past
                }
                _ => false,
            }
        };
        if shown == self.shown {
            return false;
        }
        self.from = self.progress(0.);
        self.since = Some(Instant::now());
        self.shown = shown;
        true
    }
}

/// Eases out a touch past the end and settles back (about 5pt on the sidebar's width).
fn spring(t: f32) -> f32 {
    const BACK: f32 = 0.75;
    let t = t.clamp(0., 1.) - 1.;
    1. + (BACK + 1.) * t * t * t + BACK * t * t
}

#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)] // emitted by the objc crate's macros
mod platform {
    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    use super::Pointer;

    /// An x that has all three traffic lights left of the window.
    const AWAY: f32 = -120.;
    /// Their distance from the top, as in the main window's `traffic_light_position`.
    const LIGHTS_TOP: f64 = 18.;

    thread_local! {
        static SPACING: std::cell::Cell<Option<f64>> = const { std::cell::Cell::new(None) };
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Point {
        x: f64,
        y: f64,
    }
    unsafe impl objc::Encode for Point {
        fn encode() -> objc::Encoding {
            // SAFETY: the encoding of NSPoint on 64-bit macOS.
            unsafe { objc::Encoding::from_str("{CGPoint=dd}") }
        }
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Frame {
        origin: Point,
        width: f64,
        height: f64,
    }

    fn ns_window(window: &gpui::Window) -> Option<*mut Object> {
        let handle = HasWindowHandle::window_handle(window).ok()?;
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            return None;
        };
        // SAFETY: `ns_view` is the live content view of this window, on the main thread.
        let ns_window: *mut Object =
            unsafe { msg_send![handle.ns_view.as_ptr() as *mut Object, window] };
        (!ns_window.is_null()).then_some(ns_window)
    }

    /// Where the pointer is, if this window has it: active, and (inside the window, as far
    /// right as `near`) with no other window on top there.
    pub fn pointer(window: &gpui::Window, near: f32) -> Option<Pointer> {
        if !window.is_window_active() {
            return None;
        }
        let ns_window = ns_window(window)?;
        let height = f64::from(f32::from(window.viewport_size().height));
        // SAFETY: plain getters on a live NSWindow and on AppKit classes, main thread.
        unsafe {
            let at: Point = msg_send![ns_window, mouseLocationOutsideOfEventStream];
            let buttons: usize = msg_send![class!(NSEvent), pressedMouseButtons];
            // Inclusive: against the top of the screen the pointer stops at the height itself.
            let level = (0. ..=height + 1.).contains(&at.y);
            let x = at.x as f32;
            if level && (0. ..=near).contains(&x) {
                let screen: Point = msg_send![class!(NSEvent), mouseLocation];
                let top: isize = msg_send![class!(NSWindow),
                    windowNumberAtPoint: screen
                    belowWindowWithWindowNumber: 0isize];
                let mine: isize = msg_send![ns_window, windowNumber];
                if top != mine {
                    return None;
                }
            }
            Some(Pointer {
                x,
                level,
                pressed: buttons != 0,
            })
        }
    }

    /// Puts the traffic lights at `x` (the close button's left edge), or takes them away:
    /// out of the window, where they cannot be clicked, and clear, for a frame in which
    /// GPUI has moved them back (it does on a resize).
    ///
    /// AppKit lays the title bar out again when they are hidden or moved, at its own height
    /// and spacing. So `setHidden:` is never used, and each frame sets every button's whole
    /// position, as GPUI does, rather than moving it from where it was found.
    pub fn place_traffic_lights(window: &gpui::Window, x: Option<f32>) {
        let Some(ns_window) = ns_window(window) else {
            return;
        };
        let fullscreen = window.is_fullscreen();
        // In full screen macOS shows and places them itself.
        let alpha: f64 = if fullscreen || x.is_some() { 1. } else { 0. };
        let x = f64::from(x.unwrap_or(AWAY));
        // SAFETY: the window's own three standard buttons (NSWindowButton 0, 1 and 2) and
        // plain geometry getters, on the main thread.
        unsafe {
            let mut buttons = [std::ptr::null_mut::<Object>(); 3];
            for (kind, button) in buttons.iter_mut().enumerate() {
                *button = msg_send![ns_window, standardWindowButton: kind];
                if button.is_null() {
                    return;
                }
                let now: f64 = msg_send![*button, alphaValue];
                if (now - alpha).abs() > 0.01 {
                    let _: () = msg_send![*button, setAlphaValue: alpha];
                }
            }
            if fullscreen {
                return;
            }
            let frames = buttons.map(|button| {
                let frame: Frame = msg_send![button, frame];
                frame
            });
            // The stock spacing, as first seen, before anything here has moved them.
            let spacing = SPACING.with(|spacing| match spacing.get() {
                Some(spacing) => spacing,
                None => {
                    let found = frames[1].origin.x - frames[0].origin.x;
                    spacing.set(Some(found));
                    found
                }
            });
            let outer: Frame = msg_send![ns_window, frame];
            let inner: Frame = msg_send![ns_window, contentLayoutRect];
            let y = outer.height - inner.height - LIGHTS_TOP - frames[0].height;
            let to = |kind: usize| Point {
                x: x + spacing * kind as f64,
                y,
            };
            let off = (0..3).any(|kind| {
                let (at, to) = (frames[kind].origin, to(kind));
                (at.x - to.x).abs() > 0.1 || (at.y - to.y).abs() > 0.1
            });
            if off {
                for (kind, button) in buttons.into_iter().enumerate() {
                    let frame = Frame {
                        origin: to(kind),
                        ..frames[kind]
                    };
                    let _: () = msg_send![button, setFrame: frame];
                }
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::Pointer;

    pub fn pointer(window: &gpui::Window, _near: f32) -> Option<Pointer> {
        let at = window.mouse_position();
        window.is_window_active().then(|| Pointer {
            x: f32::from(at.x),
            level: at.y >= gpui::px(0.) && at.y <= window.viewport_size().height,
            pressed: false,
        })
    }

    pub fn place_traffic_lights(_window: &gpui::Window, _x: Option<f32>) {}
}

pub use platform::{place_traffic_lights, pointer};

#[cfg(test)]
mod tests {
    use super::*;

    const RIGHT: f32 = 242.;

    fn at(x: f32) -> Option<Pointer> {
        Some(Pointer {
            x,
            level: true,
            pressed: false,
        })
    }

    fn shown() -> Peek {
        let mut peek = Peek::new();
        peek.look(at(300.), RIGHT, false);
        assert!(peek.look(at(2.), RIGHT, false));
        peek
    }

    #[test]
    fn arriving_at_the_edge_brings_it_in() {
        for (from, to) in [(300., 0.), (300., 6.), (300., -30.), (-90., 3.), (-20., 1.)] {
            let mut peek = Peek::new();
            peek.look(at(from), RIGHT, false);
            assert!(peek.look(at(to), RIGHT, false), "{from} to {to}");
        }
        for (from, to) in [(300., 7.), (300., -49.), (-90., -20.), (3., 1.)] {
            let mut peek = Peek::new();
            peek.look(at(from), RIGHT, false);
            assert!(!peek.look(at(to), RIGHT, false), "{from} to {to}");
        }
    }

    #[test]
    fn resting_dragging_and_other_windows_do_not() {
        // Already at the edge when it was hidden, or when the window became active.
        let mut peek = shown();
        peek.reset();
        assert!(!peek.look(at(2.), RIGHT, false));
        let mut peek = Peek::new();
        peek.look(None, RIGHT, false);
        assert!(!peek.look(at(2.), RIGHT, false));

        let mut peek = Peek::new();
        peek.look(at(300.), RIGHT, false);
        let drag = at(2.).map(|p| Pointer { pressed: true, ..p });
        assert!(!peek.look(drag, RIGHT, false));
        let above = at(1.).map(|p| Pointer { level: false, ..p });
        assert!(!peek.look(above, RIGHT, false));
    }

    #[test]
    fn it_leaves_with_the_pointer() {
        for stay in [0., 120., RIGHT + SLACK, -OVERSHOOT] {
            assert!(!shown().look(at(stay), RIGHT, false), "{stay}");
        }
        for leave in [RIGHT + SLACK + 1., 900., -OVERSHOOT - 1.] {
            assert!(shown().look(at(leave), RIGHT, false), "{leave}");
        }
        assert!(shown().look(None, RIGHT, false));
        // An open menu or a held button keeps it.
        assert!(!shown().look(None, RIGHT, true));
        let drag = at(900.).map(|p| Pointer { pressed: true, ..p });
        assert!(!shown().look(drag, RIGHT, false));
    }

    #[test]
    fn the_spring_overshoots_a_little_and_lands() {
        assert!(spring(0.).abs() < 1e-5 && (spring(1.) - 1.).abs() < 1e-5);
        let peak = (0..=100)
            .map(|i| spring(i as f32 / 100.))
            .fold(0., f32::max);
        assert!(peak > 1.01 && peak < 1.03, "{peak}");
    }
}
