//! The shortcuts sheet (?): every key, in tabs, on a card over the dimmed app. A window of its
//! own covering the main one, like the menus, so it sits above the email's web view.

use crate::keyboard::{Cap, SHORTCUT_TABS};

use super::*;

/// The card's width, and the height of one row of the table.
const CARD_W: f32 = 760.;
const ROW_H: f32 = 42.;
const TABLE_HEAD: f32 = 34.;
/// The main window's corner radius, so the dimmed backdrop fits it exactly.
const WINDOW_RADIUS: f32 = 16.;

pub(super) struct ShortcutsSheet {
    owner: gpui::WeakEntity<Nimbra>,
    tab: usize,
    dark: bool,
    focus: gpui::FocusHandle,
    was_active: bool,
    opened: std::time::Instant,
    _activation: gpui::Subscription,
}

impl ShortcutsSheet {
    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.remove_window();
        self.owner
            .update(cx, |owner, cx| {
                owner.shortcuts = false;
                owner.shortcuts_window = None;
                cx.notify();
            })
            .ok();
    }

    fn show_tab(&mut self, tab: usize, cx: &mut Context<Self>) {
        if tab < SHORTCUT_TABS.len() && tab != self.tab {
            self.tab = tab;
            cx.notify();
        }
    }

    fn key(&mut self, event: &gpui::KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let typed = event.keystroke.key_char.as_deref();
        match (key, typed) {
            ("escape", _) | (_, Some("?")) => self.close(window, cx),
            ("left", _) => self.show_tab(self.tab.saturating_sub(1), cx),
            ("right", _) => self.show_tab(self.tab + 1, cx),
            ("1" | "2" | "3" | "4", _) => {
                let n: usize = key.parse().unwrap_or(1);
                self.show_tab(n - 1, cx)
            }
            _ => {}
        }
    }

    /// The tabs: a segmented control like the list's All / Unread.
    fn tabs(&self, t: &Theme, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_none()
            .gap(px(2.))
            .p(px(3.))
            .rounded(px(9.))
            .bg(t.hover)
            .border_1()
            .border_color(t.border)
            .children(SHORTCUT_TABS.iter().enumerate().map(|(i, (name, _))| {
                let on = i == self.tab;
                div()
                    .id(("shortcut-tab", i))
                    .px(px(14.))
                    .h(px(28.))
                    .flex()
                    .items_center()
                    .rounded(px(7.))
                    .cursor_pointer()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(if on { t.text } else { t.text3 })
                    .when(on, |d| {
                        d.bg(t.raised)
                            .border_1()
                            .border_color(t.border)
                            .shadow(vec![shadow(
                                c(0x000000, if t.dark { 0.25 } else { 0.08 }),
                                1.,
                                3.,
                                0.,
                            )])
                    })
                    .when(!on, |d| {
                        d.border_1().border_color(gpui::transparent_black())
                    })
                    .child(name.to_string())
                    .on_click(cx.listener(move |this, _, _, cx| this.show_tab(i, cx)))
            }))
    }

    /// The table of one tab: keys on the left, what they do on the right.
    fn table(&self, t: &Theme) -> Div {
        let rows = SHORTCUT_TABS[self.tab].1;
        // As tall as the longest tab, so switching tabs never moves the card.
        let most = SHORTCUT_TABS
            .iter()
            .map(|(_, r)| r.len())
            .max()
            .unwrap_or(0);
        let cell_head = |label: &str| {
            div()
                .flex_1()
                .child(mono(label.to_string(), t.text3).text_size(px(10.5)))
        };
        div()
            .flex()
            .flex_col()
            .flex_none()
            .h(px(TABLE_HEAD + most as f32 * ROW_H + 2.))
            .rounded(px(12.))
            .border_1()
            .border_color(t.border)
            .overflow_hidden()
            .child(
                div()
                    .flex()
                    .items_center()
                    .h(px(TABLE_HEAD))
                    .px(px(14.))
                    .bg(t.hover)
                    .child(cell_head("KEY"))
                    .child(cell_head("ACTION")),
            )
            .children(rows.iter().map(|(caps, action)| {
                div()
                    .flex()
                    .items_center()
                    .h(px(ROW_H))
                    .px(px(14.))
                    .border_t_1()
                    .border_color(t.border)
                    .child(div().flex_1().flex().items_center().gap(px(6.)).children(
                        caps.iter().map(|cap| match cap {
                            Cap::Key(key) => kbd(t, key),
                            Cap::Word(word) => meta(word.to_string(), t.text3),
                        }),
                    ))
                    .child(div().flex_1().text_color(t.text2).child(action.to_string()))
            }))
    }
}

impl Render for ShortcutsSheet {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = if self.dark {
            Theme::dark()
        } else {
            Theme::light()
        };
        let card = div()
            .id("shortcuts-card")
            .w(px(CARD_W))
            .flex()
            .flex_col()
            .gap(px(20.))
            .p(px(28.))
            .rounded(px(18.))
            .bg(t.raised)
            .border_1()
            .border_color(t.border_strong)
            .shadow(vec![shadow(
                c(0x000000, if t.dark { 0.6 } else { 0.2 }),
                30.,
                80.,
                -10.,
            )])
            // A click on the card stays on it.
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex()
                    .items_start()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .child(serif("One key for everything.", 34., t.text))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .text_size(px(13.))
                                    .text_color(t.text3)
                                    .child("Press")
                                    .child(kbd(&t, "?"))
                                    .child("anytime to open this sheet."),
                            ),
                    )
                    .child(
                        icon_button(&t, "x")
                            .id("shortcuts-close")
                            .on_click(cx.listener(|this, _, window, cx| this.close(window, cx))),
                    ),
            )
            // In a row of its own, so the tab bar is only as wide as its tabs.
            .child(div().flex().child(self.tabs(&t, cx)))
            .child(self.table(&t));
        div()
            .id("shortcuts-sheet")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                this.key(event, window, cx)
            }))
            .size_full()
            .rounded(px(WINDOW_RADIUS))
            .bg(t.scrim)
            .flex()
            .items_center()
            .justify_center()
            .font_family(crate::theme::SANS)
            .text_size(px(13.))
            .line_height(px(18.))
            .text_color(t.text)
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, _, window, cx| this.close(window, cx)),
            )
            .child(card)
    }
}

impl Nimbra {
    /// Closes the sheet if it is open.
    pub(super) fn close_shortcuts(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.shortcuts_window.take() {
            remove_popup(handle, cx);
        }
        if self.shortcuts {
            self.shortcuts = false;
            cx.notify();
        }
    }

    /// ?: opens the sheet over the whole main window, or closes it.
    pub(super) fn toggle_shortcuts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(handle) = self.shortcuts_window.take() {
            remove_popup(handle, cx);
            self.shortcuts = false;
            cx.notify();
            return;
        }
        if self.window_covered() {
            return;
        }
        let bounds = window.bounds();
        let (owner, dark) = (cx.weak_entity(), self.dark);
        let opened = cx.open_window(
            gpui::WindowOptions {
                window_bounds: Some(gpui::WindowBounds::Windowed(bounds)),
                titlebar: None,
                kind: gpui::WindowKind::PopUp,
                focus: true,
                is_movable: false,
                is_resizable: false,
                is_minimizable: false,
                window_background: gpui::WindowBackgroundAppearance::Transparent,
                ..Default::default()
            },
            |window, cx| {
                crate::blur::no_window_shadow(window);
                cx.new(|cx| {
                    let focus = cx.focus_handle();
                    window.focus(&focus);
                    ShortcutsSheet {
                        owner,
                        tab: 0,
                        dark,
                        focus,
                        was_active: false,
                        opened: std::time::Instant::now(),
                        // Nimbra going to the background closes it, as the menus do.
                        _activation: cx.observe_window_activation(
                            window,
                            |this: &mut ShortcutsSheet, window, cx| {
                                if window.is_window_active() {
                                    this.was_active = true;
                                } else if this.was_active
                                    && this.opened.elapsed() > Duration::from_millis(500)
                                {
                                    this.close(window, cx);
                                }
                            },
                        ),
                    }
                })
            },
        );
        if let Ok(handle) = opened {
            self.shortcuts_window = Some(handle);
            self.shortcuts = true;
        }
        cx.notify();
    }
}
