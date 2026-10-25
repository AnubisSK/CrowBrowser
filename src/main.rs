#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]
#![warn(clippy::pedantic)]
#![allow(
    clippy::doc_markdown,
    clippy::too_many_lines,
    clippy::struct_excessive_bools,
    clippy::many_single_char_names,
    clippy::similar_names,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]

mod adblock;
mod app;
mod cert;
mod default_browser;
mod download;
mod gamepad;
mod proxy;
mod settings;
mod update;
mod vault;
#[cfg_attr(target_os = "linux", path = "wk_gtk.rs")]
#[cfg_attr(target_os = "windows", path = "wk_win.rs")]
mod wk;

use app::{App, UserEvent};
use mimalloc::MiMalloc;
#[cfg(target_os = "macos")]
use muda::{
    Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu,
    accelerator::{Accelerator, Code, Modifiers},
};
#[cfg(target_os = "macos")]
use objc2::{AllocAnyThread, MainThreadMarker};
#[cfg(target_os = "macos")]
use objc2_app_kit::{NSApplication, NSImage};
#[cfg(target_os = "macos")]
use objc2_foundation::NSData;
use tao::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    window::{Window, WindowBuilder},
};
#[cfg(target_os = "macos")]
use tao::platform::macos::WindowBuilderExtMacOS;
#[cfg(target_os = "linux")]
use tao::platform::unix::EventLoopBuilderExtUnix;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[allow(unused_mut)]
    let mut builder = EventLoopBuilder::<UserEvent>::with_user_event();
    #[cfg(target_os = "linux")]
    {
        // Wayland app_id / X11 WM_CLASS; matches com.crow.browser.desktop (icon, name).
        // GTK keeps one instance per app id; CROW_DEV=1 runs a separate dev instance.
        let id = if std::env::var_os("CROW_DEV").is_some() { "com.crow.browser.dev" } else { "com.crow.browser" };
        gtk::glib::set_prgname(Some(id));
        builder.with_app_id(id);
    }
    let event_loop = builder.build();
    // Links passed by the system or a launcher (`crow-browser %u`); only http(s) is opened.
    let urls: Vec<String> = std::env::args().skip(1).collect();
    #[cfg(target_os = "linux")]
    {
        use tao::platform::unix::EventLoopWindowTargetExtUnix;
        if default_browser::forward_to_primary(event_loop.gtk_app(), &urls) {
            return Ok(());
        }
        default_browser::listen(event_loop.gtk_app(), event_loop.create_proxy());
    }

    #[cfg(target_os = "macos")]
    let _menu = {
        let proxy = event_loop.create_proxy();
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            let _ = proxy.send_event(UserEvent::Menu(e.id.0));
        }));
        build_menu()?
    };

    let window = WindowBuilder::new()
        .with_title("CrowBrowser")
        .with_inner_size(LogicalSize::new(1200.0, 800.0))
        .with_min_inner_size(LogicalSize::new(480.0, 320.0));
    // macOS: transparent title bar, traffic lights over the tab row.
    #[cfg(target_os = "macos")]
    let window = window
        .with_titlebar_transparent(true)
        .with_title_hidden(true)
        .with_fullsize_content_view(true);
    // Linux: frameless, the toolbar draws the same traffic lights.
    #[cfg(target_os = "linux")]
    let window = window.with_decorations(wk::native_decorations());
    let window = window.build(&event_loop)?;
    set_app_icon(&window);
    #[cfg(target_os = "linux")]
    {
        // Menu shortcuts: there is no global menu bar, so they are handled on the GTK window.
        let proxy = event_loop.create_proxy();
        wk::init_window(&window, move |cmd| {
            let _ = proxy.send_event(UserEvent::Menu(cmd));
        });
    }
    #[cfg(target_os = "windows")]
    {
        // No menu bar: shortcuts come from every WebView2 controller (`AcceleratorKeyPressed`).
        let proxy = event_loop.create_proxy();
        wk::init_window(&window, move |cmd| {
            let _ = proxy.send_event(UserEvent::Menu(cmd));
        });
    }

    let mut app = App::new(window, event_loop.create_proxy());
    if !urls.is_empty() {
        app.handle(UserEvent::OpenUrls(urls));
    }

    event_loop.run(move |event, _, control_flow| {
        match event {
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                app.handle(UserEvent::Menu("quit".into()));
            }
            Event::WindowEvent { event: WindowEvent::Resized(_), .. } => app.resize(),
            Event::WindowEvent { event: WindowEvent::Focused(on), .. } => app.focused(on),
            // macOS: links from other apps once Crow is the default browser.
            Event::Opened { urls } => app.handle(UserEvent::OpenUrls(urls.iter().map(std::string::ToString::to_string).collect())),
            Event::UserEvent(e) => app.handle(e),
            Event::LoopDestroyed => app.shutdown(),
            _ => {}
        }
        app.timer();
        *control_flow = if app.exit {
            ControlFlow::Exit
        } else {
            ControlFlow::WaitUntil(app.wake())
        };
    });
}

/// Icon for `cargo run` (unbundled binary); the .app bundle uses Crow.icns.
#[cfg(target_os = "macos")]
fn set_app_icon(_: &Window) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let data = NSData::with_bytes(include_bytes!("../assets/icon-256.png"));
    if let Some(img) = NSImage::initWithData(NSImage::alloc(), &data) {
        unsafe { NSApplication::sharedApplication(mtm).setApplicationIconImage(Some(&img)) };
    }
}

/// X11 window icon (Wayland takes it from com.crow.browser.desktop, scripts/install_linux.sh);
/// Windows: title bar and taskbar icon.
#[cfg(any(target_os = "linux", target_os = "windows"))]
fn set_app_icon(window: &Window) {
    let decoder = png::Decoder::new(std::io::Cursor::new(include_bytes!("../assets/icon-256.png")));
    let Ok(mut reader) = decoder.read_info() else { return };
    let Some(size) = reader.output_buffer_size() else { return };
    let mut buf = vec![0; size];
    let Ok(info) = reader.next_frame(&mut buf) else { return };
    if info.color_type == png::ColorType::Rgba && info.bit_depth == png::BitDepth::Eight {
        buf.truncate(info.buffer_size());
        if let Ok(icon) = tao::window::Icon::from_rgba(buf, info.width, info.height) {
            window.set_window_icon(Some(icon));
        }
    }
}

#[cfg(target_os = "macos")]
fn build_menu() -> Result<Menu, muda::Error> {
    let cmd = Modifiers::META;
    let shift = Modifiers::META | Modifiers::SHIFT;
    let alt = Modifiers::META | Modifiers::ALT;
    let item = |id: &str, text: &str, mods: Modifiers, code: Code| {
        MenuItem::with_id(id, text, true, Some(Accelerator::new(mods, code)))
    };
    let sep = PredefinedMenuItem::separator;

    let digits = [
        Code::Digit1, Code::Digit2, Code::Digit3, Code::Digit4, Code::Digit5,
        Code::Digit6, Code::Digit7, Code::Digit8, Code::Digit9,
    ];
    let tab_items: Vec<MenuItem> = digits
        .iter()
        .enumerate()
        .map(|(i, code)| {
            let label = if i == 8 { "Last Tab".to_string() } else { format!("Tab {}", i + 1) };
            item(&format!("tab:{}", i + 1), &label, cmd, *code)
        })
        .collect();

    let window_menu = Submenu::with_items(
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(None),
            &PredefinedMenuItem::maximize(None),
            &sep(),
            &item("next-tab", "Next Tab", shift, Code::BracketRight),
            &item("prev-tab", "Previous Tab", shift, Code::BracketLeft),
            &sep(),
            &item("panel:downloads", "Downloads", alt, Code::KeyL),
            &sep(),
        ],
    )?;
    for t in &tab_items {
        window_menu.append(t)?;
    }

    let menu = Menu::with_items(&[
        &Submenu::with_items(
            "CrowBrowser",
            true,
            &[
                &item("panel:settings", "Settings…", cmd, Code::Comma),
                &sep(),
                &item("lock", "Lock CrowBrowser", Modifiers::META | Modifiers::CONTROL, Code::KeyL),
                &item("panic", "Panic: Wipe Session", shift, Code::KeyX),
                &sep(),
                &PredefinedMenuItem::hide(None),
                &PredefinedMenuItem::hide_others(None),
                &PredefinedMenuItem::show_all(None),
                &sep(),
                &item("quit", "Quit CrowBrowser", cmd, Code::KeyQ),
            ],
        )?,
        &Submenu::with_items(
            "File",
            true,
            &[
                &item("new-tab", "New Tab", cmd, Code::KeyT),
                &item("reopen-tab", "Reopen Closed Tab", shift, Code::KeyT),
                &item("close-tab", "Close Tab", cmd, Code::KeyW),
                &sep(),
                &item("location", "Open Location", cmd, Code::KeyL),
            ],
        )?,
        &Submenu::with_items(
            "Edit",
            true,
            &[
                &PredefinedMenuItem::undo(None),
                &PredefinedMenuItem::redo(None),
                &sep(),
                &PredefinedMenuItem::cut(None),
                &PredefinedMenuItem::copy(None),
                &PredefinedMenuItem::paste(None),
                &PredefinedMenuItem::select_all(None),
                &sep(),
                &item("find", "Find…", cmd, Code::KeyF),
                &item("find-next", "Find Next", cmd, Code::KeyG),
                &item("find-prev", "Find Previous", shift, Code::KeyG),
            ],
        )?,
        &Submenu::with_items(
            "View",
            true,
            &[
                &item("reload", "Reload", cmd, Code::KeyR),
                &sep(),
                &item("zoom-in", "Zoom In", cmd, Code::Equal),
                &item("zoom-out", "Zoom Out", cmd, Code::Minus),
                &item("zoom-reset", "Actual Size", cmd, Code::Digit0),
                &sep(),
                &PredefinedMenuItem::fullscreen(None),
            ],
        )?,
        &Submenu::with_items(
            "History",
            true,
            &[
                &item("home", "Home", shift, Code::KeyH),
                &item("back", "Back", cmd, Code::BracketLeft),
                &item("forward", "Forward", cmd, Code::BracketRight),
                &sep(),
                &item("panel:history", "Show History", cmd, Code::KeyY),
            ],
        )?,
        &Submenu::with_items(
            "Bookmarks",
            true,
            &[
                &item("bookmark", "Bookmark This Page", cmd, Code::KeyD),
                &item("panel:bookmarks", "Show Bookmarks", alt, Code::KeyB),
            ],
        )?,
        &window_menu,
    ])?;
    menu.init_for_nsapp();
    Ok(menu)
}
