//! The size of a notice's toast. It is a window of its own, so this has to be known before
//! it opens: the text is measured as it will be wrapped.

use gpui::{SharedString, TextRun, WindowTextSystem, px};

pub const LINE: f32 = 18.;
pub const FONT: f32 = 13.;
pub const GAP: f32 = 16.;
pub const LEFT: f32 = 16.;
pub const RIGHT: f32 = 8.;
pub const DISMISS: (f32, f32) = (84., 32.);
const PAD_Y: f32 = 12.;
const BORDER: f32 = 1.;
const WIDEST: f32 = 620.;
const LEAST_TEXT: f32 = 120.;
/// Longer than this ends in an ellipsis: the toast must not cover the mail it is about.
const MOST_LINES: usize = 6;
const MOST_CHARS: usize = 1000;

#[derive(Clone)]
pub struct Toast {
    pub text: SharedString,
    pub text_width: f32,
    pub width: f32,
    pub height: f32,
}

impl Toast {
    /// `room` is how wide the window under it is.
    pub fn new(text: &str, room: f32, text_system: &WindowTextSystem) -> Self {
        let chrome = LEFT + GAP + DISMISS.0 + RIGHT + 2. * BORDER;
        let wrap = ((room - 48.).min(WIDEST) - chrome).max(LEAST_TEXT);
        let measure = |text: &str| -> (usize, f32) {
            let run = TextRun {
                len: text.len(),
                font: gpui::font(crate::theme::SANS),
                color: gpui::black(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let shaped = text_system.shape_text(
                text.to_string().into(),
                px(FONT),
                &[run],
                Some(px(wrap)),
                None,
            );
            let Ok(shaped) = shaped else {
                return (1, wrap);
            };
            shaped
                .iter()
                .fold((0, 0.), |(lines, widest): (usize, f32), line| {
                    (
                        lines + line.wrap_boundaries().len() + 1,
                        widest.max(line.width().into()),
                    )
                })
        };
        let (text, lines, widest) = fit(text.trim(), measure);
        let text_width = (widest.ceil() + 1.).clamp(LEAST_TEXT, wrap);
        Self {
            text: text.into(),
            text_width,
            width: text_width + chrome,
            height: (lines as f32 * LINE).max(DISMISS.1) + 2. * (PAD_Y + BORDER),
        }
    }
}

/// As much of the text as fits in `MOST_LINES`, with the lines it takes and the widest one.
fn fit(text: &str, measure: impl Fn(&str) -> (usize, f32)) -> (String, usize, f32) {
    let chars: Vec<char> = text.chars().collect();
    let mut kept = chars.len().min(MOST_CHARS);
    loop {
        let mut shown: String = chars[..kept].iter().collect();
        if kept < chars.len() {
            shown = format!("{}…", shown.trim_end());
        }
        let (lines, widest) = measure(&shown);
        if lines <= MOST_LINES || kept <= 1 {
            return (shown, lines, widest);
        }
        kept = kept * 19 / 20;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ten_a_line(text: &str) -> (usize, f32) {
        let chars = text.chars().count();
        (chars.div_ceil(10).max(1), chars.min(10) as f32)
    }

    #[test]
    fn what_fits_is_shown_whole_and_the_rest_ends_in_an_ellipsis() {
        assert_eq!(fit("Message sent.", ten_a_line).0, "Message sent.");
        let full = "é".repeat(MOST_LINES * 10);
        assert_eq!(fit(&full, ten_a_line).0, full);
        let (shown, lines, _) = fit(&"é".repeat(5000), ten_a_line);
        assert!(shown.ends_with('…') && lines == MOST_LINES);
    }
}
