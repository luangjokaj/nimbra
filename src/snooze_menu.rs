//! The Snooze menu: quick times with the date each comes back, a calendar to pick a day, and
//! a switch to come back only if nobody has replied. A small window of its own, like the
//! toolbar menus, so it floats above the email's web view.

use chrono::{Datelike, Duration as Days, Local, NaiveDate, TimeZone, Timelike, Weekday};

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
/// The field to type any time in, above the hours.
const FIELD: f32 = 30.;
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

/// A local time on a day, if the clock has it (not inside a daylight-saving jump).
fn at(date: NaiveDate, hour: u32) -> Option<chrono::DateTime<Local>> {
    Local
        .from_local_datetime(&date.and_hms_opt(hour, 0, 0)?)
        .single()
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

/// The quick times: later today, tomorrow morning and evening, the weekend, next week.
pub(super) fn quick_times() -> Vec<SnoozeChoice> {
    let now = Local::now();
    let today = now.date_naive();
    let tomorrow = today + Days::days(1);
    let mut choices = Vec::new();
    if now.hour() < 17 {
        choices.extend(at(today, 18).map(|w| choice("clock", "Later today", w)));
    } else {
        choices.push(choice("clock", "In three hours", now + Days::hours(3)));
    }
    choices.extend(at(tomorrow, 9).map(|w| choice("calendar", "Tomorrow", w)));
    choices.extend(at(tomorrow, 18).map(|w| choice("calendar", "Tomorrow evening", w)));
    // The coming Saturday, unless it is already the weekend.
    if !matches!(now.weekday(), Weekday::Sat | Weekday::Sun) {
        let mut saturday = today + Days::days(1);
        while saturday.weekday() != Weekday::Sat {
            saturday += Days::days(1);
        }
        choices.extend(at(saturday, 9).map(|w| choice("calendar", "This weekend", w)));
    }
    let mut monday = today + Days::days(1);
    while monday.weekday() != Weekday::Mon {
        monday += Days::days(1);
    }
    choices.extend(at(monday, 9).map(|w| choice("calendar", "Next week", w)));
    // On a Friday the weekend is tomorrow, on a Sunday next week is: each time once.
    choices.sort_by_key(|c| c.until_ms);
    choices.dedup_by_key(|c| c.until_ms);
    choices
}

/// When a day picked in the calendar comes back: at the time chosen under it. None for a
/// time already gone by.
fn day_time(date: NaiveDate, (hour, minute): (u32, u32)) -> Option<chrono::DateTime<Local>> {
    let when = Local
        .from_local_datetime(&date.and_hms_opt(hour, minute, 0)?)
        .single()?;
    (when > Local::now()).then_some(when)
}

/// A time typed in: "4:30 PM", "4:30pm", "16:30", "430p", "4", "noon". Without AM or PM,
/// 1 to 7 is the afternoon (nobody snoozes to 5 in the morning by typing "5"), unless it is
/// written with a leading zero ("07:00").
fn parse_time(text: &str) -> Option<(u32, u32)> {
    let s: String = text
        .trim()
        .to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '.')
        .collect();
    match s.as_str() {
        "noon" => return Some((12, 0)),
        "midnight" => return Some((0, 0)),
        _ => {}
    }
    let (body, half) = if let Some(b) = s.strip_suffix("am").or_else(|| s.strip_suffix('a')) {
        (b, Some(false))
    } else if let Some(b) = s.strip_suffix("pm").or_else(|| s.strip_suffix('p')) {
        (b, Some(true))
    } else {
        (s.as_str(), None)
    };
    if body.is_empty() || !body.chars().all(|c| c.is_ascii_digit() || c == ':') {
        return None;
    }
    let (hour, minute): (&str, &str) = match body.split_once(':') {
        Some(split) => split,
        None if body.len() <= 2 => (body, "0"),
        None if body.len() <= 4 => body.split_at(body.len() - 2),
        None => return None,
    };
    let (h, m): (u32, u32) = (hour.parse().ok()?, minute.parse().ok()?);
    if m > 59 || hour.is_empty() {
        return None;
    }
    let h = match half {
        Some(pm) if (1..=12).contains(&h) => h % 12 + if pm { 12 } else { 0 },
        Some(_) => return None,
        None if (1..=7).contains(&h) && !hour.starts_with('0') => h + 12,
        None if h <= 23 => h,
        None => return None,
    };
    Some((h, m))
}

/// The day the calendar starts on: today if the time is still ahead today, else tomorrow.
fn first_open_day(time: (u32, u32)) -> NaiveDate {
    let today = Local::now().date_naive();
    if day_time(today, time).is_some() {
        today
    } else {
        today + Days::days(1)
    }
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
                .gap(px(10.))
                .h(px(ROW))
                .px(px(10.))
                .rounded(px(8.))
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
                .h(px(DIVIDER))
                .flex()
                .items_center()
                .px(px(4.))
                .child(div().h(px(1.)).w_full().bg(t.border))
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
            .w(px(30.))
            .h(px(18.))
            .p(px(2.))
            .rounded(px(9.))
            .bg(if on { t.violet } else { t.border_strong })
            .child(
                div()
                    .size(px(14.))
                    .rounded(px(7.))
                    .bg(gpui::white())
                    .ml(px(if on { 12. } else { 0. })),
            );
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_none()
                    .h(px(HEADER))
                    .flex()
                    .items_center()
                    .px(px(10.))
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
        let hover = t.hover;
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
                .size(px(26.))
                .rounded(px(7.))
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
                .size(px(CELL))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(30.))
                        .rounded(px(15.))
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
                        d.hover(move |s| s.bg(hover).rounded(px(8.)))
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
                    .h(px(CAL_HEAD))
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .px(px(4.))
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
            .child(div().flex().flex_none().h(px(CAL_DAYS)).children(
                ["M", "T", "W", "T", "F", "S", "S"].into_iter().map(|d| {
                    div()
                        .w(px(CELL))
                        .flex()
                        .justify_center()
                        .child(meta(d, t.text3))
                }),
            ))
            .child(div().flex().flex_wrap().w(px(7. * CELL)).children(cells))
            .child(
                div()
                    .flex_none()
                    .h(px(DIVIDER))
                    .flex()
                    .items_center()
                    .px(px(4.))
                    .child(div().h(px(1.)).w_full().bg(t.border)),
            )
            .child(
                div()
                    .id("snooze-time")
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(10.))
                    .h(px(TIME_ROW))
                    // Its edges line up with the hours' grid below it.
                    .mx(px(CHIP_INSET))
                    .px(px(10.))
                    .rounded(px(8.))
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
                        .gap(px(8.))
                        .h(px(FIELD))
                        .mt(px(CHIP_GAP))
                        .mx(px(CHIP_INSET))
                        .px(px(10.))
                        .rounded(px(7.))
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
                        .gap(px(CHIP_GAP))
                        .py(px(CHIP_GAP))
                        .px(px(CHIP_INSET))
                        .children(HOURS.map(|hour| {
                            let chosen = time == (hour, 0);
                            div()
                                .id(("snooze-hour", hour as usize))
                                .flex()
                                .items_center()
                                .justify_center()
                                .w(px((7. * CELL - 2. * CHIP_INSET - 3. * CHIP_GAP) / 4.))
                                .h(px(CHIP))
                                .rounded(px(7.))
                                .text_size(px(12.))
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
                    .h(px(CAL_FOOT))
                    .flex()
                    .items_center()
                    .justify_between()
                    .px(px(4.))
                    .child(
                        div()
                            .id("snooze-quick")
                            .flex()
                            .items_center()
                            .gap(px(2.))
                            .px(px(6.))
                            .py(px(3.))
                            .rounded(px(6.))
                            .cursor_pointer()
                            .hover(move |s| s.bg(hover))
                            .child(icon("back", 12., t.text2))
                            .child(meta("Quick times", t.text2))
                            .on_click(cx.listener(|this, _, window, cx| this.back(window, cx))),
                    )
                    .child({
                        let ready = self.chosen().is_some();
                        let label = if ready { "Snooze" } else { "Pick a later time" };
                        let button = button(t, ButtonKind::Primary, label, Some("↵"))
                            .text_size(px(12.))
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
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
            .p(px(MENU_MARGIN))
            .font_family(crate::theme::SANS)
            .text_size(px(13.))
            .line_height(px(18.))
            .text_color(t.text)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .size_full()
                    .p(px(6.))
                    .rounded(px(12.))
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
        let (vw, vh) = (f32::from(viewport.width), f32::from(viewport.height));
        let height = Mode::List.height(quick_times().len());
        let (x, y) = match at.or(self.snooze_anchor.get()) {
            Some(point) => (f32::from(point.x), f32::from(point.y)),
            // Under the reading pane's toolbar.
            None => {
                let pane_left = if self.sidebar_open { self.side_w() } else { 0. }
                    + if self.list_open { self.list_width } else { 0. };
                (pane_left + 110., TOP_BAR - 4.)
            }
        };
        let left = x.min(vw - WIDTH - 8.).max(8.);
        let top = y.min(vh - height - 8.).max(8.);
        let origin = window.bounds().origin;
        let origin = gpui::point(
            origin.x + px(left - MENU_MARGIN),
            origin.y + px(top - MENU_MARGIN),
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
            gpui::size(px(WIDTH + 2. * MENU_MARGIN), px(height + 2. * MENU_MARGIN)),
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
                let bottom = screen.bottom() - bounds.size.height - px(8.);
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
    fn the_quick_times_are_ahead_and_in_order() {
        let now = Local::now().timestamp_millis();
        let times = quick_times();
        assert!(times.len() >= 4);
        assert!(times.iter().all(|c| c.until_ms > now && !c.no_reply));
        assert!(times.windows(2).all(|w| w[0].until_ms <= w[1].until_ms));
    }

    #[test]
    fn a_day_gone_by_cannot_be_picked() {
        let today = Local::now().date_naive();
        assert!(day_time(today - Days::days(1), (9, 0)).is_none());
        let tomorrow = day_time(today + Days::days(1), (15, 0)).unwrap();
        assert_eq!((tomorrow.hour(), tomorrow.minute()), (15, 0));
        // Today at a time already gone.
        assert!(day_time(today, (0, 0)).is_none());
        assert_eq!(time_text((21, 0)), "9:00 PM");
    }

    #[test]
    fn any_time_can_be_typed() {
        assert_eq!(parse_time("4:30 PM"), Some((16, 30)));
        assert_eq!(parse_time("4:30pm"), Some((16, 30)));
        assert_eq!(parse_time(" 9:15 a.m. "), Some((9, 15)));
        assert_eq!(parse_time("16:45"), Some((16, 45)));
        assert_eq!(parse_time("430p"), Some((16, 30)));
        assert_eq!(parse_time("1015"), Some((10, 15)));
        assert_eq!(parse_time("12am"), Some((0, 0)));
        assert_eq!(parse_time("12 pm"), Some((12, 0)));
        assert_eq!(parse_time("noon"), Some((12, 0)));
        // Without AM or PM, 1 to 7 is the afternoon; a leading zero is the morning.
        assert_eq!(parse_time("5"), Some((17, 0)));
        assert_eq!(parse_time("07:00"), Some((7, 0)));
        assert_eq!(parse_time("8:20"), Some((8, 20)));
        for bad in ["", "25:00", "4:60", "13pm", "soon", "4:3:0", "12345"] {
            assert_eq!(parse_time(bad), None, "{bad}");
        }
    }
}
