use gpui::{Background, Hsla, Rgba, linear_color_stop, linear_gradient, rgb};

/// Hex colour with alpha.
pub fn c(hex: u32, a: f32) -> Hsla {
    Rgba { a, ..rgb(hex) }.into()
}

pub fn gradient(angle: f32, from: Hsla, to: Hsla) -> Background {
    linear_gradient(
        angle,
        linear_color_stop(from, 0.),
        linear_color_stop(to, 1.),
    )
}

/// A colour that fades to transparent part-way along the gradient line.
pub fn fade(angle: f32, hex: u32, alpha: f32, end: f32) -> Background {
    linear_gradient(
        angle,
        linear_color_stop(c(hex, alpha), 0.),
        linear_color_stop(c(hex, 0.), end),
    )
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

/// A two-stop gradient by its end colours.
#[derive(Clone, Copy)]
pub struct Ramp {
    pub from: u32,
    pub to: u32,
}

impl Ramp {
    pub fn along(self, angle: f32) -> Background {
        gradient(angle, c(self.from, 1.), c(self.to, 1.))
    }
}

/// The single primary action per surface. Dark enough at both ends for a white label.
pub const GRADIENT_PRIMARY: Ramp = Ramp {
    from: 0x6e5cf6,
    to: 0xc93a76,
};
pub const GRADIENT_PRIMARY_PRESSED: Ramp = Ramp {
    from: 0x6350e8,
    to: 0xb9336b,
};
/// Light rather than surface: the primary button's glow, the Brief frame, streaks, Inbox
/// zero. Never under text.
pub const GRADIENT_GLOW: Ramp = Ramp {
    from: 0x8b7cff,
    to: 0xff7ab6,
};

/// Spacing, on a 4pt grid.
pub mod space {
    pub const XS: f32 = 4.;
    pub const S: f32 = 8.;
    pub const M: f32 = 12.;
    pub const L: f32 = 16.;
    pub const XL: f32 = 24.;
    pub const XXL: f32 = 32.;
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
    pub sidebar: Hsla,
    pub surface: Hsla,
    pub raised: Hsla,
    pub hover: Hsla,
    pub selected: Hsla,
    /// What is held under the pointer: a deeper wash of the accent than selection.
    pub pressed: Hsla,
    pub focus: Hsla,
    /// Soft accent light around what has focus or is the primary action.
    pub glow: Hsla,
    /// The luminous hairline around raised glass (tooltips, menus, sheets, cards).
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
            sidebar: c(0x101017, 0.88),
            surface: c(0x0e0e14, 1.),
            raised: c(0x17171f, 1.),
            hover: c(0xffffff, 0.045),
            selected: c(0x8b7cff, 0.16),
            pressed: c(0x8b7cff, 0.24),
            focus: c(0x8b7cff, 0.6),
            glow: c(0x8b7cff, 0.35),
            edge: c(0xc9c2ff, 0.14),
            glass: c(0xffffff, 0.12),
            glass_opacity: 0.75,
            border: c(0xffffff, 0.07),
            border_strong: c(0xffffff, 0.13),
            text: c(0xf5f3ef, 1.),
            text2: c(0xc6c4ce, 1.),
            text3: c(0x918f9a, 1.),
            faint: c(0x7d7b88, 1.),
            violet: c(0x8b7cff, 1.),
            rose: c(0xff7ab6, 1.),
            amber: c(0xffb86b, 1.),
            amber_text: c(0xffb86b, 1.),
            mint: c(0x5ee6b0, 1.),
            sky: c(0x6cc4ff, 1.),
            kbd_bg: c(0xffffff, 0.06),
            kbd_border: c(0xffffff, 0.10),
            shadow: c(0x000000, 0.5),
            shadow_sheet: c(0x000000, 0.6),
            scrim: c(0x050508, 0.66),
        }
    }

    pub fn light() -> Self {
        Self {
            dark: false,
            sidebar: c(0xf4f2ee, 0.88),
            surface: c(0xffffff, 1.),
            raised: c(0xffffff, 1.),
            hover: c(0x000000, 0.035),
            selected: c(0x6e5cf6, 0.10),
            pressed: c(0x6e5cf6, 0.16),
            focus: c(0x6e5cf6, 0.6),
            glow: c(0x6e5cf6, 0.22),
            edge: c(0x6e5cf6, 0.16),
            glass: c(0x000000, 0.07),
            glass_opacity: 0.78,
            border: c(0x000000, 0.07),
            border_strong: c(0x000000, 0.12),
            text: c(0x17161c, 1.),
            text2: c(0x45434d, 1.),
            text3: c(0x6b6974, 1.),
            faint: c(0x8a8893, 1.),
            violet: c(0x6e5cf6, 1.),
            rose: c(0xc93a76, 1.),
            amber: c(0xe58a2e, 1.),
            amber_text: c(0x9e540b, 1.),
            mint: c(0x13865f, 1.),
            sky: c(0x1c74b8, 1.),
            kbd_bg: c(0x000000, 0.04),
            kbd_border: c(0x000000, 0.09),
            shadow: c(0x1a1233, 0.16),
            shadow_sheet: c(0x1a1233, 0.18),
            scrim: c(0x1a1233, 0.18),
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
    pub fn gradient(self) -> Background {
        let (a, b) = match self {
            Tone::Violet => (0x9b8cff, 0x5b47e0),
            Tone::Rose => (0xff9cc8, 0xe0457f),
            Tone::Amber => (0xffd08a, 0xf07d2e),
            Tone::Mint => (0x8bf0c7, 0x16a979),
            Tone::Sky => (0x9dd8ff, 0x2f7fe0),
            Tone::Graphite => (0x6d6c78, 0x2a2a33),
        };
        gradient(135., c(a, 1.), c(b, 1.))
    }
}

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
        for ground in [t.surface, t.raised, sidebar(&t)] {
            for text in [t.text, t.text2, t.text3] {
                assert!(contrast(text, ground) >= TEXT, "{}", contrast(text, ground));
            }
        }
    }

    #[test]
    fn dark_text_tokens_are_legible_on_every_ground() {
        let t = Theme::dark();
        for ground in [t.surface, t.raised, sidebar(&t)] {
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
    fn white_label_is_legible_across_the_primary_gradient() {
        for ramp in [GRADIENT_PRIMARY, GRADIENT_PRIMARY_PRESSED] {
            for end in [ramp.from, ramp.to] {
                let ratio = contrast(gpui::white(), c(end, 1.));
                assert!(ratio >= TEXT, "{end:06x}: {ratio}");
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
