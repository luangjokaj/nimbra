//! The eclipse: the day's way to Inbox Zero, drawn as the app icon's sun with the moon
//! across it. The moon covers as much of the sun as the day's mail is done; at zero it is
//! total, and the sun is only a ring of corona. It is drawn from state alone and never
//! moves by itself.

use gpui::{
    Bounds, Div, canvas, div, fill, linear_color_stop, linear_gradient, point, prelude::*, px, size,
};

use crate::theme::{Theme, c};
use crate::ui::shadow;

/// The sun's light, top left to bottom right, the same in both themes.
pub const SUN: (u32, u32) = (0xffb36b, 0xe0569a);

/// How much of the sun the moon covers when `done` of `done + left` is done.
pub fn covered(done: usize, left: usize) -> f32 {
    match (done, left) {
        (_, 0) => 1.,
        (done, left) => done as f32 / (done + left) as f32,
    }
}

/// The eclipse `size` across, `covered` (0 to 1) of the way to totality. `glow` is how far
/// its corona reaches past the disc.
pub fn eclipse(t: &Theme, size: f32, covered: f32, glow: f32) -> Div {
    let covered = covered.clamp(0., 1.);
    let moon = t.moon;
    let halo = if t.dark {
        c(0xff7aa0, 0.45)
    } else {
        c(0xe0569a, 0.32)
    };
    let sun = div()
        .absolute()
        .inset_0()
        .rounded_full()
        .bg(linear_gradient(
            135.,
            linear_color_stop(c(SUN.0, 1.), 0.),
            linear_color_stop(c(SUN.1, 1.), 1.),
        ))
        .shadow(vec![shadow(halo, 0., glow, 0.)]);
    let body = if covered >= 1. {
        // Totality: the moon just inside the sun, leaving a ring of corona.
        let rim = (size * 0.045).max(1.5);
        div()
            .absolute()
            .top(px(rim))
            .left(px(rim))
            .size(px(size - 2. * rim))
            .rounded_full()
            .bg(moon)
            .into_any_element()
    } else {
        // Partial: the moon, as wide as the sun, set off to the right by what is left. GPUI
        // clips to rectangles only, so the moon is drawn a line of pixels at a time, each
        // cut to the sun's chord on that line.
        let offset = (1. - covered) * size;
        canvas(
            |_, _, _| {},
            move |bounds: Bounds<gpui::Pixels>, _, window, _| {
                let scale = window.scale_factor();
                let r = size / 2.;
                let (left, top) = (f32::from(bounds.left()), f32::from(bounds.top()));
                let rows = (size * scale).ceil() as usize;
                for row in 0..rows {
                    let y = (row as f32 + 0.5) / scale;
                    let dy = y - r;
                    if dy.abs() >= r {
                        continue;
                    }
                    let half = (r * r - dy * dy).sqrt();
                    let (from, to) = (r - half + offset, r + half);
                    if from >= to {
                        continue;
                    }
                    let line = Bounds::new(
                        point(px(left + from), px(top + row as f32 / scale)),
                        size_of(to - from, 1. / scale),
                    );
                    window.paint_quad(fill(line, moon));
                }
            },
        )
        .absolute()
        .inset_0()
        .into_any_element()
    };
    div()
        .relative()
        .flex_none()
        .size(px(size))
        .child(sun)
        .child(body)
}

fn size_of(width: f32, height: f32) -> gpui::Size<gpui::Pixels> {
    size(px(width), px(height))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_left_is_totality() {
        assert_eq!(covered(0, 0), 1.);
        assert_eq!(covered(12, 0), 1.);
    }

    #[test]
    fn the_moon_covers_what_is_done() {
        assert_eq!(covered(0, 10), 0.);
        assert!((covered(16, 10) - 16. / 26.).abs() < 1e-6);
    }
}
