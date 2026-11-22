//! An eraser: paints nothing, and leaves what was painted under its shape see-through, down
//! to the window's blurred background. The floating sidebar needs one, with round corners:
//! GPUI clips to rectangles only, and the content under the panel has to follow its curve.
//!
//! GPUI 0.2 has no such blend mode. Neither it nor Nimbra draws paths, so the Metal pipeline
//! that copies paths to the window (labelled "path_sprites") is built with an erasing blend.
//! **A path therefore erases: do not draw anything else with `paint_path`.** Where the
//! pipeline is not the one expected (another GPUI, not macOS) a path is painted as usual,
//! and the shape is simply opaque, in the colour given.
//!
//! `fade` also needs to lay a see-through colour back, and GPUI adds up alpha instead of
//! compositing it: on the sidebar's material that comes out opaque, a dark band. So the
//! underline pipeline (a plain rectangle) composites alpha too. Over anything opaque, where
//! underlines of text are, that is the same as before.

use gpui::{
    Bounds, Hsla, IntoElement, PathBuilder, Pixels, ScrollHandle, Styled, UnderlineStyle, canvas,
    fill, point, px, size,
};

/// Has to run before the first window opens, as each builds its pipelines then.
pub fn install() {
    platform::install();
}

/// Erases a rounded rectangle the size of the element it is in.
pub fn rounded(radius: f32, fallback: Hsla) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            if let Some(path) = shape(bounds, radius) {
                window.paint_path(path, fallback);
            }
        },
    )
    .absolute()
    .inset_0()
}

/// Fades what scrolls up to the top edge of the element it is in into `material`, which is
/// see-through. Painting the material over it would be a darker band on the blurred desktop,
/// so each line of pixels is erased part-way and gets that much material back. Nothing shows
/// at rest. `clear` is the room above the first thing scrolled: by then it is all there.
pub fn fade(height: f32, clear: f32, material: Hsla, scroll: ScrollHandle) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let strength = (-f32::from(scroll.offset().y) / clear).clamp(0., 1.);
            if strength == 0. {
                return;
            }
            // Whole device pixels, or the lines' soft edges add up to stripes.
            let scale = window.scale_factor();
            let pixel = |at: Pixels| (f32::from(at) * scale).round();
            let (left, top, right) = (
                pixel(bounds.left()),
                pixel(bounds.top()),
                pixel(bounds.right()),
            );
            let lines = (height * scale).round() as usize;
            let line = |row: usize| {
                Bounds::new(
                    point(px(left / scale), px((top + row as f32) / scale)),
                    size(px((right - left) / scale), px(1. / scale)),
                )
            };
            let covered = |row: usize| strength * covered(row as f32 / lines as f32);
            // A layer each, so that all of it is erased before any is laid back.
            window.paint_layer(bounds, |window| {
                for row in 0..lines {
                    if let Some(path) = shape(line(row), 0.) {
                        let erased = erased(covered(row), material.a);
                        window.paint_path(
                            path,
                            Hsla {
                                a: erased,
                                ..material
                            },
                        );
                    }
                }
            });
            window.paint_layer(bounds, |window| {
                for row in 0..lines {
                    let laid = material.a * covered(row);
                    let line = line(row);
                    // An underline for how it blends: see the top of this file.
                    let style = UnderlineStyle {
                        thickness: line.size.height,
                        color: Some(Hsla {
                            a: laid,
                            ..material
                        }),
                        wavy: false,
                    };
                    window.paint_underline(line.origin, line.size.width, &style);
                }
            });
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .right_0()
    .h(px(height))
}

/// A rounded pane on `material` that is more see-through than it, in the colour `tint`:
/// `opacity` is what it comes out at. A tinted quad alone comes out opaque, as GPUI adds its
/// alpha to the material's, and the blur behind is lost.
pub fn glass(radius: f32, material: Hsla, tint: Hsla, opacity: f32) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            window.paint_layer(bounds, |window| {
                if let Some(path) = shape(bounds, radius) {
                    let a = thinned(material.a, tint.a, opacity);
                    window.paint_path(path, Hsla { a, ..material });
                }
            });
            window.paint_layer(bounds, |window| {
                window.paint_quad(fill(bounds, tint).corner_radii(px(radius)));
            });
        },
    )
    .absolute()
    .inset_0()
}

/// How much to erase so that `added`, which is added, brings it to `opacity`.
fn thinned(material: f32, added: f32, opacity: f32) -> f32 {
    (1. - (opacity - added) / material).clamp(0., 1.)
}

/// How much of what is under the fade is hidden, `at` of the way down it: eased at both ends.
fn covered(at: f32) -> f32 {
    let at = at.clamp(0., 1.);
    1. - at * at * (3. - 2. * at)
}

/// How much to erase so that laying `covered` of a material of this opacity over it leaves
/// exactly that material, with `1 - covered` of what was scrolled there showing.
fn erased(covered: f32, opacity: f32) -> f32 {
    let left = 1. - opacity * covered;
    if left <= f32::EPSILON {
        return 1.;
    }
    (1. - (1. - covered) / left).clamp(0., 1.)
}

fn shape(bounds: Bounds<Pixels>, radius: f32) -> Option<gpui::Path<Pixels>> {
    let (left, top) = (f32::from(bounds.left()), f32::from(bounds.top()));
    let (right, bottom) = (f32::from(bounds.right()), f32::from(bounds.bottom()));
    let r = radius
        .min((right - left) / 2.)
        .min((bottom - top) / 2.)
        .max(0.);
    // How far a cubic's controls reach along a quarter circle.
    let k = r * (1. - 0.552_284_8);
    let p = |x: f32, y: f32| point(px(x), px(y));
    let mut path = PathBuilder::fill();
    path.move_to(p(left + r, top));
    path.line_to(p(right - r, top));
    if r > 0. {
        path.cubic_bezier_to(p(right, top + r), p(right - k, top), p(right, top + k));
    }
    path.line_to(p(right, bottom - r));
    if r > 0. {
        path.cubic_bezier_to(
            p(right - r, bottom),
            p(right, bottom - k),
            p(right - k, bottom),
        );
    }
    path.line_to(p(left + r, bottom));
    if r > 0. {
        path.cubic_bezier_to(
            p(left, bottom - r),
            p(left + k, bottom),
            p(left, bottom - k),
        );
    }
    path.line_to(p(left, top + r));
    if r > 0. {
        path.cubic_bezier_to(p(left + r, top), p(left, top + k), p(left + k, top));
    }
    path.close();
    path.build().ok()
}

#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)] // emitted by the objc crate's macros
mod platform {
    use std::ffi::CStr;
    use std::sync::OnceLock;

    use objc::runtime::{Class, Imp, Object, Sel};
    use objc::{msg_send, sel, sel_impl};

    /// What `build_path_sprite_pipeline_state` in GPUI's Metal renderer calls it.
    const LABEL: &CStr = c"path_sprites";
    /// And the pipeline that lays colour back in `fade`.
    const OVER_LABEL: &CStr = c"underlines";
    const ZERO: usize = 0;
    const ONE_MINUS_SOURCE_ALPHA: usize = 5;

    #[link(name = "Metal", kind = "framework")]
    unsafe extern "C" {
        fn MTLCopyAllDevices() -> *mut Object;
    }

    #[link(name = "objc")]
    unsafe extern "C" {
        fn class_getMethodImplementation(class: *const Class, sel: Sel) -> Imp;
        fn class_replaceMethod(
            class: *const Class,
            sel: Sel,
            imp: Imp,
            types: *const std::os::raw::c_char,
        ) -> *const std::ffi::c_void;
    }

    type Build = unsafe extern "C" fn(&Object, Sel, *mut Object, *mut *mut Object) -> *mut Object;

    /// Each kind of GPU's class, and its own way to build a pipeline.
    static BUILDERS: OnceLock<Vec<(usize, Build)>> = OnceLock::new();

    extern "C" fn build(
        this: &Object,
        sel: Sel,
        descriptor: *mut Object,
        error: *mut *mut Object,
    ) -> *mut Object {
        let class = this.class() as *const Class as usize;
        let original = BUILDERS
            .get()
            .and_then(|all| all.iter().find(|(kind, _)| *kind == class));
        let Some((_, original)) = original else {
            return std::ptr::null_mut();
        };
        // SAFETY: Metal's own arguments, passed on to the method this stands in for. Only
        // GPUI's path and underline pipelines are changed, and only their blend.
        unsafe {
            if !descriptor.is_null() {
                let label: *mut Object = msg_send![descriptor, label];
                if !label.is_null() {
                    let text: *const std::os::raw::c_char = msg_send![label, UTF8String];
                    if !text.is_null() && CStr::from_ptr(text) == OVER_LABEL {
                        let all: *mut Object = msg_send![descriptor, colorAttachments];
                        let target: *mut Object = msg_send![all, objectAtIndexedSubscript: 0usize];
                        let _: () = msg_send![target,
                            setDestinationAlphaBlendFactor: ONE_MINUS_SOURCE_ALPHA];
                    } else if !text.is_null() && CStr::from_ptr(text) == LABEL {
                        let all: *mut Object = msg_send![descriptor, colorAttachments];
                        let target: *mut Object = msg_send![all, objectAtIndexedSubscript: 0usize];
                        // Nothing of the path, and what is there less the path's coverage.
                        let _: () = msg_send![target, setSourceRGBBlendFactor: ZERO];
                        let _: () = msg_send![target, setSourceAlphaBlendFactor: ZERO];
                        let _: () =
                            msg_send![target, setDestinationRGBBlendFactor: ONE_MINUS_SOURCE_ALPHA];
                        let _: () = msg_send![target,
                            setDestinationAlphaBlendFactor: ONE_MINUS_SOURCE_ALPHA];
                    }
                }
            }
            original(this, sel, descriptor, error)
        }
    }

    pub fn install() {
        if BUILDERS.get().is_some() {
            return;
        }
        // SAFETY: on the main thread before any window. The replacement has the method's
        // signature, and every original is taken before any is replaced.
        unsafe {
            let sel = sel!(newRenderPipelineStateWithDescriptor:error:);
            let devices = MTLCopyAllDevices();
            if devices.is_null() {
                return;
            }
            let count: usize = msg_send![devices, count];
            let mut found: Vec<(usize, Build)> = Vec::new();
            for i in 0..count {
                let device: *mut Object = msg_send![devices, objectAtIndex: i];
                let class = (*device).class() as *const Class;
                if !found.iter().any(|(kind, _)| *kind == class as usize) {
                    let original = class_getMethodImplementation(class, sel);
                    found.push((class as usize, std::mem::transmute::<Imp, Build>(original)));
                }
            }
            let _: () = msg_send![devices, release];
            if BUILDERS.set(found).is_err() {
                return;
            }
            let replacement = std::mem::transmute::<
                extern "C" fn(&Object, Sel, *mut Object, *mut *mut Object) -> *mut Object,
                Imp,
            >(build);
            for (class, _) in BUILDERS.get().into_iter().flatten() {
                // Only used where the class inherits the method: id (id, SEL, id, id *).
                class_replaceMethod(*class as *const Class, sel, replacement, c"@@:@^@".as_ptr());
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    pub fn install() {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glass_is_more_see_through_than_the_sidebar() {
        for theme in [crate::theme::Theme::dark(), crate::theme::Theme::light()] {
            let (material, tint) = (theme.sidebar.a, theme.glass.a);
            let erased = thinned(material, tint, theme.glass_opacity);
            assert!((material * (1. - erased) + tint - theme.glass_opacity).abs() < 1e-4);
            assert!(theme.glass_opacity < material);
        }
    }

    #[test]
    fn the_fade_never_changes_the_material() {
        let material = 0.88;
        for step in 0..=20 {
            let hidden = covered(step as f32 / 20.);
            // Over a bare stretch of the sidebar: the material, exactly.
            let left = material * (1. - erased(hidden, material));
            let laid = material * hidden;
            assert!((laid + left * (1. - laid) - material).abs() < 1e-4);
        }
        assert_eq!((covered(0.), covered(1.)), (1., 0.));
    }
}
