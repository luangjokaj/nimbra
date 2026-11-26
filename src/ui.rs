//! Small stateless building blocks shared by every pane.

use gpui::{
    AnimationExt, BoxShadow, Div, FontWeight, Hsla, SharedString, Stateful, Svg, div, point,
    prelude::*, px, svg,
};

use crate::theme::{
    GRADIENT_GLOW, GRADIENT_PRIMARY, GRADIENT_PRIMARY_PRESSED, MONO, SANS, Theme, Tone, c, over,
    radius, space,
};

pub fn icon(name: &str, size: f32, color: Hsla) -> Svg {
    svg()
        .path(SharedString::from(format!("icons/{name}.svg")))
        .size(px(size))
        .flex_none()
        .text_color(color)
}

/// A window's top bar with its 1pt bottom border. The 50pt above the border centre what is in
/// it on y = 25, the line of the traffic lights, with the same space above and below.
pub const TOP_BAR: f32 = 51.;

/// Timestamps, counts and keys: Geist Mono 11/14 medium.
pub fn caption(text: impl Into<SharedString>, color: Hsla) -> Div {
    div()
        .font_family(MONO)
        .font_weight(FontWeight::MEDIUM)
        .text_size(px(11.))
        .line_height(px(14.))
        .text_color(color)
        .flex_none()
        .child(text.into())
}

/// A section head: Geist Mono 10/12 medium in `text3`.
pub fn overline(t: &Theme, text: impl Into<SharedString>) -> Div {
    div()
        .font_family(MONO)
        .font_weight(FontWeight::MEDIUM)
        .text_size(px(10.))
        .line_height(px(12.))
        .text_color(t.text3)
        .child(text.into())
}

/// A secondary line: 12/16.
pub fn meta(text: impl Into<SharedString>, color: Hsla) -> Div {
    div()
        .text_size(px(12.))
        .line_height(px(16.))
        .text_color(color)
        .child(text.into())
}

/// The weight a display title takes at `size`: lighter as it grows, so large type reads as
/// light rather than as bulk.
pub fn display_weight(size: f32) -> FontWeight {
    if size >= 40. {
        FontWeight::LIGHT
    } else if size >= 26. {
        FontWeight::NORMAL
    } else {
        FontWeight::MEDIUM
    }
}

fn display_set(text: impl Into<SharedString>, size: f32, line: f32, color: Hsla) -> Div {
    div()
        .font_family(SANS)
        .font_weight(display_weight(size))
        .text_size(px(size))
        .line_height(px(line))
        .text_color(color)
        .child(text.into())
}

/// Sora as a display face at any size. Prefer the named sizes below.
pub fn display(text: impl Into<SharedString>, size: f32, color: Hsla) -> Div {
    display_set(text, size, (size * 1.2).round(), color)
}

/// Sheets, confirmations, the shortcuts list, a settings tab: 22/26.
pub fn title_s(text: impl Into<SharedString>, color: Hsla) -> Div {
    display(text, 22., color)
}

/// The reader's subject: 30/36.
pub fn title_m(text: impl Into<SharedString>, color: Hsla) -> Div {
    display(text, 30., color)
}

/// Empty and status screens: 40/48.
pub fn title_l(text: impl Into<SharedString>, color: Hsla) -> Div {
    display(text, 40., color)
}

/// Inbox zero only: 96/104.
pub fn hero(text: impl Into<SharedString>, color: Hsla) -> Div {
    display_set(text, 96., 104., color)
}

/// A key as a chip. Only where keys are the content: the `?` sheet and Settings › Shortcuts.
pub fn kbd(t: &Theme, key: &str) -> Div {
    div()
        .flex()
        .flex_none()
        .justify_center()
        .min_w(px(18.))
        .px(px(5.))
        .py(px(2.))
        .rounded(px(radius::CHIP))
        .bg(t.kbd_bg)
        .border_1()
        .border_color(t.kbd_border)
        .child(caption(key.to_string(), t.text2))
}

pub fn avatar(tone: Tone, initials: &str, size: f32) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(size))
        .rounded_full()
        .bg(tone.gradient())
        .border_1()
        .border_color(c(0xffffff, 0.18))
        .text_size(px(size * 0.37))
        .line_height(px(size * 0.5))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(gpui::white())
        .child(initials.to_string())
}

pub fn pill(t: &Theme, label: &str, dot: Hsla) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.))
        .pl(px(7.))
        .pr(px(9.))
        .py(px(2.))
        .rounded_full()
        .bg(t.hover)
        .border_1()
        .border_color(t.border)
        .child(div().size(px(6.)).rounded_full().bg(dot))
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
        .size(px(30.))
        .rounded(px(radius::CONTROL))
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .child(icon(name, 16., t.text2).group_hover("icon-button", move |s| s.text_color(bright)))
}

/// The pressed look for anything clickable, so a click is felt the moment it lands.
pub trait Press {
    /// Buttons, icon buttons, rows and menu items: `pressed` while held.
    fn press(self, t: &Theme) -> Self;
    /// A selected or picked row: its selection stays visible under the press.
    fn press_selected(self, t: &Theme) -> Self;
}

impl Press for Stateful<Div> {
    fn press(self, t: &Theme) -> Self {
        let held = t.pressed;
        self.active(move |s| s.bg(held))
    }

    fn press_selected(self, t: &Theme) -> Self {
        let held = over(t.pressed, t.selected);
        self.active(move |s| s.bg(held))
    }
}

/// Keyboard focus as a 1pt ring laid over its element (which needs `.relative()`), so
/// gaining or losing focus never moves anything.
pub fn focus_ring(t: &Theme, corner: f32) -> Div {
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .rounded(px(corner))
        .border_1()
        .border_color(t.focus)
}

pub fn shadow(color: Hsla, y: f32, blur: f32, spread: f32) -> BoxShadow {
    BoxShadow {
        color,
        offset: point(px(0.), px(y)),
        blur_radius: px(blur),
        spread_radius: px(spread),
    }
}

/// Depth as soft light: a shadow for lift, plus accent light where the eye should go.
/// Pair a raised surface with a `t.edge` hairline border.
#[derive(Clone, Copy)]
pub enum Elevation {
    /// Menus, tooltips, toasts.
    Pop,
    /// Palette, finder, confirm, shortcuts, composer: lifted, with a faint violet aura.
    Sheet,
    /// The primary action or the focused field: accent light without lift.
    Glow,
}

pub fn elevation(t: &Theme, level: Elevation) -> Vec<BoxShadow> {
    match level {
        Elevation::Pop => vec![shadow(t.shadow, 6., 16., -4.)],
        Elevation::Sheet => vec![
            shadow(t.shadow_sheet, 24., 64., -16.),
            shadow(t.glow.opacity(0.4), 0., 48., -8.),
        ],
        Elevation::Glow => vec![shadow(t.glow, 4., 20., -4.)],
    }
}

pub enum ButtonKind {
    /// The one main action of a surface. Wrap it in `glow` for its hover and press.
    Primary,
    Secondary,
    Ghost,
}

/// A text button. Its key, if it has one, goes in its tooltip (`tip`).
pub fn button(t: &Theme, kind: ButtonKind, label: &str) -> Div {
    let base = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(space::S))
        .px(px(14.))
        .py(px(6.))
        .rounded(px(radius::CONTROL))
        .cursor_pointer()
        .font_weight(FontWeight::SEMIBOLD)
        .child(label.to_string());
    let (hover, strong, selected) = (t.hover, t.border_strong, t.selected);
    match kind {
        ButtonKind::Primary => base
            .bg(GRADIENT_PRIMARY.along(90.))
            .shadow(vec![glow_shadow(0.)])
            .text_color(gpui::white()),
        ButtonKind::Secondary => base
            .bg(t.hover)
            .border_1()
            .border_color(t.border_strong)
            .hover(move |s| s.bg(selected).border_color(strong))
            .text_color(t.text),
        ButtonKind::Ghost => base.hover(move |s| s.bg(hover)).text_color(t.text),
    }
}

/// The primary button's glow, from at rest (0) to hovered (1).
fn glow_shadow(level: f32) -> BoxShadow {
    let mix = |a: f32, b: f32| a + (b - a) * level;
    shadow(
        c(GRADIENT_GLOW.from, mix(0.35, 0.6)),
        mix(6., 8.),
        mix(18., 24.),
        mix(-4., -2.),
    )
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

fn glow_hovered(key: &'static str, hovered: bool, window: &mut gpui::Window) {
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
/// wherever it is when the pointer comes or goes, and that darkens while held. `key` names
/// it for its whole life.
pub fn glow(button: Stateful<Div>, key: &'static str) -> gpui::AnimationElement<Stateful<Div>> {
    let now = GLOWS.with_borrow(|glows| glows.get(key).copied().unwrap_or_default());
    let to = if now.hovered { 1. } else { 0. };
    let from = now.level;
    let button = button
        .active(|s| s.bg(GRADIENT_PRIMARY_PRESSED.along(90.)))
        .on_hover(move |hovered, window, _| glow_hovered(key, *hovered, window));
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
        gpui::Animation::new(motion::GLOW).with_easing(motion::ease_out),
        move |button, delta| {
            let level = from + (to - from) * delta;
            GLOWS.with_borrow_mut(|glows| glows.entry(key).or_default().level = level);
            button.shadow(vec![glow_shadow(level)])
        },
    )
}

/// Builds the tooltip for a button. Pass an empty `key` when there is no shortcut, and a
/// key only while it would do something.
pub fn tip(
    dark: bool,
    label: impl Into<SharedString>,
    key: &'static str,
) -> impl Fn(&mut gpui::Window, &mut gpui::App) -> gpui::AnyView + 'static {
    let label = label.into();
    move |window, cx| crate::tooltip::build(dark, label.clone(), key, window, cx)
}

/// Shared curves, durations and the one way state-driven motion is drawn.
///
/// Every transition is either a `Tween` (state-driven and interruptible: it restarts from
/// where it is) or GPUI's `with_animation` keyed by a generation number. Either way frames
/// are asked for only while something moves, and progress comes from real elapsed time.
/// Under Reduce Motion, travel (position, size, rotation) is instant; fades keep their time.
pub mod motion {
    use std::time::{Duration, Instant};

    pub const TIP: Duration = Duration::from_millis(150);
    /// A tooltip that opens this soon after another closed shows at once.
    pub const TIP_SKIP: Duration = Duration::from_millis(300);
    pub const MENU_IN: Duration = Duration::from_millis(160);
    pub const MENU_OUT: Duration = Duration::from_millis(110);
    pub const SLIDE: Duration = Duration::from_millis(240);
    pub const FOLD: Duration = Duration::from_millis(200);
    pub const PILL: Duration = Duration::from_millis(200);
    pub const SWITCH: Duration = Duration::from_millis(150);
    pub const TOAST_IN: Duration = Duration::from_millis(220);
    pub const TOAST_OUT: Duration = Duration::from_millis(150);
    pub const GLOW: Duration = Duration::from_millis(220);
    /// A crossfade between two states of the same thing.
    pub const FADE: Duration = Duration::from_millis(180);

    /// Enter and exit: `cubic-bezier(0.23, 1, 0.32, 1)`.
    pub fn ease_out(t: f32) -> f32 {
        cubic(0.23, 1., 0.32, 1., t)
    }

    /// Panels and drawers: `cubic-bezier(0.32, 0.72, 0, 1)`.
    pub fn drawer(t: f32) -> f32 {
        cubic(0.32, 0.72, 0., 1., t)
    }

    pub fn reduced() -> bool {
        crate::system::reduces_motion()
    }

    fn cubic(x1: f32, y1: f32, x2: f32, y2: f32, t: f32) -> f32 {
        if t <= 0. {
            return 0.;
        }
        if t >= 1. {
            return 1.;
        }
        // Each axis is a cubic in the curve parameter s: a*s^3 + b*s^2 + c*s.
        let coefficients = |p1: f32, p2: f32| {
            let c = 3. * p1;
            let b = 3. * (p2 - p1) - c;
            (1. - c - b, b, c)
        };
        let (ax, bx, cx) = coefficients(x1, x2);
        let (ay, by, cy) = coefficients(y1, y2);
        let x_at = |s: f32| ((ax * s + bx) * s + cx) * s;
        let slope_at = |s: f32| (3. * ax * s + 2. * bx) * s + cx;

        let mut s = t;
        for _ in 0..8 {
            let error = x_at(s) - t;
            if error.abs() < 1e-6 {
                return ((ay * s + by) * s + cy) * s;
            }
            let slope = slope_at(s);
            if slope.abs() < 1e-6 {
                break;
            }
            s -= error / slope;
        }
        let (mut lo, mut hi) = (0f32, 1f32);
        s = t;
        while hi - lo > 1e-6 {
            if x_at(s) < t {
                lo = s;
            } else {
                hi = s;
            }
            s = (lo + hi) / 2.;
        }
        ((ay * s + by) * s + cy) * s
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Kind {
        Fade,
        Travel,
    }

    /// A value easing toward a target. Retargeting starts from wherever it is now.
    #[derive(Clone, Copy)]
    pub struct Tween {
        from: f32,
        to: f32,
        start: Instant,
        length: Duration,
        curve: fn(f32) -> f32,
        kind: Kind,
    }

    impl Tween {
        /// Opacity or colour, at rest at `at`. Keeps its time under Reduce Motion.
        pub fn fade(at: f32, length: Duration, curve: fn(f32) -> f32) -> Self {
            Self::resting(at, length, curve, Kind::Fade)
        }

        /// Position, size or rotation, at rest at `at`. Instant under Reduce Motion.
        pub fn travel(at: f32, length: Duration, curve: fn(f32) -> f32) -> Self {
            Self::resting(at, length, curve, Kind::Travel)
        }

        fn resting(at: f32, length: Duration, curve: fn(f32) -> f32, kind: Kind) -> Self {
            Self {
                from: at,
                to: at,
                start: Instant::now(),
                length,
                curve,
                kind,
            }
        }

        pub fn value(&self) -> f32 {
            self.value_at(Instant::now())
        }

        /// Heads for `to` from the current value. Asking again for the same target changes
        /// nothing, so this can be called on every render.
        pub fn retarget(&mut self, to: f32) {
            self.retarget_at(to, Instant::now(), reduced());
        }

        /// At `to` now, without moving there.
        pub fn jump(&mut self, to: f32) {
            self.from = to;
            self.to = to;
        }

        /// The value to draw now. Asks for the next frame while it is still on its way.
        pub fn frame(&self, window: &mut gpui::Window) -> f32 {
            let now = Instant::now();
            if self.moving_at(now) {
                window.request_animation_frame();
            }
            self.value_at(now)
        }

        fn progress(&self, now: Instant) -> f32 {
            if self.length.is_zero() {
                return 1.;
            }
            (now.saturating_duration_since(self.start).as_secs_f32() / self.length.as_secs_f32())
                .min(1.)
        }

        fn value_at(&self, now: Instant) -> f32 {
            self.from + (self.to - self.from) * (self.curve)(self.progress(now))
        }

        fn moving_at(&self, now: Instant) -> bool {
            self.from != self.to && self.progress(now) < 1.
        }

        fn retarget_at(&mut self, to: f32, now: Instant, reduce: bool) {
            if to == self.to {
                return;
            }
            if reduce && self.kind == Kind::Travel {
                self.jump(to);
                return;
            }
            self.from = self.value_at(now);
            self.to = to;
            self.start = now;
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn curves_start_at_zero_and_end_at_one() {
            for curve in [ease_out, drawer] {
                assert_eq!(curve(0.), 0.);
                assert_eq!(curve(1.), 1.);
            }
        }

        #[test]
        fn curves_never_step_back() {
            for curve in [ease_out, drawer] {
                let mut last = 0.;
                for i in 1..=200 {
                    let y = curve(i as f32 / 200.);
                    assert!(y + 1e-5 >= last, "{last} then {y}");
                    last = y;
                }
            }
        }

        #[test]
        fn ease_out_is_most_of_the_way_by_half_time() {
            assert!(ease_out(0.5) > 0.9, "{}", ease_out(0.5));
        }

        #[test]
        fn linear_bezier_is_the_identity() {
            for i in 0..=10 {
                let t = i as f32 / 10.;
                assert!((cubic(0.25, 0.25, 0.75, 0.75, t) - t).abs() < 1e-4);
            }
        }

        fn linear(t: f32) -> f32 {
            t
        }

        #[test]
        fn tween_runs_from_real_elapsed_time() {
            let mut tween = Tween::fade(0., Duration::from_millis(100), linear);
            let start = Instant::now();
            tween.retarget_at(1., start, false);
            let half = tween.value_at(start + Duration::from_millis(50));
            assert!((half - 0.5).abs() < 1e-3);
            assert!(!tween.moving_at(start + Duration::from_millis(100)));
        }

        #[test]
        fn retargeting_starts_from_where_it_is() {
            let mut tween = Tween::fade(0., Duration::from_millis(100), linear);
            let start = Instant::now();
            tween.retarget_at(1., start, false);
            let midway = start + Duration::from_millis(40);
            tween.retarget_at(0., midway, false);
            assert!((tween.value_at(midway) - 0.4).abs() < 1e-3);
        }

        #[test]
        fn asking_for_the_same_target_does_not_restart() {
            let mut tween = Tween::fade(0., Duration::from_millis(100), linear);
            let start = Instant::now();
            tween.retarget_at(1., start, false);
            tween.retarget_at(1., start + Duration::from_millis(60), false);
            assert!((tween.value_at(start + Duration::from_millis(60)) - 0.6).abs() < 1e-3);
        }

        #[test]
        fn reduced_motion_makes_travel_instant() {
            let mut tween = Tween::travel(0., Duration::from_millis(100), linear);
            let start = Instant::now();
            tween.retarget_at(1., start, true);
            assert_eq!(tween.value_at(start), 1.);
            assert!(!tween.moving_at(start));
        }

        #[test]
        fn reduced_motion_keeps_fades() {
            let mut tween = Tween::fade(0., Duration::from_millis(100), linear);
            let start = Instant::now();
            tween.retarget_at(1., start, true);
            assert!(tween.moving_at(start));
        }
    }
}
