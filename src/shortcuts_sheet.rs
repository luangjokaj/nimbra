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
/// The tab bar's segments: their height, the space around and between them, and the
/// padding beside each label.
const TAB_H: f32 = 28.;
const TAB_INSET: f32 = 3.;
const TAB_GAP: f32 = 2.;
const TAB_PAD: f32 = 14.;
/// The card without its table: padding, title and hint, tab bar, and the gaps between.
const CARD_CHROME: f32 = 2. * 28. + 39. + 6. + 22. + 20. + (TAB_H + 2. * TAB_INSET + 2.) + 20.;
/// How long a switch of tab moves the highlight and the table's height.
const SWITCH: Duration = Duration::from_millis(260);

/// A tab's table height: its head and rows, and the border.
fn table_height(tab: usize) -> f32 {
    TABLE_HEAD + SHORTCUT_TABS[tab].1.len() as f32 * ROW_H + 2.
}

pub(super) struct ShortcutsSheet {
    owner: gpui::WeakEntity<Nimbra>,
    tab: usize,
    /// The tab shown before the last switch, when that was, and how many switches so far
    /// (each switch animates once, from where the last one ended).
    from_tab: usize,
    switched: Option<std::time::Instant>,
    switches: usize,
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
            self.from_tab = self.tab;
            self.tab = tab;
            self.switched = Some(std::time::Instant::now());
            self.switches += 1;
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

    /// Whether a switch of tab is still moving (and the Mac allows motion).
    fn switching(&self) -> bool {
        self.switches > 0
            && self.switched.is_some_and(|at| at.elapsed() < SWITCH)
            && !crate::system::reduces_motion()
    }

    /// The tabs: a segmented control like the list's All / Unread, its highlight sliding from
    /// the old tab to the new one.
    fn tabs(&self, t: &Theme, cx: &mut Context<Self>) -> Div {
        let widths: Vec<f32> = SHORTCUT_TABS
            .iter()
            .map(|(name, _)| {
                (text_width(name, 13., crate::theme::SANS, cx) + 2. * TAB_PAD + 2.).ceil()
            })
            .collect();
        let spot = |tab: usize| {
            let x: f32 = widths[..tab].iter().map(|w| w + TAB_GAP).sum();
            (x, widths[tab])
        };
        let ((to_x, to_w), (from_x, from_w)) = (spot(self.tab), spot(self.from_tab));
        let pill = div()
            .absolute()
            .top(dp(TAB_INSET))
            .h(dp(TAB_H))
            .rounded(dp(7.))
            .bg(t.raised)
            .border_1()
            .border_color(t.border)
            .shadow(vec![shadow(
                c(0x000000, if t.dark { 0.25 } else { 0.08 }),
                1.,
                3.,
                0.,
            )]);
        let pill = if self.switching() {
            pill.with_animation(
                ("shortcut-tab-pill", self.switches),
                Animation::new(SWITCH).with_easing(ease_out_quint()),
                move |d, delta| {
                    d.left(dp(TAB_INSET + from_x + (to_x - from_x) * delta))
                        .w(dp(from_w + (to_w - from_w) * delta))
                },
            )
            .into_any_element()
        } else {
            pill.left(dp(TAB_INSET + to_x))
                .w(dp(to_w))
                .into_any_element()
        };
        div()
            .relative()
            .flex()
            .flex_none()
            .gap(dp(TAB_GAP))
            .p(dp(TAB_INSET))
            .rounded(dp(10.))
            .bg(t.hover)
            .border_1()
            .border_color(t.border)
            .child(pill)
            .children(SHORTCUT_TABS.iter().enumerate().map(|(i, (name, _))| {
                div()
                    .id(("shortcut-tab", i))
                    .w(dp(widths[i]))
                    .h(dp(TAB_H))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(if i == self.tab { t.text } else { t.text3 })
                    .child(name.to_string())
                    .on_click(cx.listener(move |this, _, _, cx| this.show_tab(i, cx)))
            }))
    }

    /// The table of one tab: keys on the left, what they do on the right.
    /// As tall as its rows, growing or shrinking to the next tab's over the switch.
    fn table(&self, t: &Theme) -> gpui::AnyElement {
        let rows = SHORTCUT_TABS[self.tab].1;
        let (to_h, from_h) = (table_height(self.tab), table_height(self.from_tab));
        let cell_head = |label: &str| {
            div()
                .flex_1()
                .child(mono(label.to_string(), t.text3).text_size(dp(10.5)))
        };
        let table = div()
            .flex()
            .flex_col()
            .flex_none()
            .rounded(dp(12.))
            .border_1()
            .border_color(t.border)
            .overflow_hidden()
            .child(
                div()
                    .flex()
                    .items_center()
                    .h(dp(TABLE_HEAD))
                    .px(dp(14.))
                    .bg(t.hover)
                    // GPUI clips to a rectangle, not the table's rounded corners: the band
                    // rounds its own, inside the border (12 less its 1).
                    .rounded_t(dp(11.))
                    .child(cell_head("KEY"))
                    .child(cell_head("ACTION")),
            )
            .children(rows.iter().map(|(caps, action)| {
                div()
                    .flex()
                    .items_center()
                    .h(dp(ROW_H))
                    .px(dp(14.))
                    .border_t_1()
                    .border_color(t.border)
                    .child(div().flex_1().flex().items_center().gap(dp(6.)).children(
                        caps.iter().map(|cap| match cap {
                            Cap::Key(key) => kbd(t, key),
                            Cap::Word(word) => meta(word.to_string(), t.text3),
                        }),
                    ))
                    .child(div().flex_1().text_color(t.text2).child(action.to_string()))
            }));
        if self.switching() {
            table
                .with_animation(
                    ("shortcut-table", self.switches),
                    Animation::new(SWITCH).with_easing(ease_out_quint()),
                    move |d, delta| d.h(dp(from_h + (to_h - from_h) * delta)),
                )
                .into_any_element()
        } else {
            table.h(dp(to_h)).into_any_element()
        }
    }
}

impl Render for ShortcutsSheet {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Every window draws at the interface zoom.
        crate::ui::apply_zoom(window);
        let t = if self.dark {
            Theme::dark()
        } else {
            Theme::light()
        };
        let tallest = (0..SHORTCUT_TABS.len())
            .map(table_height)
            .fold(0., f32::max);
        let height = from_screen(window.viewport_size().height);
        let top = ((height - CARD_CHROME - tallest) / 2.).max(16.);
        let card = div()
            .id("shortcuts-card")
            .w(dp(CARD_W))
            .flex()
            .flex_col()
            .gap(dp(20.))
            .p(dp(28.))
            .rounded(dp(18.))
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
                            .gap(dp(6.))
                            .child(serif("One key for everything.", 34., t.text))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(dp(6.))
                                    .text_size(dp(13.))
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
            .rounded(dp(WINDOW_RADIUS))
            .bg(t.scrim)
            // The card's top stays put while its table changes height: where the tallest
            // tab would sit centred.
            .flex()
            .flex_col()
            .items_center()
            .pt(dp(top))
            .font_family(crate::theme::SANS)
            .text_size(dp(13.))
            .line_height(dp(18.))
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
        let display_id = crate::popup::display_of(window, cx);
        let opened = cx.open_window(
            gpui::WindowOptions {
                window_bounds: Some(gpui::WindowBounds::Windowed(bounds)),
                titlebar: None,
                kind: gpui::WindowKind::PopUp,
                display_id,
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
                    // NIMBRA_SHORTCUTS_TAB=<n> switches to tab n a second after opening
                    // (screenshots of the switch).
                    if let Some(tab) = std::env::var("NIMBRA_SHORTCUTS_TAB")
                        .ok()
                        .and_then(|v| v.parse::<usize>().ok())
                    {
                        cx.spawn(async move |this, cx| {
                            cx.background_executor().timer(Duration::from_secs(1)).await;
                            this.update(cx, |this: &mut ShortcutsSheet, cx| this.show_tab(tab, cx))
                                .ok();
                        })
                        .detach();
                    }
                    ShortcutsSheet {
                        owner,
                        tab: 0,
                        from_tab: 0,
                        switched: None,
                        switches: 0,
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
