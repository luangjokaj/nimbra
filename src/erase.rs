//! An eraser: paints nothing, and leaves what was painted under its shape see-through, down
//! to the window's blurred background. The floating sidebar needs one, with round corners:
//! GPUI clips to rectangles only, and the content under the panel has to follow its curve.
//!
//! GPUI 0.2 has no such blend mode. Neither it nor Nimbra draws paths, so the Metal pipeline
//! that copies paths to the window (labelled "path_sprites") is built with an erasing blend.
//! **A path therefore erases: do not draw anything else with `paint_path`.** Where the
//! pipeline is not the one expected (another GPUI, not macOS) a path is painted as usual,
//! and the shape is simply opaque, in the colour given.

use gpui::{Bounds, Hsla, IntoElement, PathBuilder, Pixels, Styled, canvas, point, px};

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
        // GPUI's path pipeline is changed, and only its blend.
        unsafe {
            if !descriptor.is_null() {
                let label: *mut Object = msg_send![descriptor, label];
                if !label.is_null() {
                    let text: *const std::os::raw::c_char = msg_send![label, UTF8String];
                    if !text.is_null() && CStr::from_ptr(text) == LABEL {
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
