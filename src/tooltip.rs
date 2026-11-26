//! Tooltips. On the Mac each is a small click-through window of its own: the email is a
//! native web view, above all GPUI draws, and would cut a tooltip off where the two meet.
//!
//! GPUI still decides when one shows and goes. The view it shows is empty; that view opens
//! the window, and takes it away when GPUI lets go of it. GPUI does not always let go: when
//! the element under the pointer disappears (the sidebar sliding away) its tooltip state is
//! never freed, as it refers to itself. So the window also goes once that view is no longer
//! drawn: now and then it asks its owner for one frame, and checks that it was in it.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    Animation, AnimationExt as _, AnyElement, AnyView, AnyWindowHandle, App, Bounds, Context, Div,
    IntoElement, Pixels, Point, Render, SharedString, TextRun, Window, WindowKind, WindowOptions,
    div, point, prelude::*, px, size,
};

use crate::theme::{Theme, radius, space};
use crate::ui::{Elevation, caption, elevation, motion};

/// See-through room around the bubble, for its shadow.
const MARGIN: f32 = 28.;
/// Between the pointer and the bubble.
const GAP: f32 = 7.;
/// How often the window looks at the pointer, which GPUI cannot see over the email.
const LOOK: Duration = Duration::from_millis(80);
/// Every how many looks it asks its owner for a frame, to see that it is still drawn.
const PROBE: u32 = 12;
/// Probes without having been drawn before the window goes (one can be a late frame).
const MISSES: u8 = 2;
/// How far the pointer may go before the tooltip gives up. GPUI hears nothing once the
/// pointer is over the email, so would otherwise leave it showing.
const REACH: (f32, f32) = (260., 60.);
/// Left of the label and right of the last text.
const PAD: f32 = 10.;
/// A 16pt line, 5pt above and below it, and the border.
const HEIGHT: f32 = 28.;

thread_local! {
    /// When a tooltip last went, so that moving along a toolbar shows the next one at once.
    static LAST_GONE: Cell<Option<Instant>> = const { Cell::new(None) };
}

fn gone() {
    LAST_GONE.set(Some(Instant::now()));
}

/// Whether a tooltip opening now follows another closely enough to skip its fade.
fn follows_another() -> bool {
    LAST_GONE
        .get()
        .is_some_and(|at| at.elapsed() < motion::TIP_SKIP)
}

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
            gone();
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
        let label = measure(self.label.clone(), crate::theme::SANS, 12.).ceil() + 1.;
        let key = if self.key.is_empty() {
            0.
        } else {
            space::S + measure(self.key.into(), crate::theme::MONO, 11.).ceil() + 1.
        };
        let height = HEIGHT;
        let view = owner.viewport_size();
        let (view_w, view_h) = (f32::from(view.width), f32::from(view.height));
        let width = (PAD + label + key + PAD + 2.).min(view_w);

        // Below and right of the pointer, flipped where that leaves the window.
        let (mut x, mut y) = (f32::from(at.x) + 1., f32::from(at.y) + GAP);
        if x + width > view_w {
            x = (f32::from(at.x) - width - 1.).max(0.);
        }
        if y + height > view_h {
            y = (f32::from(at.y) - height - GAP).max(0.);
        }
        let Some(corner) = crate::blur::screen_point(owner, x - MARGIN, y - MARGIN) else {
            return;
        };

        let (parent, dark) = (owner.window_handle(), self.dark);
        let (label, key) = (self.label.clone(), self.key);
        let drawn = self.drawn.clone();
        let opened = cx.open_window(
            WindowOptions {
                window_bounds: Some(gpui::WindowBounds::Windowed(Bounds::new(
                    point(px(0.), px(0.)),
                    size(px(width + 2. * MARGIN), px(height + 2. * MARGIN)),
                ))),
                titlebar: None,
                kind: WindowKind::PopUp,
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
                        let (mut looks, mut missed, mut probing) = (0u32, 0u8, false);
                        loop {
                            cx.background_executor().timer(LOOK).await;
                            looks += 1;
                            // Redrawing the owner is a whole frame of the main window: it
                            // is asked for only once in a while, never on every look.
                            if probing {
                                probing = false;
                                missed = if drawn.get() { 0 } else { missed + 1 };
                            }
                            let probe = looks % PROBE == 0;
                            if probe {
                                drawn.set(false);
                                probing = true;
                            }
                            let keep = this.update_in(cx, |_: &mut Popup, window, cx| {
                                let active = parent
                                    .update(cx, |_, owner, _| {
                                        if probe {
                                            owner.refresh();
                                        }
                                        owner.is_window_active()
                                    })
                                    .unwrap_or(false);
                                let now = crate::blur::screen_pointer();
                                let near = (now.0 - from.0).abs() <= f64::from(REACH.0)
                                    && (now.1 - from.1).abs() <= f64::from(REACH.1);
                                let keep =
                                    active && near && missed < MISSES && !crate::blur::mouse_down();
                                if !keep {
                                    gone();
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
                    Popup {
                        dark,
                        label,
                        key,
                        instant: follows_another(),
                    }
                })
            },
        );
        self.popup = opened.ok().map(Into::into);
    }
}

impl Render for Tip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.drawn.set(true);
        if cfg!(target_os = "macos") {
            div() // the window shows it
        } else {
            // The outer padding keeps the tooltip off the pointer.
            div()
                .pt(px(6.))
                .child(fade_in(bubble(self.dark, &self.label, self.key), false))
        }
    }
}

struct Popup {
    dark: bool,
    label: SharedString,
    key: &'static str,
    instant: bool,
}

impl Render for Popup {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().p(px(MARGIN)).child(fade_in(
            bubble(self.dark, &self.label, self.key).size_full(),
            self.instant,
        ))
    }
}

/// The label, then its key as quiet text, the way macOS help tags show one.
fn bubble(dark: bool, label: &SharedString, key: &'static str) -> Div {
    let t = if dark { Theme::dark() } else { Theme::light() };
    div()
        .flex()
        .items_center()
        .gap(px(space::S))
        .h(px(HEIGHT))
        .px(px(PAD))
        .rounded(px(radius::CONTROL))
        .bg(t.raised)
        .border_1()
        .border_color(t.edge)
        .shadow(elevation(&t, Elevation::Pop))
        .font_family(crate::theme::SANS)
        .text_size(px(12.))
        .line_height(px(16.))
        .text_color(t.text)
        .child(div().whitespace_nowrap().child(label.clone()))
        .when(!key.is_empty(), |d| d.child(caption(key, t.text3)))
}

fn fade_in(bubble: Div, instant: bool) -> AnyElement {
    if instant {
        bubble.into_any_element()
    } else {
        bubble
            .with_animation(
                "tip-in",
                Animation::new(motion::TIP).with_easing(motion::ease_out),
                |bubble, delta| bubble.opacity(delta),
            )
            .into_any_element()
    }
}
