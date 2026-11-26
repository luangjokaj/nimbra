//! The main window's place and size, kept across launches next to the settings. Saved off
//! the main thread; brought back only onto the display it was on, if that is there now.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// A rectangle in points, top-left origin on its display, as GPUI places windows.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Frame {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    #[serde(default)]
    pub maximized: bool,
}

impl Frame {
    fn overlap(&self, other: &Frame) -> f32 {
        let width = (self.x + self.width).min(other.x + other.width) - self.x.max(other.x);
        let height = (self.y + self.height).min(other.y + other.height) - self.y.max(other.y);
        width.max(0.) * height.max(0.)
    }
}

/// The share of a saved window that must be on a display for it to open there.
const SEEN: f32 = 0.25;

/// The saved frame moved and shrunk to fit the display it is most on, or None when it is
/// on none of them (that display is gone): the caller places it afresh.
pub fn clamp(saved: Frame, displays: &[Frame], min: (f32, f32)) -> Option<Frame> {
    if !(saved.x.is_finite()
        && saved.y.is_finite()
        && saved.width.is_finite()
        && saved.height.is_finite()
        && saved.width > 0.
        && saved.height > 0.)
    {
        return None;
    }
    let display = displays
        .iter()
        .map(|display| (saved.overlap(display), display))
        .filter(|(seen, _)| *seen >= SEEN * saved.width * saved.height)
        .max_by(|a, b| a.0.total_cmp(&b.0))?
        .1;
    let width = saved.width.max(min.0).min(display.width);
    let height = saved.height.max(min.1).min(display.height);
    let x = saved.x.clamp(display.x, display.x + display.width - width);
    let y = saved
        .y
        .clamp(display.y, display.y + display.height - height);
    Some(Frame {
        x,
        y,
        width,
        height,
        maximized: saved.maximized,
    })
}

fn path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join("Library/Application Support/Nimbra/window.json"))
}

/// Test copies and screenshots open where they are told and leave the real one alone.
pub fn kept() -> bool {
    std::env::var_os("NIMBRA_DEMO").is_none()
        && std::env::var_os("NIMBRA_SCREEN").is_none()
        && std::env::var_os("NIMBRA_SHIFT").is_none()
}

/// The frame, and the display it is on (GPUI's stable id for it).
#[derive(Serialize, Deserialize)]
pub struct Saved {
    pub frame: Frame,
    pub display: String,
}

pub fn load() -> Option<Saved> {
    if !kept() {
        return None;
    }
    let bytes = std::fs::read(path()?).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Blocking: call it off the main thread.
pub fn save(saved: Saved) {
    if !kept() {
        return;
    }
    let Some(path) = path() else {
        return;
    };
    let partial = path.with_extension("json.partial");
    let written = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| serde_json::to_vec(&saved).map_err(std::io::Error::other))
        .and_then(|json| std::fs::write(&partial, json))
        .and_then(|()| std::fs::rename(&partial, &path));
    if written.is_err() {
        eprintln!("nimbra: could not save the window's place");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(x: f32, y: f32, width: f32, height: f32) -> Frame {
        Frame {
            x,
            y,
            width,
            height,
            maximized: false,
        }
    }

    const MIN: (f32, f32) = (1120., 720.);

    #[test]
    fn a_frame_that_fits_stays_as_it_was() {
        let display = frame(0., 0., 1920., 1080.);
        let saved = frame(100., 50., 1480., 920.);
        assert_eq!(clamp(saved, &[display], MIN), Some(saved));
    }

    #[test]
    fn a_frame_hanging_off_the_edge_is_moved_back_on() {
        let display = frame(0., 0., 1920., 1080.);
        let moved = clamp(frame(1000., 600., 1480., 920.), &[display], MIN).unwrap();
        assert_eq!(moved, frame(440., 160., 1480., 920.));
        let above = clamp(frame(-300., -40., 1480., 920.), &[display], MIN).unwrap();
        assert_eq!((above.x, above.y), (0., 0.));
    }

    #[test]
    fn a_frame_larger_than_its_display_shrinks_to_it() {
        let display = frame(0., 0., 1280., 800.);
        let fitted = clamp(frame(0., 0., 2560., 1440.), &[display], MIN).unwrap();
        assert_eq!(fitted, frame(0., 0., 1280., 800.));
    }

    #[test]
    fn a_frame_never_goes_below_the_minimum_size_unless_the_display_is_smaller() {
        let display = frame(0., 0., 1920., 1080.);
        let grown = clamp(frame(10., 10., 400., 300.), &[display], MIN).unwrap();
        assert_eq!((grown.width, grown.height), MIN);
        let small = frame(0., 0., 1024., 640.);
        let fitted = clamp(frame(0., 0., 400., 300.), &[small], MIN).unwrap();
        assert_eq!((fitted.width, fitted.height), (1024., 640.));
    }

    #[test]
    fn a_frame_on_a_display_that_is_gone_is_placed_afresh() {
        let display = frame(0., 0., 1920., 1080.);
        assert_eq!(clamp(frame(2000., 0., 1480., 920.), &[display], MIN), None);
        // A sliver left on screen is not enough to find it by.
        assert_eq!(clamp(frame(1900., 0., 1480., 920.), &[display], MIN), None);
        assert_eq!(clamp(frame(0., 0., 1480., 920.), &[], MIN), None);
    }

    #[test]
    fn a_frame_goes_to_the_display_it_is_most_on() {
        let left = frame(0., 0., 1920., 1080.);
        let right = frame(1920., -200., 2560., 1440.);
        let saved = frame(1700., 100., 1480., 920.);
        let placed = clamp(saved, &[left, right], MIN).unwrap();
        assert_eq!(placed, frame(1920., 100., 1480., 920.));
    }

    #[test]
    fn a_broken_file_does_not_place_a_window() {
        let display = frame(0., 0., 1920., 1080.);
        assert_eq!(
            clamp(frame(f32::NAN, 0., 1480., 920.), &[display], MIN),
            None
        );
        assert_eq!(
            clamp(frame(0., 0., f32::INFINITY, 920.), &[display], MIN),
            None
        );
    }

    /// AppKit's frames: bottom-left origin at the primary display's bottom-left, y upwards.
    #[derive(Clone, Copy, Debug, PartialEq)]
    struct Cocoa {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    }

    /// What GPUI 0.2.2 reports for a window (`MacWindow::bounds`, platform/mac/window.rs).
    fn gpui_bounds(window: Cocoa, screen: Cocoa) -> Frame {
        let flipped = screen.height - window.y - window.height;
        frame(
            window.x - screen.x,
            flipped + screen.y,
            window.width,
            window.height,
        )
    }

    /// Where GPUI 0.2.2 opens a window it is given `bounds` and a display for
    /// (`MacWindow::open`, then `setFrameTopLeftPoint`).
    fn gpui_open(bounds: Frame, screen: Cocoa) -> Cocoa {
        let top = screen.y + (screen.height - bounds.y);
        Cocoa {
            x: screen.x + bounds.x,
            y: top - bounds.height,
            width: bounds.width,
            height: bounds.height,
        }
    }

    #[test]
    fn a_frame_on_a_display_above_or_below_the_main_one_opens_where_it_was() {
        for screen in [
            Cocoa {
                x: 0.,
                y: 1080.,
                width: 2560.,
                height: 1440.,
            },
            Cocoa {
                x: -400.,
                y: -900.,
                width: 1920.,
                height: 900.,
            },
        ] {
            let window = Cocoa {
                x: screen.x + 300.,
                y: screen.y + 200.,
                width: 1200.,
                height: 600.,
            };
            let saved = gpui_bounds(window, screen);
            assert_eq!((saved.x, saved.y), (300., screen.height - 200. - 600.));
            // Clamped against the display as `main` does: its size, at (0, 0).
            let whole = frame(0., 0., screen.width, screen.height);
            let placed = clamp(saved, &[whole], (400., 300.)).unwrap();
            assert_eq!(placed, saved);
            assert_eq!(gpui_open(placed, screen), window);
        }
    }

    #[test]
    fn being_maximized_is_kept() {
        let display = frame(0., 0., 1920., 1080.);
        let saved = Frame {
            maximized: true,
            ..frame(100., 50., 1480., 920.)
        };
        assert!(clamp(saved, &[display], MIN).unwrap().maximized);
    }
}
