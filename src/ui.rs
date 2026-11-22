//! Small stateless building blocks shared by every pane.

use gpui::{
    BoxShadow, Div, FontWeight, Hsla, SharedString, Stateful, Svg, div, point, prelude::*, px, svg,
};

use crate::theme::{MONO, Theme, Tone, c, gradient};

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

pub fn mono(text: impl Into<SharedString>, color: Hsla) -> Div {
    div()
        .font_family(MONO)
        .font_weight(FontWeight::MEDIUM)
        .text_size(px(11.))
        .line_height(px(14.))
        .text_color(color)
        .flex_none()
        .child(text.into())
}

pub fn overline(text: impl Into<SharedString>, color: Hsla) -> Div {
    div()
        .font_family(MONO)
        .font_weight(FontWeight::MEDIUM)
        .text_size(px(10.))
        .line_height(px(12.))
        .text_color(color)
        .child(text.into())
}

pub fn meta(text: impl Into<SharedString>, color: Hsla) -> Div {
    div()
        .text_size(px(12.))
        .line_height(px(16.))
        .text_color(color)
        .child(text.into())
}

pub fn kbd(t: &Theme, key: &str) -> Div {
    div()
        .flex()
        .flex_none()
        .justify_center()
        .min_w(px(18.))
        .px(px(5.))
        .py(px(2.))
        .rounded(px(5.))
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
        .rounded(px(8.))
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
        offset: point(px(0.), px(y)),
        blur_radius: px(blur),
        spread_radius: px(spread),
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
        .gap(px(8.))
        .pl(px(14.))
        .pr(px(if key.is_some() { 10. } else { 14. }))
        .py(px(6.))
        .rounded(px(9.))
        .cursor_pointer()
        .font_weight(FontWeight::SEMIBOLD);
    let (hover, strong, selected) = (t.hover, t.border_strong, t.selected);
    match kind {
        ButtonKind::Primary => base
            .bg(gradient(90., c(0x8b7cff, 1.), c(0xff7ab6, 1.)))
            .shadow(vec![shadow(c(0x8b7cff, 0.35), 6., 18., -4.)])
            // A brighter glow on hover.
            .hover(|s| s.shadow(vec![shadow(c(0x8b7cff, 0.6), 8., 24., -2.)]))
            .text_color(gpui::white())
            .child(label.to_string())
            .children(key.map(|k| {
                div()
                    .px(px(5.))
                    .py(px(2.))
                    .rounded(px(5.))
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

/// The small label shown when the mouse rests on a button: what it does, and its shortcut.
pub struct Tip {
    dark: bool,
    label: SharedString,
    key: &'static str,
}

impl gpui::Render for Tip {
    fn render(&mut self, _: &mut gpui::Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        let t = if self.dark {
            Theme::dark()
        } else {
            Theme::light()
        };
        // The outer padding keeps the tooltip off the pointer.
        div().pt(px(6.)).child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .pl(px(10.))
                .pr(px(if self.key.is_empty() { 10. } else { 6. }))
                .py(px(5.))
                .rounded(px(8.))
                .bg(t.raised)
                .border_1()
                .border_color(t.border_strong)
                .shadow(vec![shadow(t.shadow, 8., 20., -4.)])
                .font_family(crate::theme::SANS)
                .text_size(px(12.))
                .line_height(px(16.))
                .text_color(t.text)
                .child(self.label.clone())
                .when(!self.key.is_empty(), |d| d.child(kbd(&t, self.key))),
        )
    }
}

/// Builds the tooltip for a button. Pass an empty `key` when there is no shortcut.
pub fn tip(
    dark: bool,
    label: impl Into<SharedString>,
    key: &'static str,
) -> impl Fn(&mut gpui::Window, &mut gpui::App) -> gpui::AnyView + 'static {
    let label = label.into();
    move |_, cx| {
        cx.new(|_| Tip {
            dark,
            label: label.clone(),
            key,
        })
        .into()
    }
}
