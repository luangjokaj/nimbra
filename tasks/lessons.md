# Lessons

## Window top row

- The window controls and traffic lights travel with the sidebar (and are gone while it is
  hidden) and fill its 236pt up to the reload button. Never append to that row: anything wider than a badge lands on the pane beside it.
  Status goes inside the pane's own header.
- Everything in the top row is centred on `ui::TOP_LINE`, y = 25 (traffic lights, window
  controls, list header, reading toolbar, the full-page composer's header, the message
  window's toolbar, the Settings section title). A new header element goes on that line.
  A window takes its traffic lights from `ui::traffic_lights()`.
- A bar is `ui::TOP_BAR` tall with its content centred in the 50pt above its 1pt hairline.
  Never reach that line with uneven top and bottom padding: the gaps show as soon as a
  button has a background.
- Panes meet edge to edge: the hairline between two is `ui::pane_divider`, laid over the
  right edge of the left pane, which keeps that 1pt clear (`.pr(px(1.))`). Never a border
  on one side of a pane.
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
  the content needs `erase::rounded` under it, after its shadow (shadows are filled): a
  rectangular clip leaves square corners in the content. Paths erase, so never draw one
  for anything else. A floating version of a docked panel keeps the docked one's material.
- Anything GPUI draws is under the email's web view, tooltips included: what can overlap the
  email is a window of its own (`tooltip.rs`, menus, notices) or needs a hole in the view.
- GPUI's tooltip view is not always released (a reference cycle when its element goes):
  tie nothing's lifetime to that release alone.
- A screenshot check of corners uses content that crosses them, not a plain background.
- Test copies: build them with `focus: false` and without `cx.activate(true)`.
- Test copies of the app take the keyboard focus while the user is working, and their keys
  land in it. Keep those runs few and short, and log state so stray input is recognised.

## Actions

- A button that fills in the composer must refuse when a message is already open:
  `open_compose` returns quietly then, and what follows writes into that message.
- `thread_banners` and the like are drawn in message windows too: an action there must use
  that window's thread, account and window (`row_to_act_on`), never `self.selected`,
  `picked` or the main window. When one such bug turns up, audit every shared action.

## States

- A state (picked, active, on, floating) never adds or removes a border: that changes the
  layout and the element jumps. Keep the border and change its colour, or draw it as an
  absolute overlay.

## Positions and accounts

- A row number is good for one frame. Anything kept longer (a menu, a pick, the history, a
  closure, a request) keeps the thread's key and account, and is checked when used.
- An error from a connection says whose it is. Never sign out, wipe or switch on an error
  without checking that it is the shown account's.
- Before adding a feature to All inboxes, follow every path it opens for another account
  (counts, labels, stores, timers, failures).
- Code for a platform that cannot be run here: type-check it for that target (with a
  negative control) and read the API's documentation for every flag it relies on.

## Editing

- Inserting a function before another: insert before its doc comment, not its `fn` line
  (this stranded three). When moving code, take its doc along, and diff docs per function.

## Hints

- A key hint shows only while that key would do something, decided by the same check the
  action uses. When animating or restyling a screen, ask of everything on it whether it is
  true in that state: do not carry a wrong hint along and list it as "left open".

## Waiting

- Never pair a "busy" flag with early returns. What is busy follows from a request that
  says what it is for (`checking::Checks`); what shows as loading has a deadline
  (`checking::within`) and a failed state. Never put that deadline on a send.
- A request made from `render` (the next page) stops after a failure, or it loops.
- Test an async change by switching away while it runs, and back.

## Sending and sign-in

- Only a revoked refresh token signs out (and wipes). A 401 is renewed and tried once more.
- Expiry is wall-clock time: `Instant` stands still while the Mac sleeps.
- A send is retried by itself only when it certainly never left (`gmail::never_left`). A
  timeout is not that.
- What must survive quitting is on disk before it starts, and is removed by the thread that
  finishes it: the main thread's continuation does not run while quitting.
- Test the network layer against a local HTTP fake. Stubbed `Client` methods hide its bugs.
- Every retry loop has a pause on every failing path: count the requests in the test.
- A notice about the user's mail goes through `tell`: a list arriving clears the others.
  Test the notice after the list has arrived, and read the longest one in the real toast
  (asserting on the string missed that it was cut at 90 characters).

## Keychain

- "Could not read" is never "there is none": make a key, sign out or wipe only on NoEntry.
- Work nobody waits on uses `Store::open_quietly`, or macOS asks again every 30 seconds.

## Records

- A list in the store is changed with `Store::update`, by the thread that did the work.
  Never save a copy held from before a request or an account switch.
- Never replace `target/bundle` under a running Nimbra: move the old bundle aside first
  (deleted, its signature stops verifying and the keychain may refuse it).

## Gmail's quota

- "Nothing changed" counts our own writes as well as the change feed, taken when the check
  starts. Test: act, then click the mailbox it lands in at once.
- A scroll offset read in `render` is last frame's: decide on "near the end" after layout.
- Count units per click before shipping anything that fetches (6,000 a minute per user;
  a thread in full is 120, its headers 40, the change feed 2).
- Test copies are never on the user's screen: the scratch GPUI puts them under the wallpaper,
  transparent unless capturing, with no Dock icon. `focus: false` was not enough: the user
  took a test copy's "Scheduled message sent" toast for their own mail.
- A test copy that only looks hidden can still take the keyboard: the app's own
  `show_main_window` activated it 20 times while the user worked. Every way forward is
  patched out in every rig (`blur::set_visible`, `App::activate`); after a run, check
  launchservicesd's SETFRONT log. Rebuild every old test binary when the rig changes.
- GPUI's quads add alpha up. Anything see-through laid over see-through material needs the
  compositing underline pipeline (`erase::fade`). Measure a bare column, do not eyeball it.

## Names

- A control renamed to a thing opens that thing. Asked to call a row Settings, I left its
  menu in place and only asked about it: the user had to come back for it.
- The first account's id is "". Never test an id with `is_empty` for "no account".
- Test a screen that is shared by the accounts (All inboxes) across an account switch, and
  sample what is drawn while it loads: the state after it settles hid a skeleton.
- Reread a fix for what it does when repeated: a listing sized by the rows in memory
  counted hidden ones and grew with every refresh.

## Accounts and Google

- A sign-in into an existing account is checked (`profile`) before its token is saved: the
  browser lets any account be picked, and `login_hint` is only a hint.

## Keys and web views

- A key fix is tested at the edges too: ← at the very start and → at the very end are the
  keys a web view hands back unhandled, up the responder chain to GPUI. My first arrow fix
  only covered keys going in, and the user found the boxes again at the first character.
  Log GPUI's own key entry in a test build (`handle_key_event`) to see what reaches it.
- Never put a responder of our own between WebKit and GPUI's view: it lost typed letters
  now and then. Keep unhandled keys in the page (`preventDefault`) instead.
- Repeat a key test a few times before believing it: WebKit handles keys in another process,
  and one passing run proved nothing either way.

## Composer header

- A label beside a control is not part of the control: the hover and click area is the value
  only (From: the address and its chevron), as in the fields below.
- A menu opened from the header is placed by fixed offsets in `toggle_menu`: moving
  or resizing its button means moving those too, in floating, full page and its own window.

- The reader's and the bulk pane's Label, Move To and More menus drop from where their
  button was last drawn (`menu_anchor`). A new button that opens one of them lays the same
  anchor over itself; do not go back to fixed offsets.

## Every way into a feature goes through one function
- The reader's Snooze button opened the palette without loading its times, so it was empty, while H and the list buttons loaded them. When a feature has several openers (button, key, menu, row), give it one entry point (`open_snooze`) that prepares its state, and make every opener call it.
