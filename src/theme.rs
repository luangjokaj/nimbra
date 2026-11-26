use gpui::{Hsla, Rgba, rgb};

/// Hex colour with alpha.
pub fn c(hex: u32, a: f32) -> Hsla {
    Rgba { a, ..rgb(hex) }.into()
}

/// `top` laid over `bottom`, both possibly see-through, as one colour.
pub fn over(top: Hsla, bottom: Hsla) -> Hsla {
    let (t, b) = (Rgba::from(top), Rgba::from(bottom));
    let a = t.a + b.a * (1. - t.a);
    if a <= 0. {
        return top;
    }
    let mix = |t_c: f32, b_c: f32| (t_c * t.a + b_c * b.a * (1. - t.a)) / a;
    Rgba {
        r: mix(t.r, b.r),
        g: mix(t.g, b.g),
        b: mix(t.b, b.b),
        a,
    }
    .into()
}

/// Spacing, on a 4pt grid.
pub mod space {
    pub const XS: f32 = 4.;
    pub const S: f32 = 8.;
    pub const M: f32 = 12.;
    pub const L: f32 = 16.;
    pub const L2: f32 = 20.;
    pub const XL: f32 = 24.;
    pub const XXL: f32 = 32.;
    pub const XXXL: f32 = 40.;
}

/// Corner radii, by what is rounded.
pub mod radius {
    /// Key chips, count pills, snooze chips, a segmented control's inner pill.
    pub const CHIP: f32 = 6.;
    /// Buttons, icon buttons, nav rows, menu items, tooltips, the search field.
    pub const CONTROL: f32 = 8.;
    /// List and finder rows, inputs.
    pub const ROW: f32 = 10.;
    /// Cards, menus, toasts, settings groups.
    pub const CARD: f32 = 14.;
    /// Finder, confirm, shortcuts, composer, the floating sidebar.
    pub const SHEET: f32 = 18.;
}

#[derive(Clone, Copy)]
pub struct Theme {
    pub dark: bool,
    /// The window's ground, around and between the panels: one solid colour.
    pub ground: Hsla,
    pub sidebar: Hsla,
    /// The message list's panel: a step back from the reader.
    pub panel: Hsla,
    /// The reader's panel, the frontmost.
    pub surface: Hsla,
    /// The hairline around each floating panel.
    pub rim: Hsla,
    pub raised: Hsla,
    pub hover: Hsla,
    /// What is picked or open: a tint of the accent, with `selected_edge` around it.
    pub selected: Hsla,
    pub selected_edge: Hsla,
    /// What is held under the pointer: a deeper tint of the accent than selection.
    pub pressed: Hsla,
    /// The keyboard focus ring: solid accent.
    pub focus: Hsla,
    /// The hairline around raised surfaces (tooltips, menus, sheets, cards).
    pub edge: Hsla,
    /// The open mailbox's pill in the sidebar, and how opaque it comes out: less than the
    /// sidebar around it, so that more of the blurred desktop shows through.
    pub glass: Hsla,
    pub glass_opacity: f32,
    pub border: Hsla,
    pub border_strong: Hsla,
    pub text: Hsla,
    pub text2: Hsla,
    pub text3: Hsla,
    /// Disabled icons and hairline glyphs only: too light for text in light mode.
    pub faint: Hsla,
    pub violet: Hsla,
    /// The primary button's fill at rest, hovered and held: solid, under a white label.
    pub primary: Hsla,
    pub primary_hover: Hsla,
    pub primary_pressed: Hsla,
    /// A destructive button's fill, under a white label.
    pub danger: Hsla,
    pub rose: Hsla,
    pub amber: Hsla,
    /// Amber that text can be set in (light amber is for icons and dots).
    pub amber_text: Hsla,
    pub mint: Hsla,
    pub sky: Hsla,
    pub kbd_bg: Hsla,
    pub kbd_border: Hsla,
    /// The `Elevation::Pop` shadow: menus, tooltips, toasts.
    pub shadow: Hsla,
    /// The `Elevation::Sheet` shadow: finder, confirm, shortcuts, composer.
    pub shadow_sheet: Hsla,
    pub scrim: Hsla,
}

impl Theme {
    pub fn dark() -> Self {
        Self {
            dark: true,
            ground: c(0x0a0a0e, 1.),
            sidebar: c(0x121218, 0.86),
            panel: c(0x111116, 1.),
            surface: c(0x16161c, 1.),
            rim: c(0xffffff, 0.08),
            raised: c(0x1c1c23, 1.),
            hover: c(0xffffff, 0.045),
            selected: c(0x8b7cff, 0.14),
            selected_edge: c(0x8b7cff, 0.34),
            pressed: c(0x8b7cff, 0.22),
            focus: c(0x8b7cff, 1.),
            edge: c(0xffffff, 0.1),
            glass: c(0xffffff, 0.1),
            glass_opacity: 0.75,
            border: c(0xffffff, 0.07),
            border_strong: c(0xffffff, 0.13),
            text: c(0xf2f2f5, 1.),
            text2: c(0xc4c4cc, 1.),
            text3: c(0x91919b, 1.),
            faint: c(0x80808a, 1.),
            violet: c(0x8b7cff, 1.),
            primary: c(0x6450ea, 1.),
            primary_hover: c(0x6e5cf6, 1.),
            primary_pressed: c(0x5a46d8, 1.),
            danger: c(0xc23a6e, 1.),
            rose: c(0xff7ab6, 1.),
            amber: c(0xffb86b, 1.),
            amber_text: c(0xffb86b, 1.),
            mint: c(0x5ee6b0, 1.),
            sky: c(0x6cc4ff, 1.),
            kbd_bg: c(0xffffff, 0.06),
            kbd_border: c(0xffffff, 0.10),
            shadow: c(0x000000, 0.45),
            shadow_sheet: c(0x000000, 0.55),
            scrim: c(0x050507, 0.66),
        }
    }

    pub fn light() -> Self {
        Self {
            dark: false,
            ground: c(0xe9e9ee, 1.),
            sidebar: c(0xf7f7f9, 0.86),
            panel: c(0xf7f7f9, 1.),
            surface: c(0xffffff, 1.),
            rim: c(0x000000, 0.08),
            raised: c(0xffffff, 1.),
            hover: c(0x000000, 0.035),
            selected: c(0x6e5cf6, 0.09),
            selected_edge: c(0x6e5cf6, 0.3),
            pressed: c(0x6e5cf6, 0.15),
            focus: c(0x6e5cf6, 1.),
            edge: c(0x000000, 0.09),
            glass: c(0x000000, 0.06),
            glass_opacity: 0.78,
            border: c(0x000000, 0.07),
            border_strong: c(0x000000, 0.12),
            text: c(0x16161b, 1.),
            text2: c(0x44444d, 1.),
            text3: c(0x696973, 1.),
            faint: c(0x8a8a94, 1.),
            violet: c(0x6e5cf6, 1.),
            primary: c(0x6450ea, 1.),
            primary_hover: c(0x6e5cf6, 1.),
            primary_pressed: c(0x5a46d8, 1.),
            danger: c(0xc23a6e, 1.),
            rose: c(0xc93a76, 1.),
            amber: c(0xe58a2e, 1.),
            amber_text: c(0x9e540b, 1.),
            mint: c(0x13865f, 1.),
            sky: c(0x1c74b8, 1.),
            kbd_bg: c(0x000000, 0.04),
            kbd_border: c(0x000000, 0.09),
            shadow: c(0x000000, 0.12),
            shadow_sheet: c(0x000000, 0.16),
            scrim: c(0x101014, 0.2),
        }
    }

    pub fn accent(&self, a: Accent) -> Hsla {
        match a {
            Accent::Violet => self.violet,
            Accent::Rose => self.rose,
            Accent::Amber => self.amber,
            Accent::Mint => self.mint,
            Accent::Sky => self.sky,
        }
    }
}

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub enum Accent {
    Violet,
    Rose,
    Amber,
    Mint,
    Sky,
}

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub enum Tone {
    Violet,
    Rose,
    Amber,
    Mint,
    Sky,
    Graphite,
}

impl Tone {
    /// An avatar's solid fill: muted, and dark enough for white initials.
    pub fn fill(self) -> Hsla {
        c(
            match self {
                Tone::Violet => 0x6457d6,
                Tone::Rose => 0xa9446e,
                Tone::Amber => 0x9a5f1f,
                Tone::Mint => 0x23805f,
                Tone::Sky => 0x3a6db0,
                Tone::Graphite => 0x55535f,
            },
            1.,
        )
    }
}

#[cfg(test)]
const TONES: [Tone; 6] = [
    Tone::Violet,
    Tone::Rose,
    Tone::Amber,
    Tone::Mint,
    Tone::Sky,
    Tone::Graphite,
];

pub const SANS: &str = "Sora";
pub const MONO: &str = "Geist Mono";

#[cfg(test)]
mod tests {
    use super::*;

    fn luminance(color: Hsla) -> f32 {
        let rgba = Rgba::from(color);
        let linear = |v: f32| {
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(rgba.r) + 0.7152 * linear(rgba.g) + 0.0722 * linear(rgba.b)
    }

    fn contrast(text: Hsla, ground: Hsla) -> f32 {
        let (a, b) = (luminance(text), luminance(ground));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    /// The sidebar's colour without the desktop showing through it.
    fn sidebar(t: &Theme) -> Hsla {
        Hsla { a: 1., ..t.sidebar }
    }

    const TEXT: f32 = 4.5;
    const NON_TEXT: f32 = 3.;

    #[test]
    fn light_text_tokens_are_legible_on_every_ground() {
        let t = Theme::light();
        for ground in [t.surface, t.panel, t.raised, sidebar(&t)] {
            for text in [t.text, t.text2, t.text3] {
                assert!(contrast(text, ground) >= TEXT, "{}", contrast(text, ground));
            }
        }
    }

    #[test]
    fn dark_text_tokens_are_legible_on_every_ground() {
        let t = Theme::dark();
        for ground in [t.surface, t.panel, t.raised, sidebar(&t)] {
            for text in [t.text, t.text2, t.text3] {
                assert!(contrast(text, ground) >= TEXT, "{}", contrast(text, ground));
            }
        }
    }

    #[test]
    fn light_accents_can_carry_text_on_white() {
        let t = Theme::light();
        for accent in [t.violet, t.rose, t.amber_text, t.mint, t.sky] {
            assert!(
                contrast(accent, t.surface) >= TEXT,
                "{}",
                contrast(accent, t.surface)
            );
        }
    }

    #[test]
    fn light_amber_text_is_legible_on_its_chip() {
        let t = Theme::light();
        let chip = over(Hsla { a: 0.14, ..t.amber }, t.surface);
        assert!(
            contrast(t.amber_text, chip) >= TEXT,
            "{}",
            contrast(t.amber_text, chip)
        );
    }

    #[test]
    fn dark_accents_can_carry_text() {
        let t = Theme::dark();
        for accent in [t.violet, t.rose, t.amber_text, t.mint, t.sky] {
            assert!(
                contrast(accent, t.surface) >= TEXT,
                "{}",
                contrast(accent, t.surface)
            );
        }
    }

    #[test]
    fn faint_still_reads_as_a_glyph() {
        let (light, dark) = (Theme::light(), Theme::dark());
        assert!(contrast(light.faint, light.surface) >= NON_TEXT);
        assert!(contrast(dark.faint, dark.surface) >= TEXT);
        assert!(contrast(dark.faint, dark.raised) >= 4.);
    }

    #[test]
    fn white_label_is_legible_on_every_solid_button() {
        for t in [Theme::light(), Theme::dark()] {
            for fill in [t.primary, t.primary_hover, t.primary_pressed, t.danger] {
                let ratio = contrast(gpui::white(), fill);
                assert!(ratio >= TEXT, "{ratio}");
            }
        }
    }

    #[test]
    fn white_initials_are_legible_on_every_avatar() {
        for tone in TONES {
            let ratio = contrast(gpui::white(), tone.fill());
            assert!(ratio >= TEXT, "{ratio}");
        }
    }

    #[test]
    fn text_is_legible_on_the_selection_tint() {
        for t in [Theme::light(), Theme::dark()] {
            let picked = over(t.selected, t.panel);
            for text in [t.text, t.text2, t.text3] {
                assert!(contrast(text, picked) >= TEXT, "{}", contrast(text, picked));
            }
        }
    }

    #[test]
    fn the_focus_ring_stands_out_from_every_ground() {
        for t in [Theme::light(), Theme::dark()] {
            for ground in [t.surface, t.panel, t.raised, t.ground] {
                assert!(contrast(t.focus, ground) >= NON_TEXT, "{}", contrast(t.focus, ground));
            }
        }
    }

    #[test]
    fn pressed_differs_from_hover_and_selected() {
        for t in [Theme::light(), Theme::dark()] {
            assert_ne!(t.pressed, t.hover);
            assert_ne!(t.pressed, t.selected);
        }
    }

    #[test]
    fn over_an_opaque_ground_is_opaque() {
        let mixed = over(c(0x000000, 0.5), c(0xffffff, 1.));
        assert!((mixed.a - 1.).abs() < 1e-4);
        assert!((luminance(mixed) - luminance(c(0x808080, 1.))).abs() < 0.02);
    }
}
