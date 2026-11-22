# Nimbra

A mail client concept built with [GPUI](https://www.gpui.rs). It runs on built-in sample data, or on a real Gmail inbox once you add an OAuth client.

## Design

The design reference is the Figma file: [Mail Client, Nimbra concept](https://www.figma.com/design/s6tsR2l2X4MKmoA1dNlm4I/Mail-Client?node-id=7313-2). It holds the screens (inbox in dark and light, command palette, Inbox Zero), the colour variables and text styles, and the component kit the app's look is based on.

## Connect Gmail

1. In Google Cloud Console, create a project, enable the Gmail API, and create an OAuth client of type "Desktop app". Keep the consent screen in Testing mode and add yourself as a test user.
2. Put the client in `~/.config/nimbra/oauth.env` (mode 600), never in the repo:

   ```sh
   NIMBRA_GOOGLE_CLIENT_ID=...
   NIMBRA_GOOGLE_CLIENT_SECRET=...
   ```

3. `cargo run`, then click **Connect Gmail**. Google opens in your browser.

What it does with your mailbox:

- Asks for the `gmail.modify` scope only. That scope covers reading, archiving, labelling and sending; it does not allow permanent deletion. Nimbra sends only when you press Send.
- Lists the 30 newest inbox threads and draws each message natively: headings, bold, links, lists, quotes and code. Scripts, styles and forms are dropped.
- HTML mail opens in its **original formatting** by default; `V` switches to the native reader view and back. The original view displays the email's own HTML in a system web view with the sender's scripts off, navigation blocked, and remote scripts, frames, fonts and forms blocked by a content security policy. Remote images are shown by default in this view (senders can then tell the mail was opened); **Block images** turns them off for the open thread. Links open in your browser, and only when you click them: mail that tries to send you somewhere by itself (a meta refresh) is stopped. In dark theme, light emails get a dark page and light text, with pictures and brand colours kept; Nimbra's own script does this in an isolated WebKit world, and **Settings › Reading › Darken emails in dark theme** turns it off (on by default). Printing is always light. macOS and Windows only for now; on Windows emails are not darkened, and there is no Windows build yet: its click check compiles but has never run.
- Remote images stay hidden until you click **Load images** on a message. Tracking pixels are never loaded.
- Links open in your browser, and only `http`, `https` and `mailto` links are clickable.
- The sidebar switches between Inbox, Gmail's categories (Primary, Updates, Promotions, Social), Starred, Snoozed, Drafts, Sent, Done (Gmail's archive) and your own labels, 30 newest threads each.
- Opening a thread marks it read. `S` stars, `⌘⌫` moves to Gmail's trash (kept 30 days), and `E` in Done moves a thread back to the inbox. `Z` undoes the last removal; Inbox Zero says so while there is something to undo, and shows a line for the day when there is not.
- While the app is open it reads Gmail's change feed every 30 seconds (or 1 or 5 minutes, set in Settings › General): only what changed since the last look is fetched, the open mailbox is updated in place, and a macOS notification (sender and subject) is shown for each thread with a new unread inbox message. Clicking a mailbox shows what is kept at once and asks Gmail only if something changed since it was last checked or that was over a minute ago, without the loading bar; Reload and pulling the list down always ask. The content of the first 3 threads of the open mailbox is fetched ahead and stored, so opening those is instant (more would run into Gmail's limit of 6,000 units a minute, as each costs 120). Opening the app works the same way; a full fetch only happens when there is no starting point yet, the point is older than Gmail keeps (about a week), or the mailbox is search-based (Done, Snoozed, search results), and then only threads that changed since they were last listed are read again. Clicking a notification opens that email. Notifications need the bundled `Nimbra.app` and your permission, and only arrive while Nimbra is running.
- Mailboxes, opened threads and sidebar counts are saved in an encrypted local store (`~/Library/Application Support/Nimbra/store`), so the app opens straight into your mail (or into Inbox Zero, which fades up, unless the Mac is set to reduce motion) and refreshes in the background. Each record is encrypted with ChaCha20-Poly1305 using a key kept only in the system keychain (`nimbra` / `store-key`); the files alone, or a backup of them, are unreadable. When the Google sign-in ends (Google refuses to renew it; an access token that merely ran out is renewed), the store and its key are deleted. A keychain that cannot be read (denied, locked) deletes nothing and is not taken for signed out. Search results are never stored.
- `/` or ⌘F opens the search panel: matches from the mail on this Mac appear as you type, Gmail’s own results join a moment later, ↑↓ pick one and Return opens it (or “All results”). With an empty box it shows recent searches and Gmail search tips.
- Double-click a thread to open it in a window of its own (reply, forward, done, trash, and original or reader view). Return reads it in full view, without the sidebar and list; Esc or Return brings them back.
- Each message shows its sender and recipients; click the recipients line for From, To, Cc, Reply-To and the full date. Earlier messages in a thread have their own Reply, Reply all and Forward. Attachments can be previewed with Quick Look or saved to Downloads; images embedded in an email are shown in the original view; mailing lists get an Unsubscribe button beside the subject (it opens the list’s page in your browser, or prepares the email the list asks for, for you to send from the account it came to); spam is flagged with a Not spam button, and Report spam is in the ••• and right-click menus.
- Replies and forwards include the original message, folded behind •••.
- Mailboxes include Important, All Mail, Spam and Trash; lists keep loading older threads as you scroll to the end. Threads show their labels (click × to remove one), the tag button adds a label or creates a new one, and the folder button moves a thread to the inbox, a label or Spam. Trash has Restore, Spam has Not spam. The + next to Labels creates a label (“Parent/Child” nests it), and right-clicking a label renames or deletes it. In Trash, right-click (or ⌫) offers Move to inbox and Delete permanently; the first permanent delete asks Google for the extra permission it needs, and every one asks for confirmation.
- ⌘-click and Shift-click pick several threads (⌘A picks all); the reading pane then offers Mark done, Read or unread, Star, Label, Move to, Snooze, Spam and Trash for all of them, and one Undo brings a batch back.
- `H` (or ⌘K) snoozes: later today, tomorrow, this weekend, next week, or “remind me if no reply”. Snoozed threads leave the inbox and come back unread with a notification (a reminder only if nobody replied). Snoozing is kept on this Mac, because Gmail’s API has no snooze, and happens while Nimbra is running. `E` in Snoozed brings a thread back now.
- Messages are written in a formatted editor: bold (⌘B), italic (⌘I), underline (⌘U), links (⌘K), bulleted and numbered lists, quotes. They are sent with both an HTML and a plain-text version. Pasted text keeps simple formatting only; scripts, styles and images are removed.
- Several Gmail accounts: switch from the Accounts list in the sidebar or in Settings › Accounts, where Add account is too. Each account has its own sign-in in the keychain and its own encrypted store and key; Sign out removes only that account. With two or more accounts the sidebar adds **All inboxes** (every account's inbox merged, newest first, each row badged in its account's colour) and an Accounts list to see one account on its own. Every account is checked for new mail and notifies; snoozes and scheduled sends run for all of them; the Dock badge counts them all. In All inboxes, actions and replies go to the message's own account (replies say which address they go out from); labels stay per account.
- Offline, archive, star, read and the like still work: changes are kept (encrypted, on this Mac) and sent once Gmail can be reached, and messages sent while offline go out then too. An Offline pill left of Compose (just its dot when the list is narrow) shows how many changes are waiting; when the list is not showing, the reload button (or, with the sidebar hidden too, the window's top-left corner) gets an amber dot and says the same in its tooltip.
- The menu bar has File (New Message ⌘N, Check for Mail ⌘R, Print ⌘P), Edit, View and Message menus; the Dock icon shows the inbox’s unread count. The mail store keeps the newest 300 threads per mailbox, with full text for the newest 100.
- Settings (⌘, or click Settings at the bottom of the sidebar): signature, light, dark or system appearance, original formatting and remote images, notifications, open at login, and Sign out (which removes the mail kept on this Mac).
- The composer has Cc and Bcc (click “Cc Bcc”), suggests addresses from your mail as you type (Tab or Enter picks one), and takes attachments from the paperclip or by dropping files on it (25 MB in total). Drafts save themselves a moment after you stop typing. Sending waits six seconds so you can undo it (⌘Z), and Discard can be undone the same way. From Send until Gmail has it a message is kept encrypted on this Mac: quitting sends it first (waiting up to 30 seconds), switching accounts leaves it with its own account, and one that could not leave is sent when that account next connects. One that may have arrived is never sent again by itself: it goes to Drafts, with a note to check Sent, as does one Gmail failed on (a 5xx). Quitting also waits for changes on their way (those that do not get through are made on the next launch) and saves what you are writing to Drafts. Send later sends the saved draft from this Mac at the chosen time while Nimbra is running.
- `⌫` (or ⌘⌫) moves the open thread to the trash; `Z` undoes it.
- `E` archives the thread in Gmail (removes it from the inbox) and `Z` puts it back. Both only work in the Inbox. 
- The sign-in is stored in the system keychain under `nimbra`. Delete that entry to sign out.

Without the file, or with `NIMBRA_DEMO=1`, the app uses sample data and never touches the network. Messages are written in a plain-text composer (`C`, or `R` to reply in the thread). A thread in Drafts previews first; **Edit draft** in the toolbar, or a click on its text, opens it in the composer; the quoted or forwarded message stays folded behind a `•••` toggle and is kept when you save (`⌘S`) or send. The floating composer can be dragged by its header and resized from its bottom-right corner. The toolbar also offers reply all, forward, adding a label, mark as unread and Open in Gmail. Search uses Gmail's own syntax (`from:ana has:attachment newer_than:7d`). Not connected yet: snooze, reminders, attachments, Cc/Bcc, and the AI brief.

## Run

```sh
cargo run
```

GPUI is built with the `runtime_shaders` feature, so Xcode's Metal toolchain is not required.

## Build the Mac app

```sh
./scripts/bundle-macos.sh
open target/bundle/Nimbra.app
```

This builds the release binary and wraps it in `Nimbra.app` with the app icon. The icon source is `assets/AppIcon.icon`, an Icon Composer document (SVG layers plus `icon.json`); the same file works for iOS. Open it in Icon Composer to tune it, or render a preview from the command line:

```sh
"/Applications/Xcode.app/Contents/Applications/Icon Composer.app/Contents/Executables/ictool" \
  assets/AppIcon.icon --export-image --output-file icon.png \
  --platform macOS --rendition Default --width 512 --height 512 --scale 1
```

Settings › About shows `assets/app-icon.png`, a render of the same file. After changing the icon, render it again with the command above, using `--output-file assets/app-icon.png --width 192 --height 192`.

Needs Xcode 26 or newer. The script signs the app with your Developer ID or Apple Development certificate if one is in the keychain (override with `NIMBRA_SIGN_IDENTITY`). Signing matters for day-to-day use: macOS ties Keychain permission to the app's signature, so an unsigned build asks for the Keychain after every rebuild. For development, `./scripts/run-dev.sh` builds, signs and runs the debug binary the same way.

## Keys

| Key | Action |
| --- | --- |
| `J` / `K` or arrows | Next / previous thread |
| `E` | Mark done (clear them all to reach Inbox Zero) |
| `Z`, `⌘Z` or `Ctrl+Z` | Undo the last archive, move or trash (an Undo pill also appears for a few seconds) |
| `/` or `⌘F` | Search (Enter runs it, `esc` clears it) |
| `S` | Star or unstar |
| `U` | Mark the open thread as unread, or read again (also a toolbar button) |
| Right-click a thread | Menu: mark read or unread, star, mark done, trash, open in Gmail |
| `V` | Switch between the reader view and the email's original formatting |
| `C` / `R` | Compose a new message / reply to the open thread |
| `⌘↵` | Send (in the composer) |
| `⌘⇧F` | Full-page composer on or off |
| `⌘⌫` | Move to trash |
| `⌘K` or `H` | Command palette (snooze) |
| `↵` / `esc` | Run command / close palette |
| `⌘⇧L` | Toggle light and dark |
| `⌘B` | Show the sidebar, or hide it: hidden, it floats in, with the same blurred material, when the pointer reaches the window’s left edge. Nimbra starts the way you left it |
| `⌘W` | Close the window; Nimbra keeps running and checking mail (click the Dock icon to bring it back, `⌘Q` to quit) |
| `⌘⇧B` | Show or hide the message list (also the first toolbar button; drag the list's right edge to resize it) |
| `⌘[` / `⌘]` | Back / forward through opened threads |
| `⌘R` | Reload (mock mail check: restores the inbox) |
| Pull the list down and let go | Refresh (trackpad, when the list is at the top) |

## Screenshot states

`NIMBRA_SCREEN=palette`, `zero`, `loading`, `compose` or `compose-full` opens that screen directly on the sample data. `NIMBRA_THEME=light` starts in light mode. `NIMBRA_SIDEBAR=closed` (or `open`) starts with the sidebar hidden (auto-hide) or docked, whatever was saved. `NIMBRA_OPEN=<row>` opens that thread first. `NIMBRA_READER=1` starts in the native reader view. `NIMBRA_SEARCH=<query>` starts with a search run. `NIMBRA_SHIFT=<px>` moves the window sideways.

## Sidebar blur

The sidebar is translucent and blurs whatever is behind the window. GPUI's own blurred window mode did not blur on macOS 27, so `src/blur.rs` asks the macOS window server directly through a private API. That is fine for a prototype but is a risk for App Store review. Windows and Linux fall back to GPUI's built-in behaviour.

## Layout

- `src/theme.rs`: colours for dark and light, fonts, gradients
- `src/gmail.rs`: OAuth sign-in (PKCE, loopback redirect), keychain storage, Gmail REST calls
- `src/input.rs`: the text field (adapted from GPUI's example) used by the composer
- `src/webview.rs`: the locked-down web view behind Show original
- `src/store.rs`: the encrypted on-disk mail store
- `src/checking.rs`: which check for mail is running, and how long loading may wait (60 seconds)
- `src/arrival.rs`: the timing of Inbox Zero coming up at launch
- `src/leaving.rs`: what is on its way to Gmail, and what quitting does about it
- `src/scheduled.rs`: each account's send-later list, and sending from it once
- `src/sending.rs`: a message from Send until Gmail has it, across quitting and switching accounts
- `src/inspiration.rs`: the lines Inbox Zero shows when there is nothing to undo
- `src/rich.rs`: turns HTML or plain-text mail into styled blocks; fetches images on request
- `src/data.rs`: the mail model, sample inbox and palette commands
- `src/ui.rs`: icon, kbd, avatar, pill, buttons
- `src/app.rs`: state, key actions, and the sidebar, list, reading pane, palette and Inbox Zero views
- `assets/`: bundled Geist and Instrument Serif fonts (OFL) and Lucide-style icons
