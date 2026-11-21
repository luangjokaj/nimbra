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
- HTML mail opens in its **original formatting** by default; `V` switches to the native reader view and back. The original view displays the email's own HTML in a system web view with scripts off, navigation blocked, and remote scripts, frames, fonts and forms blocked by a content security policy. Remote images are shown by default in this view (senders can then tell the mail was opened); **Block images** turns them off for the open thread. Links open in your browser. macOS and Windows only for now.
- Remote images stay hidden until you click **Load images** on a message. Tracking pixels are never loaded.
- Links open in your browser, and only `http`, `https` and `mailto` links are clickable.
- The sidebar switches between Inbox, Gmail's categories (Primary, Updates, Promotions, Social), Starred, Snoozed, Drafts, Sent, Done (Gmail's archive) and your own labels, 30 newest threads each.
- Opening a thread marks it read. `S` stars, `⌘⌫` moves to Gmail's trash (kept 30 days), and `E` in Done moves a thread back to the inbox. `Z` undoes the last removal.
- While the app is open it reads Gmail's change feed every 30 seconds: only what changed since the last look is fetched, the open mailbox is updated in place, and a macOS notification (sender and subject) is shown for each thread with a new unread inbox message. The content of the 20 newest threads in the open mailbox is fetched ahead in the background and stored, so opening them is instant. Opening the app and pressing reload work the same way; a full fetch only happens when there is no starting point yet, the point is older than Gmail keeps (about a week), or the mailbox is search-based (Done, Snoozed, search results). Clicking a notification opens that email. Notifications need the bundled `Nimbra.app` and your permission, and only arrive while Nimbra is running.
- Mailboxes, opened threads and sidebar counts are saved in an encrypted local store (`~/Library/Application Support/Nimbra/store`), so the app opens straight into your mail and refreshes in the background. Each record is encrypted with ChaCha20-Poly1305 using a key kept only in the system keychain (`nimbra` / `store-key`); the files alone, or a backup of them, are unreadable. When the Google sign-in ends, the store and its key are deleted. Search results are never stored.
- Each message shows its sender and recipients; click the recipients line for From, To, Cc, Reply-To and the full date. Earlier messages in a thread have their own Reply, Reply all and Forward. Attachments can be previewed with Quick Look or saved to Downloads; images embedded in an email are shown in the original view; mailing lists get an Unsubscribe link; spam is flagged with a Not spam button, and Report spam is in the ••• and right-click menus.
- Replies and forwards include the original message, folded behind •••.
- Mailboxes include Important, All Mail, Spam and Trash; lists load 30 threads at a time with Load more at the end. Threads show their labels (click × to remove one), the tag button adds a label or creates a new one, and the folder button moves a thread to the inbox, a label or Spam. Trash has Restore, Spam has Not spam.
- Messages are written in a formatted editor: bold (⌘B), italic (⌘I), underline (⌘U), links (⌘K), bulleted and numbered lists, quotes. They are sent with both an HTML and a plain-text version. Pasted text keeps simple formatting only; scripts, styles and images are removed.
- Settings (⌘, or click your account in the sidebar): signature, light, dark or system appearance, original formatting and remote images, notifications, and Sign out (which removes the mail kept on this Mac).
- The composer has Cc and Bcc (click “Cc Bcc”), suggests addresses from your mail as you type (Tab or Enter picks one), and takes attachments from the paperclip or by dropping files on it (25 MB in total). Drafts save themselves a moment after you stop typing. Sending waits six seconds so you can undo it (⌘Z), and Discard can be undone the same way. Send later sends the saved draft from this Mac at the chosen time while Nimbra is running.
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
| `⌘B` | Show or hide the sidebar |
| `⌘W` | Close the window; Nimbra keeps running and checking mail (click the Dock icon to bring it back, `⌘Q` to quit) |
| `⌘⇧B` | Show or hide the message list (also the first toolbar button; drag the list's right edge to resize it) |
| `⌘[` / `⌘]` | Back / forward through opened threads |
| `⌘R` | Reload (mock mail check: restores the inbox) |
| Pull the list down and let go | Refresh (trackpad, when the list is at the top) |

## Screenshot states

`NIMBRA_SCREEN=palette`, `zero`, `loading`, `compose` or `compose-full` opens that screen directly on the sample data. `NIMBRA_THEME=light` starts in light mode. `NIMBRA_SIDEBAR=closed` starts with the sidebar hidden. `NIMBRA_OPEN=<row>` opens that thread first. `NIMBRA_READER=1` starts in the native reader view. `NIMBRA_SEARCH=<query>` starts with a search run. `NIMBRA_SHIFT=<px>` moves the window sideways.

## Sidebar blur

The sidebar is translucent and blurs whatever is behind the window. GPUI's own blurred window mode did not blur on macOS 27, so `src/blur.rs` asks the macOS window server directly through a private API. That is fine for a prototype but is a risk for App Store review. Windows and Linux fall back to GPUI's built-in behaviour.

## Layout

- `src/theme.rs`: colours for dark and light, fonts, gradients
- `src/gmail.rs`: OAuth sign-in (PKCE, loopback redirect), keychain storage, Gmail REST calls
- `src/input.rs`: the text field (adapted from GPUI's example) used by the composer
- `src/webview.rs`: the locked-down web view behind Show original
- `src/store.rs`: the encrypted on-disk mail store
- `src/rich.rs`: turns HTML or plain-text mail into styled blocks; fetches images on request
- `src/data.rs`: the mail model, sample inbox and palette commands
- `src/ui.rs`: icon, kbd, avatar, pill, buttons
- `src/app.rs`: state, key actions, and the sidebar, list, reading pane, palette and Inbox Zero views
- `assets/`: bundled Geist and Instrument Serif fonts (OFL) and Lucide-style icons
