use std::time::Duration;

use gpui::{
    Animation, AnimationExt as _, AnyElement, App, Context, Div, FocusHandle, FontStyle,
    FontWeight, HighlightStyle, Hsla, MouseButton, SharedString, Stateful, StyledText,
    Transformation, Window, actions, div, ease_out_quint, percentage, prelude::*, px, relative,
};

use crate::data::{COMMANDS, Mail, inbox};
use crate::theme::{Accent, SERIF, Theme, Tone, c, fade, gradient};
use crate::ui::{
    ButtonKind, avatar, button, icon, icon_button, kbd, meta, mono, overline, pill, shadow,
};

actions!(
    nimbra,
    [
        Next,
        Prev,
        MarkDone,
        Undo,
        TogglePalette,
        Dismiss,
        Confirm,
        ToggleTheme,
        ToggleSidebar,
        Back,
        Forward,
        Reload,
        Quit
    ]
);

pub struct Nimbra {
    focus: FocusHandle,
    mails: Vec<Mail>,
    done: Vec<bool>,
    history: Vec<usize>,
    selected: usize,
    palette: bool,
    palette_sel: usize,
    dark: bool,
    sidebar_open: bool,
    /// Bumped on every toggle / reload so the matching animation replays.
    sidebar_gen: usize,
    reload_gen: usize,
    /// Threads visited before / after the current one, for the back and forward buttons.
    back: Vec<usize>,
    forward: Vec<usize>,
}

impl Nimbra {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let mails = inbox();
        // NIMBRA_SCREEN=palette|zero and NIMBRA_THEME=light open the mock in a given state,
        // which is handy for screenshots.
        let screen = std::env::var("NIMBRA_SCREEN").unwrap_or_default();
        let zero = screen == "zero";
        Self {
            focus: cx.focus_handle(),
            done: vec![zero; mails.len()],
            history: if zero {
                (0..mails.len()).collect()
            } else {
                Vec::new()
            },
            mails,
            selected: 0,
            palette: screen == "palette",
            palette_sel: 0,
            dark: std::env::var("NIMBRA_THEME").as_deref() != Ok("light"),
            sidebar_open: std::env::var("NIMBRA_SIDEBAR").as_deref() != Ok("closed"),
            sidebar_gen: 0,
            reload_gen: 0,
            back: Vec::new(),
            forward: Vec::new(),
        }
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    fn theme(&self) -> Theme {
        if self.dark {
            Theme::dark()
        } else {
            Theme::light()
        }
    }

    fn visible(&self) -> Vec<usize> {
        (0..self.mails.len()).filter(|i| !self.done[*i]).collect()
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.palette {
            let max = COMMANDS.len() as isize - 1;
            self.palette_sel = (self.palette_sel as isize + delta).clamp(0, max) as usize;
        } else {
            let visible = self.visible();
            if let Some(pos) = visible.iter().position(|i| *i == self.selected) {
                let next = (pos as isize + delta).clamp(0, visible.len() as isize - 1);
                self.open(visible[next as usize]);
            }
        }
        cx.notify();
    }

    /// Opens a thread and records the previous one so Back can return to it.
    fn open(&mut self, i: usize) {
        if i != self.selected {
            self.back.push(self.selected);
            self.forward.clear();
            self.selected = i;
        }
    }

    fn can_go_back(&self) -> bool {
        self.back.iter().any(|i| !self.done[*i])
    }

    fn can_go_forward(&self) -> bool {
        self.forward.iter().any(|i| !self.done[*i])
    }

    fn go_back(&mut self, cx: &mut Context<Self>) {
        // Skip threads that were marked done in the meantime.
        while let Some(i) = self.back.pop() {
            if !self.done[i] {
                self.forward.push(self.selected);
                self.selected = i;
                break;
            }
        }
        cx.notify();
    }

    fn go_forward(&mut self, cx: &mut Context<Self>) {
        while let Some(i) = self.forward.pop() {
            if !self.done[i] {
                self.back.push(self.selected);
                self.selected = i;
                break;
            }
        }
        cx.notify();
    }

    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_open = !self.sidebar_open;
        self.sidebar_gen += 1;
        cx.notify();
    }

    /// Runs `apply` with how open the sidebar is (0 = hidden, 1 = shown), animating
    /// towards the current state after a toggle.
    fn with_sidebar_progress<E: IntoElement + 'static>(
        &self,
        key: &'static str,
        element: E,
        apply: impl Fn(E, f32) -> E + 'static,
    ) -> AnyElement {
        let (from, to) = if self.sidebar_open {
            (0., 1.)
        } else {
            (1., 0.)
        };
        if self.sidebar_gen == 0 {
            return apply(element, to).into_any_element();
        }
        element
            .with_animation(
                (key, self.sidebar_gen),
                Animation::new(Duration::from_millis(260)).with_easing(ease_out_quint()),
                move |element, delta| apply(element, from + (to - from) * delta),
            )
            .into_any_element()
    }

    /// Mock "check for mail": everything that was cleared arrives again.
    fn reload(&mut self, cx: &mut Context<Self>) {
        self.reload_gen += 1;
        self.done.fill(false);
        self.history.clear();
        self.back.clear();
        self.forward.clear();
        self.selected = 0;
        self.palette = false;
        cx.notify();
    }

    fn mark_done(&mut self, cx: &mut Context<Self>) {
        let visible = self.visible();
        let Some(pos) = visible.iter().position(|i| *i == self.selected) else {
            return;
        };
        self.done[self.selected] = true;
        self.history.push(self.selected);
        if let Some(next) = visible
            .get(pos + 1)
            .or(pos.checked_sub(1).map(|p| &visible[p]))
        {
            self.selected = *next;
        }
        cx.notify();
    }

    fn undo(&mut self, cx: &mut Context<Self>) {
        if let Some(i) = self.history.pop() {
            self.done[i] = false;
            self.selected = i;
            cx.notify();
        }
    }
}

// ---------------------------------------------------------------- helpers

fn serif(text: &str, size: f32, color: Hsla) -> Div {
    // The tail of the title (after the last comma, or the last word) is set in italic.
    let start = text
        .rfind(", ")
        .map(|i| i + 2)
        .or(text.rfind(' ').map(|i| i + 1))
        .unwrap_or(text.len());
    let italic = HighlightStyle {
        font_style: Some(FontStyle::Italic),
        ..Default::default()
    };
    div()
        .font_family(SERIF)
        .text_size(px(size))
        .line_height(px((size * 1.14).round()))
        .text_color(color)
        .child(StyledText::new(text.to_string()).with_highlights([(start..text.len(), italic)]))
}

fn section(t: &Theme, label: &str) -> Div {
    div()
        .pl(px(10.))
        .pt(px(18.))
        .pb(px(8.))
        .child(overline(label.to_string(), t.text3))
}

fn nav(
    t: &Theme,
    name: &str,
    label: &str,
    count: Option<String>,
    selected: bool,
    tint: Option<Hsla>,
) -> Div {
    let hover = t.hover;
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .px(px(10.))
        .py(px(6.))
        .rounded(px(8.))
        .cursor_pointer()
        .when(selected, |d| {
            d.bg(t.selected).font_weight(FontWeight::MEDIUM)
        })
        .when(!selected, |d| d.hover(move |s| s.bg(hover)))
        .child(icon(
            name,
            16.,
            tint.unwrap_or(if selected { t.violet } else { t.text2 }),
        ))
        .child(
            div()
                .flex_1()
                .text_color(if selected { t.text } else { t.text2 })
                .child(label.to_string()),
        )
        .children(count.map(|n| mono(n, if selected { t.violet } else { t.text3 })))
}

fn card(t: &Theme) -> Div {
    div()
        .bg(t.hover)
        .border_1()
        .border_color(t.border)
        .rounded(px(12.))
}

// ---------------------------------------------------------------- panes

impl Nimbra {
    /// Sidebar toggle, back, forward and reload, sitting beside the traffic lights.
    /// Drawn above the panes so it stays put when the sidebar is hidden.
    fn window_controls(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let hover = t.hover;
        let control = |name: &str, enabled: bool| {
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(28.))
                .rounded(px(7.))
                .when(enabled, |d| d.cursor_pointer().hover(move |s| s.bg(hover)))
                .child(icon(name, 17., if enabled { t.text2 } else { t.faint }))
        };
        div()
            .absolute()
            .top(px(11.))
            .left(px(84.))
            .flex()
            .items_center()
            .gap(px(4.))
            .child(
                control("sidebar", true)
                    .id("toggle-sidebar")
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx))),
            )
            .child(div().w(px(6.)))
            .child(
                control("back", self.can_go_back())
                    .id("go-back")
                    .on_click(cx.listener(|this, _, _, cx| this.go_back(cx))),
            )
            .child(
                control("next", self.can_go_forward())
                    .id("go-forward")
                    .on_click(cx.listener(|this, _, _, cx| this.go_forward(cx))),
            )
            .child(div().w(px(6.)))
            .child(
                div()
                    .id("reload")
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(28.))
                    .rounded(px(7.))
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .on_click(cx.listener(|this, _, _, cx| this.reload(cx)))
                    .child(if self.reload_gen == 0 {
                        icon("reload", 17., t.text2).into_any_element()
                    } else {
                        // One full turn each time mail is checked.
                        icon("reload", 17., t.text2)
                            .with_animation(
                                ("reload-spin", self.reload_gen),
                                Animation::new(Duration::from_millis(700))
                                    .with_easing(ease_out_quint()),
                                |svg, delta| {
                                    svg.with_transformation(Transformation::rotate(percentage(
                                        delta,
                                    )))
                                },
                            )
                            .into_any_element()
                    }),
            )
    }

    fn sidebar(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let left = self.visible().len();
        let unread = self
            .visible()
            .iter()
            .filter(|i| self.mails[**i].unread)
            .count();
        let bars = [9., 14., 8., 17., 12., 20., 15.];

        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(236.))
            .h_full()
            .px(px(12.))
            .pb(px(14.))
            .bg(t.sidebar)
            .border_r_1()
            .border_color(t.border)
            // Space for the native traffic lights; dragging here moves the window.
            .child(
                div()
                    .h(px(52.))
                    .flex_none()
                    .on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move()),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .pl(px(10.))
                    .pr(px(6.))
                    .py(px(6.))
                    .rounded(px(9.))
                    .bg(t.hover)
                    .border_1()
                    .border_color(t.border)
                    .cursor_pointer()
                    .id("jump")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.palette = true;
                        cx.notify();
                    }))
                    .child(icon("search", 16., t.text3))
                    .child(
                        div()
                            .flex_1()
                            .text_color(t.text3)
                            .child("Search or jump to…"),
                    )
                    .child(kbd(t, "⌘"))
                    .child(kbd(t, "K")),
            )
            .child(section(t, "SPLIT INBOX"))
            .child(nav(
                t,
                "inbox",
                "Important",
                (unread > 0).then(|| unread.to_string()),
                true,
                None,
            ))
            .child(nav(t, "users", "Team", Some("4".into()), false, None))
            .child(nav(
                t,
                "calendar",
                "Calendar",
                Some("2".into()),
                false,
                None,
            ))
            .child(nav(
                t,
                "news",
                "Newsletters",
                Some("31".into()),
                false,
                None,
            ))
            .child(nav(t, "zap", "Other", Some("23".into()), false, None))
            .child(section(t, "MAILBOX"))
            .child(nav(t, "star", "Starred", None, false, None))
            .child(nav(t, "clock", "Snoozed", Some("3".into()), false, None))
            .child(nav(t, "bell", "Reminders", Some("2".into()), false, None))
            .child(nav(t, "draft", "Drafts", Some("1".into()), false, None))
            .child(nav(t, "send", "Sent", None, false, None))
            .child(nav(
                t,
                "done",
                "Done",
                (!self.history.is_empty()).then(|| self.history.len().to_string()),
                false,
                None,
            ))
            .child(section(t, "LABELS"))
            .child(nav(t, "tag", "Product", None, false, Some(t.violet)))
            .child(nav(t, "tag", "Design", None, false, Some(t.rose)))
            .child(nav(t, "tag", "Finance", None, false, Some(t.amber)))
            .child(nav(t, "tag", "Travel", None, false, Some(t.sky)))
            .child(div().flex_1())
            .child(
                card(t)
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .p(px(12.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(icon("zap", 16., t.amber))
                            .child(
                                div()
                                    .flex_1()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child("Inbox zero streak"),
                            )
                            .child(mono("7d", t.text3)),
                    )
                    .child(
                        div()
                            .flex()
                            .items_end()
                            .gap(px(5.))
                            .h(px(22.))
                            .children(bars.map(|h: f32| {
                                div().flex_1().h(px(h)).rounded(px(2.)).bg(gradient(
                                    180.,
                                    c(0xff7ab6, 1.),
                                    c(0x8b7cff, 1.),
                                ))
                            })),
                    )
                    .child(meta(
                        if left == 0 {
                            "Done at 10:52 · 28 min faster".to_string()
                        } else {
                            format!("{left} left · usually done by 11:20")
                        },
                        t.text3,
                    )),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .mt(px(10.))
                    .p(px(6.))
                    .child(avatar(Tone::Graphite, "LG", 28.))
                    .child(
                        div()
                            .flex_1()
                            .child(div().font_weight(FontWeight::SEMIBOLD).child("Luan Gjokaj"))
                            .child(meta("Work · 3 accounts", t.text3)),
                    )
                    .child(
                        icon_button(t, if self.dark { "sun" } else { "moon" })
                            .id("theme")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.dark = !this.dark;
                                cx.notify();
                            })),
                    ),
            )
    }

    fn row(&self, t: &Theme, i: usize, cx: &mut Context<Self>) -> Stateful<Div> {
        let m = &self.mails[i];
        let selected = i == self.selected;
        let hover = t.hover;

        div()
            .id(("row", i))
            .flex()
            .gap(px(12.))
            .pl(px(10.))
            .pr(px(14.))
            .py(px(12.))
            .rounded(px(12.))
            .cursor_pointer()
            .when(selected, |d| d.bg(t.selected))
            .when(!selected, |d| d.hover(move |s| s.bg(hover)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.open(i);
                cx.notify();
            }))
            .child(
                div().w(px(6.)).pt(px(11.)).flex_none().child(
                    div()
                        .size(px(6.))
                        .rounded_full()
                        .when(m.unread, |d| d.bg(t.violet)),
                ),
            )
            .child(avatar(m.tone, m.initials, 28.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(0.))
                    .gap(px(3.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .h(px(22.))
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .flex_none()
                                    .child(m.sender),
                            )
                            .children(m.thread.map(|n| mono(n, t.text3)))
                            .children(m.tag.map(|(label, a)| pill(t, label, t.accent(a))))
                            .child(div().flex_1())
                            .when(!selected, |d| d.child(mono(m.time, t.text3)))
                            .when(selected, |d| {
                                d.child(
                                    div()
                                        .flex()
                                        .gap(px(8.))
                                        .child(icon("done", 15., t.text2))
                                        .child(icon("clock", 15., t.text2))
                                        .child(icon("bell", 15., t.text2)),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(6.))
                            .child(
                                div()
                                    .flex_none()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(if selected { t.text } else { t.text2 })
                                    .child(
                                        m.subject
                                            .split(',')
                                            .next()
                                            .unwrap_or(m.subject)
                                            .to_string(),
                                    ),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .line_clamp(1)
                                    .text_ellipsis()
                                    .text_color(t.text3)
                                    .child(m.snippet),
                            ),
                    ),
            )
    }

    fn list(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let visible = self.visible();
        let unread = visible.iter().filter(|i| self.mails[**i].unread).count();
        let group = |label: &str| {
            div()
                .pl(px(14.))
                .pt(px(14.))
                .pb(px(6.))
                .child(overline(label.to_string(), t.faint))
        };
        let today: Vec<usize> = visible
            .iter()
            .copied()
            .filter(|i| !self.mails[*i].yesterday)
            .collect();
        let earlier: Vec<usize> = visible
            .iter()
            .copied()
            .filter(|i| self.mails[*i].yesterday)
            .collect();

        let segment = |label: &str, count: Option<(&str, Hsla)>, active: bool| {
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .px(px(12.))
                .py(px(4.))
                .rounded(px(7.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(if active { t.text } else { t.text3 })
                .when(active, |d| {
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
                .child(label.to_string())
                .children(count.map(|(n, color)| mono(n.to_string(), color)))
        };

        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(452.))
            .h_full()
            .bg(t.surface)
            .border_r_1()
            .border_color(t.border)
            // Makes room for the window controls while the sidebar is hidden.
            .child(
                self.with_sidebar_progress("list-top", div().flex_none(), |d, open| {
                    d.h(px(34. * (1. - open)))
                }),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .pl(px(24.))
                    .pr(px(18.))
                    .pt(px(26.))
                    .pb(px(10.))
                    .on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move())
                    .child(overline(format!("SPLIT INBOX · {unread} UNREAD"), t.text3))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .child(serif("Important", 38., t.text).flex_1())
                            .child(button(t, ButtonKind::Secondary, "Compose", Some("C"))),
                    )
                    .child(
                        div().flex().mt(px(10.)).child(
                            div()
                                .flex()
                                .gap(px(2.))
                                .p(px(3.))
                                .rounded(px(10.))
                                .bg(t.hover)
                                .border_1()
                                .border_color(t.border)
                                .child(segment("All", None, true))
                                .child(segment(
                                    "Unread",
                                    Some((&unread.to_string(), t.faint)),
                                    false,
                                ))
                                .child(segment("Needs reply", Some(("2", t.rose)), false)),
                        ),
                    ),
            )
            .child(
                div()
                    .id("rows")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .gap(px(2.))
                    .px(px(10.))
                    .pb(px(10.))
                    .overflow_y_scroll()
                    .when(!today.is_empty(), |d| d.child(group("TODAY")))
                    .children(today.into_iter().map(|i| self.row(t, i, cx)))
                    .when(!earlier.is_empty(), |d| d.child(group("YESTERDAY")))
                    .children(earlier.into_iter().map(|i| self.row(t, i, cx)))
                    .child(div().flex().justify_center().pt(px(18.)).child(meta(
                        "That’s everything important. 23 more in Other.",
                        t.faint,
                    ))),
            )
    }

    fn brief(&self, t: &Theme, m: &Mail) -> impl IntoElement {
        let todo = |text: &str, due: &str| {
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .child(
                    div()
                        .size(px(16.))
                        .flex_none()
                        .rounded(px(5.))
                        .border_1()
                        .border_color(t.border_strong),
                )
                .child(div().flex_1().child(text.to_string()))
                .child(mono(due.to_string(), t.amber))
        };
        // GPUI has no gradient strokes, so the border is a 1px gradient frame around the card.
        div()
            .p(px(1.))
            .rounded(px(14.))
            .bg(gradient(100., c(0x8b7cff, 0.75), c(0xffb86b, 0.3)))
            .shadow(vec![shadow(c(0x8b7cff, 0.16), 12., 40., -12.)])
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .px(px(20.))
                    .pt(px(16.))
                    .pb(px(18.))
                    .rounded(px(13.))
                    .bg(t.raised)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(icon("sparkles", 16., t.violet))
                            .child(div().font_weight(FontWeight::SEMIBOLD).child("Brief"))
                            .child(
                                meta(
                                    match m.thread {
                                        Some(n) => format!("from {n} messages"),
                                        None => "from 1 message".to_string(),
                                    },
                                    t.text3,
                                )
                                .flex_1(),
                            )
                            .child(kbd(t, "⌘"))
                            .child(kbd(t, "J")),
                    )
                    .child(
                        div()
                            .text_size(px(15.))
                            .line_height(px(26.))
                            .text_color(t.text2)
                            .child(m.brief),
                    )
                    .when(m.rich, |d| {
                        d.child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(8.))
                                .child(todo("Confirm pricing tiers on slide 14", "Fri"))
                                .child(todo("Sign off on the press embargo date", "Fri")),
                        )
                    }),
            )
    }

    fn composer(&self, t: &Theme, m: &Mail) -> impl IntoElement {
        let (typed, ghost) = m.draft;
        let first = m.sender.split(' ').next().unwrap_or(m.sender);
        let draft: SharedString = if typed.is_empty() {
            format!("Write a reply to {first}…").into()
        } else {
            format!("{typed}{ghost}").into()
        };
        let ghost_style = HighlightStyle {
            color: Some(t.faint),
            ..Default::default()
        };
        let ghost_from = if typed.is_empty() { 0 } else { typed.len() };
        let len = draft.len();

        div().px(px(32.)).pt(px(6.)).pb(px(24.)).child(
            div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .pl(px(18.))
                .pr(px(14.))
                .pt(px(14.))
                .pb(px(12.))
                .rounded(px(16.))
                .bg(t.raised)
                .border_1()
                .border_color(t.border_strong)
                .shadow(vec![shadow(t.shadow, 18., 48., -8.)])
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(icon("reply", 16., t.text3))
                        .child(
                            div()
                                .flex_1()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(t.text2)
                                .child(format!("Reply to {first}")),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .px(px(8.))
                                .py(px(3.))
                                .rounded_full()
                                .bg(t.selected)
                                .child(icon("sparkles", 14., t.violet))
                                .child(meta("Draft in your voice", t.violet)),
                        ),
                )
                .child(div().text_size(px(15.)).line_height(px(26.)).child(
                    StyledText::new(draft).with_highlights([(ghost_from..len, ghost_style)]),
                ))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(2.))
                        .child(icon_button(t, "clip"))
                        .child(icon_button(t, "image"))
                        .child(icon_button(t, "link"))
                        .child(icon_button(t, "clock"))
                        .when(!ghost.is_empty(), |d| {
                            d.child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .pl(px(8.))
                                    .child(kbd(t, "Tab"))
                                    .child(meta("to accept", t.text3)),
                            )
                        })
                        .child(div().flex_1())
                        .child(button(t, ButtonKind::Ghost, "Send later", None))
                        .child(button(t, ButtonKind::Primary, "Send", Some("⌘↵"))),
                ),
        )
    }

    fn reading(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let m = &self.mails[self.selected];
        let visible = self.visible();
        let pos = visible
            .iter()
            .position(|i| *i == self.selected)
            .unwrap_or(0)
            + 1;
        let divider = || div().h(px(1.)).bg(t.border);

        let earlier = |tone: Tone, initials: &str, name: &str, snippet: &str, when: &str| {
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .py(px(12.))
                .border_t_1()
                .border_color(t.border)
                .child(avatar(tone, initials, 24.))
                .child(
                    div()
                        .flex_none()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(t.text2)
                        .child(name.to_string()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .line_clamp(1)
                        .text_ellipsis()
                        .text_color(t.text3)
                        .child(snippet.to_string()),
                )
                .child(mono(when.to_string(), t.faint))
        };

        let attachment = |name: &str, info: &str, from: u32, to: u32| {
            card(t)
                .flex()
                .items_center()
                .gap(px(10.))
                .pl(px(8.))
                .pr(px(14.))
                .py(px(8.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(30.))
                        .rounded(px(8.))
                        .bg(gradient(135., c(from, 1.), c(to, 1.)))
                        .child(icon("file", 16., gpui::white())),
                )
                .child(
                    div()
                        .child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .child(name.to_string()),
                        )
                        .child(meta(info.to_string(), t.text3)),
                )
        };

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .bg(t.surface)
            // Toolbar
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(2.))
                    .pl(px(28.))
                    .pr(px(20.))
                    .py(px(11.))
                    .border_b_1()
                    .border_color(t.border)
                    .on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move())
                    .child(
                        icon_button(t, "done")
                            .id("mark-done")
                            .on_click(cx.listener(|this, _, _, cx| this.mark_done(cx))),
                    )
                    .child(kbd(t, "E"))
                    .child(div().w(px(6.)))
                    .child(
                        icon_button(t, "clock")
                            .id("snooze")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.palette = true;
                                cx.notify();
                            })),
                    )
                    .child(icon_button(t, "bell"))
                    .child(icon_button(t, "tag"))
                    .child(icon_button(t, "trash"))
                    .child(div().w(px(1.)).h(px(16.)).mx(px(6.)).bg(t.border_strong))
                    .child(icon_button(t, "reply"))
                    .child(icon_button(t, "reply-all"))
                    .child(icon_button(t, "forward"))
                    .child(div().flex_1())
                    .child(mono(format!("{pos} / {}", visible.len()), t.text3).mr(px(6.)))
                    .child(
                        icon_button(t, "up")
                            .id("prev")
                            .on_click(cx.listener(|this, _, _, cx| this.step(-1, cx))),
                    )
                    .child(
                        icon_button(t, "down")
                            .id("next")
                            .on_click(cx.listener(|this, _, _, cx| this.step(1, cx))),
                    )
                    .child(icon_button(t, "more")),
            )
            // Conversation
            .child(
                div()
                    .id("conversation")
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(22.))
                    .px(px(64.))
                    .pt(px(40.))
                    .pb(px(24.))
                    .child(
                        div()
                            .flex()
                            .gap(px(6.))
                            .when(m.rich, |d| d.child(pill(t, "Product", t.violet)))
                            .children(m.tag.map(|(label, a)| pill(t, label, t.accent(a))))
                            .when(m.rich, |d| d.child(pill(t, "Due Fri", t.amber)))
                            .when(!m.rich && m.tag.is_none(), |d| d.child(pill(t, "Important", t.accent(Accent::Violet)))),
                    )
                    .child(serif(m.subject, 38., t.text))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .child(
                                div()
                                    .relative()
                                    .flex_none()
                                    .h(px(28.))
                                    .w(px(if m.rich { 68. } else { 48. }))
                                    .children(
                                        [Some((m.tone, m.initials)), m.rich.then_some((Tone::Mint, "JR")), Some((Tone::Graphite, "LG"))]
                                            .into_iter()
                                            .flatten()
                                            .enumerate()
                                            .map(|(n, (tone, initials))| {
                                                avatar(tone, initials, 28.).absolute().top_0().left(px(n as f32 * 20.))
                                            }),
                                    ),
                            )
                            .child(
                                meta(
                                    if m.rich {
                                        "Mira, Jonah and you · 3 messages".to_string()
                                    } else {
                                        format!("{} and you", m.sender)
                                    },
                                    t.text3,
                                )
                                .flex_1(),
                            )
                            .child(mono(m.time, t.text3)),
                    )
                    .child(self.brief(t, m))
                    .when(m.rich, |d| {
                        d.child(
                            div()
                                .child(earlier(Tone::Mint, "JR", "Jonah Reyes", "Numbers look right to me. Looping Luan in for final sign-off.", "Yesterday"))
                                .child(earlier(Tone::Violet, "MK", "Mira Kovač", "First pass of the deck. Timeline still says Nov 28, fixing now.", "Tue")),
                        )
                    })
                    .child(divider())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .child(avatar(m.tone, m.initials, 28.))
                            .child(
                                div()
                                    .flex_1()
                                    .child(div().font_weight(FontWeight::SEMIBOLD).child(m.sender))
                                    .child(meta(if m.rich { "to you, Jonah" } else { "to you" }, t.text3)),
                            )
                            .child(mono(m.time, t.text3)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(12.))
                            .pl(px(40.))
                            .text_size(px(15.))
                            .line_height(px(26.))
                            .text_color(t.text2)
                            .children(m.body.iter().map(|p| div().child(*p))),
                    )
                    .when(m.rich, |d| {
                        d.child(
                            div()
                                .flex()
                                .gap(px(10.))
                                .pl(px(40.))
                                .child(attachment("Q4-Launch-Plan.pdf", "PDF · 4.2 MB", 0xff7ab6, 0xff4f7b))
                                .child(attachment("Pricing-tiers.fig", "Figma · 1.1 MB", 0x8b7cff, 0x5b47e0)),
                        )
                    }),
            ))
            .child(self.composer(t, m))
    }

    fn zero(&self, t: &Theme) -> impl IntoElement {
        let ink = c(0xf5f3ef, 1.);
        let soft = c(0xffffff, 0.7);
        let glass = |d: Div| {
            d.bg(c(0xffffff, 0.07))
                .border_1()
                .border_color(c(0xffffff, 0.14))
                .rounded(px(14.))
        };
        let stat = |name: &str, color: Hsla, label: &str, value: &str| {
            glass(div())
                .flex()
                .flex_col()
                .gap(px(6.))
                .pl(px(20.))
                .pr(px(28.))
                .py(px(16.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(icon(name, 16., color))
                        .child(meta(label.to_string(), soft)),
                )
                .child(
                    div()
                        .font_family(SERIF)
                        .text_size(px(40.))
                        .line_height(px(44.))
                        .text_color(ink)
                        .child(value.to_string()),
                )
        };
        // Deterministic star field.
        let mut seed = 7u32;
        let mut rnd = move || {
            seed = (seed.wrapping_mul(9301).wrapping_add(49297)) % 233280;
            seed as f32 / 233280.
        };
        let stars: Vec<Div> = (0..48)
            .map(|_| {
                let (x, y, a) = (rnd(), rnd() * 0.42, 0.2 + rnd() * 0.6);
                div()
                    .absolute()
                    .left(relative(x))
                    .top(relative(y))
                    .size(px(if a > 0.7 { 2.5 } else { 1.5 }))
                    .rounded_full()
                    .bg(c(0xffffff, a))
            })
            .collect();
        let layer = |angle: f32, color: u32, alpha: f32, end: f32| {
            div()
                .absolute()
                .inset_0()
                .bg(fade(angle, color, alpha, end))
        };

        div()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .p(px(20.))
            .when(!self.sidebar_open, |d| d.pt(px(52.)))
            .bg(t.surface)
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .items_center()
                    .size_full()
                    .overflow_hidden()
                    .rounded(px(20.))
                    .border_1()
                    .border_color(t.border)
                    .bg(c(0x0a0912, 1.))
                    // Aurora: GPUI has no blur filter, so the glow is layered two-stop gradients.
                    .child(layer(45., 0x6b4dff, 0.9, 0.62))
                    .child(layer(315., 0xffa94d, 0.6, 0.5))
                    .child(layer(0., 0xff4fa0, 0.55, 0.42))
                    .child(layer(300., 0x3fa9ff, 0.22, 0.4))
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .right_0()
                            .top(relative(0.72))
                            .h(px(1.))
                            .bg(c(0xffffff, 0.22)),
                    )
                    .children(stars)
                    .child(div().h(px(72.)).flex_none())
                    .child(overline("THURSDAY, NOVEMBER 20", c(0xffffff, 0.55)))
                    .child(div().h(px(18.)).flex_none())
                    .child(serif("Inbox zero.", 120., ink))
                    .child(div().h(px(14.)).flex_none())
                    .child(
                        div()
                            .text_size(px(17.))
                            .line_height(px(26.))
                            .text_color(soft)
                            .child(
                                "Every important thread is handled. Go make something wonderful.",
                            ),
                    )
                    .child(div().h(px(40.)).flex_none())
                    .child(
                        div()
                            .flex()
                            .gap(px(12.))
                            .child(stat(
                                "done",
                                t.mint,
                                "threads handled",
                                &(32 + self.history.len()).to_string(),
                            ))
                            .child(stat("clock", t.sky, "time in inbox", "38m"))
                            .child(stat("zap", t.amber, "zero streak", "8 days")),
                    )
                    .child(div().flex_1())
                    .child(
                        glass(div())
                            .bg(c(0x14131c, 0.72))
                            .flex()
                            .items_center()
                            .gap(px(14.))
                            .pl(px(14.))
                            .pr(px(10.))
                            .py(px(10.))
                            .text_color(ink)
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .size(px(34.))
                                    .rounded(px(9.))
                                    .bg(Tone::Sky.gradient())
                                    .child(icon("calendar", 16., gpui::white())),
                            )
                            .child(
                                div()
                                    .mr(px(40.))
                                    .child(
                                        div()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child("Up next · Design crit"),
                                    )
                                    .child(meta(
                                        "2:00 PM · Studio B · in 42 min",
                                        c(0xffffff, 0.55),
                                    )),
                            )
                            .child(button(
                                &Theme::dark(),
                                ButtonKind::Secondary,
                                "Join",
                                Some("J"),
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .mt(px(18.))
                            .mb(px(36.))
                            .child(meta("Press", c(0xffffff, 0.5)))
                            .child(kbd(&Theme::dark(), "Z"))
                            .child(meta("to bring the last thread back", c(0xffffff, 0.5))),
                    ),
            )
    }

    fn palette(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let m = &self.mails[self.selected];
        let items =
            COMMANDS
                .iter()
                .enumerate()
                .flat_map(|(i, (name, label, hint, key, header))| {
                    let active = i == self.palette_sel;
                    let hover = t.hover;
                    let head = header.map(|h| {
                        div()
                            .pl(px(12.))
                            .pt(px(12.))
                            .pb(px(6.))
                            .child(overline(h, t.faint))
                            .into_any_element()
                    });
                    let row = div()
                        .id(("command", i))
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .pl(px(10.))
                        .pr(px(12.))
                        .py(px(8.))
                        .rounded(px(10.))
                        .cursor_pointer()
                        .when(active, |d| d.bg(t.selected))
                        .when(!active, |d| d.hover(move |s| s.bg(hover)))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.palette = false;
                            this.mark_done(cx);
                        }))
                        .child(
                            div()
                                .flex()
                                .flex_none()
                                .items_center()
                                .justify_center()
                                .size(px(28.))
                                .rounded(px(8.))
                                .when(active, |d| {
                                    d.bg(gradient(135., c(0x8b7cff, 1.), c(0xff7ab6, 1.)))
                                })
                                .when(!active, |d| d.bg(t.hover).border_1().border_color(t.border))
                                .child(icon(
                                    name,
                                    16.,
                                    if active { gpui::white() } else { t.text2 },
                                )),
                        )
                        .child(
                            div()
                                .flex_none()
                                .font_weight(FontWeight::MEDIUM)
                                .child(*label),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .line_clamp(1)
                                .text_ellipsis()
                                .text_color(t.text3)
                                .child(*hint),
                        )
                        .child(kbd(t, key))
                        .into_any_element();
                    head.into_iter().chain(std::iter::once(row))
                });

        let hint = |keys: &[&str], label: &str| {
            div()
                .flex()
                .items_center()
                .gap(px(5.))
                .children(keys.iter().map(|k| kbd(t, k)))
                .child(meta(label.to_string(), t.text3))
        };

        div()
            .id("scrim")
            .absolute()
            .inset_0()
            .flex()
            .items_start()
            .justify_center()
            .pt(px(150.))
            .bg(t.scrim)
            .occlude()
            .on_click(cx.listener(|this, _, _, cx| {
                this.palette = false;
                cx.notify();
            }))
            .child(
                div()
                    .id("palette")
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .flex()
                    .flex_col()
                    .w(px(660.))
                    .h_auto()
                    .max_h(px(520.))
                    .overflow_hidden()
                    .rounded(px(18.))
                    .bg(t.raised)
                    .border_1()
                    .border_color(t.border_strong)
                    .shadow(vec![
                        shadow(c(0x8b7cff, 0.22), 0., 120., 10.),
                        shadow(c(0x000000, if t.dark { 0.6 } else { 0.2 }), 30., 80., -10.),
                    ])
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .pl(px(20.))
                            .pr(px(16.))
                            .py(px(18.))
                            .border_b_1()
                            .border_color(t.border)
                            .child(icon("search", 18., t.text3))
                            .child(
                                div()
                                    .flex()
                                    .flex_1()
                                    .items_center()
                                    .gap(px(2.))
                                    .text_size(px(17.))
                                    .line_height(px(24.))
                                    .child("snooze until i’m free")
                                    .child(div().w(px(2.)).h(px(20.)).rounded(px(1.)).bg(t.violet)),
                            )
                            .child(pill(
                                t,
                                m.subject.split(',').next().unwrap_or(m.subject),
                                t.violet,
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .px(px(8.))
                            .pt(px(6.))
                            .pb(px(8.))
                            .children(items),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(16.))
                            .pl(px(20.))
                            .pr(px(18.))
                            .py(px(12.))
                            .bg(t.hover)
                            .border_t_1()
                            .border_color(t.border)
                            .child(hint(&["↑", "↓"], "Navigate"))
                            .child(hint(&["↵"], "Snooze"))
                            .child(hint(&["esc"], "Close"))
                            .child(div().flex_1())
                            .child(
                                div()
                                    .font_family(SERIF)
                                    .italic()
                                    .text_size(px(18.))
                                    .line_height(px(20.))
                                    .text_color(t.text3)
                                    .child("Nimbra"),
                            ),
                    ),
            )
    }
}

impl Render for Nimbra {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme();
        let empty = self.visible().is_empty();

        div()
            .id("nimbra")
            .key_context("Nimbra")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Next, _, cx| this.step(1, cx)))
            .on_action(cx.listener(|this, _: &Prev, _, cx| this.step(-1, cx)))
            .on_action(cx.listener(|this, _: &MarkDone, _, cx| {
                if !this.palette {
                    this.mark_done(cx)
                }
            }))
            .on_action(cx.listener(|this, _: &Undo, _, cx| this.undo(cx)))
            .on_action(cx.listener(|this, _: &TogglePalette, _, cx| {
                this.palette = !this.palette && !this.visible().is_empty();
                this.palette_sel = 0;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Dismiss, _, cx| {
                this.palette = false;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Confirm, _, cx| {
                if this.palette {
                    this.palette = false;
                    this.mark_done(cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ToggleTheme, _, cx| {
                this.dark = !this.dark;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| this.toggle_sidebar(cx)))
            .on_action(cx.listener(|this, _: &Back, _, cx| this.go_back(cx)))
            .on_action(cx.listener(|this, _: &Forward, _, cx| this.go_forward(cx)))
            .on_action(cx.listener(|this, _: &Reload, _, cx| this.reload(cx)))
            .on_action(|_: &Quit, _, cx: &mut App| cx.quit())
            .relative()
            .flex()
            .size_full()
            .font_family(crate::theme::SANS)
            .text_size(px(13.))
            .line_height(px(18.))
            .text_color(t.text)
            .child(
                self.with_sidebar_progress(
                    "sidebar-width",
                    div()
                        .flex_none()
                        .h_full()
                        .overflow_hidden()
                        .child(self.sidebar(&t, cx)),
                    |d, open| d.w(px(236. * open)),
                ),
            )
            .when(empty, |d| d.child(self.zero(&t)))
            .when(!empty, |d| {
                d.child(self.list(&t, cx)).child(self.reading(&t, cx))
            })
            .child(self.window_controls(&t, cx))
            .when(self.palette && !empty, |d| d.child(self.palette(&t, cx)))
    }
}
