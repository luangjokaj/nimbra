//! The shortcuts window (?, Help › Keyboard Shortcuts): every key, in tabs. A window of its
//! own, as Settings is: it stays open beside the mail until it is closed.

use crate::keyboard::{Piece, SHORTCUT_TABS, pieces};

use super::*;

/// The window's width, and the height of one row of the table.
const CARD_W: f32 = 760.;
const ROW_H: f32 = 42.;
const TABLE_HEAD: f32 = 34.;
/// The tab bar's segments: their height, the space around and between them, and the
/// padding beside each label.
const TAB_H: f32 = 28.;
const TAB_INSET: f32 = 3.;
const TAB_GAP: f32 = 2.;
const TAB_PAD: f32 = 14.;
/// The padding at the top, below the traffic lights, and at the sides and bottom.
const PAD_TOP: f32 = 52.;
const PAD: f32 = 28.;
/// The window without its table: padding, title and hint, tab bar, and the gaps between.
const CARD_CHROME: f32 = PAD_TOP + PAD + 39. + 6. + 22. + 20. + (TAB_H + 2. * TAB_INSET + 2.) + 20.;
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
}

impl ShortcutsSheet {
    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.remove_window();
        self.owner
            .update(cx, |owner, _| owner.shortcuts_window = None)
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
        // The key that opened it closes it too, as changed in Settings.
        let pressed = event.keystroke.unparse();
        if crate::shortcuts::keys("shortcuts").contains(&pressed) {
            return self.close(window, cx);
        }
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
                        pieces(caps).into_iter().map(|piece| match piece {
                            Piece::Key(key) => kbd(t, &key),
                            Piece::Word(word) => meta(word, t.text3),
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
        div()
            .id("shortcuts-sheet")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                this.key(event, window, cx)
            }))
            .size_full()
            .flex()
            .flex_col()
            .gap(dp(20.))
            .pt(dp(PAD_TOP))
            .px(dp(PAD))
            .pb(dp(PAD))
            .bg(t.raised)
            .font_family(crate::theme::SANS)
            .text_size(dp(13.))
            .line_height(dp(18.))
            .text_color(t.text)
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
                            .map(|d| {
                                let key = crate::shortcuts::label("shortcuts");
                                if key.is_empty() {
                                    d.child("Open this window from Help › Keyboard Shortcuts.")
                                } else {
                                    d.child("Press")
                                        .child(kbd(&t, &key))
                                        .child("anytime to open this window.")
                                }
                            }),
                    ),
            )
            // In a row of its own, so the tab bar is only as wide as its tabs.
            .child(div().flex().child(self.tabs(&t, cx)))
            .child(self.table(&t))
    }
}

impl Nimbra {
    /// ? and Help › Keyboard Shortcuts: the shortcuts window, brought forward if it is open.
    pub(super) fn open_shortcuts(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = &self.shortcuts_window
            && handle
                .update(cx, |_, window, _| window.activate_window())
                .is_ok()
        {
            return;
        }
        let owner = cx.weak_entity();
        let dark = self.dark;
        let tallest = (0..SHORTCUT_TABS.len())
            .map(table_height)
            .fold(0., f32::max);
        // Opened once this update is over, as Settings is.
        cx.defer(move |cx| {
            let size = gpui::size(to_screen(CARD_W), to_screen(CARD_CHROME + tallest));
            let bounds = gpui::Bounds::centered(None, size, cx);
            let me = owner.clone();
            let opened = cx.open_window(
                gpui::WindowOptions {
                    window_bounds: Some(gpui::WindowBounds::Windowed(bounds)),
                    titlebar: Some(gpui::TitlebarOptions {
                        title: Some("Keyboard Shortcuts".into()),
                        appears_transparent: true,
                        traffic_light_position: Some(gpui::point(px(16.), px(16.))),
                    }),
                    is_resizable: false,
                    ..Default::default()
                },
                |window, cx| {
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
                                this.update(cx, |this: &mut ShortcutsSheet, cx| {
                                    this.show_tab(tab, cx)
                                })
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
                        }
                    })
                },
            );
            me.update(cx, |n, _| n.shortcuts_window = opened.ok()).ok();
        });
    }
}
