mod adblock;
mod app;
mod settings;
mod vault;
mod wk;

use app::{App, UserEvent};
use mimalloc::MiMalloc;
use muda::{
    Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu,
    accelerator::{Accelerator, Code, Modifiers},
};
use settings::Settings;
use tao::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    platform::macos::WindowBuilderExtMacOS,
    window::WindowBuilder,
};

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg = Settings::load();
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();

    let proxy = event_loop.create_proxy();
    MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
        let _ = proxy.send_event(UserEvent::Menu(e.id.0));
    }));
    let _menu = build_menu()?;

    let window = WindowBuilder::new()
        .with_title("Crow")
        .with_inner_size(LogicalSize::new(1200.0, 800.0))
        .with_min_inner_size(LogicalSize::new(480.0, 320.0))
        .with_theme(app::theme_of(&cfg.theme))
        .with_titlebar_transparent(true)
        .with_title_hidden(true)
        .with_fullsize_content_view(true)
        .build(&event_loop)?;

    let mut app = App::new(window, event_loop.create_proxy(), cfg);

    event_loop.run(move |event, _, control_flow| {
        match event {
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                app.handle(UserEvent::Menu("quit".into()))
            }
            Event::WindowEvent { event: WindowEvent::Resized(_), .. } => app.resize(),
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
            "Crow",
            true,
            &[
                &item("panel:settings", "Settings…", cmd, Code::Comma),
                &sep(),
                &item("lock", "Lock Crow", Modifiers::META | Modifiers::CONTROL, Code::KeyL),
                &sep(),
                &PredefinedMenuItem::hide(None),
                &PredefinedMenuItem::hide_others(None),
                &PredefinedMenuItem::show_all(None),
                &sep(),
                &item("quit", "Quit Crow", cmd, Code::KeyQ),
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
