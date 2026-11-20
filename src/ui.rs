//! Small stateless building blocks shared by every pane.

use gpui::{BoxShadow, Div, FontWeight, Hsla, SharedString, Svg, div, point, prelude::*, px, svg};

use crate::theme::{MONO, Theme, Tone, c, gradient};

pub fn icon(name: &str, size: f32, color: Hsla) -> Svg {
    svg()
        .path(SharedString::from(format!("icons/{name}.svg")))
        .size(px(size))
        .flex_none()
        .text_color(color)
}

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

pub fn icon_button(t: &Theme, name: &str) -> Div {
    let hover = t.hover;
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(30.))
        .rounded(px(8.))
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .child(icon(name, 16., t.text2))
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
    match kind {
        ButtonKind::Primary => base
            .bg(gradient(90., c(0x8b7cff, 1.), c(0xff7ab6, 1.)))
            .shadow(vec![shadow(c(0x8b7cff, 0.35), 6., 18., -4.)])
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
            .text_color(t.text)
            .child(label.to_string())
            .children(key.map(|k| kbd(t, k))),
        ButtonKind::Ghost => base
            .text_color(t.text)
            .child(label.to_string())
            .children(key.map(|k| kbd(t, k))),
    }
}
