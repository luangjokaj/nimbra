//! Signature pictures drawn natively, over the core's cache (`nimbra_core::signature_images`):
//! the downloaded copies as GPUI images, and downloading the missing ones in the background.

use std::collections::HashMap;
use std::sync::Arc;

use nimbra_core::rich::ImageKind;
use nimbra_core::signature_images::{cached, fetch, wanted};

thread_local! {
    static DRAWN: std::cell::RefCell<HashMap<String, Arc<gpui::Image>>> = Default::default();
}

/// The kept copy, for drawing natively (Settings' Preview).
pub fn image(url: &str) -> Option<Arc<gpui::Image>> {
    if let Some(image) = DRAWN.with(|drawn| drawn.borrow().get(url).cloned()) {
        return Some(image);
    }
    let (kind, bytes) = cached(url)?;
    let format = match kind {
        ImageKind::Png => gpui::ImageFormat::Png,
        ImageKind::Jpeg => gpui::ImageFormat::Jpeg,
        ImageKind::Gif => gpui::ImageFormat::Gif,
        ImageKind::Webp => gpui::ImageFormat::Webp,
    };
    let image = Arc::new(gpui::Image::from_bytes(format, bytes.as_ref().clone()));
    DRAWN.with(|drawn| drawn.borrow_mut().insert(url.to_string(), image.clone()));
    Some(image)
}

/// Downloads the images of `urls` that are not kept yet, in the background, then calls
/// `done` (on the main thread) when at least one arrived. Nothing happens when they are
/// all kept already.
pub fn ensure(urls: Vec<String>, cx: &mut gpui::App, done: impl FnOnce(&mut gpui::App) + 'static) {
    let missing = wanted(&urls);
    if missing.is_empty() {
        return;
    }
    cx.spawn(async move |cx| {
        let arrived = cx
            .background_executor()
            .spawn(async move {
                // Every one is tried, not just up to the first that arrives.
                missing.iter().filter(|url| fetch(url).is_ok()).count() > 0
            })
            .await;
        if arrived {
            cx.update(done).ok();
        }
    })
    .detach();
}

/// Forgets the kept copies of `urls`, so the next `ensure` downloads them again (the
/// "Refresh images" button, after a logo changed).
pub fn forget(urls: &[String]) {
    nimbra_core::signature_images::forget(urls);
    for url in urls {
        DRAWN.with(|drawn| drawn.borrow_mut().remove(url));
    }
}
