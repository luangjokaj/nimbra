<div align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset=".github/assets/icon-dark.png" />
    <img src=".github/assets/icon-light.png" alt="Nimbra" width="128" height="128" />
  </picture>

  <h1>Nimbra</h1>

  <p><strong>Blazing fast mail. No bloat.</strong></p>

  <p>
    A native email client for macOS, written in Rust and drawn on the GPU with <a href="https://www.gpui.rs">GPUI</a>.<br />
    Every action is one key, your mail stays on your Mac, and there is nothing in it you didn’t ask for.
  </p>

  <p>
    <img src="https://img.shields.io/badge/macOS-26%2B-black?style=flat&logo=apple" alt="macOS 26 or newer" />
    <img src="https://img.shields.io/badge/Rust-2024-b7410e?style=flat&logo=rust" alt="Rust" />
    <img src="https://img.shields.io/badge/UI-GPUI-8b7cff?style=flat" alt="GPUI" />
    <img src="https://img.shields.io/badge/mail-Gmail%20%C2%B7%20Microsoft%20%C2%B7%20IMAP-555?style=flat" alt="Gmail, Microsoft and IMAP" />
    <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-3da639?style=flat" alt="MIT license" /></a>
  </p>

  <p>
    <a href="#features">Features</a> ·
    <a href="#get-started">Get started</a> ·
    <a href="#connect-your-mail">Connect your mail</a> ·
    <a href="#keys">Keys</a> ·
    <a href="#privacy-and-security">Privacy</a> ·
    <a href="#telemetry">Telemetry</a> ·
    <a href="#development">Development</a> ·
    <a href="CONTRIBUTING.md">Contributing</a>
  </p>
</div>

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset=".github/assets/showcase-dark.png" />
    <img src=".github/assets/showcase-light.png" alt="Nimbra's window: the sidebar with the split inbox, mailboxes and labels, the thread list, and an open email." width="100%" />
  </picture>
</p>

## Features

- **Fast by nature.** Native Rust, drawn on the GPU, no Electron. Mailboxes open from an encrypted local store and refresh in the background, so the app opens straight into your mail.
- **One key for everything.** `E` done, `H` snooze, `R` reply, `S` star, `/` search, `Z` undo. Hints show the key only while it would do something, `?` lists them all, and any of them can be changed in Settings › Shortcuts.
- **Split inbox.** Gmail's Primary, Updates, Promotions and Social, each with its unread count, plus Starred, Important, Snoozed, Reminders, Drafts, Sent, Done, All Mail, Spam, Trash and your labels. IMAP accounts keep one inbox.
- **Snooze and reminders.** Quick times, or any day and time from the calendar; “Only if nobody replies” turns a snooze into a reminder. Works for Gmail and IMAP.
- **Block senders.** Their mail goes straight to a Blocked label and never notifies. It uses the same label as Spark, so a list kept there carries on.
- **Every account in one place.** Several Gmail, Microsoft and IMAP accounts, All inboxes merged newest first, one search across every account (or just the ones you pick), and replies that go out from the right address. Nimbra opens again on the account and mailbox you left.
- **Conversations that read well.** Every message folds to one line, the latest included, and can show newest first. Emails you have read, and the ones next to the open conversation, open at once. Brands that pass DMARC show their own logo (BIMI) instead of initials.
- **Writing that gets out of the way.** A formatted editor, address suggestions, attachments by drag and drop, drafts that save themselves, undo send and send later.
- **Works offline.** Archive, star, read and send while offline; Nimbra sends it all once it can.
- **At home on the Mac.** Nimbra can be the default mail app for email links, notifications can be archived or marked read where they appear, the Dock icon has a menu and the unread count, and mail is checked the moment the Mac wakes or the network returns.
- **Light and dark,** with emails darkened to match in dark mode, a sidebar of glass over your desktop picture, and an app icon with every macOS appearance (light, dark, clear and tinted).
- **Made to fit you.** Four mail text sizes up to Extra large, an interface zoom, a list row with the subject on its own line or beside the sender, and a list as wide as you drag it.

## Get started

```sh
cargo run
```

Nimbra's mail engine is its own repo, [nimbra-core](https://github.com/nimbra-mail/core); Cargo fetches it from GitHub at the release tag in `Cargo.toml`. To change the engine and the app together, see [Working on the engine](#working-on-the-engine).

Or download a signed build from [Releases](https://github.com/nimbra-mail/desktop/releases) (see [CHANGELOG.md](CHANGELOG.md) for what's new). Nimbra is in alpha: expect rough edges.

Without an account set up, or with `NIMBRA_DEMO=1`, Nimbra runs on built-in sample data and never touches the network.

To build the Mac app:

```sh
./scripts/bundle-macos.sh
open target/bundle/Nimbra.app
```

This builds the release binary and wraps it in `Nimbra.app` with the app icon. It needs Xcode 26 or newer. The script signs the app with your Developer ID or Apple Development certificate if one is in the keychain (override with `NIMBRA_SIGN_IDENTITY`). Signing matters for day-to-day use: macOS ties Keychain permission to the app's signature, so an unsigned build asks for the Keychain after every rebuild. For development, `./scripts/run-dev.sh` builds, signs and runs the debug binary the same way.

GPUI is built with the `runtime_shaders` feature, so Xcode's Metal toolchain is not required.

## Connect your mail

### Gmail

The builds on the Releases page sign in to Google and Microsoft as they are. The steps below are for building Nimbra yourself.

1. In Google Cloud Console, create a project, enable the Gmail API, and create an OAuth client of type "Desktop app". Keep the consent screen in Testing mode and add yourself as a test user.
2. Put the client in `~/.config/nimbra/oauth.env` (mode 600), never in the repo:

   ```sh
   NIMBRA_GOOGLE_CLIENT_ID=...
   NIMBRA_GOOGLE_CLIENT_SECRET=...
   ```

3. `cargo run`, then click **Connect Gmail**. Google opens in your browser.

Nimbra asks for the `gmail.modify` scope only. It covers reading, archiving, labelling and sending; it does not allow permanent deletion (the first permanent delete in Trash asks Google for that permission, and every one asks for confirmation). Nimbra sends only when you press Send. Signing in again takes only that same Google account: another one picked in the browser is refused before anything is saved.

Your Google profile photo comes from the People API (turn it on in your Cloud project), which tells your own photo from the letter Google draws.

### Other providers

**Add account** takes Google, or any address with a password (Zoho, iCloud, Fastmail, Yahoo, a company's own server). A Gmail or Google Workspace address can also be added this way with an [app password](https://myaccount.google.com/apppasswords), over IMAP and SMTP; Nimbra says so if Google wants one rather than your Google password. Nimbra finds the servers from the address (or you type them), checks that both take the password, and keeps the password in the keychain. A custom domain hosted at Google, Microsoft or Zoho is recognised from its mail (MX) records.

Such an account's folders are its labels and archiving moves to its Archive folder. New mail arrives at once where the server can say so (IMAP IDLE), and only what changed is asked for (CONDSTORE).

Outlook.com, Hotmail and Microsoft 365 sign in with Microsoft in the browser (**Continue with Microsoft**). That needs `NIMBRA_MICROSOFT_CLIENT_ID` (an app registered with Microsoft) in `~/.config/nimbra/oauth.env`; the sign-in is kept in the keychain, and IMAP and SMTP use it (XOAUTH2).

### Several accounts

Switch from the Accounts list in the sidebar or in Settings › Accounts. Each account has its own sign-in in the keychain and its own encrypted store and key; Sign out removes only that account. One account is the default (Settings › Accounts › Make default): its profile is at the bottom of the sidebar and new messages are from it. Drag an account by its handle in Settings › Accounts to change the order the sidebar and From list them in.

With two or more accounts the sidebar adds **All inboxes**: every account's inbox merged, newest first, loading further as you scroll. Every account is checked for new mail and notifies, snoozes and scheduled sends run for all of them, and the Dock badge counts them all. In All inboxes, actions and replies go to the message's own account; labels stay per account.

## Keys

Everything has a key. The ones you'll use most:

| Key | Action |
| --- | --- |
| `J` / `K` | Next / previous thread |
| `E` | Mark done |
| `H` | Snooze |
| `R` / `C` | Reply / new message |
| `/` | Search |
| `Z` | Undo |
| `?` | Every shortcut on one sheet |

The full list is in [docs/keys.md](docs/keys.md). Every key can be changed in **Settings › Shortcuts**: a key already in use asks before it moves, and **Reset all shortcuts** brings back the ones above.

## Privacy and security

- **Your mail stays on your Mac.** Nimbra talks straight to Gmail or your IMAP and SMTP servers; there is no server of its own in between, and your mail never goes anywhere else.
- **Encrypted at rest.** Mailboxes, opened threads, counts, the offline queue and snoozes are kept in `~/Library/Application Support/Nimbra/store`. Each record is encrypted with ChaCha20-Poly1305 using a key kept only in the system keychain (`nimbra` / `store-key`); the files alone, or a backup of them, are unreadable. Search results are never stored. When a Google sign-in ends, that account's store and its key are deleted; a keychain that cannot be read deletes nothing.
- **Secrets in the Keychain.** Sign-ins, refresh tokens and passwords live only in the system keychain. OAuth client files stay in `~/.config/nimbra`, never in the repo.
- **Locked-down email.** Mail is drawn natively by default, with scripts, styles and forms dropped. **Show original** (`V`) renders the email's own HTML in a system web view with scripts off, navigation blocked, and remote scripts, frames, fonts and forms blocked by a content security policy; mail that tries to send you somewhere by itself (a meta refresh) is stopped. Only `http`, `https` and `mailto` links are clickable, and they open in your browser when you click them.
- **Remote images.** In the reader view they stay hidden until you click **Load images**, and tracking pixels are never loaded. The original view shows them by default (senders can then tell the mail was opened); **Block images** turns them off for the open thread.
- **TLS everywhere.** IMAP and SMTP use rustls with certificates always checked.
- **Updates.** Downloaded builds look for a new version at `nimbra.me/releases/latest.json` a minute after opening and every six hours after that; that request carries nothing about you or your mail. A new version is downloaded from GitHub and only kept if its checksum matches, it is signed with Nimbra's Developer ID (Team 62TSR9B3L6) and notarized by Apple, and it is the version it says. Then the sidebar offers **Restart**, and it replaces the old app when Nimbra quits. Turn it off in **Settings › Privacy & Security › Update automatically**; builds you make yourself never update.
- **Telemetry: two anonymous counts, nothing else.** See [Telemetry](#telemetry) for exactly what is sent and how to turn it off.

## Telemetry

Nimbra has telemetry, and this is all of it. We count two things: how many Macs open Nimbra each day, and how many times a day it is opened. That's how we know whether anyone uses it.

**What is sent.** Each time an official build of Nimbra is launched, it sends this one event to [PostHog](https://posthog.com), hosted in the EU:

```json
{
  "api_key": "<Nimbra's PostHog project key: public, and can only send events>",
  "event": "app_opened",
  "distinct_id": "<random, new for every event>",
  "properties": {
    "first_today": true,
    "app_version": "0.1.0",
    "macos_version": "26",
    "$process_person_profile": false,
    "$geoip_disable": true
  }
}
```

- `first_today` is true on the first launch of the day on that Mac. Counting those gives daily users; counting every event gives opens. The day of the last launch is kept on your Mac in `~/Library/Application Support/Nimbra/last-opened`, and never sent.
- `distinct_id` is random and new every time, so two launches can't be linked to each other or to you.
- `$process_person_profile: false` tells PostHog to keep no profile, and `$geoip_disable: true` to skip working out where you are. The project is also set to discard IP addresses.

**What is never sent.** Your name, your addresses, your accounts or which providers you use, anything about your mail (senders, subjects, counts, labels), what you do in the app, crash data, or any ID that lasts beyond one event.

**When it isn't sent.** When you turn off **Settings › Privacy & Security › Anonymous usage counts**; in debug builds and test runs; in builds made without `NIMBRA_POSTHOG_KEY`, which includes any build you make yourself; and whenever you're offline (that launch just isn't counted).

**Check it yourself.** [`src/usage.rs`](src/usage.rs) is the whole thing, and a test there fails if the event ever carries anything more.

## How it works

<details>
<summary><strong>Checking for mail</strong></summary>

While the app is open it reads Gmail's change feed every 30 seconds (or 10 seconds, 1 or 5 minutes, set in Settings › General): only what changed since the last look is fetched, the open mailbox is updated in place, and a macOS notification (sender and subject) is shown for each thread with a new unread inbox message. Clicking a notification opens that email. Notifications need the bundled `Nimbra.app` and your permission, and only arrive while Nimbra is running.

Clicking a mailbox shows what is kept at once and asks Gmail only if something changed since it was last checked or that was over a minute ago. The content of the first 3 threads of the open mailbox is fetched ahead and stored, so opening those is instant (more would run into Gmail's limit of 6,000 units a minute). A full fetch only happens when there is no starting point yet, the point is older than Gmail keeps (about a week), or the mailbox is a search, and then only threads that changed since they were last listed are read again. The mail store keeps the newest 300 threads per mailbox, with full text for the newest 100.

</details>

<details>
<summary><strong>Reading</strong></summary>

HTML mail opens in its original formatting by default; `V` switches to the native reader view and back. In dark theme, light emails get a dark page and light text, with pictures and brand colours kept; Nimbra's own script does this in an isolated WebKit world, and **Settings › Reading › Darken emails in dark theme** turns it off. Printing is always light.

A conversation opens on its latest message. Every message folds to its line and opens again from its header or that line, and **Settings › Appearance › Newest message first** puts the latest at the top. The emails of the last few conversations you left stay ready, and the ones above and below the open one are prepared ahead, so going back or pressing `J` and `K` shows them at once.

A sender whose mail passed DMARC, from a domain with an enforced policy and a BIMI record, shows the brand's logo instead of initials; the sender's address stays beside it. Logos are looked up once per domain and kept for a week, in a store all accounts share.

Each message shows its sender and recipients; click the recipients line for From, To, Cc, Reply-To and the full date. Earlier messages in a thread have their own Reply, Reply all and Forward. Attachments can be previewed with Quick Look or saved to Downloads. Mailing lists get an Unsubscribe button beside the subject; spam is flagged with a Not spam button. Double-click a thread to open it in a window of its own; Return reads it in full view.

</details>

<details>
<summary><strong>Organising</strong></summary>

Opening a thread marks it read. `E` archives it (in Done, `E` moves it back to the inbox), `⌫` moves it to the trash, and `Z` undoes either. Threads show their labels (click × to remove one), the tag button adds a label or creates one, and the folder button moves a thread to the inbox, a label or Spam. The + next to Labels creates a label (“Parent/Child” nests it), and right-clicking a label renames or deletes it. Under All inboxes, deleting a label that several accounts have asks which ones to delete it from. On a server of its own (not Gmail's) a label is a folder, and deleting it deletes the mail in it; the question says so. ⌘-click and Shift-click pick several threads (⌘A picks all), and one Undo brings a batch back.

`/` or ⌘F opens search: matches from the mail on this Mac appear as you type, and the server's own results join a moment later. Search uses Gmail's syntax (`from:ana has:attachment newer_than:7d`). From All inboxes it searches every account at once, newest first; with several accounts, the chips under the field (or Tab) pick All accounts or one of them. IMAP servers understand `from:`, `to:`, `subject:`, `is:unread`, `has:attachment`, `newer_than:` and the like, and leave out what only Gmail has (`label:`, `category:`); a Gmail account added with an app password keeps all of Gmail's syntax.

</details>

<details>
<summary><strong>Snooze and reminders</strong></summary>

`H` (or ⌘K, or the clock in the toolbar or on a row) opens the Snooze menu: later today, tomorrow, tomorrow evening, this weekend or next week, each with the time it comes back, or **Pick a date** for a calendar: click a day and press Snooze (or Return). Its Time row opens a field to type any time in (“4:45 PM”, “16:45”, “430p”; a bare 1 to 7 is the afternoon) and the hours from 6 AM to 9 PM as shortcuts. The **Only if nobody replies** switch makes any of them a reminder.

Snoozed threads leave the inbox and come back unread with a notification; a reply that arrives first brings the thread back by itself and ends the snooze, as in Gmail, and a reminder only comes back if nobody replied. Snoozed and Reminders list every account's snoozes with the time each comes back. Each snooze is kept on this Mac with its own account, because Gmail's API has no snooze and IMAP has none at all, and happens while Nimbra is running (anything due while it was closed comes back when it opens).

</details>

<details>
<summary><strong>Blocking senders</strong></summary>

**Block sender** (in the reading pane's ⋯ menu) moves the open thread to a Blocked label, made if the account has none, which is where Spark files blocked mail, and blocks the sender's address for that account. Their new mail goes straight to Blocked, marked read, and never notifies; what they left in the inbox goes too. The first time each account is opened, the senders already in its Blocked label are blocked as well. Undo takes the sender off the list again, and **Unblock sender** (on a thread in Blocked) moves it back to the inbox. The list is kept on this Mac, so mail that arrives while Nimbra is closed is moved when it opens.

</details>

<details>
<summary><strong>Writing and sending</strong></summary>

Messages are written in a formatted editor: bold (⌘B), italic (⌘I), underline (⌘U), links (⌘K), bulleted and numbered lists, quotes. They are sent with both an HTML and a plain-text version, and pasted text keeps simple formatting only. Replies and forwards include the original message, folded behind •••; a forward also takes the original's attachments. With several accounts, a new message or a forward can be sent from any of them.

Each account has its own signatures, made in **Settings › Composing**: pick the account, then add as many as you like, name them and choose the default (or No signature). Edit one as formatted text, as HTML, or see it in Preview. Signatures can hold links, tables and images from the web; scripts, forms and unsafe links are taken out when they are saved. A new message, reply or forward starts with its account's default under what you write, kept apart from your text; click the small handle above it to pick another or none, and changing From switches to that account's default. The editor and Preview load nothing from the web themselves: Nimbra downloads each image once (https on a public host only, up to 1 MB, PNG, JPEG, GIF or WebP), keeps it in `~/Library/Caches/Nimbra/signature-images`, and shows that copy; **Refresh images** in Preview downloads them again. The message that is sent still points to the image on the web.

The composer has Cc and Bcc, and takes attachments from the paperclip or by dropping files on it (25 MB in total). As you type in To, Cc or Bcc it suggests people from every account by name, address or domain, the ones you write to first. It learns them from the mail you list, open and send, and once per account from the headers of your newest 2,000 sent and 1,000 received messages (read slowly in the background, and never Promotions, Social, Updates or Forums). They're kept encrypted on this Mac, and Settings › Privacy's clear forgets them too. Drafts save themselves a moment after you stop typing. Sending waits six seconds so you can undo it (⌘Z). From Send until the server has it, a message is kept encrypted on this Mac: quitting sends it first, and one that could not leave is sent when its account next connects. **Send later** sends the saved draft at the chosen time while Nimbra is running.

Offline, archive, star, read and the like still work: changes are kept (encrypted, on this Mac) and sent once the server can be reached, and an Offline pill shows how many are waiting.

</details>

## App icon

The icon source is `assets/AppIcon.icon`, an Icon Composer document (SVG layers plus `icon.json`); the same file works for iOS. Its layers are designed in Figma as one 1024×1024 frame per layer, inside the Light, Eclipse and Flap groups, and exported as SVG into `assets/AppIcon.icon/Assets`.

`icon.json` gives it all of macOS's appearances: Default is the light icon (a cream-to-lavender background, the flap and disc in ink, a softer glow), Dark is the violet-to-black one with the paper-white flap, and Clear and Tinted hide the glow and the dark disc, so the mark is a ring over an envelope flap. Layers that use glass must be filled shapes: outline strokes in Figma before exporting, because an open stroke is filled as a closed shape.

Render a preview from the command line (`--rendition` takes `Default`, `Dark`, `ClearLight`, `ClearDark`, `TintedLight` or `TintedDark`):

```sh
"/Applications/Xcode.app/Contents/Applications/Icon Composer.app/Contents/Executables/ictool" \
  assets/AppIcon.icon --export-image --output-file icon.png \
  --platform macOS --rendition Default --width 512 --height 512 --scale 1
```

Settings › About shows `assets/app-icon.png` in light mode and `assets/app-icon-dark.png` in dark mode, and this README shows `.github/assets/icon-light.png` and `icon-dark.png`. After changing the icon, render them again with the command above (`--rendition Default` for the light ones, `--rendition Dark` for the dark ones; 192 px for the app, 256 px for the README).

## Development

### Screenshot states

`NIMBRA_SCREEN=welcome`, `palette`, `finder` (the search panel, holding `NIMBRA_SEARCH` if set), `zero`, `loading`, `compose`, `compose-full`, `compose-suggest`, `draft`, `bulk` (three threads picked), `window`, `more`, `offline`, `snooze`, `snooze-time`, `shortcuts`, `account` (and `account-servers`), `update`, `update-download`, `updated`, `up-to-date`, `settings`, `settings-accounts`, `settings-remove`, `settings-appearance`, `settings-reading`, `settings-composing` (also `settings-composing-html` and `settings-composing-preview`, on the sample's HTML signature), `settings-notifications`, `settings-privacy`, `delete-label` (a label two accounts have, asking which to delete it from), `settings-shortcuts` (also `settings-shortcuts-record`, `settings-shortcuts-replace`, `settings-shortcuts-search` and `settings-shortcuts-reset`), `settings-about` or `about` opens that screen directly on the sample data. `NIMBRA_SHORTCUTS_TAB=<n>` switches the shortcuts sheet to tab n a second after it opens. `NIMBRA_THEME=light` starts in light mode. `NIMBRA_SIDEBAR=closed` (or `open`) starts with the sidebar hidden or docked, and `NIMBRA_LIST=closed` with the list hidden. `NIMBRA_LIST_WIDTH=<px>` sets the list's width. `NIMBRA_MAILBOX=sent` (or `starred`, `snoozed`, `drafts`, `done`, `all`, `important`, `spam`, `trash`) opens that mailbox first. `NIMBRA_OPEN=<row>` opens that thread first. `NIMBRA_READER=1` starts in the native reader view. `NIMBRA_SEARCH=<query>` starts with a search run. `NIMBRA_SHIFT=<px>` moves the window sideways. `NIMBRA_NOTIFY_TEST=1` posts one sample notification, to check the setup.

### Working on the engine

To build the app against your own copy of [nimbra-core](https://github.com/nimbra-mail/core) instead of the release, clone it next to this repo and switch to it:

```sh
git clone https://github.com/nimbra-mail/core ../core
scripts/local-core.sh on      # build nimbra-core from ../core
scripts/local-core.sh status  # which one is in use
scripts/local-core.sh off     # back to the release tag
```

`on` writes a gitignored `.cargo/config.toml` with a `[patch]` for the git dependency; `Cargo.toml` stays as it is. While it is on, `Cargo.lock` points nimbra-core at the local path: run `off` before committing (CI rejects a `Cargo.lock` without the GitHub source). To take up a new engine release, change the tag in `Cargo.toml` (both entries) and run `cargo update -p nimbra-core`.

### Releases

`./scripts/release <version>` builds, signs, notarizes and publishes a release; see [docs/releasing.md](docs/releasing.md). What changed in each version is in [CHANGELOG.md](CHANGELOG.md).

### Live tests

The IMAP and SMTP code, in [nimbra-core](https://github.com/nimbra-mail/core), has live tests against local Dovecot and Mailpit servers (OrbStack or Docker); its `docs/testing.md` has the setup.

### Sidebar glass

The sidebar is glass over the desktop picture. `src/wallpaper.rs` puts a native layer with the desktop picture, blurred, under the app's drawing, placed where the picture is on screen, so the glass shows the desktop and never another app's window; macOS moves it with the window, so it never lags. Under it, `src/blur.rs` still asks the window server for a backdrop blur, because GPUI's own blurred window mode did not blur on macOS 27. That call is a private API: fine for now, but a risk for App Store review. Windows and Linux fall back to GPUI's built-in behaviour.

### Layout

The mail engine (providers, MIME, discovery, the encrypted store, sending, contacts, signatures, snooze) is not in
this repo: it is [nimbra-core](https://github.com/nimbra-mail/core), which the iOS app runs too. Cargo takes it from
GitHub at a release tag (or from `../core`, with [`scripts/local-core.sh on`](#working-on-the-engine)); `crate::mail`, `crate::gmail` and the rest are its
modules, brought in by name in `src/main.rs`. Changes to how mail is fetched, kept or sent go there.

**The Mac app** (`src/`, GPUI):

- `src/theme.rs`: colours for dark and light, fonts, gradients
- `src/usage.rs`: the anonymous usage count
- `src/update.rs`: the app's own updates (the feed, the download and its checks)
- `src/checking.rs`, `src/arrival.rs`, `src/quitting.rs`: checks for mail, Inbox Zero at launch, and what quitting has to finish
- `src/signature_images.rs`: signature images drawn natively, from the core's downloaded copies
- `src/webview.rs`, `src/mail_views.rs`: the locked-down web view, and the emails in their own formatting inside a conversation
- `src/editor.rs`, `src/editor.html`: the formatted message editor, a system web view running Nimbra's own page
- `src/app.rs`: the app's state, key actions and the window's layout; the rest of the `Nimbra` type continues in `accounts.rs`, `mailbox.rs`, `threads.rs`, `labels.rs`, `finder.rs`, `composer.rs`, `outgoing.rs` and `menus.rs`
- `src/sidebar.rs`, `src/list.rs`, `src/reading.rs`, `src/snooze_menu.rs`, `src/palette.rs`: the sidebar, the thread list (with Inbox Zero), the reading pane, the Snooze menu and the command palette
- `src/integration.rs`, `src/system_events.rs`, `src/notify.rs`, `src/system.rs`: what macOS tells the app (links, notification answers, wake, network, appearance, display options) and what the app does with it; notifications, the Dock badge and opening at login
- `src/settings_window.rs`, `src/message_window.rs`, `src/add_account.rs`: the Settings window, a conversation in a window of its own, and the sheet that adds an IMAP account
- `src/window_frame.rs`, `src/place.rs`: the main window's frame, and the account and mailbox it shows, across launches
- `src/wallpaper.rs`, `src/blur.rs`: the sidebar's glass (see [Sidebar glass](#sidebar-glass))
- `src/ui.rs`, `src/input.rs`: shared building blocks and the text field
- `assets/`: the app icon, bundled Geist and Instrument Serif fonts (OFL) and Lucide-style icons

## Contributing

Bug fixes are welcome; for anything bigger, start a discussion first. [CONTRIBUTING.md](CONTRIBUTING.md) has how to
run it, what a good change looks like, and the commit style ([Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/)).
Found a security problem? Report it privately: see [SECURITY.md](SECURITY.md).

## License

[MIT](LICENSE)
