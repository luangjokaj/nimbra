# Nimbra

A mail client concept built with [GPUI](https://www.gpui.rs). It runs on built-in sample data, or on a real Gmail inbox once you add an OAuth client.

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
- Remote images stay hidden until you click **Load images** on a message. Tracking pixels are never loaded.
- Links open in your browser, and only `http`, `https` and `mailto` links are clickable.
- The sidebar switches between Inbox, Gmail's categories (Primary, Updates, Promotions, Social), Starred, Snoozed, Drafts, Sent, Done (Gmail's archive) and your own labels, 30 newest threads each.
- Opening a thread marks it read. `S` stars, `⌘⌫` moves to Gmail's trash (kept 30 days), and `E` in Done moves a thread back to the inbox. `Z` undoes the last removal.
- Mailboxes and opened threads are cached in memory for the session: going back to a mailbox shows it at once and refreshes in the background. Nothing is written to disk.
- `E` archives the thread in Gmail (removes it from the inbox) and `Z` puts it back. Both only work in the Inbox. 
- The sign-in is stored in the system keychain under `nimbra`. Delete that entry to sign out.

Without the file, or with `NIMBRA_DEMO=1`, the app uses sample data and never touches the network. Messages are written in a plain-text composer (`C`, or `R` to reply in the thread). Not connected yet: search, snooze, reminders, applying labels, attachments, Cc/Bcc, drafts saving, and the AI brief.

## Run

```sh
cargo run
```

GPUI is built with the `runtime_shaders` feature, so Xcode's Metal toolchain is not required.

## Keys

| Key | Action |
| --- | --- |
| `J` / `K` or arrows | Next / previous thread |
| `E` | Mark done (clear them all to reach Inbox Zero) |
| `Z` | Undo the last archive, move or trash |
| `S` | Star or unstar |
| `C` / `R` | Compose a new message / reply to the open thread |
| `⌘↵` | Send (in the composer) |
| `⌘⌫` | Move to trash |
| `⌘K` or `H` | Command palette (snooze) |
| `↵` / `esc` | Run command / close palette |
| `⌘⇧L` | Toggle light and dark |
| `⌘B` | Show or hide the sidebar |
| `⌘[` / `⌘]` | Back / forward through opened threads |
| `⌘R` | Reload (mock mail check: restores the inbox) |

## Screenshot states

`NIMBRA_SCREEN=palette`, `zero`, `loading` or `compose` opens that screen directly on the sample data. `NIMBRA_THEME=light` starts in light mode. `NIMBRA_SIDEBAR=closed` starts with the sidebar hidden. `NIMBRA_OPEN=<row>` opens that thread first. `NIMBRA_SHIFT=<px>` moves the window sideways.

## Sidebar blur

The sidebar is translucent and blurs whatever is behind the window. GPUI's own blurred window mode did not blur on macOS 27, so `src/blur.rs` asks the macOS window server directly through a private API. That is fine for a prototype but is a risk for App Store review. Windows and Linux fall back to GPUI's built-in behaviour.

## Layout

- `src/theme.rs`: colours for dark and light, fonts, gradients
- `src/gmail.rs`: OAuth sign-in (PKCE, loopback redirect), keychain storage, Gmail REST calls
- `src/input.rs`: the text field (adapted from GPUI's example) used by the composer
- `src/rich.rs`: turns HTML or plain-text mail into styled blocks; fetches images on request
- `src/data.rs`: the mail model, sample inbox and palette commands
- `src/ui.rs`: icon, kbd, avatar, pill, buttons
- `src/app.rs`: state, key actions, and the sidebar, list, reading pane, palette and Inbox Zero views
- `assets/`: bundled Geist and Instrument Serif fonts (OFL) and Lucide-style icons
