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

#[derive(Clone, Copy)]
pub struct Theme {
    pub dark: bool,
    pub sidebar: Hsla,
    pub surface: Hsla,
    pub raised: Hsla,
    pub hover: Hsla,
    pub selected: Hsla,
    pub border: Hsla,
    pub border_strong: Hsla,
    pub text: Hsla,
    pub text2: Hsla,
    pub text3: Hsla,
    pub faint: Hsla,
    pub violet: Hsla,
    pub rose: Hsla,
    pub amber: Hsla,
    pub mint: Hsla,
    pub sky: Hsla,
    pub kbd_bg: Hsla,
    pub kbd_border: Hsla,
    pub shadow: Hsla,
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
            selected: c(0x8b7cff, 0.13),
            border: c(0xffffff, 0.07),
            border_strong: c(0xffffff, 0.13),
            text: c(0xf5f3ef, 1.),
            text2: c(0xc6c4ce, 1.),
            text3: c(0x918f9a, 1.),
            faint: c(0x62616d, 1.),
            violet: c(0x8b7cff, 1.),
            rose: c(0xff7ab6, 1.),
            amber: c(0xffb86b, 1.),
            mint: c(0x5ee6b0, 1.),
            sky: c(0x6cc4ff, 1.),
            kbd_bg: c(0xffffff, 0.06),
            kbd_border: c(0xffffff, 0.10),
            shadow: c(0x000000, 0.45),
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
            selected: c(0x6e5cf6, 0.09),
            border: c(0x000000, 0.07),
            border_strong: c(0x000000, 0.12),
            text: c(0x17161c, 1.),
            text2: c(0x45434d, 1.),
            text3: c(0x75737e, 1.),
            faint: c(0xa5a3ad, 1.),
            violet: c(0x6e5cf6, 1.),
            rose: c(0xe4528f, 1.),
            amber: c(0xe58a2e, 1.),
            mint: c(0x1faf7c, 1.),
            sky: c(0x2b93e0, 1.),
            kbd_bg: c(0x000000, 0.04),
            kbd_border: c(0x000000, 0.09),
            shadow: c(0x1a1233, 0.14),
            scrim: c(0xe7e3dd, 0.6),
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
    #[allow(dead_code)]
    Amber,
    #[allow(dead_code)]
    Mint,
    Sky,
}

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub enum Tone {
    Violet,
    Rose,
    #[allow(dead_code)]
    Amber,
    #[allow(dead_code)]
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

pub const SANS: &str = "Geist";
pub const MONO: &str = "Geist Mono";
pub const SERIF: &str = "Instrument Serif";
