//! Native gamepad bridge: controllers are read with gilrs (IOKit HID on macOS, evdev on Linux;
//! USB and Bluetooth alike) instead of WebKit's own Gamepad API, normalized to the W3C standard
//! layout and handed to the active tab as JSON (`App` → page shim in `defuser.js`).
//!
//! One worker thread owns the gilrs context for the whole process: gilrs' macOS backend keeps its
//! own run-loop thread until exit, so the bridge is started once (when some site may use gamepads)
//! and never recreated. Dropping the bridge stops and joins the worker.

use crate::{adblock::PAD_ID, app::UserEvent};
use gilrs::{Axis, Button, EventType, GamepadId, Gilrs, MappingSource};
use serde::Serialize;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use tao::event_loop::EventLoopProxy;

/// At most 60 updates per second, and only when the input changed.
const FRAME: Duration = Duration::from_micros(16_667);
/// Longest wait for input: how fast the worker notices `stop` / `resync`.
const IDLE: Duration = Duration::from_millis(100);
/// Analog button counts as pressed from here (like Chromium's trigger threshold, 30/255).
const PRESS: f32 = 0.12;

/// W3C "standard" button order.
const BUTTONS: [Button; 17] = [
    Button::South,
    Button::East,
    Button::West,
    Button::North,
    Button::LeftTrigger,
    Button::RightTrigger,
    Button::LeftTrigger2,
    Button::RightTrigger2,
    Button::Select,
    Button::Start,
    Button::LeftThumb,
    Button::RightThumb,
    Button::DPadUp,
    Button::DPadDown,
    Button::DPadLeft,
    Button::DPadRight,
    Button::Mode,
];

/// W3C "standard" axes; gilrs reports Y up, the web expects Y down (`true` = inverted).
const AXES: [(Axis, bool); 4] = [
    (Axis::LeftStickX, false),
    (Axis::LeftStickY, true),
    (Axis::RightStickX, false),
    (Axis::RightStickY, true),
];

#[derive(Clone, Serialize, PartialEq)]
pub struct GamepadButtonSnapshot {
    pub pressed: bool,
    pub touched: bool,
    pub value: f32,
}

/// What a page gets for one controller. `id` is always `PAD_ID`: no model, vendor or product.
#[derive(Clone, Serialize, PartialEq)]
pub struct GamepadSnapshot {
    pub index: usize,
    pub id: String,
    pub connected: bool,
    pub mapping: &'static str,
    pub buttons: Vec<GamepadButtonSnapshot>,
    pub axes: Vec<f32>,
}

/// A connected controller as Crow sees it (crow:gamepad, gamepad popover); never sent to pages.
#[derive(Clone, Serialize)]
pub struct PadInfo {
    pub index: usize,
    pub name: String,
    pub mapping: &'static str,
}

pub struct Bridge {
    stop: Arc<AtomicBool>,
    /// Updates are wanted (focused window, active tab allowed to use gamepads).
    active: Arc<AtomicBool>,
    /// Send the full state once even without new input (new page or tab).
    resync: Arc<AtomicBool>,
    pads: Arc<Mutex<Vec<PadInfo>>>,
    worker: Option<JoinHandle<()>>,
}

impl Bridge {
    pub fn start(proxy: EventLoopProxy<UserEvent>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let active = Arc::new(AtomicBool::new(false));
        let resync = Arc::new(AtomicBool::new(false));
        let pads = Arc::new(Mutex::new(Vec::new()));
        let w = Worker {
            stop: stop.clone(),
            active: active.clone(),
            resync: resync.clone(),
            pads: pads.clone(),
            proxy,
        };
        let worker = thread::Builder::new()
            .name("crow-gamepad".into())
            .spawn(move || w.run())
            .map_err(|e| eprintln!("gamepad: worker not started: {e}"))
            .ok();
        Self { stop, active, resync, pads, worker }
    }

    pub fn set_active(&self, on: bool) {
        if !self.active.swap(on, Ordering::AcqRel) && on {
            self.resync();
        }
    }

    pub fn resync(&self) {
        self.resync.store(true, Ordering::Release);
    }

    /// Connected controllers with their real names.
    pub fn pads(&self) -> Vec<PadInfo> {
        self.pads.lock().map(|p| p.clone()).unwrap_or_default()
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}

struct Worker {
    stop: Arc<AtomicBool>,
    active: Arc<AtomicBool>,
    resync: Arc<AtomicBool>,
    pads: Arc<Mutex<Vec<PadInfo>>>,
    proxy: EventLoopProxy<UserEvent>,
}

impl Worker {
    fn run(self) {
        let mut gilrs = match Gilrs::new() {
            Ok(g) => g,
            Err(e) => return eprintln!("gamepad: {e}"),
        };
        // W3C indices: lowest free slot on connect, kept until that controller disconnects.
        let mut slots: Vec<Option<GamepadId>> = Vec::new();
        for (id, _) in gilrs.gamepads() {
            slot_of(&mut slots, id);
        }
        self.publish(&gilrs, &slots);
        let mut last: Option<Vec<GamepadSnapshot>> = None;
        let mut sent = Instant::now().checked_sub(FRAME).unwrap_or_else(Instant::now);
        let mut dirty = false;
        while !self.stop.load(Ordering::Acquire) {
            let wait = if dirty { FRAME.saturating_sub(sent.elapsed()) } else { IDLE };
            let mut next = gilrs.next_event_blocking(Some(wait));
            while let Some(ev) = next {
                match ev.event {
                    EventType::Connected => {
                        slot_of(&mut slots, ev.id);
                        self.publish(&gilrs, &slots);
                    }
                    EventType::Disconnected => {
                        if let Some(s) = slots.iter_mut().find(|s| **s == Some(ev.id)) {
                            *s = None;
                        }
                        while slots.last().is_some_and(Option::is_none) {
                            slots.pop();
                        }
                        self.publish(&gilrs, &slots);
                    }
                    _ => {}
                }
                dirty = true;
                next = gilrs.next_event();
            }
            if self.resync.swap(false, Ordering::AcqRel) {
                last = None;
                dirty = true;
            }
            if !dirty || sent.elapsed() < FRAME {
                continue;
            }
            dirty = false;
            if !self.active.load(Ordering::Acquire) {
                continue;
            }
            let state = snapshot(&gilrs, &slots);
            if last.as_ref() == Some(&state) {
                continue;
            }
            let Ok(json) = serde_json::to_string(&state) else { continue };
            sent = Instant::now();
            last = Some(state);
            if self.proxy.send_event(UserEvent::Gamepads(json)).is_err() {
                break; // event loop gone
            }
        }
    }

    fn publish(&self, gilrs: &Gilrs, slots: &[Option<GamepadId>]) {
        let list = slots
            .iter()
            .enumerate()
            .filter_map(|(index, id)| {
                let g = gilrs.connected_gamepad((*id)?)?;
                Some(PadInfo { index, name: g.name().to_string(), mapping: mapping_of(g.mapping_source()) })
            })
            .collect();
        if let Ok(mut p) = self.pads.lock() {
            *p = list;
        }
    }
}

fn slot_of(slots: &mut Vec<Option<GamepadId>>, id: GamepadId) {
    if slots.contains(&Some(id)) {
        return;
    }
    match slots.iter_mut().find(|s| s.is_none()) {
        Some(s) => *s = Some(id),
        None => slots.push(Some(id)),
    }
}

/// "standard" only when the layout is known (SDL mapping or a driver with a unified layout).
fn mapping_of(m: MappingSource) -> &'static str {
    if m == MappingSource::None { "" } else { "standard" }
}

/// Values rounded to 1/1000: jitter below that is no change.
fn round(v: f32) -> f32 {
    (v * 1000.0).round() / 1000.0
}

fn snapshot(gilrs: &Gilrs, slots: &[Option<GamepadId>]) -> Vec<GamepadSnapshot> {
    slots
        .iter()
        .enumerate()
        .filter_map(|(index, id)| {
            let g = gilrs.connected_gamepad((*id)?)?;
            let buttons = BUTTONS
                .iter()
                .map(|&b| {
                    let (value, on) = g.button_data(b).map_or((0.0, false), |d| (round(d.value().clamp(0.0, 1.0)), d.is_pressed()));
                    let pressed = on || value >= PRESS;
                    GamepadButtonSnapshot { pressed, touched: pressed || value > 0.0, value }
                })
                .collect();
            let axes = AXES
                .iter()
                .map(|&(a, flip)| {
                    let v = round(g.value(a).clamp(-1.0, 1.0));
                    if flip && v != 0.0 { -v } else { v }
                })
                .collect();
            Some(GamepadSnapshot {
                index,
                id: PAD_ID.to_string(),
                connected: true,
                mapping: mapping_of(g.mapping_source()),
                buttons,
                axes,
            })
        })
        .collect()
}
