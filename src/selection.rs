//! Selecting text in the reader: drag across one paragraph or many (and across the messages
//! of a thread), Shift-click to extend, ⌘C (or Edit › Copy) to copy. The reader's text is
//! GPUI's, which has no selection of its own; the original view's web page has its own.
//!
//! A point is a block (by the id the reading pane gives it) and a byte offset in its text.

use std::ops::Range;

use super::*;

/// Ids of the latest message's blocks start at 0; an earlier message's at (k + 1) * 1000.
/// On screen the latest comes last.
fn order(block: usize) -> usize {
    if block < 1000 {
        1_000_000 + block
    } else {
        block
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Point {
    pub(super) block: usize,
    pub(super) offset: usize,
}

impl Point {
    fn key(self) -> (usize, usize) {
        (order(self.block), self.offset)
    }
}

/// Where text can be selected.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Place {
    /// The main window's reader, showing this thread.
    Main(ThreadKey),
    /// A message window, by the key it knows its thread by.
    Window(String),
}

/// Text that can be selected where it is drawn: the place, and the record of its blocks as
/// last drawn there (to find the one under the pointer).
#[derive(Clone)]
pub(super) struct Selectable {
    pub(super) place: Place,
    pub(super) layouts: Layouts,
}

pub(super) type Layouts = std::rc::Rc<std::cell::RefCell<Vec<(usize, gpui::TextLayout)>>>;

/// The selection in the reader.
pub(super) struct ReaderSelection {
    /// Where it is: the main window's reader (for the thread open there) or a message
    /// window. Another thread opening, or a press somewhere else, clears it.
    pub(super) place: Place,
    pub(super) anchor: Point,
    pub(super) head: Point,
    /// The mouse is down and dragging.
    pub(super) dragging: bool,
}

impl ReaderSelection {
    fn ordered(&self) -> (Point, Point) {
        if self.anchor.key() <= self.head.key() {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }

    /// The part of a block's text that is selected, if any.
    pub(super) fn in_block(&self, block: usize, len: usize) -> Option<Range<usize>> {
        let (start, end) = self.ordered();
        let at = order(block);
        if at < order(start.block) || at > order(end.block) {
            return None;
        }
        let from = if start.block == block {
            start.offset
        } else {
            0
        };
        let to = if end.block == block { end.offset } else { len };
        let range = from.min(len)..to.min(len);
        (!range.is_empty()).then_some(range)
    }
}

/// A block's highlights with the selection laid over them: spans are cut at its edges, the
/// parts inside get its background, and text with no span inside it gets one of its own.
/// Highlights stay sorted and never overlap, as GPUI needs.
pub(super) fn with_selection(
    spans: Vec<(Range<usize>, HighlightStyle)>,
    selected: Option<Range<usize>>,
    background: Hsla,
) -> Vec<(Range<usize>, HighlightStyle)> {
    let Some(selected) = selected else {
        return spans;
    };
    let lit = |mut style: HighlightStyle| {
        style.background_color = Some(background);
        style
    };
    let mut out = Vec::new();
    let mut covered = selected.start;
    for (range, style) in spans {
        // Gap before this span, inside the selection.
        if range.start > covered && covered < selected.end {
            let gap = covered..range.start.min(selected.end);
            if !gap.is_empty() {
                out.push((gap, lit(HighlightStyle::default())));
            }
        }
        let inside = range.start.max(selected.start)..range.end.min(selected.end);
        if inside.is_empty() {
            out.push((range.clone(), style));
        } else {
            if range.start < inside.start {
                out.push((range.start..inside.start, style));
            }
            out.push((inside.clone(), lit(style)));
            if inside.end < range.end {
                out.push((inside.end..range.end, style));
            }
        }
        covered = covered.max(range.end);
    }
    if covered < selected.end {
        out.push((
            covered.max(selected.start)..selected.end,
            lit(HighlightStyle::default()),
        ));
    }
    out.sort_by_key(|(range, _)| range.start);
    out
}

/// The text position under the pointer: in the block it is over, else the end of the
/// nearest block above it (or the start of the first, above them all).
fn point_at(layouts: &[(usize, gpui::TextLayout)], at: gpui::Point<gpui::Pixels>) -> Option<Point> {
    let mut above: Option<(usize, &gpui::TextLayout)> = None;
    let mut first: Option<(usize, &gpui::TextLayout)> = None;
    for (block, layout) in layouts {
        let bounds = layout.bounds();
        if first.is_none_or(|(_, f)| bounds.top() < f.bounds().top()) {
            first = Some((*block, layout));
        }
        if at.y >= bounds.top() && at.y <= bounds.bottom() {
            let offset = layout
                .index_for_position(at)
                .unwrap_or_else(|nearest| nearest);
            return Some(Point {
                block: *block,
                offset,
            });
        }
        if bounds.bottom() < at.y
            && above.is_none_or(|(_, a)| bounds.bottom() > a.bounds().bottom())
        {
            above = Some((*block, layout));
        }
    }
    match above {
        Some((block, layout)) => Some(Point {
            block,
            offset: layout.text().len(),
        }),
        None => first.map(|(block, _)| Point { block, offset: 0 }),
    }
}

impl Nimbra {
    /// Where the main window's reader is now.
    pub(super) fn main_place(&self) -> Option<Place> {
        let thread = self.mails.get(self.selected)?;
        Some(Place::Main(self.thread_key(thread)))
    }

    /// The selection, if it is in `place`.
    pub(super) fn selection_in(&self, place: &Place) -> Option<&ReaderSelection> {
        self.reader_selection.as_ref().filter(|s| s.place == *place)
    }

    /// The mouse went down on a block's text: start a selection there (or extend it).
    pub(super) fn select_from(
        &mut self,
        place: Place,
        point: Point,
        extend: bool,
        cx: &mut Context<Self>,
    ) {
        match &mut self.reader_selection {
            Some(selection) if extend && selection.place == place => {
                selection.head = point;
                selection.dragging = true;
            }
            _ => {
                self.reader_selection = Some(ReaderSelection {
                    place,
                    anchor: point,
                    head: point,
                    dragging: true,
                })
            }
        }
        cx.notify();
    }

    /// The mouse moved over a block's text while dragging.
    pub(super) fn select_to(&mut self, point: Point, cx: &mut Context<Self>) {
        if let Some(selection) = &mut self.reader_selection
            && selection.dragging
            && selection.head != point
        {
            selection.head = point;
            cx.notify();
        }
    }

    pub(super) fn end_selecting(&mut self) {
        if let Some(selection) = &mut self.reader_selection {
            selection.dragging = false;
        }
    }

    /// While a selection is being dragged, follows the pointer over the whole window (GPUI
    /// gives an element moves only while the pointer is over it), to the text block under
    /// it, or the nearest one above or below. Lets go when the button is released.
    pub(super) fn selection_watch(
        &self,
        layouts: Layouts,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let entity = cx.entity();
        gpui::canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                let (layouts, moved) = (layouts.clone(), entity.clone());
                window.on_mouse_event(move |event: &gpui::MouseMoveEvent, phase, _, cx| {
                    if phase != gpui::DispatchPhase::Bubble {
                        return;
                    }
                    let dragging = moved
                        .read(cx)
                        .reader_selection
                        .as_ref()
                        .is_some_and(|s| s.dragging);
                    if !dragging {
                        return;
                    }
                    if event.pressed_button != Some(MouseButton::Left) {
                        moved.update(cx, |this, _| this.end_selecting());
                        return;
                    }
                    let Some(point) = point_at(&layouts.borrow(), event.position) else {
                        return;
                    };
                    moved.update(cx, |this, cx| this.select_to(point, cx));
                });
                let released = entity.clone();
                window.on_mouse_event(move |_: &gpui::MouseUpEvent, phase, _, cx| {
                    if phase == gpui::DispatchPhase::Bubble {
                        released.update(cx, |this, _| this.end_selecting());
                    }
                });
            },
        )
        .absolute()
        .size_0()
    }

    /// ⌘C in the main window's reader: the text picked in the reader, or else what is
    /// selected in the email shown in its original formatting.
    pub(super) fn copy_reader_selection(&mut self, cx: &mut Context<Self>) {
        if let (Some(place), Some(mail)) = (self.main_place(), self.mails.get(self.selected))
            && self.selection_in(&place).is_some()
        {
            let mail = mail.clone();
            self.copy_selection(&place, &mail, cx);
            return;
        }
        self.mail_views.copy();
    }

    /// ⌘C where `place` shows `mail`: the selected text, a blank line between blocks.
    pub(super) fn copy_selection(&self, place: &Place, mail: &Mail, cx: &mut App) {
        let Some(selection) = self.selection_in(place) else {
            return;
        };
        // Every block with its id, in the order they are on screen.
        let mut blocks: Vec<(usize, &str)> = Vec::new();
        for (k, earlier) in mail.earlier.iter().enumerate() {
            for (i, block) in earlier.blocks.iter().enumerate() {
                blocks.push(((k + 1) * 1000 + i, block.text.as_str()));
            }
        }
        for (i, block) in mail.body.iter().enumerate() {
            blocks.push((i, block.text.as_str()));
        }
        let parts: Vec<&str> = blocks
            .iter()
            .filter_map(|(id, text)| {
                let range = selection.in_block(*id, text.len())?;
                text.get(range)
            })
            .collect();
        if !parts.is_empty() {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(parts.join("\n\n")));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bold() -> HighlightStyle {
        HighlightStyle {
            font_weight: Some(FontWeight::BOLD),
            ..Default::default()
        }
    }

    #[test]
    fn a_selection_spans_blocks_in_screen_order() {
        let selection = ReaderSelection {
            place: Place::Window("t".into()),
            // From the latest message's second block back up into an earlier message.
            anchor: Point {
                block: 1,
                offset: 3,
            },
            head: Point {
                block: 1002,
                offset: 4,
            },
            dragging: false,
        };
        assert_eq!(selection.in_block(1002, 10), Some(4..10));
        assert_eq!(selection.in_block(1003, 8), Some(0..8));
        assert_eq!(selection.in_block(0, 5), Some(0..5));
        assert_eq!(selection.in_block(1, 9), Some(0..3));
        assert_eq!(selection.in_block(2, 9), None);
        assert_eq!(selection.in_block(1001, 9), None);
    }

    #[test]
    fn the_selection_is_laid_over_spans_without_overlaps() {
        let bg = gpui::black();
        // "Hello bold world": bold on 6..10, selection 3..8.
        let out = with_selection(vec![(6..10, bold())], Some(3..8), bg);
        let ranges: Vec<_> = out.iter().map(|(r, _)| r.clone()).collect();
        assert_eq!(ranges, vec![3..6, 6..8, 8..10]);
        assert_eq!(out[0].1.background_color, Some(bg));
        assert_eq!(out[1].1.background_color, Some(bg));
        assert_eq!(out[1].1.font_weight, Some(FontWeight::BOLD));
        assert_eq!(out[2].1.background_color, None);
        // No spans at all.
        let out = with_selection(vec![], Some(2..5), bg);
        assert_eq!(
            out.iter().map(|(r, _)| r.clone()).collect::<Vec<_>>(),
            vec![2..5]
        );
        // Selection after every span.
        let out = with_selection(vec![(0..2, bold())], Some(4..6), bg);
        assert_eq!(
            out.iter().map(|(r, _)| r.clone()).collect::<Vec<_>>(),
            vec![0..2, 4..6]
        );
        // Nothing selected: the spans as they were.
        assert_eq!(with_selection(vec![(0..2, bold())], None, bg).len(), 1);
    }
}
