//! The loading screen: the mail screen itself, with ink where the words will be.
//!
//! The bars along the top are the real components, dimmed and out of reach, and everything
//! under them has the real layout's margins and line heights, so nothing moves when the mail
//! arrives. A slow, faint wave of light runs down both panes a few times, then holds still:
//! a wave that never ends redraws the whole window for as long as loading lasts.
//!
//! It is all in the two greys of `inks`: no accent colours, and nothing marked as open or
//! unread, so that it reads as a place being filled and not as mail.

use super::*;
use crate::ui::motion;

/// A row's height with the 2pt under it, as the list draws it.
const ROW_STEP: f32 = ROW_H + ROW_GAP;
/// How many times the wave runs down the screen.
const WAVES: usize = 3;
/// The rows above EARLIER.
const TODAY: usize = 4;
/// How dim the bars' controls are until there is mail to use them on.
const WAITING: f32 = 0.4;

/// One message in the list, as shares of the room it has.
struct Row {
    sender: f32,
    subject: f32,
    snippet: f32,
    label: Option<f32>,
}

const fn row(sender: f32, subject: f32, snippet: f32, label: Option<f32>) -> Row {
    Row {
        sender,
        subject,
        snippet,
        label,
    }
}

/// Uneven, as mail is. Repeated down the list.
const MAIL: [Row; 9] = [
    row(0.30, 0.46, 0.40, Some(54.)),
    row(0.20, 0.34, 0.60, None),
    row(0.38, 0.62, 0.27, None),
    row(0.26, 0.28, 0.52, Some(40.)),
    row(0.17, 0.52, 0.38, None),
    row(0.33, 0.40, 0.44, None),
    row(0.24, 0.70, 0.18, None),
    row(0.29, 0.31, 0.63, Some(62.)),
    row(0.21, 0.48, 0.30, None),
];

/// The email's paragraphs, as the share of the column each line fills.
const GREETING: [f32; 1] = [0.22];
const OPENING: [f32; 4] = [0.97, 0.92, 0.99, 0.58];
const MIDDLE: [f32; 3] = [0.94, 0.98, 0.41];
const SIGN_OFF: [f32; 2] = [0.16, 0.27];

/// The two greys everything is drawn in: where bold words will be, and the rest.
fn inks(t: &Theme) -> (Hsla, Hsla) {
    (t.border, t.border.opacity(0.6))
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
    // Where the light is at the start and end of a pass, so the last one ends on it.
    let rest = 0.86 + 0.14 * (down * 0.8 * std::f32::consts::TAU).cos();
    if motion::reduced() {
        return element.opacity(rest).into_any_element();
    }
    let pass = Animation::new(Duration::from_millis(1700));
    element
        .with_animations(
            ("skeleton", key),
            vec![pass; WAVES],
            move |element, _, delta| {
                let light = ((delta - down * 0.8) * std::f32::consts::TAU).cos();
                element.opacity(0.86 + 0.14 * light)
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
    pub(super) fn skeleton(&self, t: &Theme, cx: &App) -> impl IntoElement {
        div()
            .flex()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .when(self.list_open, |d| d.child(self.skeleton_list(t, cx)))
            .child(self.skeleton_reading(t))
    }

    fn skeleton_list(&self, t: &Theme, cx: &App) -> Div {
        let (strong, soft) = inks(t);
        // All, and Unread with room for a count of one digit.
        let widths = segment_widths("0", cx);
        let options = [("All", widths[0]), ("Unread", widths[1])];
        let switch_w = switch_width(widths);
        let room = self.list_width - 1. - LIST_LEFT - LIST_RIGHT;
        let compose = !self.sidebar_open;
        let fit = HeaderFit::new(room, switch_w, None, compose);
        let switch = div()
            .flex()
            .flex_none()
            .gap(px(SEGMENT_GAP))
            .p(px(SWITCH_INSET - 1.))
            .rounded(px(radius::CONTROL))
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
                    .rounded(px(radius::CHIP))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(if label == "All" { t.text } else { t.text2 })
                    .when(label == "All", |d| d.bg(t.raised))
                    .child(label)
            }));

        let heading = |width: f32| {
            div()
                .flex_none()
                .h(px(GROUP_H))
                .pl(px(ROW_PAD))
                .pt(px(space::L2))
                .child(ink(px(width), 6., 16., soft))
        };
        // Only those that fit: the wave draws each one again every frame.
        let height = self.frame.map_or(920., |f| f32::from(f.size.height));
        let fitting = ((height - TOP_BAR) / ROW_STEP).ceil().max(1.) as usize;
        let rows = (0..fitting).flat_map(|i| {
            let mail = &MAIL[i % MAIL.len()];
            let top_line = div()
                .flex()
                .items_center()
                .gap(px(8.))
                .h(px(20.))
                .child(ink(relative(mail.sender), 10., 20., strong))
                .children(mail.label.map(|width| {
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(px(6.))
                        .h(px(20.))
                        .pl(px(7.))
                        .pr(px(9.))
                        .rounded_full()
                        .border_1()
                        .border_color(t.border)
                        .child(div().size(px(6.)).rounded_full().bg(soft))
                        .child(ink(px(width), 6., 16., soft))
                }))
                .child(div().flex_1())
                .child(ink(px(26.), 7., 20., soft).flex_none());
            let row = div()
                .relative()
                .flex()
                .flex_col()
                .flex_none()
                .justify_center()
                .gap(px(2.))
                .h(px(ROW_H))
                .px(px(ROW_PAD))
                .rounded(px(radius::ROW))
                .child(top_line)
                .child(
                    div()
                        .flex()
                        .gap(px(6.))
                        .h(px(18.))
                        .child(ink(relative(mail.subject), 8., 18., strong))
                        .child(ink(relative(mail.snippet), 8., 18., soft)),
                );
            let before = match i {
                0 => Some(heading(34.).into_any_element()),
                TODAY => Some(heading(44.).into_any_element()),
                _ => None,
            };
            before.into_iter().chain([wave(i, i as f32 / 14., row)])
        });

        div()
            .relative()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(self.list_width))
            .h_full()
            .overflow_hidden()
            .pr(px(1.))
            .bg(t.panel)
            .child(pane_divider(t))
            .child(
                div()
                    .relative()
                    .flex_none()
                    .h(px(TOP_BAR - 1.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .size_full()
                            .pl(px(LIST_LEFT))
                            .pr(px(LIST_RIGHT))
                            .gap(px(HEADER_GAP))
                            .opacity(WAITING)
                            .child(
                                div()
                                    .flex_1()
                                    .when(fit.title, |d| d.child(ink(px(72.), 8., 20., strong))),
                            )
                            .child(switch)
                            .when(compose, |d| {
                                d.child(compose_button(t, ComposeLook::Icon, "compose-outline"))
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
                    .gap(px(ROW_GAP))
                    .px(px(LIST_SIDE))
                    .children(rows),
            )
    }

    fn skeleton_reading(&self, t: &Theme) -> Div {
        let tools = toolbar_outline(t).opacity(WAITING);

        div()
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .overflow_hidden()
            .bg(t.surface)
            .child(
                div()
                    .relative()
                    .flex_none()
                    .h(px(TOP_BAR))
                    // On the top line, as the reader's own bar above its hairline.
                    .child(div().h(px(TOP_BAR - 1.)).child(tools))
                    .child(shield()),
            )
            .map(|d| {
                // Whichever view mail opens in.
                if self.original {
                    d.child(self.skeleton_original_head(t))
                        .child(self.skeleton_email(t, 28., 20.))
                } else {
                    d.child(self.skeleton_reader_head(t)).child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_h(px(0.))
                            .pl(px(BODY_INSET))
                            .child(self.skeleton_email(t, COLUMN_PAD, 0.)),
                    )
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
            .px(px(28.))
            .pt(px(16.))
            .pb(px(12.))
            .child(wave(100, 0.05, ink(relative(0.46), 18., 30., strong)))
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
        let title_line = (TITLE_SIZE * 1.2).round();
        div()
            .flex()
            .flex_col()
            .flex_none()
            .px(px(COLUMN_PAD))
            .pt(px(space::XXL))
            .pb(px(HEADER_TO_BODY))
            .child(wave(
                100,
                0.1,
                ink(relative(0.62), TITLE_SIZE * 0.6, title_line, strong),
            ))
            .child(div().h(px(space::M)))
            .child(wave(
                101,
                0.18,
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::M))
                    .h(px(24.))
                    .child(ink(px(180.), 8., 16., soft)),
            ))
            .child(div().h(px(space::XXL)))
            .child(wave(
                102,
                0.24,
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::M))
                    .child(div().flex_none().size(px(AVATAR)).rounded_full().bg(soft))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w(px(0.))
                            .child(ink(px(112.), 10., 20., strong))
                            .child(ink(px(160.), 7., 16., soft)),
                    )
                    .child(ink(px(28.), 7., 16., soft).flex_none()),
            ))
    }

    /// An email that could be one: a greeting, text, a picture, a button and a sign-off.
    fn skeleton_email(&self, t: &Theme, side: f32, top: f32) -> Div {
        let (_, soft) = inks(t);
        let paragraph = |lines: &'static [f32]| {
            div().flex().flex_col().children(
                lines
                    .iter()
                    .map(|&share| ink(relative(share), 8., 22., soft)),
            )
        };
        let picture = div()
            .flex()
            .items_center()
            .justify_center()
            .w_full()
            .h(px(168.))
            .rounded(px(radius::CARD))
            .bg(t.hover)
            .border_1()
            .border_color(t.border)
            .child(icon("image", 26., t.faint));
        let action = div().flex().child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .w(px(148.))
                .h(px(36.))
                .rounded(px(radius::CONTROL))
                .bg(t.hover)
                .child(ink(px(84.), 8., 16., soft)),
        );
        let parts = [
            (0.22, paragraph(&GREETING)),
            (0.30, paragraph(&OPENING)),
            (0.46, picture),
            (0.62, paragraph(&MIDDLE)),
            (0.72, action),
            (0.82, paragraph(&SIGN_OFF)),
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
