# Lessons

## Window top row

- The window controls and traffic lights travel with the sidebar (and are gone while it is
  hidden) and fill its 236pt up to the reload button. Never append to that row: anything wider than a badge lands on the pane beside it.
  Status goes inside the pane's own header.
- Everything in the top row is centred on y = 25 (traffic lights, window controls, list
  header, reading toolbar). A new header element goes on that line.
- A bar is `ui::TOP_BAR` tall with its content centred. Never reach that line with uneven
  top and bottom padding: the gaps show as soon as a button has a background.
- Check a header change with the sidebar and the list each open and closed, an empty
  mailbox, loading, both themes, and the list at its 300pt minimum.

## Reading mail

- Anything that changes how the sender's mail looks gets a switch in Settings › Reading, on
  by default when it is the better experience. Do not plan such a feature without one.
- The CSP is not the whole lockdown: meta refresh and preconnect both get past it. Test a
  web view change with a hostile mail and a localhost listener, and with a negative control.

## Native window work

- Never open a window during `render`: GPUI draws it at once and clears the element arena
  shared with the frame being built (a segfault). Use a click handler or `cx.defer_in`.
- Moving or hiding a traffic light makes AppKit lay all three out again: set every button's
  whole frame, every frame, and never use `setHidden:` on them.
- The window's blur shows only where nothing opaque is painted. A see-through panel over
  the content needs that content clipped away, and a shadow only outside it (shadows are
  filled). A floating version of a docked panel keeps the docked one's material.
- Test copies: build them with `focus: false` and without `cx.activate(true)`.
- Test copies of the app take the keyboard focus while the user is working, and their keys
  land in it. Keep those runs few and short, and log state so stray input is recognised.

## Actions

- A button that fills in the composer must refuse when a message is already open:
  `open_compose` returns quietly then, and what follows writes into that message.
- `thread_banners` and the like are drawn in message windows too: an action there must use
  that window's thread and window, never `self.selected` or the main window.

## States

- A state (picked, active, on, floating) never adds or removes a border: that changes the
  layout and the element jumps. Keep the border and change its colour, or draw it as an
  absolute overlay.
