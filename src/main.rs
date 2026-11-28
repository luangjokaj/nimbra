//! Nimbra — a mail client concept, mocked in GPUI with static data.

mod about;
mod app;
mod arrival;
mod autohide;
mod blur;
mod checking;
mod data;
mod drag_scroll;
mod editor;
mod erase;
mod help;
mod input;
mod inspiration;
mod integration;
mod keyboard;
mod listing;
mod mail_views;
mod notify;
mod pending;
mod place;
mod popup;
mod quitting;
mod shortcuts;
mod signature_images;
mod sky;
mod system;
mod theme;
mod toast;
mod tooltip;
mod ui;
mod update;
mod usage;
mod wallpaper;
mod webkeys;
mod webview;
mod when;
mod window_frame;

// The engine, as it was before it moved to its own crate: `crate::mail` and the like.
use nimbra_core::{
    blocking, contacts, discover, gmail, imap_mail, leaving, mail, mailto, microsoft, outbox, rich,
    scheduled, sending, servers, settings, signature, smtp, store,
};

use std::borrow::Cow;

use crate::ui::to_screen;

use gpui::{
    App, Application, AssetSource, Bounds, KeyBinding, SharedString, TitlebarOptions,
    WindowBackgroundAppearance, WindowBounds, WindowOptions, point, prelude::*, px, size,
};

use app::{
    AboutNimbra, ActualSize, CheckForUpdates, Compose, Dismiss, DockDrafts, DockInbox, DockSent,
    DockSnoozed, DockStarred, GoNumber, HideApp, HideOthers, HideWindow, MenuCompose,
    MenuForwardMessage, MenuMarkDone, MenuMarkUnread, MenuReload, MenuReply, MenuReplyAll,
    MenuSearch, MenuShortcuts, MenuSnooze, MenuStar, MenuToggleList, MenuToggleOriginal,
    MenuToggleSidebar, MenuToggleTheme, MenuTrash, MenuUndo, Minimize, Nimbra, OpenSettings,
    OpenSidebarRow, Print, Quit, ReleaseNotes, Reload, ReportProblem, ShowAll, ToggleList,
    ToggleTheme, UseDarkAppearance, UseLightAppearance, UseSystemAppearance, ZoomIn, ZoomOut,
};

/// Icons are compiled into the binary so the app has no runtime file dependencies.
macro_rules! icons {
    ($($name:literal),* $(,)?) => {
        &[$((
            concat!("icons/", $name, ".svg"),
            include_bytes!(concat!("../assets/icons/", $name, ".svg")) as &[u8],
        )),*]
    };
}

static ICONS: &[(&str, &[u8])] = icons![
    "inbox",
    "star",
    "clock",
    "send",
    "draft",
    "done",
    "copy",
    "x",
    "detach",
    "attach",
    "trash",
    "sign-out",
    "settings",
    "search",
    "reply",
    "reply-all",
    "forward",
    "up",
    "down",
    "sparkles",
    "clip",
    "grip",
    "play",
    "more",
    "tag",
    "bell",
    "users",
    "calendar",
    "news",
    "zap",
    "file",
    "image",
    "link",
    "sun",
    "moon",
    "sidebar",
    "back",
    "next",
    "reload",
    "list",
    "mail",
    "expand",
    "shrink",
    "bold",
    "italic",
    "underline",
    "bullets",
    "numbers",
    "quote",
    "clear-format",
    "folder",
    "ban",
    "alert",
    "archive",
    "pulse",
    "eye",
    "lock",
    "key",
];

struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        Ok(ICONS
            .iter()
            .find(|(name, _)| *name == path)
            .map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, _path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(ICONS.iter().map(|(name, _)| (*name).into()).collect())
    }
}

thread_local! {
    /// The main window, to bring back when the Dock icon is clicked.
    static MAIN_WINDOW: std::cell::Cell<Option<gpui::AnyWindowHandle>> =
        const { std::cell::Cell::new(None) };
}

/// The main window, with its app.
fn main_window(_cx: &App) -> Option<gpui::WindowHandle<Nimbra>> {
    MAIN_WINDOW.with(|main| main.get())?.downcast::<Nimbra>()
}

/// Shows the main window again (Dock icon click while it is hidden).
fn show_main_window(cx: &mut App) {
    if let Some(handle) = MAIN_WINDOW.with(|main| main.get()) {
        handle
            .update(cx, |_, window, _| blur::set_visible(window, true))
            .ok();
    }
}

/// Saves where the main window is, a moment after it stops moving or resizing.
fn keep_frame(window: &mut gpui::Window, cx: &mut gpui::Context<Nimbra>) {
    let moves = std::rc::Rc::new(std::cell::Cell::new(0u64));
    cx.observe_window_bounds(window, move |_, window, cx| {
        if window.is_fullscreen() {
            return;
        }
        let (bounds, maximized) = match window.window_bounds() {
            WindowBounds::Maximized(bounds) => (bounds, true),
            WindowBounds::Windowed(bounds) => (bounds, false),
            WindowBounds::Fullscreen(_) => return,
        };
        let Some(display) = window
            .display(cx)
            .and_then(|display| display.uuid().ok())
            .map(|id| id.to_string())
        else {
            return;
        };
        let saved = window_frame::Saved {
            frame: window_frame::Frame {
                x: bounds.origin.x.into(),
                y: bounds.origin.y.into(),
                width: bounds.size.width.into(),
                height: bounds.size.height.into(),
                maximized,
            },
            display,
        };
        moves.set(moves.get() + 1);
        let (moves, this_move) = (moves.clone(), moves.get());
        cx.spawn(async move |_, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(500))
                .await;
            if moves.get() == this_move {
                cx.background_spawn(async move { window_frame::save(saved) })
                    .await;
            }
        })
        .detach();
    })
    .detach();
}

/// The keys that stay as they are, whatever is changed in Settings › Shortcuts: Quit, copy,
/// zoom, Escape, switching account. Every other key is in `shortcuts::TABLE`.
pub fn fixed_bindings() -> Vec<KeyBinding> {
    vec![
        // Copies text selected in the reader.
        KeyBinding::new("cmd-c", input::Copy, Some("Mail")),
        KeyBinding::new("ctrl-c", input::Copy, Some("Mail")),
        KeyBinding::new("escape", Dismiss, None),
        // Zoom In takes ⌘= too: ⌘+ is ⌘⇧= on most keyboards.
        KeyBinding::new("cmd-=", ZoomIn, None),
        KeyBinding::new("cmd-+", ZoomIn, None),
        KeyBinding::new("cmd--", ZoomOut, None),
        KeyBinding::new("cmd-0", ActualSize, None),
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("alt-cmd-h", HideOthers, None),
        KeyBinding::new("alt-shift-cmd-i", ReportProblem, None),
        // Walking the window: ← to the sidebar (a shortcut of the table), → back with what
        // is under the cursor.
        KeyBinding::new(
            "right",
            OpenSidebarRow,
            Some("Mail && Sidebar && !TextInput"),
        ),
        KeyBinding::new("cmd-1", GoNumber(1), Some("Mail")),
        KeyBinding::new("cmd-2", GoNumber(2), Some("Mail")),
        KeyBinding::new("cmd-3", GoNumber(3), Some("Mail")),
        KeyBinding::new("cmd-4", GoNumber(4), Some("Mail")),
        KeyBinding::new("cmd-5", GoNumber(5), Some("Mail")),
        KeyBinding::new("cmd-6", GoNumber(6), Some("Mail")),
        KeyBinding::new("cmd-7", GoNumber(7), Some("Mail")),
        KeyBinding::new("cmd-8", GoNumber(8), Some("Mail")),
        KeyBinding::new("cmd-9", GoNumber(9), Some("Mail")),
        // Escape belongs to the message editor while it has the keyboard (it closes its
        // link box). Its formatting and the composer's keys come from the table.
        KeyBinding::new("escape", gpui::NoAction, Some("Editor")),
    ]
}

/// The fixed keys a shortcut cannot take, each with what it does (the editor's own keys
/// are not the app's, and are left out).
pub fn fixed_keys() -> Vec<(String, &'static str)> {
    fixed_bindings()
        .iter()
        .filter(|binding| !binding.action().as_any().is::<gpui::NoAction>())
        .map(|binding| {
            let key = binding
                .keystrokes()
                .iter()
                .map(|k| k.unparse())
                .collect::<Vec<_>>()
                .join(" ");
            (key, fixed_name(binding.action().name()))
        })
        .collect()
}

/// What a fixed key does, as the conflict prompt says it.
fn fixed_name(action: &str) -> &'static str {
    match action.rsplit("::").next().unwrap_or(action) {
        "SendMail" => "Send",
        "SaveDraft" => "Save draft",
        "ToggleFullCompose" => "Full page composer",
        "Undo" => "Undo",
        "Copy" => "Copy",
        "Dismiss" => "Close or go back",
        "Confirm" => "Full view",
        "ZoomIn" => "Zoom in",
        "ZoomOut" => "Zoom out",
        "ActualSize" => "Actual size",
        "Quit" => "Quit",
        "HideApp" => "Hide Nimbra",
        "HideOthers" => "Hide others",
        "Minimize" => "Minimise",
        "OpenSettings" => "Settings",
        "ReportProblem" => "Report a problem",
        "Print" => "Print",
        "OpenSidebarRow" => "Open the sidebar row",
        "GoNumber" => "Switch account",
        "HideWindow" => "Close window",
        _ => "another shortcut",
    }
}

/// A menu item for an action whose keys can be changed. macOS gives the item the action's
/// first key that works anywhere in the app as its key equivalent. An action with no such
/// key (only letters, say, which would fire while typing) goes through its `twin`, which
/// has no key, and the title names the key instead.
fn shortcut_item(
    title: &str,
    id: &str,
    action: impl gpui::Action,
    twin: impl gpui::Action,
) -> gpui::MenuItem {
    let anywhere = shortcuts::find(id).is_some_and(|shortcut| {
        shortcuts::keys(id)
            .iter()
            .any(|key| shortcuts::context_for(shortcut, key).is_none())
    });
    if anywhere {
        gpui::MenuItem::action(title.to_string(), action)
    } else {
        gpui::MenuItem::action(with_key(title, id), twin)
    }
}

/// "Reply  (R)": a title with the action's key, for an item without a key equivalent.
fn with_key(title: &str, id: &str) -> String {
    let key = shortcuts::label(id);
    if key.is_empty() {
        title.to_string()
    } else {
        format!("{title}  ({key})")
    }
}

/// The menu bar. Items only carry shortcuts that use ⌘ and never clash with the message
/// editor; the rest (E, R, S…) are menu-only twins without a shortcut, so a letter typed in
/// the editor never triggers them. See `MenuReply` and friends.
fn app_menus() -> Vec<gpui::Menu> {
    use gpui::{Menu, MenuItem, OsAction};
    vec![
        Menu {
            name: "Nimbra".into(),
            items: vec![
                MenuItem::action("About Nimbra", AboutNimbra),
                MenuItem::action("Check for Updates…", CheckForUpdates),
                MenuItem::separator(),
                MenuItem::action("Settings…", OpenSettings),
                MenuItem::separator(),
                MenuItem::action("Hide Nimbra", HideApp),
                MenuItem::action("Hide Others", HideOthers),
                MenuItem::action("Show All", ShowAll),
                MenuItem::separator(),
                MenuItem::action("Quit Nimbra", Quit),
            ],
        },
        Menu {
            name: "File".into(),
            items: vec![
                shortcut_item("New Message", "compose", Compose, MenuCompose),
                shortcut_item("Check for Mail", "reload", Reload, MenuReload),
                MenuItem::separator(),
                MenuItem::action("Print…", Print),
                MenuItem::separator(),
                MenuItem::action("Close Window", HideWindow),
            ],
        },
        // The Edit menu gives ⌘X, ⌘C, ⌘V and ⌘A to whichever view has the keyboard,
        // including the message editor (a web view only gets them through the menu).
        Menu {
            name: "Edit".into(),
            items: vec![
                MenuItem::os_action("Cut", input::Cut, OsAction::Cut),
                MenuItem::os_action("Copy", input::Copy, OsAction::Copy),
                MenuItem::os_action("Paste", input::Paste, OsAction::Paste),
                MenuItem::os_action("Select All", input::SelectAll, OsAction::SelectAll),
                MenuItem::separator(),
                MenuItem::action("Find Mail", MenuSearch),
            ],
        },
        Menu {
            name: "View".into(),
            items: vec![
                MenuItem::action("Show or Hide Sidebar", MenuToggleSidebar),
                shortcut_item(
                    "Show or Hide List",
                    "toggle-list",
                    ToggleList,
                    MenuToggleList,
                ),
                MenuItem::action("Original Formatting or Reader", MenuToggleOriginal),
                MenuItem::separator(),
                MenuItem::action("Actual Size", ActualSize),
                MenuItem::action("Zoom In", ZoomIn),
                MenuItem::action("Zoom Out", ZoomOut),
                MenuItem::separator(),
                // Checked by `set_menu_check`, which finds it by these titles.
                MenuItem::submenu(Menu {
                    name: "Appearance".into(),
                    items: vec![
                        MenuItem::action("System", UseSystemAppearance),
                        MenuItem::action("Light", UseLightAppearance),
                        MenuItem::action("Dark", UseDarkAppearance),
                        MenuItem::separator(),
                        shortcut_item(
                            "Switch Light and Dark",
                            "toggle-theme",
                            ToggleTheme,
                            MenuToggleTheme,
                        ),
                    ],
                }),
            ],
        },
        Menu {
            name: "Message".into(),
            items: vec![
                MenuItem::action(with_key("Reply", "reply"), MenuReply),
                MenuItem::action(with_key("Reply All", "reply-all"), MenuReplyAll),
                MenuItem::action(with_key("Forward", "forward-message"), MenuForwardMessage),
                MenuItem::separator(),
                MenuItem::action(with_key("Mark Done", "mark-done"), MenuMarkDone),
                MenuItem::action(
                    with_key("Mark Read or Unread", "mark-unread"),
                    MenuMarkUnread,
                ),
                MenuItem::action(with_key("Star", "toggle-star"), MenuStar),
                MenuItem::action(with_key("Snooze…", "snooze"), MenuSnooze),
                MenuItem::action("Move to Trash", MenuTrash),
                MenuItem::separator(),
                MenuItem::action(with_key("Undo", "undo"), MenuUndo),
            ],
        },
        Menu {
            name: "Window".into(),
            items: vec![
                MenuItem::action("Minimize", Minimize),
                MenuItem::action("Close Window", HideWindow),
            ],
        },
        Menu {
            name: "Help".into(),
            items: vec![
                MenuItem::action("Report a Problem…", ReportProblem),
                MenuItem::action(with_key("Keyboard Shortcuts", "shortcuts"), MenuShortcuts),
                MenuItem::action("Release Notes", ReleaseNotes),
            ],
        },
    ]
}

/// Binds every key again, with the shortcuts in effect (`shortcuts::current`), and sets the
/// menu bar to show them. At launch, and whenever a shortcut changes.
pub fn apply_shortcuts(cx: &mut App) {
    cx.clear_key_bindings();
    input::bind_keys(cx);
    cx.bind_keys(shortcuts::bindings(&shortcuts::current()));
    cx.bind_keys(fixed_bindings());
    cx.set_menus(app_menus());
}

fn main() {
    // Before any window builds its pipelines.
    erase::install();
    quitting::install();
    let app = Application::new().with_assets(Assets);
    // Clicking the Dock icon while the window is hidden brings it back.
    app.on_reopen(show_main_window);
    // Links from other apps, also the one that launched Nimbra: they wait for the app.
    app.on_open_urls(|urls| integration::post(integration::Event::OpenUrls(urls)));
    app.run(|cx: &mut App| {
        integration::install();
        app::install_system_actions(cx);
        cx.text_system()
            .add_fonts(vec![
                Cow::Borrowed(include_bytes!("../assets/fonts/Geist-Regular.ttf").as_slice()),
                Cow::Borrowed(include_bytes!("../assets/fonts/Geist-Medium.ttf").as_slice()),
                Cow::Borrowed(include_bytes!("../assets/fonts/Geist-SemiBold.ttf").as_slice()),
                Cow::Borrowed(include_bytes!("../assets/fonts/GeistMono-Regular.ttf").as_slice()),
                Cow::Borrowed(include_bytes!("../assets/fonts/GeistMono-Medium.ttf").as_slice()),
                Cow::Borrowed(
                    include_bytes!("../assets/fonts/InstrumentSerif-Regular.ttf").as_slice(),
                ),
                Cow::Borrowed(
                    include_bytes!("../assets/fonts/InstrumentSerif-Italic.ttf").as_slice(),
                ),
            ])
            .expect("bundled fonts should load");

        // Arrows and the like go to a web view that has the keyboard, not through GPUI.
        webkeys::install();
        cx.on_action(|_: &AboutNimbra, _| about::show());
        cx.on_action(|_: &ReportProblem, cx| help::report_problem(cx));
        cx.on_action(|_: &ReleaseNotes, cx| help::release_notes(cx));
        cx.on_action(|_: &HideApp, cx| cx.hide());
        cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
        cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
        // ⌘Q from any window. ⌘W in a window other than the main one (a message, a
        // composer, Settings) closes it; the main window handles its own ⌘W (it hides).
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_action(|_: &HideWindow, cx| {
            let main = MAIN_WINDOW.with(|main| main.get());
            if let Some(window) = cx.active_window()
                && Some(window) != main
            {
                // After this key press is handled: the window is busy dispatching it now.
                cx.defer(move |cx| {
                    window
                        .update(cx, |_, window, _| window.remove_window())
                        .ok();
                });
            }
        });
        // The keys and the menu bar, with the shortcuts changed on this Mac.
        shortcuts::set_current(shortcuts::load());
        apply_shortcuts(cx);

        use gpui::MenuItem;
        cx.set_dock_menu(vec![
            MenuItem::action("New Message", Compose),
            MenuItem::separator(),
            MenuItem::action("Inbox", DockInbox),
            MenuItem::action("Starred", DockStarred),
            MenuItem::action("Snoozed", DockSnoozed),
            MenuItem::action("Drafts", DockDrafts),
            MenuItem::action("Sent", DockSent),
        ]);

        // The interface zoom, before anything is sized for it.
        crate::ui::set_zoom(crate::settings::load().zoom());
        const MIN_SIZE: (f32, f32) = (1120., 720.);
        // Back where it was, on the same display, if that is still connected.
        let saved = window_frame::load().and_then(|saved| {
            let display = cx
                .displays()
                .into_iter()
                .find(|d| d.uuid().is_ok_and(|id| id.to_string() == saved.display))?;
            let size = display.bounds().size;
            let whole = window_frame::Frame {
                x: 0.,
                y: 0.,
                width: size.width.into(),
                height: size.height.into(),
                maximized: false,
            };
            Some((
                display.id(),
                window_frame::clamp(saved.frame, &[whole], MIN_SIZE)?,
            ))
        });
        let mut bounds = Bounds::centered(None, size(to_screen(1480.), to_screen(920.)), cx);
        // NIMBRA_SHIFT=<px> nudges the window sideways (used to test the backdrop blur).
        if let Some(shift) = std::env::var("NIMBRA_SHIFT")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
        {
            bounds.origin.x += to_screen(shift);
        }
        let window_bounds = match saved {
            Some((_, frame)) => {
                let restored = Bounds::new(
                    point(px(frame.x), px(frame.y)),
                    size(px(frame.width), px(frame.height)),
                );
                if frame.maximized {
                    WindowBounds::Maximized(restored)
                } else {
                    WindowBounds::Windowed(restored)
                }
            }
            None => WindowBounds::Windowed(bounds),
        };
        let main_window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(window_bounds),
                    display_id: saved.map(|(display, _)| display),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Nimbra".into()),
                        appears_transparent: true,
                        traffic_light_position: Some(point(px(18.), px(18.))),
                    }),
                    window_background: WindowBackgroundAppearance::Blurred,
                    window_min_size: Some(size(to_screen(MIN_SIZE.0), to_screen(MIN_SIZE.1))),
                    ..Default::default()
                },
                |window, cx| {
                    blur::apply(window, 40);
                    // The glass shows the desktop picture, not the windows behind.
                    wallpaper::install(window);
                    // Hovers and the pointer follow it with another app in front too.
                    blur::track_pointer_always(window);
                    // The red close button hides the window too, instead of quitting.
                    window.on_window_should_close(cx, |window, _| {
                        blur::set_visible(window, false);
                        false
                    });
                    let view = cx.new(Nimbra::new);
                    view.update(cx, |_, cx| keep_frame(window, cx));
                    window.focus(&view.read(cx).focus_handle());
                    view
                },
            )
            .expect("window should open");
        MAIN_WINDOW.with(|main| main.set(Some(main_window.into())));
        // Quit with the main window. A menu pop-up closing must not end the app.
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        // Set up notifications before anything else, so a click on one that launched the
        // app is not missed (it waits in `integration` until the app reads it).
        notify::show_while_active();
        cx.activate(true);
    });
}
