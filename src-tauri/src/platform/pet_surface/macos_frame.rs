//! Keep a native image on screen while WKWebView asynchronously adopts a new frame.
//! A DOM/layout acknowledgement is not a WebKit presentation acknowledgement.
use crate::pet::surface::geometry::Rect;
use block2::RcBlock;
use objc2::{rc::Retained, MainThreadMarker, MainThreadOnly};
use objc2_foundation::{NSError, NSPoint};
use objc2_web_kit::{WKSnapshotConfiguration, WKWebView};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};
use tauri::{AppHandle, Manager};
use tauri_nspanel::{
    objc2_app_kit::{
        NSAnimationContext, NSBackingStoreType, NSColor, NSImage, NSImageView, NSWindow,
        NSWindowStyleMask,
    },
    ManagerExt,
};

struct Cover {
    window: Retained<NSWindow>,
    source: Retained<NSWindow>,
    frame: Rect,
}
#[derive(Default)]
struct State {
    epoch: u64,
    cover: Option<Cover>,
}
thread_local! { static STATE: RefCell<State> = RefCell::default(); }

pub fn cancel() {
    STATE.with_borrow_mut(|s| {
        s.epoch += 1;
        if let Some(c) = s.cover.take() {
            NSAnimationContext::beginGrouping();
            c.source.setAlphaValue(1.);
            c.source.displayIfNeeded();
            c.window.close();
            NSAnimationContext::endGrouping();
        }
    });
}

pub fn reposition(previous: Rect, frame: Rect) {
    STATE.with_borrow_mut(|s| {
        if let Some(c) = &mut s.cover {
            // A drag moves the frozen visual with the window. Only resizing
            // compensates the new origin to preserve its screen position.
            if previous.width == frame.width && previous.height == frame.height {
                c.frame.x += frame.x - previous.x;
                c.frame.y += frame.y - previous.y;
            }
            if previous.width == frame.width && previous.height == frame.height {
                c.window
                    .setFrameOrigin(NSPoint::new(c.frame.x, -c.frame.y - c.frame.height));
            }
        }
    });
}

pub async fn begin(app: AppHandle) -> Result<u64, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let window = app
        .get_webview_window("pet")
        .ok_or("pet window unavailable")?;
    let handle = app.clone();
    let epoch = Arc::new(AtomicU64::new(0));
    let active_epoch = epoch.clone();
    window
        .with_webview(move |raw| {
            let id = STATE.with_borrow_mut(|s| {
                s.epoch += 1;
                s.epoch
            });
            active_epoch.store(id, Ordering::SeqCst);
            if STATE.with_borrow(|s| s.cover.is_some()) {
                let _ = tx.send(Ok(id));
                return;
            }
            let webview = unsafe { Retained::retain(raw.inner() as *mut WKWebView) }.unwrap();
            let mtm = MainThreadMarker::new().unwrap();
            let config = unsafe { WKSnapshotConfiguration::new(MainThreadMarker::new().unwrap()) };
            unsafe {
                config.setAfterScreenUpdates(true);
            }
            let sender = Rc::new(RefCell::new(Some(tx)));
            let view = webview.clone();
            let block = RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
                let result = (|| {
                    if !STATE.with_borrow(|s| s.epoch == id) {
                        return Err("superseded frame capture".into());
                    }
                    let image = unsafe { image.as_ref() }
                        .ok_or_else(|| format!("frame snapshot failed: {error:p}"))?;
                    let panel = handle
                        .get_webview_panel("pet")
                        .map_err(|e| format!("{e:?}"))?;
                    let native = panel.as_panel().frame();
                    let frame = Rect {
                        x: native.origin.x,
                        y: -native.origin.y - native.size.height,
                        width: native.size.width,
                        height: native.size.height,
                    };
                    let source = view.window().ok_or("pet WebView has no window")?;
                    let cover = NSImageView::initWithFrame(NSImageView::alloc(mtm), view.frame());
                    cover.setImage(Some(image));
                    let overlay = unsafe {
                        NSWindow::initWithContentRect_styleMask_backing_defer(
                            NSWindow::alloc(mtm),
                            native,
                            NSWindowStyleMask::Borderless,
                            NSBackingStoreType::Buffered,
                            false,
                        )
                    };
                    unsafe {
                        overlay.setReleasedWhenClosed(false);
                    }
                    overlay.setOpaque(false);
                    overlay.setBackgroundColor(Some(&NSColor::clearColor()));
                    overlay.setHasShadow(false);
                    overlay.setIgnoresMouseEvents(true);
                    overlay.setLevel(source.level() + 1);
                    overlay.setCollectionBehavior(source.collectionBehavior());
                    overlay.setContentView(Some(&cover));
                    NSAnimationContext::beginGrouping();
                    overlay.orderFrontRegardless();
                    overlay.displayIfNeeded();
                    source.setAlphaValue(0.);
                    NSAnimationContext::endGrouping();
                    STATE.with_borrow_mut(|s| {
                        s.cover = Some(Cover {
                            window: overlay,
                            source,
                            frame,
                        })
                    });
                    Ok(id)
                })();
                if let Some(tx) = sender.borrow_mut().take() {
                    let _ = tx.send(result);
                }
            });
            unsafe {
                webview.takeSnapshotWithConfiguration_completionHandler(Some(&config), &block);
            }
        })
        .map_err(|e| e.to_string())?;
    match tokio::time::timeout(std::time::Duration::from_secs(2), rx).await {
        Ok(result) => result.map_err(|e| e.to_string())?,
        Err(_) => {
            let id = epoch.load(Ordering::SeqCst);
            let _ = app.run_on_main_thread(move || {
                if STATE.with_borrow(|s| s.epoch == id) {
                    cancel();
                }
            });
            Err("frame snapshot timed out".into())
        }
    }
}

pub async fn end(app: AppHandle, token: u64, abort: bool) -> Result<(), String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let window = app
        .get_webview_window("pet")
        .ok_or("pet window unavailable")?;
    window
        .with_webview(move |raw| {
            if !STATE.with_borrow(|s| s.epoch == token && s.cover.is_some()) {
                let _ = tx.send(Ok(()));
                return;
            }
            if abort {
                cancel();
                let _ = tx.send(Ok(()));
                return;
            }
            let webview = unsafe { &*(raw.inner() as *mut WKWebView) };
            let config = unsafe { WKSnapshotConfiguration::new(MainThreadMarker::new().unwrap()) };
            unsafe {
                config.setAfterScreenUpdates(true);
            }
            let sender = Rc::new(RefCell::new(Some(tx)));
            // The image itself is not needed. Completion waits for WebKit to present
            // the DOM's first/final frame, including its new viewport dimensions.
            let block = RcBlock::new(move |_image: *mut NSImage, error: *mut NSError| {
                if STATE.with_borrow(|s| s.epoch == token) {
                    cancel();
                }
                if let Some(tx) = sender.borrow_mut().take() {
                    let _ = tx.send(if error.is_null() {
                        Ok(())
                    } else {
                        Err("WebView presentation failed".into())
                    });
                }
            });
            unsafe {
                webview.takeSnapshotWithConfiguration_completionHandler(Some(&config), &block);
            }
        })
        .map_err(|e| e.to_string())?;
    match tokio::time::timeout(std::time::Duration::from_secs(2), rx).await {
        Ok(result) => result.map_err(|e| e.to_string())?,
        Err(_) => {
            let _ = app.run_on_main_thread(move || {
                if STATE.with_borrow(|s| s.epoch == token) {
                    cancel();
                }
            });
            Err("WebView presentation timed out".into())
        }
    }
}
