//! macOS main-window integration. Keep AppKit Space and view geometry handling
//! here rather than compensating with frontend viewport or terminal dimensions.

use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, rc::Retained, sel};
use objc2_app_kit::{NSView, NSWindowDidExitFullScreenNotification};
use objc2_foundation::{MainThreadMarker, NSNotification, NSNotificationCenter, NSObject};
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{Manager, Window, WindowEvent};

#[derive(Default)]
pub struct MainWindowState {
    hide_after_fullscreen: AtomicBool,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = tauri::AppHandle]
    struct FullscreenObserver;

    impl FullscreenObserver {
        #[unsafe(method(windowDidExitFullScreen:))]
        fn did_exit_fullscreen(&self, _notification: &NSNotification) {
            let app = self.ivars();
            if app.state::<MainWindowState>()
                .hide_after_fullscreen.swap(false, Ordering::AcqRel)
                && let Some(window) = app.get_webview_window("main")
            {
                hide_main_window(&window.as_ref().window());
            }
        }
    }
);

thread_local! {
    // Selector observers are not retained by NSNotificationCenter. Keep this
    // main-thread-only object alive for the desktop event loop's lifetime.
    static FULLSCREEN_OBSERVER: RefCell<Option<Retained<FullscreenObserver>>> = const { RefCell::new(None) };
}

pub fn install(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    let app = window.app_handle().clone();
    let native_window = window.ns_window()?;
    let mtm = MainThreadMarker::new().expect("macOS window setup must run on the main thread");
    let observer = FullscreenObserver::alloc(mtm).set_ivars(app);
    let observer: Retained<FullscreenObserver> = unsafe { msg_send![super(observer), init] };
    unsafe {
        NSNotificationCenter::defaultCenter().addObserver_selector_name_object(
            &observer,
            sel!(windowDidExitFullScreen:),
            Some(NSWindowDidExitFullScreenNotification),
            Some(&*native_window.cast()),
        );
    }
    FULLSCREEN_OBSERVER.with(|slot| *slot.borrow_mut() = Some(observer));
    Ok(())
}

pub fn cancel_pending_close(app: &tauri::AppHandle) {
    app.state::<MainWindowState>()
        .hide_after_fullscreen
        .store(false, Ordering::Release);
}

pub fn handle_event(window: &Window, event: &WindowEvent) {
    if window.label() != "main" {
        return;
    }

    match event {
        WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            // Ordering out only the window leaves an empty fullscreen Space.
            // Exit first and wait for AppKit's completion notification. Hiding
            // the entire application would also hide Agent notifications.
            let state = window.app_handle().state::<MainWindowState>();
            if state.hide_after_fullscreen.load(Ordering::Acquire) {
                return;
            }
            match window.is_fullscreen() {
                Ok(true) => {
                    state.hide_after_fullscreen.store(true, Ordering::Release);
                    if let Err(error) = window.set_fullscreen(false) {
                        state.hide_after_fullscreen.store(false, Ordering::Release);
                        eprintln!("Failed to exit fullscreen before hiding: {error}");
                    }
                }
                Ok(false) => hide_main_window(window),
                Err(error) => eprintln!("Failed to query macOS fullscreen state: {error}"),
            }
        }
        WindowEvent::Resized(_) => {
            let Some(webview) = window.app_handle().get_webview_window("main") else {
                return;
            };
            // The fullscreen transition also emits a final resize. Use the
            // live parent bounds on the UI thread, not an intermediate event
            // size or the outer NSWindow frame (which includes titlebar chrome).
            if let Err(error) = webview.with_webview(|platform| unsafe {
                // Tauri supplies a live WKWebView (an NSView subclass), and
                // with_webview runs this callback on the main thread.
                let view = &*platform.inner().cast::<NSView>();
                if let Some(parent) = view.superview() {
                    let bounds = parent.bounds();
                    if bounds.size.width > 0.0 && bounds.size.height > 0.0 && view.frame() != bounds
                    {
                        view.setFrame(bounds);
                    }
                }
            }) {
                eprintln!("Failed to synchronize macOS webview bounds: {error}");
            }
        }
        _ => {}
    }
}

fn hide_main_window(window: &Window) {
    if let Err(error) = window.hide() {
        eprintln!("Failed to hide macOS main window: {error}");
    }
}
