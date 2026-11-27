//! The Snooze menu: quick times with the date each comes back, a calendar to pick a day, and
//! a switch to come back only if nobody has replied. A small window of its own, like the
//! toolbar menus, so it floats above the email's web view.

use chrono::{Datelike, Duration as Days, Local, NaiveDate};

use super::*;

const WIDTH: f32 = 288.;
const ROW: f32 = 32.;
const HEADER: f32 = 28.;
const DIVIDER: f32 = 9.;
/// The card's padding and border, top and bottom together.
const CHROME: f32 = 2. * 6. + 2.;
const CELL: f32 = (WIDTH - 2. * 6. - 2.) / 7.;
const CAL_HEAD: f32 = 36.;
const CAL_DAYS: f32 = 22.;
/// The footer with Quick times and the Snooze button.
const CAL_FOOT: f32 = 44.;
/// The height of the footer's two buttons.
const FOOT_BUTTON: f32 = 30.;
/// The field to type any time in, above the hours.
const FIELD: f32 = 38.;
/// The Time row under the calendar, and the hours it opens to.
const TIME_ROW: f32 = 34.;
const CHIP: f32 = 28.;
/// Between the hours, and around them: the same all round.
const CHIP_GAP: f32 = 4.;
const CHIP_INSET: f32 = 4.;
/// The hours offered: 6 AM to 9 PM, four to a row.
const HOURS: std::ops::RangeInclusive<u32> = 6..=21;
const TIME_GRID: f32 = 4. * CHIP + 5. * CHIP_GAP;

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Mode {
    List,
    /// A month (its first day) to pick a day from, with the hours open below it or not.
    Calendar(NaiveDate, bool),
}

impl Mode {
    fn height(self, rows: usize) -> f32 {
        match self {
            Mode::List => CHROME + HEADER + rows as f32 * ROW + DIVIDER + ROW + DIVIDER + ROW,
            Mode::Calendar(_, hours) => {
                CHROME
                    + CAL_HEAD
                    + CAL_DAYS
                    + 6. * CELL
                    + DIVIDER
                    + TIME_ROW
                    + if hours {
                        FIELD + CHIP_GAP + TIME_GRID
                    } else {
                        0.
                    }
                    + CAL_FOOT
            }
        }
    }
}

/// How a time reads in the menu: the weekday and time within the week, the date after that.
fn when_text(when: chrono::DateTime<Local>) -> String {
    let days = (when.date_naive() - Local::now().date_naive()).num_days();
    match days {
        0 => when.format("Today, %-I:%M %p").to_string(),
        1..=6 => when.format("%a, %-I:%M %p").to_string(),
        _ => when.format("%a, %b %-d, %-I:%M %p").to_string(),
    }
}

fn choice(icon: &'static str, label: &str, when: chrono::DateTime<Local>) -> SnoozeChoice {
    SnoozeChoice {
        icon,
        label: label.to_string(),
        hint: when_text(when),
        header: None,
        until_ms: when.timestamp_millis(),
        no_reply: false,
    }
}

/// The quick times: later today, tomorrow morning and evening, the weekend, next week
/// (`snooze::quick_times`), as the menu shows them.
pub(super) fn quick_times() -> Vec<SnoozeChoice> {
    use nimbra_core::snooze::Quick;
    nimbra_core::snooze::quick_times(Local::now())
        .into_iter()
        .map(|(kind, when)| match kind {
            Quick::LaterToday => choice("clock", "Later today", when),
            Quick::InThreeHours => choice("clock", "In three hours", when),
            Quick::Tomorrow => choice("calendar", "Tomorrow", when),
            Quick::TomorrowEvening => choice("calendar", "Tomorrow evening", when),
            Quick::ThisWeekend => choice("calendar", "This weekend", when),
            Quick::NextWeek => choice("calendar", "Next week", when),
        })
        .collect()
}

/// When a day picked in the calendar comes back: at the time chosen under it. None for a
/// time already gone by.
fn day_time(date: NaiveDate, time: (u32, u32)) -> Option<chrono::DateTime<Local>> {
    nimbra_core::snooze::day_time(date, time, Local::now())
}

// A time typed in (the engine's: both apps read them the same way).
use nimbra_core::snooze::parse_time;

/// The day the calendar starts on: today if the time is still ahead today, else tomorrow.
fn first_open_day(time: (u32, u32)) -> NaiveDate {
    nimbra_core::snooze::first_open_day(time, Local::now())
}

/// "9:00 AM".
fn time_text((hour, minute): (u32, u32)) -> String {
    chrono::NaiveTime::from_hms_opt(hour, minute, 0)
        .map(|t| t.format("%-I:%M %p").to_string())
        .unwrap_or_default()
}

pub(super) struct SnoozeMenu {
    owner: gpui::WeakEntity<Nimbra>,
    mode: Mode,
    choices: Vec<SnoozeChoice>,
    /// Come back only if nobody replied (a reminder).
    alert: bool,
    /// The time a day picked in the calendar comes back at (hour, minute).
    time: (u32, u32),
    /// The day picked in the calendar; the Snooze button under it confirms.
    day: NaiveDate,
    /// Where any time can be typed (while the hours are open).
    typed: gpui::Entity<crate::input::TextInput>,
    /// What is typed there is not a time.
    typed_bad: bool,
    _typing: gpui::Subscription,
    /// The row lit for the keyboard: the times, then Pick a date, then the switch.
    sel: usize,
    dark: bool,
    focus: gpui::FocusHandle,
    was_active: bool,
    opened: std::time::Instant,
    _activation: gpui::Subscription,
}

impl SnoozeMenu {
    fn rows(&self) -> usize {
        self.choices.len() + 2
    }

    fn pick(&mut self, choice: SnoozeChoice, window: &mut Window, cx: &mut Context<Self>) {
        let alert = self.alert;
        window.remove_window();
        self.owner
            .update(cx, |owner, cx| owner.snooze_picked(choice, alert, cx))
            .ok();
    }

    fn calendar(
        &mut self,
        month: NaiveDate,
        hours: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let origin = window.bounds().origin;
        window.remove_window();
        self.owner
            .update(cx, |owner, cx| {
                owner.open_snooze_window(origin, Mode::Calendar(month, hours), cx)
            })
            .ok();
    }

    /// The moment the calendar's day and time make, if it is still ahead.
    fn chosen(&self) -> Option<chrono::DateTime<Local>> {
        day_time(self.day, self.time)
    }

    /// Snoozes until the day and time picked in the calendar.
    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(when) = self.chosen() else {
            return;
        };
        let picked = SnoozeChoice {
            icon: "calendar",
            label: self.day.format("%A, %B %-d").to_string(),
            hint: when_text(when),
            header: None,
            until_ms: when.timestamp_millis(),
            no_reply: false,
        };
        self.pick(picked, window, cx);
    }

    /// A day clicked in the calendar: picked, not yet snoozed.
    fn set_day(&mut self, day: NaiveDate, cx: &mut Context<Self>) {
        self.day = day;
        self.owner
            .update(cx, |owner, _| owner.snooze_day = Some(day))
            .ok();
        cx.notify();
    }

    /// Something typed in the time field: the time follows it while it reads as one.
    fn typed_time(&mut self, cx: &mut Context<Self>) {
        let text = self.typed.read(cx).text();
        match parse_time(&text) {
            Some(time) => {
                self.time = time;
                self.typed_bad = false;
                self.owner
                    .update(cx, |owner, _| owner.snooze_time = time)
                    .ok();
            }
            None => self.typed_bad = !text.trim().is_empty(),
        }
        cx.notify();
    }

    /// A time picked under the calendar: kept for next time, and the hours fold away.
    fn set_time(
        &mut self,
        month: NaiveDate,
        time: (u32, u32),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.owner
            .update(cx, |owner, _| owner.snooze_time = time)
            .ok();
        self.calendar(month, false, window, cx);
    }

    /// From the calendar back to the quick times.
    fn back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let origin = window.bounds().origin;
        window.remove_window();
        self.owner
            .update(cx, |owner, cx| {
                owner.open_snooze_window(origin, Mode::List, cx)
            })
            .ok();
    }

    fn toggle_alert(&mut self, cx: &mut Context<Self>) {
        self.alert = !self.alert;
        let alert = self.alert;
        self.owner
            .update(cx, |owner, _| owner.snooze_alert = alert)
            .ok();
        cx.notify();
    }

    fn activate(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(choice) = self.choices.get(index).cloned() {
            self.pick(choice, window, cx);
        } else if index == self.choices.len() {
            let today = Local::now().date_naive();
            self.calendar(today.with_day(1).unwrap_or(today), false, window, cx);
        } else {
            self.toggle_alert(cx);
        }
    }

    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.remove_window();
        self.owner
            .update(cx, |owner, _| owner.snooze_window = None)
            .ok();
    }

    fn key(&mut self, event: &gpui::KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        match (self.mode, event.keystroke.key.as_str()) {
            (_, "escape") => self.dismiss(window, cx),
            (Mode::List, "down") => {
                self.sel = (self.sel + 1) % self.rows();
                cx.notify();
            }
            (Mode::List, "up") => {
                self.sel = (self.sel + self.rows() - 1) % self.rows();
                cx.notify();
            }
            (Mode::List, "enter" | "space") => self.activate(self.sel, window, cx),
            (Mode::Calendar(..), "backspace")
                if !self.typed.focus_handle(cx).is_focused(window) =>
            {
                self.back(window, cx)
            }
            (Mode::Calendar(..), "enter") => self.confirm(window, cx),
            _ => {}
        }
    }

    fn list(&self, t: &Theme, cx: &mut Context<Self>) -> Div {
        let hover = t.hover;
        let row = |index: usize| {
            let lit = index == self.sel;
            div()
                .id(("snooze-row", index))
                .flex()
                .flex_none()
                .items_center()
                .gap(dp(10.))
                .h(dp(ROW))
                .px(dp(10.))
                .rounded(dp(8.))
                .cursor_pointer()
                .when(lit, |d| d.bg(hover))
                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    if *hovered && this.sel != index {
                        this.sel = index;
                        cx.notify();
                    }
                }))
        };
        let divider = || {
            div()
                .flex_none()
                .h(dp(DIVIDER))
                .flex()
                .items_center()
                .px(dp(4.))
                .child(div().h(dp(1.)).w_full().bg(t.border))
        };
        let times = self.choices.iter().enumerate().map(|(index, c)| {
            let picked = c.clone();
            row(index)
                .child(icon(c.icon, 16., t.violet))
                .child(div().flex_1().child(c.label.clone()))
                .child(meta(c.hint.clone(), t.text3))
                .on_click(
                    cx.listener(move |this, _, window, cx| this.pick(picked.clone(), window, cx)),
                )
        });
        let pick_date = self.choices.len();
        let alert_row = pick_date + 1;
        let on = self.alert;
        // The switch: a pill with a knob, lit when on.
        let switch = div()
            .flex_none()
            .w(dp(30.))
            .h(dp(18.))
            .p(dp(2.))
            .rounded(dp(9.))
            .bg(if on { t.violet } else { t.border_strong })
            .child(
                div()
                    .size(dp(14.))
                    .rounded(dp(7.))
                    .bg(gpui::white())
                    .ml(dp(if on { 12. } else { 0. })),
            );
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_none()
                    .h(dp(HEADER))
                    .flex()
                    .items_center()
                    .px(dp(10.))
                    .child(meta("Snooze", t.text3)),
            )
            .children(times)
            .child(divider())
            .child(
                row(pick_date)
                    .child(icon("calendar", 16., t.text2))
                    .child(div().flex_1().child("Pick a date"))
                    .child(icon("next", 14., t.text3))
                    .on_click(
                        cx.listener(move |this, _, window, cx| {
                            this.activate(pick_date, window, cx)
                        }),
                    ),
            )
            .child(divider())
            .child(
                row(alert_row)
                    .child(icon("bell", 16., t.amber))
                    .child(div().flex_1().child("Only if nobody replies"))
                    .child(switch)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_alert(cx))),
            )
    }

    fn month(&self, first: NaiveDate, hours: bool, t: &Theme, cx: &mut Context<Self>) -> Div {
        let time = self.time;
        let today = Local::now().date_naive();
        let (hover, selected) = (t.hover, t.selected);
        let prev = first
            .checked_sub_months(chrono::Months::new(1))
            .unwrap_or(first);
        let next = first
            .checked_add_months(chrono::Months::new(1))
            .unwrap_or(first);
        // Not before this month.
        let can_go_back = prev >= today.with_day(1).unwrap_or(today);
        let nav = |id: &'static str, name: &'static str, enabled: bool| {
            div()
                .id(id)
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(dp(26.))
                .rounded(dp(7.))
                .child(icon(name, 14., if enabled { t.text2 } else { t.faint }))
                .when(enabled, |d| d.cursor_pointer().hover(move |s| s.bg(hover)))
        };
        // Weeks start on Monday; six rows hold every month.
        let lead = first.weekday().num_days_from_monday() as i64;
        let start = first - Days::days(lead);
        let cells = (0..42).map(|i| {
            let date = start + Days::days(i);
            let inside = date.month() == first.month();
            let open = date >= today;
            let is_today = date == today;
            let picked_day = date == self.day;
            let mut cell = div()
                .id(("snooze-day", i as usize))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(dp(CELL))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(dp(30.))
                        .rounded(dp(15.))
                        .when(is_today && !picked_day, |d| {
                            d.border_1().border_color(t.violet)
                        })
                        .when(picked_day, |d| d.bg(t.violet))
                        .text_color(if picked_day {
                            gpui::white()
                        } else if !open {
                            t.faint
                        } else if inside {
                            t.text
                        } else {
                            t.text3
                        })
                        .child(date.day().to_string()),
                );
            if open {
                cell = cell
                    .cursor_pointer()
                    .when(!picked_day, |d| {
                        d.hover(move |s| s.bg(hover).rounded(dp(8.)))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| this.set_day(date, cx)));
            }
            cell
        });
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_none()
                    .h(dp(CAL_HEAD))
                    .flex()
                    .items_center()
                    .gap(dp(4.))
                    .px(dp(4.))
                    .child(
                        nav("snooze-prev", "back", can_go_back).when(can_go_back, |d| {
                            d.on_click(cx.listener(move |this, _, window, cx| {
                                this.calendar(prev, hours, window, cx)
                            }))
                        }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_center()
                            .font_weight(FontWeight::MEDIUM)
                            .child(first.format("%B %Y").to_string()),
                    )
                    .child(nav("snooze-next", "next", true).on_click(cx.listener(
                        move |this, _, window, cx| this.calendar(next, hours, window, cx),
                    ))),
            )
            .child(div().flex().flex_none().h(dp(CAL_DAYS)).children(
                ["M", "T", "W", "T", "F", "S", "S"].into_iter().map(|d| {
                    div()
                        .w(dp(CELL))
                        .flex()
                        .justify_center()
                        .child(meta(d, t.text3))
                }),
            ))
            .child(div().flex().flex_wrap().w(dp(7. * CELL)).children(cells))
            .child(
                div()
                    .flex_none()
                    .h(dp(DIVIDER))
                    .flex()
                    .items_center()
                    .px(dp(4.))
                    .child(div().h(dp(1.)).w_full().bg(t.border)),
            )
            .child(
                div()
                    .id("snooze-time")
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(dp(10.))
                    .h(dp(TIME_ROW))
                    // Its edges line up with the hours' grid below it.
                    .mx(dp(CHIP_INSET))
                    .px(dp(10.))
                    .rounded(dp(8.))
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .when(hours, |d| d.bg(hover))
                    .child(icon("clock", 16., t.violet))
                    .child(div().flex_1().child("Time"))
                    .child(div().text_color(t.text2).child(time_text(time)))
                    .child(icon(if hours { "up" } else { "down" }, 14., t.text3))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.calendar(first, !hours, window, cx)
                    })),
            )
            .when(hours, |d| {
                d.child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(dp(8.))
                        .h(dp(FIELD))
                        .mt(dp(CHIP_GAP))
                        .mx(dp(CHIP_INSET))
                        .px(dp(12.))
                        .rounded(dp(8.))
                        .text_size(dp(14.))
                        .border_1()
                        .border_color(if self.typed_bad {
                            t.rose
                        } else {
                            t.border_strong
                        })
                        .child(div().flex_1().child(self.typed.clone()))
                        .when(self.typed_bad, |d| d.child(meta("Not a time", t.rose))),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .flex_none()
                        .gap(dp(CHIP_GAP))
                        .py(dp(CHIP_GAP))
                        .px(dp(CHIP_INSET))
                        .children(HOURS.map(|hour| {
                            let chosen = time == (hour, 0);
                            div()
                                .id(("snooze-hour", hour as usize))
                                .flex()
                                .items_center()
                                .justify_center()
                                .w(dp((7. * CELL - 2. * CHIP_INSET - 3. * CHIP_GAP) / 4.))
                                .h(dp(CHIP))
                                .rounded(dp(7.))
                                .text_size(dp(12.))
                                .cursor_pointer()
                                .when(chosen, |d| d.bg(t.violet).text_color(gpui::white()))
                                .when(!chosen, |d| {
                                    d.border_1()
                                        .border_color(t.border)
                                        .text_color(t.text2)
                                        .hover(move |s| s.bg(hover))
                                })
                                .child(
                                    chrono::NaiveTime::from_hms_opt(hour, 0, 0)
                                        .map(|t| t.format("%-I %p").to_string())
                                        .unwrap_or_default(),
                                )
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.set_time(first, (hour, 0), window, cx)
                                }))
                        })),
                )
            })
            .child(
                div()
                    .flex_none()
                    .h(dp(CAL_FOOT))
                    .flex()
                    .items_center()
                    .justify_between()
                    .px(dp(4.))
                    .child(
                        // A secondary button the Snooze button's size, so the two line up.
                        div()
                            .id("snooze-quick")
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap(dp(6.))
                            .h(dp(FOOT_BUTTON))
                            .pl(dp(10.))
                            .pr(dp(14.))
                            .rounded(dp(9.))
                            .bg(t.hover)
                            .border_1()
                            .border_color(t.border_strong)
                            .cursor_pointer()
                            .hover(move |s| s.bg(selected))
                            .text_size(dp(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(t.text)
                            .child(icon("back", 12., t.text2))
                            .child("Quick times")
                            .press_soft()
                            .on_click(cx.listener(|this, _, window, cx| this.back(window, cx))),
                    )
                    .child({
                        let ready = self.chosen().is_some();
                        let label = if ready { "Snooze" } else { "Pick a later time" };
                        let button = button(t, ButtonKind::Primary, label, Some("↵"))
                            .text_size(dp(12.))
                            .h(dp(FOOT_BUTTON))
                            .py_0()
                            .id("snooze-confirm")
                            .press_soft()
                            .when(!ready, |d| d.opacity(0.5).cursor_default())
                            .when(ready, |d| {
                                d.on_click(
                                    cx.listener(|this, _, window, cx| this.confirm(window, cx)),
                                )
                            });
                        crate::ui::glow(button, "snooze-confirm", true)
                    }),
            )
    }
}

impl Render for SnoozeMenu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Every window draws at the interface zoom.
        crate::ui::apply_zoom(window);
        let t = if self.dark {
            Theme::dark()
        } else {
            Theme::light()
        };
        let body = match self.mode {
            Mode::List => self.list(&t, cx),
            Mode::Calendar(first, hours) => self.month(first, hours, &t, cx),
        };
        div()
            .id("snooze-menu")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                this.key(event, window, cx)
            }))
            .size_full()
            .p(dp(MENU_MARGIN))
            .font_family(crate::theme::SANS)
            .text_size(dp(13.))
            .line_height(dp(18.))
            .text_color(t.text)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .size_full()
                    .p(dp(6.))
                    .rounded(dp(12.))
                    .bg(t.raised)
                    .border_1()
                    .border_color(t.border_strong)
                    .shadow(vec![popup_shadow(&t)])
                    .child(body),
            )
    }
}

impl Nimbra {
    /// Opens the Snooze menu for the open (or picked) threads: under the reader's Snooze
    /// button, or at `at` (window coordinates) when opened from somewhere else.
    pub(super) fn open_snooze(
        &mut self,
        at: Option<gpui::Point<gpui::Pixels>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_snooze_in(at, Mode::List, window, cx)
    }

    /// The same, showing `mode` first.
    pub(super) fn open_snooze_in(
        &mut self,
        at: Option<gpui::Point<gpui::Pixels>>,
        mode: Mode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.compose.is_some() || self.visible().is_empty() {
            return;
        }
        if self.snooze_window.is_some() {
            self.close_snooze_window(cx);
            return;
        }
        self.palette_for = Some(self.meant_for(&self.targets()));
        // A fresh menu starts from the first open day again.
        self.snooze_day = None;
        let viewport = window.viewport_size();
        let (vw, vh) = (from_screen(viewport.width), from_screen(viewport.height));
        let height = Mode::List.height(quick_times().len());
        let (x, y) = match at.or(self.snooze_anchor.get()) {
            Some(point) => (from_screen(point.x), from_screen(point.y)),
            // Under the reading pane's toolbar.
            None => {
                let pane_left = if self.sidebar_open { self.side_w() } else { 0. }
                    + if self.list_open { self.list_width } else { 0. };
                (pane_left + 110., crate::ui::top_bar() - 4.)
            }
        };
        let left = x.min(vw - WIDTH - 8.).max(8.);
        let top = y.min(vh - height - 8.).max(8.);
        let origin = window.bounds().origin;
        let origin = gpui::point(
            origin.x + to_screen(left - MENU_MARGIN),
            origin.y + to_screen(top - MENU_MARGIN),
        );
        self.open_snooze_window(origin, mode, cx);
    }

    /// The menu's window, its top-left corner (margin included) at `origin` on screen.
    pub(super) fn open_snooze_window(
        &mut self,
        origin: gpui::Point<gpui::Pixels>,
        mode: Mode,
        cx: &mut Context<Self>,
    ) {
        self.close_snooze_window(cx);
        let choices = quick_times();
        let height = mode.height(choices.len());
        let bounds = gpui::Bounds::new(
            origin,
            gpui::size(
                to_screen(WIDTH + 2. * MENU_MARGIN),
                to_screen(height + 2. * MENU_MARGIN),
            ),
        );
        let (owner, dark, alert, time) = (
            cx.weak_entity(),
            self.dark,
            self.snooze_alert,
            self.snooze_time,
        );
        let day = self
            .snooze_day
            .filter(|d| *d >= Local::now().date_naive())
            .unwrap_or_else(|| first_open_day(time));
        // Kept on screen as it grows (the hours open downwards).
        let origin = match cx
            .displays()
            .into_iter()
            .map(|d| d.bounds())
            .find(|b| b.contains(&origin))
        {
            Some(screen) => {
                let bottom = screen.bottom() - bounds.size.height - to_screen(8.);
                gpui::point(origin.x, origin.y.min(bottom).max(screen.top()))
            }
            None => origin,
        };
        let bounds = gpui::Bounds::new(origin, bounds.size);
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
                    // Closes on a click anywhere else, as the other menus do.
                    cx.spawn_in(window, async move |this, cx| {
                        let mut armed = false;
                        loop {
                            cx.background_executor()
                                .timer(Duration::from_millis(30))
                                .await;
                            if !crate::blur::mouse_down() {
                                armed = true;
                            }
                            let open = this.update_in(cx, |this: &mut SnoozeMenu, window, cx| {
                                if armed
                                    && crate::blur::mouse_pressed_outside(
                                        window,
                                        MENU_MARGIN as f64,
                                    )
                                {
                                    this.dismiss(window, cx);
                                    false
                                } else {
                                    true
                                }
                            });
                            if !matches!(open, Ok(true)) {
                                break;
                            }
                        }
                    })
                    .detach();
                    let focus = cx.focus_handle();
                    let t = if dark { Theme::dark() } else { Theme::light() };
                    let typed = cx.new(|cx| {
                        crate::input::TextInput::new(
                            cx,
                            "Any time, like 4:45 PM",
                            t.faint,
                            t.violet,
                        )
                    });
                    let _typing = cx.subscribe_in(
                        &typed,
                        window,
                        |this: &mut SnoozeMenu, _, event: &crate::input::InputEvent, window, cx| {
                            match event {
                                crate::input::InputEvent::Changed => this.typed_time(cx),
                                crate::input::InputEvent::Enter => this.confirm(window, cx),
                                _ => {}
                            }
                        },
                    );
                    // With the hours open, typing goes straight to the time.
                    if matches!(mode, Mode::Calendar(_, true)) {
                        window.focus(&typed.focus_handle(cx));
                    } else {
                        window.focus(&focus);
                    }
                    SnoozeMenu {
                        owner,
                        mode,
                        choices,
                        alert,
                        time,
                        day,
                        typed,
                        typed_bad: false,
                        _typing,
                        sel: 0,
                        dark,
                        focus,
                        was_active: false,
                        opened: std::time::Instant::now(),
                        _activation: cx.observe_window_activation(
                            window,
                            |this: &mut SnoozeMenu, window, cx| {
                                if window.is_window_active() {
                                    this.was_active = true;
                                } else if this.was_active
                                    && this.opened.elapsed() > Duration::from_millis(500)
                                {
                                    this.dismiss(window, cx);
                                }
                            },
                        ),
                    }
                })
            },
        );
        self.snooze_window = opened.ok();
    }

    pub(super) fn close_snooze_window(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.snooze_window.take() {
            remove_popup(handle, cx);
        }
    }

    /// A time was picked: snooze what the menu was opened for, if it is still there.
    fn snooze_picked(&mut self, choice: SnoozeChoice, alert: bool, cx: &mut Context<Self>) {
        self.snooze_window = None;
        let meant = self.palette_for.clone();
        if meant.is_some() && !self.still_meant(&meant, cx) {
            return;
        }
        self.snooze(
            SnoozeChoice {
                no_reply: alert,
                ..choice
            },
            cx,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_time_reads_on_the_twelve_hour_clock() {
        assert_eq!(time_text((21, 0)), "9:00 PM");
    }
}
