# Developing Nimbra

How to build and run Nimbra, open any screen directly, work on the engine beside it, and where the code lives. [CONTRIBUTING.md](../CONTRIBUTING.md) has what a good change looks like, and [testing.md](testing.md) how it is tested.

- [Building](#building)
- [Screenshot states](#screenshot-states)
- [Looking at the printed page](#looking-at-the-printed-page)
- [Working on the engine](#working-on-the-engine)
- [Layout](#layout)
- [Sidebar glass](#sidebar-glass)
- [App icon](#app-icon)
- [Live tests](#live-tests)
- [Releases](#releases)

## Building

```sh
cargo run
```

Nimbra's mail engine is its own repo, [nimbra-core](https://github.com/nimbra-mail/core); Cargo fetches it from GitHub at the release tag in `Cargo.toml`. To change the engine and the app together, see [Working on the engine](#working-on-the-engine).

Without an account set up, or with `NIMBRA_DEMO=1`, Nimbra runs on built-in sample data and never touches the network. To use a real account in a build of your own, see [Connecting your mail](accounts.md#gmail).

To build the Mac app:

```sh
./scripts/bundle-macos.sh
open target/bundle/Nimbra.app
```

This builds the release binary and wraps it in `Nimbra.app` with the app icon. It needs Xcode 26 or newer. The script signs the app with your Developer ID or Apple Development certificate if one is in the keychain (override with `NIMBRA_SIGN_IDENTITY`). Signing matters for day-to-day use: macOS ties Keychain permission to the app's signature, so an unsigned build asks for the Keychain after every rebuild. For development, `./scripts/run-dev.sh` builds, signs and runs the debug binary the same way.

GPUI is built with the `runtime_shaders` feature, so Xcode's Metal toolchain is not required.

## Screenshot states

`NIMBRA_SCREEN=welcome`, `palette`, `finder` (the search panel, holding `NIMBRA_SEARCH` if set), `zero`, `loading`, `compose`, `compose-full`, `compose-suggest`, `draft`, `bulk` (three threads picked), `window`, `more`, `offline`, `snooze`, `snooze-time`, `shortcuts`, `account` (and `account-servers`), `update`, `update-download`, `updated`, `up-to-date`, `settings`, `settings-accounts`, `settings-remove`, `settings-appearance`, `settings-reading`, `settings-composing` (also `settings-composing-html` and `settings-composing-preview`, on the sample's HTML signature), `settings-notifications`, `settings-privacy`, `delete-label` (a label two accounts have, asking which to delete it from), `settings-shortcuts` (also `settings-shortcuts-record`, `settings-shortcuts-replace`, `settings-shortcuts-search` and `settings-shortcuts-reset`), `settings-about` or `about` opens that screen directly on the sample data. `NIMBRA_SHORTCUTS_TAB=<n>` switches the shortcuts sheet to tab n a second after it opens. `NIMBRA_THEME=light` starts in light mode. `NIMBRA_SIDEBAR=closed` (or `open`) starts with the sidebar hidden or docked, and `NIMBRA_LIST=closed` with the list hidden. `NIMBRA_LIST_WIDTH=<px>` sets the list's width. `NIMBRA_MAILBOX=sent` (or `starred`, `snoozed`, `drafts`, `done`, `all`, `important`, `spam`, `trash`) opens that mailbox first. `NIMBRA_OPEN=<row>` opens that thread first. `NIMBRA_READER=1` starts in the native reader view. `NIMBRA_SEARCH=<query>` starts with a search run. `NIMBRA_SHIFT=<px>` moves the window sideways. `NIMBRA_NOTIFY_TEST=1` posts one sample notification, to check the setup.

## Looking at the printed page

The printed page (`src/print.rs`) can be looked at without printing: `NIMBRA_PRINT_OUT=<folder> cargo test print_sample -- --ignored` writes the sample thread as a conversation (in its reader text and in its own formatting) and as one message, as HTML files to open in Safari.

## Working on the engine

To build the app against your own copy of [nimbra-core](https://github.com/nimbra-mail/core) instead of the release, clone it next to this repo and switch to it:

```sh
git clone https://github.com/nimbra-mail/core ../core
scripts/local-core.sh on      # build nimbra-core from ../core
scripts/local-core.sh status  # which one is in use
scripts/local-core.sh off     # back to the release tag
```

`on` writes a gitignored `.cargo/config.toml` with a `[patch]` for the git dependency; `Cargo.toml` stays as it is. While it is on, `Cargo.lock` points nimbra-core at the local path: run `off` before committing (CI rejects a `Cargo.lock` without the GitHub source). To take up a new engine release, change the tag in `Cargo.toml` (both entries) and run `cargo update -p nimbra-core`.

## Layout

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

## Sidebar glass

The sidebar is glass over the desktop picture. `src/wallpaper.rs` puts a native layer with the desktop picture, blurred, under the app's drawing, placed where the picture is on screen, so the glass shows the desktop and never another app's window; macOS moves it with the window, so it never lags. Under it, `src/blur.rs` still asks the window server for a backdrop blur, because GPUI's own blurred window mode did not blur on macOS 27. That call is a private API: fine for now, but a risk for App Store review. Windows and Linux fall back to GPUI's built-in behaviour.

## App icon

The icon source is `assets/AppIcon.icon`, an Icon Composer document (SVG layers plus `icon.json`); the same file works for iOS. Its layers are designed in Figma as one 1024×1024 frame per layer, inside the Light, Eclipse and Flap groups, and exported as SVG into `assets/AppIcon.icon/Assets`.

`icon.json` gives it all of macOS's appearances: Default is the light icon (a cream-to-lavender background, the flap and disc in ink, a softer glow), Dark is the violet-to-black one with the paper-white flap, and Clear and Tinted hide the glow and the dark disc, so the mark is a ring over an envelope flap. Layers that use glass must be filled shapes: outline strokes in Figma before exporting, because an open stroke is filled as a closed shape.

Render a preview from the command line (`--rendition` takes `Default`, `Dark`, `ClearLight`, `ClearDark`, `TintedLight` or `TintedDark`):

```sh
"/Applications/Xcode.app/Contents/Applications/Icon Composer.app/Contents/Executables/ictool" \
  assets/AppIcon.icon --export-image --output-file icon.png \
  --platform macOS --rendition Default --width 512 --height 512 --scale 1
```

Settings › About shows `assets/app-icon.png` in light mode and `assets/app-icon-dark.png` in dark mode, and the README shows `.github/assets/icon-light.png` and `icon-dark.png`. After changing the icon, render them again with the command above (`--rendition Default` for the light ones, `--rendition Dark` for the dark ones; 192 px for the app, 256 px for the README).

## Live tests

The IMAP and SMTP code, in [nimbra-core](https://github.com/nimbra-mail/core), has live tests against local Dovecot and Mailpit servers (OrbStack or Docker); its `docs/testing.md` has the setup.

## Releases

`./scripts/release <version>` builds, signs, notarizes and publishes a release; see [releasing.md](releasing.md). What changed in each version is in [CHANGELOG.md](../CHANGELOG.md).
