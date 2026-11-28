//! Tooltips. On the Mac each is a small click-through window of its own: the email is a
//! native web view, above all GPUI draws, and would cut a tooltip off where the two meet.
//!
//! GPUI still decides when one shows and goes. The view it shows is empty; that view opens
//! the window, and takes it away when GPUI lets go of it. GPUI does not always let go: when
//! the element under the pointer disappears (the sidebar sliding away) its tooltip state is
//! never freed, as it refers to itself. So the window also goes once that view is no longer
//! drawn: it asks its owner for a frame on each look, and checks that it was in it.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnyView, AnyWindowHandle, App, Bounds, Context, Div, IntoElement, Pixels, Point, Render,
    SharedString, TextRun, Window, WindowKind, WindowOptions, div, point, prelude::*, px, size,
};

use crate::theme::{Theme, c};
use crate::ui::{dp, from_screen, kbd, shadow, to_screen};

/// See-through room around the bubble, for its shadow.
const MARGIN: f32 = 28.;
/// Between the pointer and the bubble.
const GAP: f32 = 7.;
/// Looks without having been drawn before the window goes (one can be a late frame).
const MISSES: u8 = 2;
/// How far the pointer may go before the tooltip gives up. GPUI hears nothing once the
/// pointer is over the email, so would otherwise leave it showing.
const REACH: (f32, f32) = (260., 60.);

pub struct Tip {
    dark: bool,
    label: SharedString,
    key: &'static str,
    popup: Option<AnyWindowHandle>,
    /// Set each time GPUI draws this.
    drawn: Rc<Cell<bool>>,
}

/// The tooltip for what is under the pointer now.
pub fn build(
    dark: bool,
    label: SharedString,
    key: &'static str,
    window: &mut Window,
    cx: &mut App,
) -> AnyView {
    let tip = cx.new(|cx| {
        cx.on_release(|tip: &mut Tip, cx| {
            if let Some(popup) = tip.popup.take() {
                cx.defer(move |cx| {
                    popup.update(cx, |_, window, _| window.remove_window()).ok();
                });
            }
        })
        .detach();
        Tip {
            dark,
            label,
            key,
            popup: None,
            drawn: Rc::new(Cell::new(true)),
        }
    });
    if cfg!(target_os = "macos") {
        let (weak, at) = (tip.downgrade(), window.mouse_position());
        // Never while a window draws: opening one clears the elements being drawn.
        window.defer(cx, move |window, cx| {
            if let Some(tip) = weak.upgrade() {
                tip.update(cx, |tip, cx| tip.open(at, window, cx));
            }
        });
    }
    tip.into()
}

impl Tip {
    fn open(&mut self, at: Point<Pixels>, owner: &mut Window, cx: &mut Context<Self>) {
        if !owner.is_window_active() {
            return;
        }
        let measure = |text: SharedString, family: &'static str, size: f32| {
            let run = TextRun {
                len: text.len(),
                font: gpui::font(family),
                color: gpui::black(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            f32::from(
                owner
                    .text_system()
                    .shape_line(text, px(size), &[run], None)
                    .width,
            )
        };
        // The same paddings as `bubble`: around the label, then around the key.
        let label = measure(self.label.clone(), crate::theme::SANS, 12.).ceil() + 1.;
        let (tail, height) = if self.key.is_empty() {
            (10., 28.)
        } else {
            let key = measure(self.key.into(), crate::theme::MONO, 11.).ceil() + 1.;
            // The key is mono 11 at medium, in 5pt of padding and a border, at least 18 wide.
            (8. + (key + 12.).max(18.) + 6., 32.)
        };
        let view = owner.viewport_size();
        let (view_w, view_h) = (from_screen(view.width), from_screen(view.height));
        let width = (10. + label + tail + 2.).min(view_w);

        // Below and right of the pointer, flipped where that leaves the window.
        let (mut x, mut y) = (from_screen(at.x) + 1., from_screen(at.y) + GAP);
        if x + width > view_w {
            x = (from_screen(at.x) - width - 1.).max(0.);
        }
        if y + height > view_h {
            y = (from_screen(at.y) - height - GAP).max(0.);
        }
        let Some(corner) = crate::blur::screen_point(owner, x - MARGIN, y - MARGIN) else {
            return;
        };

        let (parent, dark) = (owner.window_handle(), self.dark);
        let (label, key) = (self.label.clone(), self.key);
        let drawn = self.drawn.clone();
        // On its window's display, so it is drawn at that display's scale where it is moved.
        let display_id = crate::popup::display_of(owner, cx);
        let opened = cx.open_window(
            WindowOptions {
                window_bounds: Some(gpui::WindowBounds::Windowed(Bounds::new(
                    point(px(0.), px(0.)),
                    size(
                        to_screen(width + 2. * MARGIN),
                        to_screen(height + 2. * MARGIN),
                    ),
                ))),
                titlebar: None,
                kind: WindowKind::PopUp,
                display_id,
                focus: false,
                is_movable: false,
                is_resizable: false,
                is_minimizable: false,
                window_background: gpui::WindowBackgroundAppearance::Transparent,
                ..Default::default()
            },
            |window, cx| {
                crate::blur::no_window_shadow(window);
                crate::blur::click_through(window);
                // GPUI's window coordinates are per display; the screen's own are not.
                crate::blur::move_to(window, corner);
                cx.new(|cx| {
                    let from = crate::blur::screen_pointer();
                    cx.spawn_in(window, async move |this, cx| {
                        let mut missed = 0;
                        loop {
                            cx.background_executor()
                                .timer(Duration::from_millis(80))
                                .await;
                            missed = if drawn.replace(false) { 0 } else { missed + 1 };
                            let keep = this.update_in(cx, |_: &mut Popup, window, cx| {
                                let active = parent
                                    .update(cx, |_, owner, _| {
                                        owner.refresh();
                                        owner.is_window_active()
                                    })
                                    .unwrap_or(false);
                                let now = crate::blur::screen_pointer();
                                let near = (now.0 - from.0).abs() <= f64::from(REACH.0)
                                    && (now.1 - from.1).abs() <= f64::from(REACH.1);
                                let keep =
                                    active && near && missed < MISSES && !crate::blur::mouse_down();
                                if !keep {
                                    window.remove_window();
                                }
                                keep
                            });
                            if !matches!(keep, Ok(true)) {
                                break;
                            }
                        }
                    })
                    .detach();
                    Popup { dark, label, key }
                })
            },
        );
        self.popup = opened.ok().map(Into::into);
    }
}

impl Render for Tip {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        // Every window draws at the interface zoom.
        crate::ui::apply_zoom(window);
        self.drawn.set(true);
        if cfg!(target_os = "macos") {
            div() // the window shows it
        } else {
            // The outer padding keeps the tooltip off the pointer.
            div()
                .pt(dp(6.))
                .child(bubble(self.dark, &self.label, self.key))
        }
    }
}

struct Popup {
    dark: bool,
    label: SharedString,
    key: &'static str,
}

impl Render for Popup {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        // Every window draws at the interface zoom.
        crate::ui::apply_zoom(window);
        div()
            .size_full()
            .p(dp(MARGIN))
            .child(bubble(self.dark, &self.label, self.key).size_full())
    }
}

fn bubble(dark: bool, label: &SharedString, key: &'static str) -> Div {
    let t = if dark { Theme::dark() } else { Theme::light() };
    div()
        .flex()
        .items_center()
        .gap(dp(8.))
        .pl(dp(10.))
        .pr(dp(if key.is_empty() { 10. } else { 6. }))
        .py(dp(5.))
        .rounded(dp(8.))
        .bg(t.raised)
        .border_1()
        .border_color(t.border_strong)
        // Close and faint: a tooltip is small, and a wide soft shadow under it read as a
        // smudge on the dark theme.
        .shadow(vec![shadow(
            c(0x000000, if dark { 0.3 } else { 0.1 }),
            1.,
            4.,
            0.,
        )])
        .font_family(crate::theme::SANS)
        .text_size(dp(12.))
        .line_height(dp(16.))
        .text_color(t.text)
        .child(div().whitespace_nowrap().child(label.clone()))
        .when(!key.is_empty(), |d| d.child(kbd(&t, key)))
}
