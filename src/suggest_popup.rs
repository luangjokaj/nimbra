//! Address suggestions under a recipient field, floating over the fields below instead of
//! pushing them down. They are a small window of their own: the message editor is a native
//! view, above all GPUI draws, and would cut a list drawn in the composer off where the two
//! meet. The window never takes the keyboard, so typing goes on in the field.
//!
//! The composer draws an anchor under the field; each time it is drawn it hands the window
//! its place and what to show (`place_suggestions`). The window keeps its own copy of the
//! rows, so drawing it never reads the composer.

use super::menus::{MENU_MARGIN, popup_shadow, remove_popup};
use super::*;
use gpui::{AnyWindowHandle, WeakEntity};

/// The list's width.
pub(super) const SUGGEST_WIDTH: f32 = 380.;
/// Each suggestion's height.
const ROW: f32 = 46.;
/// The card's padding and border, around the rows.
const CHROME: f32 = 2. * 4. + 2.;
/// Between the field and the list.
const GAP: f32 = 4.;

/// The list's window.
pub(super) struct SuggestPopup {
    owner: WeakEntity<Nimbra>,
    /// The window the composer is in, to give the keyboard back to after a click.
    composer_window: AnyWindowHandle,
    people: Vec<mail::Person>,
    selected: usize,
    dark: bool,
}

/// The address suggestions' window as shown: where it was put, its rows and its display.
pub(super) type Shown = (
    gpui::WindowHandle<SuggestPopup>,
    (f64, f64),
    usize,
    Option<gpui::DisplayId>,
);

impl Nimbra {
    /// Puts the suggestions where the field's anchor was drawn: `at` is the anchor's top
    /// left on the screen (AppKit's, from the bottom). Opens the window, or updates and
    /// moves it; closes it when there is nothing to suggest any more.
    pub(super) fn place_suggestions(
        &mut self,
        at: (f64, f64),
        composer_window: AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        let Some((people, selected)) = self
            .compose
            .as_ref()
            .and_then(|c| c.suggest.as_ref())
            .filter(|s| !s.people.is_empty())
            .map(|s| (s.people.clone(), s.selected))
        else {
            self.close_suggestions(cx);
            return;
        };
        let rows = people.len();
        let dark = self.dark;
        // `at` is on the screen already; the margins are design points.
        let zoom = f64::from(crate::ui::zoom());
        let margin = f64::from(MENU_MARGIN) * zoom;
        let corner = (at.0 - margin, at.1 - f64::from(GAP) * zoom + margin);
        let size = gpui::size(
            to_screen(SUGGEST_WIDTH + 2. * MENU_MARGIN),
            to_screen(rows as f32 * ROW + CHROME + 2. * MENU_MARGIN),
        );
        // Already showing: given the rows, and moved or resized when that changed.
        // On the display it goes to, so it is drawn at that display's scale: one moving to
        // another display is opened there again.
        let display_id = crate::popup::display_at(at, cx);
        if let Some((handle, _, _, shown_on)) = self.suggest_window
            && shown_on != display_id
        {
            self.suggest_window = None;
            remove_popup(handle, cx);
        }
        if let Some((handle, placed, placed_rows, _)) = self.suggest_window {
            let kept = handle
                .update(cx, |popup, window, cx| {
                    if popup.people != people || popup.selected != selected || popup.dark != dark {
                        popup.people = people.clone();
                        popup.selected = selected;
                        popup.dark = dark;
                        cx.notify();
                    }
                    if placed_rows != rows {
                        window.resize(size);
                    }
                    // A resize keeps the bottom where it was: the top is put back too.
                    if placed != corner || placed_rows != rows {
                        crate::blur::move_to(window, corner);
                    }
                })
                .is_ok();
            if kept {
                self.suggest_window = Some((handle, corner, rows, display_id));
                return;
            }
            self.suggest_window = None;
        }
        let owner = cx.weak_entity();
        let opened = cx.open_window(
            gpui::WindowOptions {
                window_bounds: Some(gpui::WindowBounds::Windowed(gpui::Bounds::new(
                    gpui::point(px(0.), px(0.)),
                    size,
                ))),
                titlebar: None,
                kind: gpui::WindowKind::PopUp,
                // On the display it goes to, so it is drawn at that display's scale.
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
                // GPUI's window coordinates are per display; the screen's own are not.
                crate::blur::move_to(window, corner);
                cx.new(|cx| {
                    // Gone with the composer's window: switching to another window or app
                    // leaves no list floating over it. A press on the list itself can take
                    // the window's focus for a moment, so not while a button is down.
                    // Screenshots of it are taken from a window that is not in front.
                    let pinned = std::env::var("NIMBRA_SCREEN").as_deref() == Ok("compose-suggest");
                    cx.spawn_in(window, async move |this, cx| {
                        loop {
                            cx.background_executor()
                                .timer(std::time::Duration::from_millis(150))
                                .await;
                            let keep = this.update(cx, |popup: &mut SuggestPopup, cx| {
                                let active = popup
                                    .composer_window
                                    .update(cx, |_, window, _| window.is_window_active())
                                    .unwrap_or(false);
                                (
                                    active || pinned || crate::blur::mouse_down(),
                                    popup.owner.clone(),
                                )
                            });
                            match keep {
                                Ok((true, _)) => continue,
                                Ok((false, owner)) => {
                                    owner
                                        .update(cx, |this, cx| {
                                            if let Some(composer) = &mut this.compose {
                                                composer.suggest = None;
                                            }
                                            this.close_suggestions(cx);
                                            cx.notify();
                                        })
                                        .ok();
                                    break;
                                }
                                Err(_) => break,
                            }
                        }
                    })
                    .detach();
                    SuggestPopup {
                        owner,
                        composer_window,
                        people,
                        selected,
                        dark,
                    }
                })
            },
        );
        if let Ok(handle) = opened {
            self.suggest_window = Some((handle, corner, rows, display_id));
        }
    }

    pub(super) fn close_suggestions(&mut self, cx: &mut Context<Self>) {
        if let Some((handle, _, _, _)) = self.suggest_window.take() {
            remove_popup(handle, cx);
        }
    }

    /// Closes the suggestions window once the composer no longer suggests anything (the
    /// anchor that places it is then not drawn).
    pub(super) fn sync_suggestions(&mut self, cx: &mut Context<Self>) {
        // Never over "Save this draft?": that sheet covers the composer.
        let suggesting = !self.compose_ask
            && self
                .compose
                .as_ref()
                .is_some_and(|c| c.suggest.as_ref().is_some_and(|s| !s.people.is_empty()));
        if !suggesting {
            self.close_suggestions(cx);
        }
    }
}

impl Render for SuggestPopup {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        // Every window draws at the interface zoom.
        crate::ui::apply_zoom(window);
        let t = if self.dark {
            Theme::dark()
        } else {
            Theme::light()
        };
        let hover = t.hover;
        let selected = self.selected;
        let rows = self.people.iter().enumerate().map(|(i, p)| {
            let owner = self.owner.clone();
            let composer_window = self.composer_window;
            div()
                .id(("suggest", i))
                .press(&t)
                .flex_none()
                .h(dp(ROW))
                .flex()
                .items_center()
                .gap(dp(10.))
                .px(dp(8.))
                .rounded(dp(7.))
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .when(i == selected, |d| d.bg(t.hover))
                .child(avatar(
                    crate::data::tone_for(p.label()),
                    &crate::data::initials(p.label()),
                    24.,
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w(dp(0.))
                        .child(short(p.label(), 44))
                        .when(!p.name.is_empty(), |d| {
                            d.child(meta(short(&p.address, 50), t.text3))
                        }),
                )
                .on_click(move |_, _, cx| {
                    owner
                        .update(cx, |this, cx| this.accept_suggestion(Some(i), cx))
                        .ok();
                    // The click may have taken the keyboard from the composer.
                    composer_window
                        .update(cx, |_, window, _| window.activate_window())
                        .ok();
                })
        });
        div()
            .size_full()
            .p(dp(MENU_MARGIN))
            .font_family(crate::theme::SANS)
            .text_size(dp(13.))
            .text_color(t.text)
            .child(
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .p(dp(4.))
                    .rounded(dp(10.))
                    .bg(t.raised)
                    .border_1()
                    .border_color(t.border_strong)
                    .shadow(vec![popup_shadow(&t)])
                    .children(rows),
            )
    }
}
