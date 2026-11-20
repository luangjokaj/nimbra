# Nimbra

A mail client concept, mocked in [GPUI](https://www.gpui.rs) with static data. No network, no accounts.

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
| `Z` | Undo |
| `⌘K` or `H` | Command palette (snooze) |
| `↵` / `esc` | Run command / close palette |
| `⌘⇧L` | Toggle light and dark |

## Screenshot states

`NIMBRA_SCREEN=palette` or `NIMBRA_SCREEN=zero` opens that screen directly. `NIMBRA_THEME=light` starts in light mode. `NIMBRA_SHIFT=<px>` moves the window sideways.

## Sidebar blur

The sidebar is translucent and blurs whatever is behind the window. GPUI's own blurred window mode did not blur on macOS 27, so `src/blur.rs` asks the macOS window server directly through a private API. That is fine for a prototype but is a risk for App Store review. Windows and Linux fall back to GPUI's built-in behaviour.

## Layout

- `src/theme.rs`: colours for dark and light, fonts, gradients
- `src/data.rs`: mock inbox and palette commands
- `src/ui.rs`: icon, kbd, avatar, pill, buttons
- `src/app.rs`: state, key actions, and the sidebar, list, reading pane, palette and Inbox Zero views
- `assets/`: bundled Geist and Instrument Serif fonts (OFL) and Lucide-style icons
