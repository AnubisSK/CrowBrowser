mod settings;

use mimalloc::MiMalloc;
use muda::{
    Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu,
    accelerator::{Accelerator, Code, Modifiers},
};
use settings::{Settings, js_str};
use tao::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy},
    platform::macos::WindowBuilderExtMacOS,
    window::{Theme, Window, WindowBuilder},
};
use wry::{PageLoadEvent, RGBA, Rect, WebView, WebViewBuilder, dpi};

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

const TOOLBAR_H: f64 = 40.0;
const BG_DARK: RGBA = (0x1f, 0x1f, 0x23, 0xff);
const BG_LIGHT: RGBA = (0xf4, 0xf4, 0xf6, 0xff);
const ZOOM_STEPS: &[f64] = &[0.5, 0.67, 0.75, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0];

enum UserEvent {
    Menu(MenuEvent),
    Ipc(String),
    PageLoad(bool, String),
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut cfg = Settings::load();
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();

    let proxy = event_loop.create_proxy();
    MenuEvent::set_event_handler(Some(move |e| {
        let _ = proxy.send_event(UserEvent::Menu(e));
    }));

    let item = |id: &str, text: &str, mods: Modifiers, code| {
        MenuItem::with_id(id, text, true, Some(Accelerator::new(mods, code)))
    };
    let cmd = Modifiers::META;
    let menu = Menu::with_items(&[
        &Submenu::with_items(
            "Crow",
            true,
            &[
                &item("settings", "Settings…", cmd, Code::Comma),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::hide(None),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::quit(None),
            ],
        )?,
        &Submenu::with_items(
            "Edit",
            true,
            &[
                &PredefinedMenuItem::undo(None),
                &PredefinedMenuItem::redo(None),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::cut(None),
                &PredefinedMenuItem::copy(None),
                &PredefinedMenuItem::paste(None),
                &PredefinedMenuItem::select_all(None),
            ],
        )?,
        &Submenu::with_items(
            "View",
            true,
            &[
                &item("location", "Open Location", cmd, Code::KeyL),
                &item("reload", "Reload", cmd, Code::KeyR),
                &PredefinedMenuItem::separator(),
                &item("zoom-in", "Zoom In", cmd, Code::Equal),
                &item("zoom-out", "Zoom Out", cmd, Code::Minus),
                &item("zoom-reset", "Actual Size", cmd, Code::Digit0),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::fullscreen(None),
            ],
        )?,
        &Submenu::with_items(
            "History",
            true,
            &[
                &item("home", "Home", cmd | Modifiers::SHIFT, Code::KeyH),
                &item("back", "Back", cmd, Code::BracketLeft),
                &item("forward", "Forward", cmd, Code::BracketRight),
            ],
        )?,
        &Submenu::with_items(
            "Window",
            true,
            &[
                &PredefinedMenuItem::minimize(None),
                &PredefinedMenuItem::maximize(None),
            ],
        )?,
    ])?;
    menu.init_for_nsapp();

    let window = WindowBuilder::new()
        .with_title("Crow")
        .with_inner_size(LogicalSize::new(1200.0, 800.0))
        .with_theme(theme_of(&cfg.theme))
        .with_titlebar_transparent(true)
        .with_title_hidden(true)
        .with_fullsize_content_view(true)
        .build(&event_loop)
        .expect("window");

    let (toolbar_rect, content_rect) = layout(&window);
    let bg = bg_of(&window);

    let ipc_proxy = event_loop.create_proxy();
    let proxy = ipc_proxy.clone();
    let toolbar = WebViewBuilder::new()
        .with_html(include_str!("toolbar.html"))
        .with_background_color(bg)
        .with_bounds(toolbar_rect)
        .with_ipc_handler(move |req| {
            let _ = proxy.send_event(UserEvent::Ipc(req.into_body()));
        })
        .build_as_child(&window)?;

    let proxy = event_loop.create_proxy();
    let content = WebViewBuilder::new()
        .with_url(cfg.start_url())
        .with_background_color(bg)
        .with_bounds(content_rect)
        .with_focused(true)
        .with_on_page_load_handler(move |ev, url| {
            let _ = proxy.send_event(UserEvent::PageLoad(matches!(ev, PageLoadEvent::Started), url));
        })
        .build_as_child(&window)?;
    let _ = content.zoom(cfg.zoom);

    let mut zoom = cfg.zoom;
    // Created on demand, dropped on close to free memory
    let mut settings_view: Option<WebView> = None;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        let cmd = match event {
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                *control_flow = ControlFlow::Exit;
                return;
            }
            Event::WindowEvent { event: WindowEvent::Resized(_), .. } => {
                let (t, c) = layout(&window);
                let _ = toolbar.set_bounds(t);
                let _ = content.set_bounds(c);
                if let Some(s) = &settings_view {
                    let _ = s.set_bounds(c);
                }
                return;
            }
            Event::UserEvent(UserEvent::PageLoad(started, url)) => {
                let _ = toolbar.evaluate_script(&format!(
                    "setLoading({started});setUrl({})",
                    js_str(&url)
                ));
                if !started && url.starts_with("http") {
                    cfg.last_url = url;
                    if cfg.startup == "last" {
                        cfg.save();
                    }
                }
                return;
            }
            Event::UserEvent(UserEvent::Menu(e)) => e.id.0,
            Event::UserEvent(UserEvent::Ipc(msg)) => msg,
            _ => return,
        };

        let close_settings = |view: &mut Option<WebView>| {
            if view.take().is_some() {
                let _ = toolbar.evaluate_script("setSettingsOpen(false)");
                let _ = content.focus();
            }
        };

        let _ = match cmd.as_str() {
            "back" => content.evaluate_script("history.back()"),
            "forward" => content.evaluate_script("history.forward()"),
            "reload" => content.reload(),
            "stop" => content.evaluate_script("window.stop()"),
            "home" => {
                close_settings(&mut settings_view);
                content.load_url(&cfg.homepage)
            }
            "zoom-in" | "zoom-out" | "zoom-reset" => {
                zoom = match cmd.as_str() {
                    "zoom-in" => ZOOM_STEPS.iter().copied().find(|z| *z > zoom + 0.001).unwrap_or(zoom),
                    "zoom-out" => ZOOM_STEPS.iter().rev().copied().find(|z| *z < zoom - 0.001).unwrap_or(zoom),
                    _ => cfg.zoom,
                };
                content.zoom(zoom)
            }
            "drag" => {
                let _ = window.drag_window();
                Ok(())
            }
            "location" => toolbar
                .focus()
                .and_then(|_| toolbar.evaluate_script("focusUrl()")),
            "settings" => {
                if settings_view.is_some() {
                    close_settings(&mut settings_view);
                    Ok(())
                } else {
                    open_settings(&window, &cfg, ipc_proxy.clone()).map(|view| {
                        settings_view = Some(view);
                        let _ = toolbar.evaluate_script("setSettingsOpen(true)");
                    })
                }
            }
            "clear-data" => content.clear_all_browsing_data(),
            _ => {
                if let Some(kv) = cmd.strip_prefix("set:") {
                    let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
                    if cfg.set(k, v) {
                        cfg.save();
                        match k {
                            "theme" => {
                                window.set_theme(theme_of(&cfg.theme));
                                let bg = bg_of(&window);
                                let _ = toolbar.set_background_color(bg);
                                let _ = content.set_background_color(bg);
                            }
                            "zoom" => {
                                zoom = cfg.zoom;
                                let _ = content.zoom(zoom);
                            }
                            _ => {}
                        }
                    }
                    Ok(())
                } else if let Some(input) = cmd.strip_prefix("go:").map(str::trim) {
                    if input.is_empty() {
                        Ok(())
                    } else {
                        close_settings(&mut settings_view);
                        content
                            .load_url(&to_url(input, cfg.search_prefix()))
                            .and_then(|_| content.focus())
                    }
                } else {
                    Ok(())
                }
            }
        };
    });
}

fn open_settings(
    window: &Window,
    cfg: &Settings,
    proxy: EventLoopProxy<UserEvent>,
) -> wry::Result<WebView> {
    let html = include_str!("settings.html").replace("/*SETTINGS*/", &cfg.to_js());
    WebViewBuilder::new()
        .with_html(html)
        .with_background_color(bg_of(window))
        .with_bounds(layout(window).1)
        .with_focused(true)
        .with_ipc_handler(move |req| {
            let _ = proxy.send_event(UserEvent::Ipc(req.into_body()));
        })
        .build_as_child(window)
}

fn theme_of(name: &str) -> Option<Theme> {
    match name {
        "dark" => Some(Theme::Dark),
        "light" => Some(Theme::Light),
        _ => None,
    }
}

fn bg_of(window: &Window) -> RGBA {
    if window.theme() == Theme::Light { BG_LIGHT } else { BG_DARK }
}

fn layout(window: &Window) -> (Rect, Rect) {
    let size = window.inner_size().to_logical::<f64>(window.scale_factor());
    let rect = |y: f64, h: f64| Rect {
        position: dpi::LogicalPosition::new(0.0, y).into(),
        size: dpi::LogicalSize::new(size.width, h.max(0.0)).into(),
    };
    (rect(0.0, TOOLBAR_H), rect(TOOLBAR_H, size.height - TOOLBAR_H))
}

fn to_url(input: &str, search_prefix: &str) -> String {
    let has_scheme = ["http://", "https://", "file://", "about:"]
        .iter()
        .any(|s| input.starts_with(s));
    if has_scheme {
        input.to_string()
    } else if !input.contains(' ') && (input.contains('.') || input.starts_with("localhost")) {
        format!("https://{input}")
    } else {
        let mut q = String::from(search_prefix);
        for b in input.bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    q.push(b as char)
                }
                b' ' => q.push('+'),
                _ => q.push_str(&format!("%{b:02X}")),
            }
        }
        q
    }
}
