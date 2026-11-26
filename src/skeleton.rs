//! The loading screen: the mail screen itself, with ink where the words will be.
//!
//! The bars along the top are the real components, dimmed and out of reach, and everything
//! under them has the real layout's margins and line heights, so nothing moves when the mail
//! arrives. A slow, faint wave of light runs down both panes.
//!
//! It is all in the two greys of `inks`, in thin lines: no accent colours, no pictures,
//! labels or buttons, and nothing marked as open or unread, so that it reads as a place
//! quietly being filled and not as mail.

use super::*;

/// A row's height with the 2pt under it: its padding and two lines of text.
const ROW_H: f32 = super::list::ROW_H + 2.;
/// The rows above EARLIER.
const TODAY: usize = 4;
/// How dim the bars' controls are until there is mail to use them on.
const WAITING: f32 = 0.6;

/// One message in the list, as shares of the room it has.
struct Row {
    sender: f32,
    subject: f32,
    snippet: f32,
}

const fn row(sender: f32, subject: f32, snippet: f32) -> Row {
    Row {
        sender,
        subject,
        snippet,
    }
}

/// Uneven, as mail is. Repeated down the list.
const MAIL: [Row; 9] = [
    row(0.30, 0.46, 0.40),
    row(0.20, 0.34, 0.60),
    row(0.38, 0.62, 0.27),
    row(0.26, 0.28, 0.52),
    row(0.17, 0.52, 0.38),
    row(0.33, 0.40, 0.44),
    row(0.24, 0.70, 0.18),
    row(0.29, 0.31, 0.63),
    row(0.21, 0.48, 0.30),
];

/// The email's paragraphs, as the share of the column each line fills.
const GREETING: [f32; 1] = [0.22];
const OPENING: [f32; 4] = [0.97, 0.92, 0.99, 0.58];
const MIDDLE: [f32; 3] = [0.94, 0.98, 0.41];
const CLOSING: [f32; 2] = [0.88, 0.36];
const SIGN_OFF: [f32; 2] = [0.16, 0.27];

/// The two greys everything is drawn in: where bold words will be, and the rest.
fn inks(t: &Theme) -> (Hsla, Hsla) {
    (t.border.opacity(0.8), t.border.opacity(0.45))
}

/// A line of text's ink, centred in that line's height.
fn ink(width: impl Into<gpui::Length> + Clone, thick: f32, line: f32, color: Hsla) -> Div {
    div()
        .flex()
        .flex_shrink()
        .min_w(px(0.))
        .items_center()
        .h(px(line))
        .w(width)
        .child(div().w_full().h(px(thick)).rounded_full().bg(color))
}

/// The wave. `down` is how far down the screen the element is, 0 to 1, so that the light
/// travels from the top in both panes at once.
fn wave(key: usize, down: f32, element: Div) -> AnyElement {
    element
        .with_animation(
            ("skeleton", key),
            Animation::new(Duration::from_millis(2400)).repeat(),
            move |element, delta| {
                let light = ((delta - down * 0.8) * std::f32::consts::TAU).cos();
                element.opacity(0.9 + 0.1 * light)
            },
        )
        .into_any_element()
}

/// Lies over a bar of real controls, so none hovers or clicks; it still moves the window.
fn shield() -> Div {
    div()
        .absolute()
        .inset_0()
        .occlude()
        .on_mouse_down(MouseButton::Left, drag_or_zoom)
}

impl Nimbra {
    pub(super) fn skeleton(&self, t: &Theme) -> impl IntoElement {
        div()
            .flex()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .bg(t.surface)
            .when(self.list_open, |d| d.child(self.skeleton_list(t)))
            .child(self.skeleton_reading(t))
    }

    fn skeleton_list(&self, t: &Theme) -> Div {
        let (strong, soft) = inks(t);
        // All, and Unread with room for a count of one digit.
        let options = [("All", 44.), ("Unread", 77.)];
        let switch_w = options.iter().map(|(_, w)| w).sum::<f32>() + 2. + 8.;
        let room = self.list_width - 1. - LIST_LEFT - LIST_RIGHT;
        let compose = HeaderFit::new(room, switch_w, None).compose;
        let switch = div()
            .flex()
            .flex_none()
            .gap(px(2.))
            .p(px(3.))
            .rounded(px(10.))
            .bg(t.hover)
            .border_1()
            .border_color(t.border)
            .children(options.map(|(label, width)| {
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(width))
                    .h(px(SEGMENT_H))
                    .rounded(px(7.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(if label == "All" { t.text } else { t.text3 })
                    .when(label == "All", |d| d.bg(t.raised))
                    .child(label)
            }));

        let heading = |width: f32| {
            div()
                .flex_none()
                .pl(px(ROW_PAD))
                .pt(px(14.))
                .pb(px(6.))
                .child(ink(px(width), 6., 12., soft))
        };
        // Only those that fit: the wave draws each one again every frame.
        let height = self.frame.map_or(920., |f| f32::from(f.size.height));
        let fitting = ((height - TOP_BAR) / ROW_H).ceil().max(1.) as usize;
        let rows = (0..fitting).flat_map(|i| {
            let mail = &MAIL[i % MAIL.len()];
            let top_line = div()
                .flex()
                .items_center()
                .gap(px(8.))
                .h(px(22.))
                .child(ink(relative(mail.sender), 8., 22., strong))
                .child(div().flex_1())
                .child(ink(px(26.), 6., 22., soft).flex_none());
            let row = div()
                .relative()
                .flex()
                .flex_none()
                .gap(px(ROW_PAD))
                .px(px(ROW_PAD))
                .py(px(12.))
                .rounded(px(12.))
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .child(div().size(px(28.)).rounded_full().bg(soft)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w(px(0.))
                        .gap(px(super::list::LINE_GAP))
                        .child(top_line)
                        .child(
                            div()
                                .flex()
                                .gap(px(6.))
                                .child(ink(relative(mail.subject), 7., 18., strong))
                                .child(ink(relative(mail.snippet), 7., 18., soft)),
                        ),
                );
            let before = match i {
                0 => Some(heading(34.).into_any_element()),
                TODAY => Some(heading(44.).into_any_element()),
                _ => None,
            };
            before.into_iter().chain([wave(i, i as f32 / 14., row)])
        });

        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(self.list_width))
            .h_full()
            .overflow_hidden()
            .border_r_1()
            .border_color(t.border)
            .child(
                div()
                    .relative()
                    .flex_none()
                    .h(px(TOP_BAR))
                    .border_b_1()
                    .border_color(t.border)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .size_full()
                            .pl(px(LIST_LEFT))
                            .pr(px(LIST_RIGHT))
                            .opacity(WAITING)
                            .child(switch)
                            .child(div().flex_1())
                            // As the real bar (`list.rs`).
                            .child({
                                let worded = compose != ComposeLook::Icon;
                                super::reading::action_button(t, "draft", "Compose", worded)
                                    .mr(px(if worded { -9. } else { -7. }))
                            }),
                    )
                    .child(shield()),
            )
            // The checking-for-mail strip.
            .child(div().flex_none().h(px(2.)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_hidden()
                    .gap(px(2.))
                    .pl(px(LIST_LEFT))
                    .pr(px(LIST_RIGHT))
                    .children(rows),
            )
    }

    fn skeleton_reading(&self, t: &Theme) -> Div {
        let worded = self
            .frame
            .is_none_or(|f| f32::from(f.size.width) - self.pane_left() >= 600.);
        let tools = div()
            .flex()
            .items_center()
            .gap(px(2.))
            .size_full()
            .overflow_hidden()
            .pl(px(LIST_RIGHT - 7.))
            .pr(px(LIST_RIGHT - 7.))
            .opacity(WAITING)
            // As the real toolbar (`reading.rs`).
            .child(icon_button(t, "list"))
            .child(super::reading::toolbar_rule(
                t,
                7.,
                if worded { 9. } else { 7. },
            ))
            .child(super::reading::action_button(t, "done", "Done", worded))
            .child(super::reading::action_button(t, "clock", "Snooze", worded))
            .child(super::reading::action_button(t, "reply", "Reply", worded))
            .child(super::reading::action_button(
                t, "forward", "Forward", worded,
            ))
            .child(div().flex_1())
            .children(["star", "tag", "folder", "trash", "more"].map(|name| icon_button(t, name)))
            .when(worded, |d| {
                d.child(super::reading::toolbar_rule(t, 7., 7.))
                    .children(["up", "down"].map(|name| icon_button(t, name)))
            });

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .overflow_hidden()
            .child(
                div()
                    .relative()
                    .flex_none()
                    .h(px(TOP_BAR))
                    .border_b_1()
                    .border_color(t.border)
                    .child(tools)
                    .child(shield()),
            )
            .map(|d| {
                // Whichever view mail opens in.
                if self.original {
                    d.child(self.skeleton_original_head(t))
                        .child(self.skeleton_email(t, super::reading::ORIGINAL_SIDE, 20.))
                } else {
                    d.child(self.skeleton_reader_head(t))
                        .child(self.skeleton_email(t, 64., 8.))
                }
            })
    }

    /// Subject and sender, as they are over an email in its original formatting.
    fn skeleton_original_head(&self, t: &Theme) -> Div {
        let (strong, soft) = inks(t);
        div()
            .flex()
            .flex_col()
            .flex_none()
            .gap(px(12.))
            .px(px(super::reading::ORIGINAL_SIDE))
            .pt(px(16.))
            .pb(px(super::reading::ORIGINAL_SIDE))
            .border_b_1()
            .border_color(t.border)
            .child(wave(100, 0.05, ink(relative(0.46), 14., 30., strong)))
            // Where a label or a Spam note would be.
            .child(div())
            .child(wave(
                101,
                0.12,
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(div().flex_none().size(px(28.)).rounded_full().bg(soft))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w(px(0.))
                            .child(
                                div()
                                    .flex()
                                    .gap(px(8.))
                                    .child(ink(px(112.), 10., 18., strong))
                                    .child(ink(px(168.), 8., 18., soft)),
                            )
                            .child(ink(px(138.), 7., 16., soft)),
                    )
                    .child(ink(px(28.), 7., 16., soft).flex_none()),
            ))
    }

    /// Label, subject and people, as they are in the reader.
    fn skeleton_reader_head(&self, t: &Theme) -> Div {
        let (strong, soft) = inks(t);
        div()
            .flex()
            .flex_col()
            .flex_none()
            .gap(px(22.))
            .px(px(64.))
            .pt(px(40.))
            .pb(px(24.))
            // Where a label would be.
            .child(div().h(px(22.)))
            .child(wave(100, 0.1, ink(relative(0.62), 20., 43., strong)))
            .child(div())
            .child(wave(
                101,
                0.18,
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(div().flex_none().size(px(28.)).rounded_full().bg(soft))
                    .child(ink(px(180.), 8., 18., soft))
                    .child(div().flex_1())
                    .child(ink(px(28.), 7., 16., soft).flex_none()),
            ))
    }

    /// An email that could be one: a greeting, a few paragraphs and a sign-off, in thin
    /// lines only.
    fn skeleton_email(&self, t: &Theme, side: f32, top: f32) -> Div {
        let (_, soft) = inks(t);
        let paragraph = |lines: &'static [f32]| {
            div().flex().flex_col().children(
                lines
                    .iter()
                    .map(|&share| ink(relative(share), 7., 22., soft)),
            )
        };
        let parts = [
            (0.22, paragraph(&GREETING)),
            (0.30, paragraph(&OPENING)),
            (0.46, paragraph(&MIDDLE)),
            (0.58, paragraph(&CLOSING)),
            (0.70, paragraph(&SIGN_OFF)),
        ];
        div().flex_1().min_h(px(0.)).overflow_hidden().child(
            div()
                .flex()
                .flex_col()
                .gap(px(20.))
                .w_full()
                .max_w(px(680. + 2. * side))
                .px(px(side))
                .pt(px(top))
                .children(
                    parts
                        .into_iter()
                        .enumerate()
                        .map(|(i, (down, part))| wave(110 + i, down, part)),
                ),
        )
    }
}
