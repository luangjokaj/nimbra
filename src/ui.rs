//! Small stateless building blocks shared by every pane.

use gpui::{
    AnimationExt, BoxShadow, Div, FontWeight, Hsla, Pixels, Rems, SharedString, Stateful, Svg,
    Window, div, point, prelude::*, px, rems, svg,
};

// ------------------------------------------------------------------ interface zoom
//
// The interface is laid out in design points: its size at 100%. Every window draws them
// through GPUI's rem (`apply_zoom` sets it to 16 design points at the zoom), so sizes made
// with `dp` grow and shrink with Settings › Appearance › Interface size. What arrives from
// the screen (pointer positions, measured bounds) is in screen points: `from_screen` turns
// it into design points, and `to_screen` turns design points into what takes only screen
// points (window sizes, scroll offsets, shadows).

/// The interface's scale, as `f32` bits (1.0 until the settings say otherwise).
static ZOOM: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x3f80_0000);

/// The interface's scale (1 is its own size).
pub fn zoom() -> f32 {
    f32::from_bits(ZOOM.load(std::sync::atomic::Ordering::Relaxed))
}

/// Sets the scale every window draws at from its next frame.
pub fn set_zoom(zoom: f32) {
    ZOOM.store(zoom.to_bits(), std::sync::atomic::Ordering::Relaxed);
}

/// A size in design points: `n` points at 100%, scaled with the interface zoom.
pub fn dp(n: f32) -> Rems {
    rems(n / 16.)
}

/// Design points as screen points, for what takes only those.
pub fn to_screen(n: f32) -> Pixels {
    px(n * zoom())
}

/// Screen points (a pointer's position, measured bounds) as design points.
pub fn from_screen(p: Pixels) -> f32 {
    f32::from(p) / zoom()
}

/// Draws this window at the interface zoom: called first in each window's `render`.
pub fn apply_zoom(window: &mut Window) {
    window.set_rem_size(px(16. * zoom()));
}

use crate::theme::{MONO, Theme, Tone, c, gradient};

pub fn icon(name: &str, size: f32, color: Hsla) -> Svg {
    svg()
        .path(SharedString::from(format!("icons/{name}.svg")))
        .size(dp(size))
        .flex_none()
        .text_color(color)
}

/// A window's top bar with its 1pt bottom border. The 50pt above the border centre what is in
/// it on y = 25, the line of the traffic lights, with the same space above and below.
pub const TOP_BAR: f32 = 51.;

/// `TOP_BAR` in design points: the traffic lights don't zoom, so the bar they line up with
/// keeps its 51 screen points whatever the interface zoom.
pub fn top_bar() -> f32 {
    TOP_BAR / zoom()
}

pub fn mono(text: impl Into<SharedString>, color: Hsla) -> Div {
    div()
        .font_family(MONO)
        .font_weight(FontWeight::MEDIUM)
        .text_size(dp(11.))
        .line_height(dp(14.))
        .text_color(color)
        .flex_none()
        .child(text.into())
}

pub fn overline(text: impl Into<SharedString>, color: Hsla) -> Div {
    div()
        .font_family(MONO)
        .font_weight(FontWeight::MEDIUM)
        .text_size(dp(10.))
        .line_height(dp(12.))
        .text_color(color)
        .child(text.into())
}

pub fn meta(text: impl Into<SharedString>, color: Hsla) -> Div {
    div()
        .text_size(dp(12.))
        .line_height(dp(16.))
        .text_color(color)
        .child(text.into())
}

pub fn kbd(t: &Theme, key: &str) -> Div {
    div()
        .flex()
        .flex_none()
        .justify_center()
        .min_w(dp(18.))
        .px(dp(5.))
        .py(dp(2.))
        .rounded(dp(5.))
        .bg(t.kbd_bg)
        .border_1()
        .border_color(t.kbd_border)
        .child(mono(key.to_string(), t.text2))
}

pub fn avatar(tone: Tone, initials: &str, size: f32) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(dp(size))
        .rounded_full()
        .bg(tone.gradient())
        .border_1()
        .border_color(c(0xffffff, 0.18))
        .text_size(dp(size * 0.37))
        .line_height(dp(size * 0.5))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(gpui::white())
        .child(initials.to_string())
}

pub fn pill(t: &Theme, label: &str, dot: Hsla) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(dp(6.))
        .pl(dp(7.))
        .pr(dp(9.))
        .py(dp(2.))
        .rounded_full()
        .bg(t.hover)
        .border_1()
        .border_color(t.border)
        .child(div().size(dp(6.)).rounded_full().bg(dot))
        .child(meta(label.to_string(), t.text2))
}

/// A square icon button. On hover the background lights up and the icon brightens; give it
/// an id and `.press(t)` for the pressed state.
pub fn icon_button(t: &Theme, name: &str) -> Div {
    let (hover, bright) = (t.hover, t.text);
    div()
        .group("icon-button")
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(dp(30.))
        .rounded(dp(8.))
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .child(icon(name, 16., t.text2).group_hover("icon-button", move |s| s.text_color(bright)))
}

/// The pressed look for anything clickable, so a click is felt the moment it lands.
pub trait Press {
    /// Icon buttons, rows and menu items: a firmer background while held.
    fn press(self, t: &Theme) -> Self;
    /// Text buttons (their own colours stay): dimmed a little while held.
    fn press_soft(self) -> Self;
}

impl Press for Stateful<Div> {
    fn press(self, t: &Theme) -> Self {
        let held = t.selected;
        self.active(move |s| s.bg(held))
    }

    fn press_soft(self) -> Self {
        self.active(|s| s.opacity(0.72))
    }
}

pub fn shadow(color: Hsla, y: f32, blur: f32, spread: f32) -> BoxShadow {
    BoxShadow {
        color,
        offset: point(px(0.), to_screen(y)),
        blur_radius: to_screen(blur),
        spread_radius: to_screen(spread),
    }
}

pub enum ButtonKind {
    Primary,
    Secondary,
    Ghost,
}

pub fn button(t: &Theme, kind: ButtonKind, label: &str, key: Option<&str>) -> Div {
    let base = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(dp(8.))
        .pl(dp(14.))
        .pr(dp(if key.is_some() { 10. } else { 14. }))
        .py(dp(6.))
        .rounded(dp(9.))
        .cursor_pointer()
        .font_weight(FontWeight::SEMIBOLD);
    let (hover, strong, selected) = (t.hover, t.border_strong, t.selected);
    match kind {
        ButtonKind::Primary => base
            // Brighter on hover, eased in and out by `glow`.
            .bg(primary_fill(0.))
            .text_color(gpui::white())
            .child(label.to_string())
            .children(key.map(|k| {
                div()
                    .px(dp(5.))
                    .py(dp(2.))
                    .rounded(dp(5.))
                    .bg(c(0xffffff, 0.18))
                    .child(mono(k.to_string(), gpui::white()))
            })),
        ButtonKind::Secondary => base
            .bg(t.hover)
            .border_1()
            .border_color(t.border_strong)
            .hover(move |s| s.bg(selected).border_color(strong))
            .text_color(t.text)
            .child(label.to_string())
            .children(key.map(|k| kbd(t, k))),
        ButtonKind::Ghost => base
            .hover(move |s| s.bg(hover))
            .text_color(t.text)
            .child(label.to_string())
            .children(key.map(|k| kbd(t, k))),
    }
}

/// The primary button's gradient, from at rest (0) to hovered (1): brighter on hover,
/// inside the button. (A glow around it would spill onto whatever is behind.)
pub fn primary_fill(level: f32) -> gpui::Background {
    let mix = |rest: u32, hover: u32| {
        let (a, b) = (gpui::rgb(rest), gpui::rgb(hover));
        let lerp = |x: f32, y: f32| x + (y - x) * level;
        gpui::Hsla::from(gpui::Rgba {
            r: lerp(a.r, b.r),
            g: lerp(a.g, b.g),
            b: lerp(a.b, b.b),
            a: 1.,
        })
    };
    gradient(90., mix(0x8b7cff, 0xa397ff), mix(0xff7ab6, 0xff95c6))
}

/// Each primary button's glow, by key: (hovered, times it changed, level shown last).
#[derive(Clone, Copy, Default)]
struct Glow {
    hovered: bool,
    turns: usize,
    level: f32,
}

thread_local! {
    static GLOWS: std::cell::RefCell<std::collections::HashMap<&'static str, Glow>> =
        Default::default();
}

const GLOW_TIME: std::time::Duration = std::time::Duration::from_millis(220);

/// Tells a primary button's glow whether the pointer is on it. For a button that has its
/// own hover listener (one per element); others use `glow`, which listens itself.
pub fn glow_hovered(key: &'static str, hovered: bool, window: &mut gpui::Window) {
    GLOWS.with_borrow_mut(|glows| {
        let glow = glows.entry(key).or_default();
        if glow.hovered != hovered {
            glow.hovered = hovered;
            glow.turns += 1;
        }
    });
    window.refresh();
}

/// A primary button whose glow brightens and fades over a moment instead of jumping, from
/// wherever it is when the pointer comes or goes. `key` names it for its whole life.
pub fn glow(
    button: Stateful<Div>,
    key: &'static str,
    listen: bool,
) -> gpui::AnimationElement<Stateful<Div>> {
    let now = GLOWS.with_borrow(|glows| glows.get(key).copied().unwrap_or_default());
    let to = if now.hovered { 1. } else { 0. };
    let from = now.level;
    let button = if listen {
        button.on_hover(move |hovered, window, _| glow_hovered(key, *hovered, window))
    } else {
        button
    };
    // A button gone while hovered (Send closes the message) is not hovered when it is back:
    // whenever it is drawn, where the pointer really is wins.
    let button = button.relative().child(
        gpui::canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                let hovered = now.hovered;
                if hovered && !bounds.contains(&window.mouse_position()) {
                    GLOWS.with_borrow_mut(|glows| {
                        let glow = glows.entry(key).or_default();
                        // At rest at once: nobody saw it leave.
                        *glow = Glow {
                            turns: glow.turns + 1,
                            ..Glow::default()
                        };
                    });
                    window.refresh();
                }
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full(),
    );
    button.with_animation(
        (key, now.turns),
        gpui::Animation::new(GLOW_TIME).with_easing(|t| 1. - (1. - t).powi(3)),
        move |button, delta| {
            let level = from + (to - from) * delta;
            GLOWS.with_borrow_mut(|glows| glows.entry(key).or_default().level = level);
            button.bg(primary_fill(level))
        },
    )
}

/// Builds the tooltip for a button. Pass an empty `key` when there is no shortcut.
pub fn tip(
    dark: bool,
    label: impl Into<SharedString>,
    key: &'static str,
) -> impl Fn(&mut gpui::Window, &mut gpui::App) -> gpui::AnyView + 'static {
    let label = label.into();
    move |window, cx| crate::tooltip::build(dark, label.clone(), key, window, cx)
}
