//! The night: Nimbra's one material, kept for the screens that hold no mail (Inbox Zero,
//! signing in, About). A dusk horizon rendered offline by `scripts/night.py`, stretched to
//! its place, with film grain laid over it one texel to a device pixel so that it never
//! bands. It is still: nothing here moves or asks for frames.

use std::sync::{Arc, LazyLock};

use gpui::{
    App, Asset, Bounds, Corners, Div, Hsla, Image, ImageFormat, IntoElement, ObjectFit,
    ParentElement, Pixels, RenderImage, Styled, StyledImage, Window, canvas, div, img, point, px,
    relative, size,
};

use crate::theme::{Theme, c};

/// Where the horizon is, as a share of the night's height. `scripts/night.py` draws it there.
pub const HORIZON: f32 = 0.72;

/// The grain tile's side in texels.
pub const GRAIN_TEXELS: f32 = 256.;

fn png(bytes: &'static [u8]) -> Arc<Image> {
    Arc::new(Image::from_bytes(ImageFormat::Png, bytes.to_vec()))
}

static FIELD_DARK: LazyLock<Arc<Image>> =
    LazyLock::new(|| png(include_bytes!("../assets/night/night-dark.png")));
static FIELD_LIGHT: LazyLock<Arc<Image>> =
    LazyLock::new(|| png(include_bytes!("../assets/night/night-light.png")));
static GRAIN: LazyLock<Arc<Image>> =
    LazyLock::new(|| png(include_bytes!("../assets/night/grain.png")));

/// The night's colours for what is drawn on it.
pub struct Night {
    /// Under the field, the same as its top: what shows before it is decoded.
    pub ground: Hsla,
    pub star: Hsla,
    pub ink: Hsla,
    pub soft: Hsla,
    pub card: Hsla,
    pub edge: Hsla,
    /// How strongly the grain is laid on.
    grain: f32,
}

impl Night {
    pub fn of(t: &Theme) -> Self {
        if t.dark {
            Night {
                ground: c(0x07070c, 1.),
                star: c(0xffffff, 1.),
                ink: t.text,
                soft: c(0xffffff, 0.64),
                card: c(0xffffff, 0.04),
                edge: c(0xffffff, 0.12),
                grain: 0.7,
            }
        } else {
            Night {
                ground: c(0xf6f6f9, 1.),
                star: c(0x2b2a3a, 0.55),
                ink: t.text,
                soft: t.text2,
                card: c(0xffffff, 0.72),
                edge: c(0x000000, 0.08),
                grain: 0.5,
            }
        }
    }
}

/// The images decoded once, on the background executor, and kept.
enum Decoded {}

impl Asset for Decoded {
    type Source = Arc<Image>;
    type Output = Option<Arc<RenderImage>>;

    fn load(
        source: Self::Source,
        cx: &mut App,
    ) -> impl Future<Output = Self::Output> + Send + 'static {
        let renderer = cx.svg_renderer();
        async move { source.to_image_data(renderer).ok() }
    }
}

/// The image decoded, or `None` while that runs on the background executor; the view being
/// drawn is drawn again once it is done.
fn ready(image: &Arc<Image>, window: &mut Window, cx: &mut App) -> Option<Arc<RenderImage>> {
    window.use_asset::<Decoded>(image, cx).flatten()
}

fn field(t: &Theme) -> &'static Arc<Image> {
    if t.dark { &FIELD_DARK } else { &FIELD_LIGHT }
}

/// Keeps the night of the theme shown decoded, so that a screen with it never shows it a
/// frame late. Cheap after the first call: a lookup.
pub fn keep_ready(t: &Theme, window: &mut Window, cx: &mut App) {
    ready(field(t), window, cx);
    ready(&GRAIN, window, cx);
}

/// The night as a layer filling its parent (which is `relative`), rounded to `corner` points.
/// Until it is decoded, its ground colour alone.
pub fn layer(t: &Theme, corner: f32, window: &mut Window, cx: &mut App) -> Div {
    let night = Night::of(t);
    let field = ready(field(t), window, cx);
    let grain = ready(&GRAIN, window, cx);
    div()
        .absolute()
        .inset_0()
        .rounded(px(corner))
        .bg(night.ground)
        .children(field.map(|field| {
            img(field)
                .absolute()
                .inset_0()
                .size_full()
                .rounded(px(corner))
                .object_fit(ObjectFit::Fill)
        }))
        .children(grain.map(|tile| {
            div()
                .absolute()
                .inset_0()
                .opacity(night.grain)
                .child(grained(tile, corner, window.scale_factor()))
        }))
}

/// Film grain over the whole of its bounds, the tile repeated at one texel per device pixel.
fn grained(tile: Arc<RenderImage>, corner: f32, scale: f32) -> impl IntoElement {
    let side = GRAIN_TEXELS / scale;
    canvas(
        |_, _, _| {},
        move |bounds: Bounds<Pixels>, _, window, _| {
            let area = (f32::from(bounds.size.width), f32::from(bounds.size.height));
            for (x, y, corners) in tiles(area, side, corner) {
                let place = Bounds::new(
                    bounds.origin + point(px(x), px(y)),
                    size(px(side), px(side)),
                );
                window
                    .paint_image(place, corners.map(|r| px(*r)), tile.clone(), 0, false)
                    .ok();
            }
        },
    )
    .size_full()
}

/// Where each grain tile goes in an area, and how its corners are rounded: a tile on a
/// corner of the area takes that corner's rounding. The area is then a whole number of
/// tiles, or its rounding is too small for a cut tile to show past it.
fn tiles((width, height): (f32, f32), side: f32, corner: f32) -> Vec<(f32, f32, Corners<f32>)> {
    let (columns, rows) = (
        (width / side).ceil() as usize,
        (height / side).ceil() as usize,
    );
    let mut placed = Vec::with_capacity(columns * rows);
    for row in 0..rows {
        for column in 0..columns {
            let (x, y) = (column as f32 * side, row as f32 * side);
            let (left, top) = (column == 0, row == 0);
            let (right, bottom) = (x + side >= width, y + side >= height);
            let round = |on: bool| if on { corner } else { 0. };
            placed.push((
                x,
                y,
                Corners {
                    top_left: round(left && top),
                    top_right: round(right && top),
                    bottom_left: round(left && bottom),
                    bottom_right: round(right && bottom),
                },
            ));
        }
    }
    placed
}

/// The stars over the night, `count` of a fixed field (so every night is the same), each as
/// bright as `shown` says for its index.
pub fn stars(night: &Night, count: usize, shown: impl Fn(usize) -> f32) -> Vec<Div> {
    let mut seed = 7u32;
    let mut rnd = move || {
        seed = (seed.wrapping_mul(9301).wrapping_add(49297)) % 233280;
        seed as f32 / 233280.
    };
    (0..count)
        .map(|i| {
            let (x, y, a) = (rnd(), rnd() * 0.42, 0.2 + rnd() * 0.6);
            let size = if a > 0.7 { 2.5 } else { 1.5 };
            div()
                .absolute()
                .left(relative(x))
                .top(relative(y))
                .size(px(size))
                .rounded_full()
                .bg(night.star.opacity(night.star.a * a * shown(i)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A PNG's width and height, from its header.
    fn png_size(bytes: &[u8]) -> (u32, u32) {
        let at = |i: usize| u32::from_be_bytes(bytes[i..i + 4].try_into().unwrap());
        assert_eq!(&bytes[12..16], b"IHDR");
        (at(16), at(20))
    }

    #[test]
    fn the_grain_tile_is_drawn_one_texel_to_a_pixel() {
        let (width, height) = png_size(include_bytes!("../assets/night/grain.png"));
        assert_eq!((width as f32, height as f32), (GRAIN_TEXELS, GRAIN_TEXELS));
    }

    #[test]
    fn grain_covers_its_area_without_gaps() {
        let side = 128.;
        let placed = tiles((300., 200.), side, 18.);
        assert_eq!(placed.len(), 3 * 2);
        let right = placed.iter().map(|(x, ..)| x + side).fold(0., f32::max);
        let bottom = placed.iter().map(|(_, y, _)| y + side).fold(0., f32::max);
        assert!(right >= 300. && bottom >= 200.);
        for (x, y, _) in &placed {
            assert!(x % side == 0. && y % side == 0.);
        }
    }

    #[test]
    fn only_the_outer_corners_of_the_corner_tiles_are_rounded() {
        let placed = tiles((512., 256.), 128., 18.);
        let rounded: Vec<_> = placed
            .iter()
            .flat_map(|(x, y, c)| {
                [
                    (c.top_left, (*x, *y)),
                    (c.top_right, (x + 128., *y)),
                    (c.bottom_left, (*x, y + 128.)),
                    (c.bottom_right, (x + 128., y + 128.)),
                ]
            })
            .filter(|(r, _)| *r > 0.)
            .map(|(_, at)| at)
            .collect();
        assert_eq!(rounded, [(0., 0.), (512., 0.), (0., 256.), (512., 256.)]);
    }
}
