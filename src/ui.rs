//! Small stateless building blocks shared by every pane.

use gpui::{
    BoxShadow, Div, FontWeight, Hsla, SharedString, Stateful, Svg, div, point, prelude::*, px,
    svg,
};

use crate::theme::{MONO, SANS, Theme, Tone, over, radius, space};

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

/// The backdrop showing between the main window's panels and around them.
pub const GUTTER: f32 = 8.;

/// The line the main window's top row is centred on: a top bar's centre, inside a panel
/// that starts a gutter down.
pub const TOP_LINE: f32 = GUTTER + 25.;

/// What the main window's panels float over: the theme's ground, one solid colour.
pub fn backdrop(t: &Theme) -> Div {
    div().absolute().inset_0().bg(t.ground)
}

/// How far forward a floating panel sits: the reader is in front of the list.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Depth {
    Back,
    Front,
}

/// A floating panel of the main window: a solid surface on the ground, set down by a faint
/// neutral shadow. Finish it with `rim` as its last child.
pub fn panel(t: &Theme, depth: Depth) -> Div {
    div()
        .relative()
        .rounded(px(radius::SHEET))
        .bg(match depth {
            Depth::Back => t.panel,
            Depth::Front => t.surface,
        })
        .shadow(lift(t, depth))
}

/// The shadow under a floating panel: close and faint, so the panel sits on the ground
/// rather than hovering over it.
pub fn lift(t: &Theme, depth: Depth) -> Vec<BoxShadow> {
    let strength = match depth {
        Depth::Back => 0.6,
        Depth::Front => 0.8,
    };
    vec![shadow(t.shadow.opacity(t.shadow.a * strength), 1., 4., 0.)]
}

/// A panel's edge: a hairline all round, laid over it so it takes no room.
pub fn rim(t: &Theme, corner: f32) -> Div {
    div()
        .absolute()
        .inset_0()
        .rounded(px(corner))
        .border_1()
        .border_color(t.rim)
}

/// A floating sheet (the composer, Add Account, a question over either): a raised solid
/// surface with a hairline edge and a neutral shadow. Its `t.edge` border is where a drop
/// target lights up.
pub fn sheet(t: &Theme) -> Div {
    div()
        .relative()
        .rounded(px(radius::SHEET))
        .bg(t.raised)
        .border_1()
        .border_color(t.edge)
        .shadow(elevation(t, Elevation::Sheet))
}

/// A window drawn as the main one is: panels a gutter apart on the ground. Lay its panels
/// out as children, in a row.
pub fn panels(t: &Theme) -> Div {
    div()
        .relative()
        .flex()
        .size_full()
        .p(px(GUTTER))
        .gap(px(GUTTER))
        .child(backdrop(t))
}

/// Where a window of panels puts its traffic lights: the close button's top-left, so that
/// they are centred on the top line, as far into the first panel as its content.
pub fn traffic_lights() -> gpui::Point<gpui::Pixels> {
    point(px(18. + GUTTER), px(TOP_LINE - 7.))
}

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

/// A section head: Sora 11/16 semibold in `text3`.
pub fn overline(t: &Theme, text: impl Into<SharedString>) -> Div {
    div()
        .font_family(SANS)
        .font_weight(FontWeight::SEMIBOLD)
        .text_size(px(11.))
        .line_height(px(16.))
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
        .bg(tone.fill())
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

/// Keyboard focus as a crisp 2pt ring laid over its element (which needs `.relative()`),
/// so gaining or losing focus never moves anything.
pub fn focus_ring(t: &Theme, corner: f32) -> Div {
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .rounded(px(corner))
        .border_2()
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

/// Depth: neutral shadows only. Pair a raised surface with a `t.edge` hairline border.
#[derive(Clone, Copy)]
pub enum Elevation {
    /// Menus, tooltips, toasts.
    Pop,
    /// Finder, confirm, shortcuts, composer: lifted further.
    Sheet,
}

pub fn elevation(t: &Theme, level: Elevation) -> Vec<BoxShadow> {
    match level {
        Elevation::Pop => vec![shadow(t.shadow, 6., 16., -4.)],
        Elevation::Sheet => vec![
            shadow(t.shadow_sheet, 16., 40., -12.),
            shadow(t.shadow_sheet.opacity(t.shadow_sheet.a * 0.5), 1., 3., 0.),
        ],
    }
}

pub enum ButtonKind {
    /// A hairline outline with no fill.
    Secondary,
    Ghost,
}

fn button_base(label: &str) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(px(space::S))
        .h(px(BUTTON_H))
        .px(px(space::L))
        .rounded(px(radius::CONTROL))
        .cursor_pointer()
        .font_weight(FontWeight::SEMIBOLD)
        .child(label.to_string())
}

/// The height of every text button.
pub const BUTTON_H: f32 = 32.;

/// A text button. Its key, if it has one, goes in its tooltip (`tip`).
pub fn button(t: &Theme, kind: ButtonKind, label: &str) -> Div {
    let (hover, strong) = (t.hover, t.border_strong);
    match kind {
        ButtonKind::Secondary => button_base(label)
            .border_1()
            .border_color(t.border_strong)
            .hover(move |s| s.bg(hover).border_color(strong))
            .text_color(t.text),
        ButtonKind::Ghost => button_base(label)
            .hover(move |s| s.bg(hover))
            .text_color(t.text),
    }
}

/// The one main action of a surface: flat solid violet under a white label, a touch lighter
/// on hover and darker while held.
pub fn primary(t: &Theme, id: impl Into<gpui::ElementId>, label: &str) -> Stateful<Div> {
    solid(button_base(label).id(id), t.primary, t.primary_hover, t.primary_pressed)
}

/// A destructive confirmation: solid rose under a white label.
pub fn destructive(t: &Theme, id: impl Into<gpui::ElementId>, label: &str) -> Stateful<Div> {
    let held = over(gpui::black().opacity(0.12), t.danger);
    let hover = over(gpui::white().opacity(0.08), t.danger);
    solid(button_base(label).id(id), t.danger, hover, held)
}

/// A solid fill with its hover and pressed steps, under a white label.
pub fn solid(d: Stateful<Div>, rest: Hsla, hover: Hsla, held: Hsla) -> Stateful<Div> {
    d.bg(rest)
        .text_color(gpui::white())
        .hover(move |s| s.bg(hover))
        .active(move |s| s.bg(held))
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

#[cfg(test)]
mod tests {
    use super::*;

    /// What lifts a panel or a sheet is the theme's own neutral shadow, never the accent.
    #[test]
    fn panels_and_sheets_cast_only_neutral_shadows() {
        for t in [Theme::dark(), Theme::light()] {
            let shadows = [lift(&t, Depth::Back), lift(&t, Depth::Front)]
                .into_iter()
                .flatten()
                .chain(elevation(&t, Elevation::Sheet));
            for s in shadows {
                let neutral = [t.shadow, t.shadow_sheet]
                    .iter()
                    .any(|n| (n.h - s.color.h).abs() < 1e-3 && (n.s - s.color.s).abs() < 1e-3);
                assert!(neutral, "{:?}", s.color);
            }
        }
    }
}
