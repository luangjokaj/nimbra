//! Nimbra — a mail client concept, mocked in GPUI with static data.

mod app;
mod blur;
mod data;
mod gmail;
mod input;
mod rich;
mod theme;
mod ui;
mod webview;

use std::borrow::Cow;

use gpui::{
    App, Application, AssetSource, Bounds, KeyBinding, SharedString, TitlebarOptions,
    WindowBackgroundAppearance, WindowBounds, WindowOptions, point, prelude::*, px, size,
};

use app::{
    Back, Compose, Confirm, Dismiss, FocusSearch, Forward, MarkDone, Next, Nimbra, Prev, Quit,
    Reload, Reply, SaveDraft, SendMail, ToggleFullCompose, ToggleOriginal, TogglePalette,
    ToggleSidebar, ToggleStar, ToggleTheme, Trash, Undo,
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
    "expand",
    "shrink",
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

fn main() {
    Application::new().with_assets(Assets).run(|cx: &mut App| {
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

        input::bind_keys(cx);
        // Single-letter shortcuts belong to the mail view only, so they never fire while
        // you are typing in the composer.
        let mail = Some("Mail && !TextInput");
        cx.bind_keys([
            KeyBinding::new("/", FocusSearch, mail),
            KeyBinding::new("cmd-f", FocusSearch, Some("Mail")),
            KeyBinding::new("ctrl-f", FocusSearch, Some("Mail")),
            KeyBinding::new("c", Compose, mail),
            KeyBinding::new("r", Reply, mail),
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
            KeyBinding::new("s", ToggleStar, mail),
            KeyBinding::new("v", ToggleOriginal, mail),
            KeyBinding::new("cmd-backspace", Trash, mail),
            KeyBinding::new("ctrl-backspace", Trash, mail),
            KeyBinding::new("cmd-k", TogglePalette, None),
            KeyBinding::new("ctrl-k", TogglePalette, None),
            KeyBinding::new("h", TogglePalette, mail),
            KeyBinding::new("escape", Dismiss, None),
            KeyBinding::new("enter", Confirm, mail),
            KeyBinding::new("cmd-shift-l", ToggleTheme, None),
            KeyBinding::new("ctrl-shift-l", ToggleTheme, None),
            KeyBinding::new("cmd-b", ToggleSidebar, None),
            KeyBinding::new("ctrl-b", ToggleSidebar, None),
            KeyBinding::new("cmd-[", Back, None),
            KeyBinding::new("cmd-]", Forward, None),
            KeyBinding::new("cmd-r", Reload, None),
            KeyBinding::new("ctrl-r", Reload, None),
            KeyBinding::new("cmd-q", Quit, None),
        ]);

        let mut bounds = Bounds::centered(None, size(px(1480.), px(920.)), cx);
        // NIMBRA_SHIFT=<px> nudges the window sideways (used to test the backdrop blur).
        if let Some(shift) = std::env::var("NIMBRA_SHIFT")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
        {
            bounds.origin.x += px(shift);
        }
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Nimbra".into()),
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(18.), px(18.))),
                }),
                window_background: WindowBackgroundAppearance::Blurred,
                window_min_size: Some(size(px(1120.), px(720.))),
                ..Default::default()
            },
            |window, cx| {
                blur::apply(window, 40);
                let view = cx.new(Nimbra::new);
                window.focus(&view.read(cx).focus_handle());
                view
            },
        )
        .expect("window should open");
        cx.on_window_closed(|cx| cx.quit()).detach();
        cx.activate(true);
    });
}
