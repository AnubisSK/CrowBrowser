use crow_browser::wk;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tao::dpi::{LogicalPosition, LogicalSize};
use wry::Rect;

#[test]
fn test_ipc_isolation() {
    #[cfg(target_os = "linux")]
    use tao::platform::unix::EventLoopBuilderExtUnix;
    #[cfg(target_os = "windows")]
    use tao::platform::windows::EventLoopBuilderExtWindows;

    let mut builder = tao::event_loop::EventLoopBuilder::new();
    #[cfg(target_os = "windows")]
    builder.with_any_thread(true);
    #[cfg(target_os = "linux")]
    builder.with_any_thread(true);

    let event_loop = builder.build();
    let window = tao::window::WindowBuilder::new()
        .with_visible(false)
        .build(&event_loop)
        .unwrap();

    let rect = Rect {
        position: LogicalPosition::new(0.0, 0.0).into(),
        size: LogicalSize::new(400.0, 300.0).into(),
    };

    // 1. Client content webview (`wk::builder`) strictly lacks `window.ipc`
    let store = wk::new_store();
    let config = wk::config(&store, false);
    let client_builder = wk::builder(config, String::new())
        .with_html("<!DOCTYPE html><html><body>client</body></html>");
    let client_view = wk::attach(client_builder, rect, &window).expect("attach client view");

    // 2. Internal UI webview receives `window.ipc`
    let ui_builder = wk::ui_builder()
        .with_html("<!DOCTYPE html><html><body>ui</body></html>")
        .with_ipc_handler(|_| {});
    let ui_view = wk::attach(ui_builder, rect, &window).expect("attach ui view");

    let client_ipc_type = Arc::new(Mutex::new(None::<String>));
    let client_done = Arc::new(AtomicBool::new(false));

    let c_ipc = client_ipc_type.clone();
    let c_done = client_done.clone();
    client_view
        .evaluate_script_with_callback("typeof window.ipc", move |res| {
            *c_ipc.lock().unwrap() = Some(res);
            c_done.store(true, Ordering::SeqCst);
        })
        .expect("eval on client view");

    let ui_ipc_type = Arc::new(Mutex::new(None::<String>));
    let ui_done = Arc::new(AtomicBool::new(false));

    let u_ipc = ui_ipc_type.clone();
    let u_done = ui_done.clone();
    ui_view
        .evaluate_script_with_callback("typeof window.ipc", move |res| {
            *u_ipc.lock().unwrap() = Some(res);
            u_done.store(true, Ordering::SeqCst);
        })
        .expect("eval on ui view");

    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(5);
    while (!client_done.load(Ordering::SeqCst) || !ui_done.load(Ordering::SeqCst))
        && start.elapsed() < timeout
    {
        #[cfg(target_os = "windows")]
        unsafe {
            use windows::Win32::UI::WindowsAndMessaging::{
                DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage,
            };
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        #[cfg(target_os = "linux")]
        {
            while gtk::events_pending() {
                gtk::main_iteration();
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    let client_res = client_ipc_type.lock().unwrap().take().unwrap_or_default();
    let ui_res = ui_ipc_type.lock().unwrap().take().unwrap_or_default();

    assert_eq!(client_res.trim_matches('"'), "undefined");
    assert_ne!(ui_res.trim_matches('"'), "undefined");
    assert_eq!(ui_res.trim_matches('"'), "object");
}
