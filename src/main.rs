//! Nimbra — a mail client concept, mocked in GPUI with static data.

mod app;
mod arrival;
mod autohide;
mod blocking;
mod blur;
mod checking;
mod data;
mod discover;
mod dns;
mod editor;
mod erase;
mod gmail;
mod imap_mail;
mod input;
mod inspiration;
mod integration;
mod keyboard;
mod leaving;
mod listing;
mod mail;
mod mailto;
mod microsoft;
mod mime;
mod night;
mod notify;
mod quitting;
mod rich;
mod scheduled;
mod sending;
mod servers;
mod settings;
mod sky;
mod smtp;
mod store;
mod system;
mod theme;
mod tls;
mod toast;
mod tooltip;
mod ui;
mod webkeys;
mod webview;
mod window_frame;

use std::borrow::Cow;

use gpui::{
    App, Application, AssetSource, Bounds, KeyBinding, SharedString, TitlebarOptions,
    WindowBackgroundAppearance, WindowBounds, WindowOptions, point, prelude::*, px, size,
};

use app::{
    Back, Compose, Confirm, Dismiss, DockDrafts, DockInbox, DockSent, DockSnoozed, DockStarred,
    First, FocusSearch, FocusSidebar, Forward, ForwardMessage, GoAllMail, GoDrafts, GoInbox,
    GoNumber, GoSent, GoSnoozed, GoStarred, HideApp, HideOthers, HideWindow, Last, MarkDone,
    MarkUnread, MenuForwardMessage, MenuMarkDone, MenuMarkUnread, MenuReply, MenuReplyAll,
    MenuSearch, MenuSnooze, MenuStar, MenuToggleOriginal, MenuToggleSidebar, MenuTrash, MenuUndo,
    Minimize, Next, Nimbra, OpenInWindow, OpenSettings, OpenSidebarRow, PageDown, PageUp, PickDown,
    PickThread, PickUp, Prev, Print, Quit, Reload, Reply, ReplyAll, SaveDraft, SelectAllThreads,
    SendMail, ShowAll, ToggleFullCompose, ToggleList, ToggleOriginal, ToggleShortcuts,
    ToggleSidebar, ToggleSnooze, ToggleStar, ToggleTheme, Trash, Undo, UseDarkAppearance,
    UseLightAppearance, UseSystemAppearance,
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
    "search",
    "reply",
    "reply-all",
    "forward",
    "up",
    "down",
    "sparkles",
    "clip",
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
    "archive",
    "pulse",
    "eye",
    "plus",
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
                Cow::Borrowed(include_bytes!("../assets/fonts/Sora-Light.ttf").as_slice()),
                Cow::Borrowed(include_bytes!("../assets/fonts/Sora-Regular.ttf").as_slice()),
                Cow::Borrowed(include_bytes!("../assets/fonts/Sora-Medium.ttf").as_slice()),
                Cow::Borrowed(include_bytes!("../assets/fonts/Sora-SemiBold.ttf").as_slice()),
                Cow::Borrowed(include_bytes!("../assets/fonts/Sora-Bold.ttf").as_slice()),
                Cow::Borrowed(include_bytes!("../assets/fonts/GeistMono-Regular.ttf").as_slice()),
                Cow::Borrowed(include_bytes!("../assets/fonts/GeistMono-Medium.ttf").as_slice()),
            ])
            .expect("bundled fonts should load");

        input::bind_keys(cx);
        // Arrows and the like go to a web view that has the keyboard, not through GPUI.
        webkeys::install();
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
        // Single-letter shortcuts belong to the mail view only, so they never fire while
        // you are typing in the composer.
        let mail = Some("Mail && !TextInput");
        cx.bind_keys([
            KeyBinding::new("/", FocusSearch, mail),
            KeyBinding::new("cmd-f", FocusSearch, Some("Mail")),
            KeyBinding::new("ctrl-f", FocusSearch, Some("Mail")),
            KeyBinding::new("c", Compose, mail),
            KeyBinding::new("r", Reply, mail),
            KeyBinding::new("a", ReplyAll, mail),
            KeyBinding::new("f", ForwardMessage, mail),
            KeyBinding::new("cmd-enter", SendMail, Some("Composer")),
            KeyBinding::new("cmd-s", SaveDraft, Some("Composer")),
            KeyBinding::new("ctrl-s", SaveDraft, Some("Composer")),
            KeyBinding::new("cmd-shift-f", ToggleFullCompose, Some("Composer")),
            KeyBinding::new("ctrl-shift-f", ToggleFullCompose, Some("Composer")),
            KeyBinding::new("ctrl-enter", SendMail, Some("Composer")),
            KeyBinding::new("j", Next, mail),
            KeyBinding::new("down", Next, mail),
            KeyBinding::new("k", Prev, mail),
            KeyBinding::new("up", Prev, mail),
            KeyBinding::new("e", MarkDone, mail),
            KeyBinding::new("z", Undo, mail),
            KeyBinding::new("cmd-z", Undo, Some("Mail")),
            // Copies text selected in the reader.
            KeyBinding::new("cmd-c", input::Copy, Some("Mail")),
            KeyBinding::new("ctrl-c", input::Copy, Some("Mail")),
            KeyBinding::new("ctrl-z", Undo, Some("Mail")),
            KeyBinding::new("s", ToggleStar, mail),
            KeyBinding::new("u", MarkUnread, mail),
            KeyBinding::new("v", ToggleOriginal, mail),
            KeyBinding::new("cmd-backspace", Trash, mail),
            // Backspace or Delete on its own moves the open (or picked) thread to the trash too.
            KeyBinding::new("backspace", Trash, mail),
            KeyBinding::new("delete", Trash, mail),
            KeyBinding::new("ctrl-backspace", Trash, mail),
            KeyBinding::new("cmd-k", ToggleSnooze, None),
            KeyBinding::new("ctrl-k", ToggleSnooze, None),
            KeyBinding::new("h", ToggleSnooze, mail),
            KeyBinding::new("escape", Dismiss, None),
            KeyBinding::new("enter", Confirm, mail),
            KeyBinding::new("cmd-shift-l", ToggleTheme, None),
            KeyBinding::new("ctrl-shift-l", ToggleTheme, None),
            KeyBinding::new("cmd-b", ToggleSidebar, None),
            KeyBinding::new("cmd-shift-b", ToggleList, None),
            KeyBinding::new("ctrl-shift-b", ToggleList, None),
            KeyBinding::new("ctrl-b", ToggleSidebar, None),
            KeyBinding::new("cmd-[", Back, None),
            KeyBinding::new("cmd-]", Forward, None),
            KeyBinding::new("cmd-r", Reload, None),
            KeyBinding::new("ctrl-r", Reload, None),
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-h", HideApp, None),
            KeyBinding::new("alt-cmd-h", HideOthers, None),
            KeyBinding::new("cmd-m", Minimize, None),
            KeyBinding::new("cmd-,", OpenSettings, None),
            KeyBinding::new("cmd-n", Compose, None),
            KeyBinding::new("cmd-p", Print, None),
            // Walking the window: ← to the sidebar, → back with what is under the cursor.
            KeyBinding::new("left", FocusSidebar, mail),
            KeyBinding::new(
                "right",
                OpenSidebarRow,
                Some("Mail && Sidebar && !TextInput"),
            ),
            KeyBinding::new("home", First, mail),
            KeyBinding::new("end", Last, mail),
            KeyBinding::new("cmd-up", First, mail),
            KeyBinding::new("cmd-down", Last, mail),
            KeyBinding::new("x", PickThread, mail),
            KeyBinding::new("shift-down", PickDown, mail),
            KeyBinding::new("shift-j", PickDown, mail),
            KeyBinding::new("shift-up", PickUp, mail),
            KeyBinding::new("shift-k", PickUp, mail),
            KeyBinding::new("o", OpenInWindow, mail),
            KeyBinding::new("space", PageDown, mail),
            KeyBinding::new("shift-space", PageUp, mail),
            KeyBinding::new("?", ToggleShortcuts, mail),
            KeyBinding::new("shift-/", ToggleShortcuts, mail),
            // Gmail's: G, then where.
            KeyBinding::new("g i", GoInbox, mail),
            KeyBinding::new("g s", GoStarred, mail),
            KeyBinding::new("g b", GoSnoozed, mail),
            KeyBinding::new("g t", GoSent, mail),
            KeyBinding::new("g d", GoDrafts, mail),
            KeyBinding::new("g a", GoAllMail, mail),
            KeyBinding::new("cmd-a", SelectAllThreads, mail),
            KeyBinding::new("ctrl-a", SelectAllThreads, mail),
            KeyBinding::new("cmd-1", GoNumber(1), Some("Mail")),
            KeyBinding::new("cmd-2", GoNumber(2), Some("Mail")),
            KeyBinding::new("cmd-3", GoNumber(3), Some("Mail")),
            KeyBinding::new("cmd-4", GoNumber(4), Some("Mail")),
            KeyBinding::new("cmd-5", GoNumber(5), Some("Mail")),
            KeyBinding::new("cmd-6", GoNumber(6), Some("Mail")),
            KeyBinding::new("cmd-7", GoNumber(7), Some("Mail")),
            KeyBinding::new("cmd-8", GoNumber(8), Some("Mail")),
            KeyBinding::new("cmd-9", GoNumber(9), Some("Mail")),
            // ⌘W hides the window; Nimbra keeps running and checking mail until ⌘Q.
            KeyBinding::new("cmd-w", HideWindow, None),
            // While writing in the message editor these keys format text (bold, link) or
            // belong to the editor (Escape closes its link box), so the app leaves them.
            KeyBinding::new("cmd-b", gpui::NoAction, Some("Editor")),
            KeyBinding::new("ctrl-b", gpui::NoAction, Some("Editor")),
            KeyBinding::new("cmd-k", gpui::NoAction, Some("Editor")),
            KeyBinding::new("ctrl-k", gpui::NoAction, Some("Editor")),
            KeyBinding::new("escape", gpui::NoAction, Some("Editor")),
            // The editor page reports these itself (see editor.html), so they run once.
            KeyBinding::new("cmd-enter", gpui::NoAction, Some("Editor")),
            KeyBinding::new("ctrl-enter", gpui::NoAction, Some("Editor")),
            KeyBinding::new("cmd-s", gpui::NoAction, Some("Editor")),
            KeyBinding::new("ctrl-s", gpui::NoAction, Some("Editor")),
            KeyBinding::new("cmd-shift-f", gpui::NoAction, Some("Editor")),
            KeyBinding::new("ctrl-shift-f", gpui::NoAction, Some("Editor")),
        ]);
        // The Edit menu gives ⌘X, ⌘C, ⌘V and ⌘A to whichever view has the keyboard,
        // including the message editor (a web view only gets them through the menu).
        // The menu bar. Items only carry shortcuts that use ⌘ and never clash with the
        // message editor; the rest (E, R, S…) are menu-only twins without a shortcut, so a
        // letter typed in the editor never triggers them. See `MenuReply` and friends.
        use gpui::{Menu, MenuItem, OsAction};
        cx.set_menus(vec![
            Menu {
                name: "Nimbra".into(),
                items: vec![
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
                    MenuItem::action("New Message", Compose),
                    MenuItem::action("Check for Mail", Reload),
                    MenuItem::separator(),
                    MenuItem::action("Print…", Print),
                    MenuItem::separator(),
                    MenuItem::action("Close Window", HideWindow),
                ],
            },
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
                    MenuItem::action("Show or Hide List", ToggleList),
                    MenuItem::action("Original Formatting or Reader", MenuToggleOriginal),
                    MenuItem::separator(),
                    // Checked by `set_menu_check`, which finds it by these titles.
                    MenuItem::submenu(Menu {
                        name: "Appearance".into(),
                        items: vec![
                            MenuItem::action("System", UseSystemAppearance),
                            MenuItem::action("Light", UseLightAppearance),
                            MenuItem::action("Dark", UseDarkAppearance),
                            MenuItem::separator(),
                            MenuItem::action("Switch Light and Dark", ToggleTheme),
                        ],
                    }),
                ],
            },
            Menu {
                name: "Message".into(),
                items: vec![
                    MenuItem::action("Reply  (R)", MenuReply),
                    MenuItem::action("Reply All  (A)", MenuReplyAll),
                    MenuItem::action("Forward  (F)", MenuForwardMessage),
                    MenuItem::separator(),
                    MenuItem::action("Mark Done  (E)", MenuMarkDone),
                    MenuItem::action("Mark Read or Unread  (U)", MenuMarkUnread),
                    MenuItem::action("Star  (S)", MenuStar),
                    MenuItem::action("Snooze…  (H)", MenuSnooze),
                    MenuItem::action("Move to Trash", MenuTrash),
                    MenuItem::separator(),
                    MenuItem::action("Undo  (Z)", MenuUndo),
                ],
            },
            Menu {
                name: "Window".into(),
                items: vec![
                    MenuItem::action("Minimize", Minimize),
                    MenuItem::action("Close Window", HideWindow),
                ],
            },
        ]);

        cx.set_dock_menu(vec![
            MenuItem::action("New Message", Compose),
            MenuItem::separator(),
            MenuItem::action("Inbox", DockInbox),
            MenuItem::action("Starred", DockStarred),
            MenuItem::action("Snoozed", DockSnoozed),
            MenuItem::action("Drafts", DockDrafts),
            MenuItem::action("Sent", DockSent),
        ]);

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
        let mut bounds = Bounds::centered(None, size(px(1480.), px(920.)), cx);
        // NIMBRA_SHIFT=<px> nudges the window sideways (used to test the backdrop blur).
        if let Some(shift) = std::env::var("NIMBRA_SHIFT")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
        {
            bounds.origin.x += px(shift);
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
                        traffic_light_position: Some(ui::traffic_lights()),
                    }),
                    window_background: WindowBackgroundAppearance::Blurred,
                    window_min_size: Some(size(px(MIN_SIZE.0), px(MIN_SIZE.1))),
                    ..Default::default()
                },
                |window, cx| {
                    blur::apply(window, 40);
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
