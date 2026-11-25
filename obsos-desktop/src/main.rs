//! OBSOS для Windows и Linux: окно winit, кадр — softbuffer.
//!
//! Рисует всё `obsos-core` (страницу — swgl, оболочку — egui в пиксели), как
//! на FossAi; здесь только окно, перевод событий winit в события окна FossAi
//! (`obsos_core::window::WinEvent`) и службы системы (`platform.rs`).
//!
//! `obsos [--profile <имя>] [--private] [адрес]`

mod platform;

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::Duration;

use obsos_core::window::*;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};
use winit::platform::pump_events::{EventLoopExtPumpEvents, PumpStatus};
use winit::window::{CursorIcon, Window as WinitWindow, WindowAttributes, WindowId};

/// Приёмник событий winit: окно создаётся в `resumed`, события копятся в
/// очереди в кодировке окна FossAi.
struct Handler {
    attributes: Option<WindowAttributes>,
    window: Option<Rc<WinitWindow>>,
    queue: VecDeque<WinEvent>,
    mods: ModifiersState,
    pointer: (i32, i32),
    /// Остаток прокрутки тачпада в точках (строка — 40 точек).
    wheel_rest: f64,
}

impl Handler {
    fn mask(&self) -> i32 {
        (self.mods.shift_key() as i32) | ((self.mods.control_key() as i32) << 1) | ((self.mods.alt_key() as i32) << 2)
    }

    fn push(&mut self, kind: u32, code: u32, x: i32, y: i32) {
        self.queue.push_back(WinEvent { kind, code, x, y });
    }
}

/// Номер именованной клавиши в `WinEvent` (`WIN_KEY_NAMED + n`) или знак.
fn key_code(key: &Key, text: Option<&str>) -> Option<u32> {
    let named = |n: u32| Some(WIN_KEY_NAMED + n);
    match key {
        Key::Named(k) => match k {
            NamedKey::ArrowLeft => named(1),
            NamedKey::ArrowRight => named(2),
            NamedKey::ArrowUp => named(3),
            NamedKey::ArrowDown => named(4),
            NamedKey::Home => named(5),
            NamedKey::End => named(6),
            NamedKey::PageUp => named(7),
            NamedKey::PageDown => named(8),
            NamedKey::Delete => named(9),
            NamedKey::Insert => named(11),
            NamedKey::F1 => named(12),
            NamedKey::F2 => named(13),
            NamedKey::F3 => named(14),
            NamedKey::F4 => named(15),
            NamedKey::F5 => named(16),
            NamedKey::F6 => named(17),
            NamedKey::F7 => named(18),
            NamedKey::F8 => named(19),
            NamedKey::F9 => named(20),
            NamedKey::F10 => named(21),
            NamedKey::F11 => named(22),
            NamedKey::F12 => named(23),
            NamedKey::Shift => named(24),
            NamedKey::Control => named(25),
            NamedKey::Alt | NamedKey::AltGraph => named(26),
            NamedKey::Backspace => Some(8),
            NamedKey::Tab => Some(9),
            NamedKey::Enter => Some(13),
            NamedKey::Escape => Some(27),
            NamedKey::Space => Some(' ' as u32),
            _ => None,
        },
        Key::Character(s) => s.chars().next().or_else(|| text.and_then(|t| t.chars().next())).map(|c| c as u32),
        _ => None,
    }
}

/// Латинский знак физической клавиши (сочетания Ctrl+буква в любой раскладке).
fn latin(key: PhysicalKey) -> u32 {
    use KeyCode as K;
    let PhysicalKey::Code(code) = key else { return 0 };
    let c = match code {
        K::KeyA => 'a',
        K::KeyB => 'b',
        K::KeyC => 'c',
        K::KeyD => 'd',
        K::KeyE => 'e',
        K::KeyF => 'f',
        K::KeyG => 'g',
        K::KeyH => 'h',
        K::KeyI => 'i',
        K::KeyJ => 'j',
        K::KeyK => 'k',
        K::KeyL => 'l',
        K::KeyM => 'm',
        K::KeyN => 'n',
        K::KeyO => 'o',
        K::KeyP => 'p',
        K::KeyQ => 'q',
        K::KeyR => 'r',
        K::KeyS => 's',
        K::KeyT => 't',
        K::KeyU => 'u',
        K::KeyV => 'v',
        K::KeyW => 'w',
        K::KeyX => 'x',
        K::KeyY => 'y',
        K::KeyZ => 'z',
        K::Digit0 => '0',
        K::Digit1 => '1',
        K::Digit2 => '2',
        K::Digit3 => '3',
        K::Digit4 => '4',
        K::Digit5 => '5',
        K::Digit6 => '6',
        K::Digit7 => '7',
        K::Digit8 => '8',
        K::Digit9 => '9',
        K::Minus | K::NumpadSubtract => '-',
        K::Equal | K::NumpadAdd => '=',
        K::BracketLeft => '[',
        K::BracketRight => ']',
        K::Semicolon => ';',
        K::Quote => '\'',
        K::Comma => ',',
        K::Period => '.',
        K::Slash => '/',
        K::Backslash => '\\',
        K::Backquote => '`',
        _ => return 0,
    };
    c as u32
}

impl ApplicationHandler for Handler {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        if let Some(attributes) = self.attributes.take() {
            match event_loop.create_window(attributes) {
                Ok(w) => self.window = Some(Rc::new(w)),
                Err(e) => println!("browser: window open failed: {e}"),
            }
        }
    }

    fn window_event(&mut self, _event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => self.push(WIN_CLOSE, 0, 0, 0),
            WindowEvent::Resized(size) => {
                if size.width > 0 && size.height > 0 {
                    self.push(WIN_RESIZE, 0, size.width as i32, size.height as i32);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.pointer = (position.x as i32, position.y as i32);
                self.push(WIN_MOVE, 0, self.pointer.0, self.pointer.1);
            }
            WindowEvent::CursorLeft { .. } => self.push(WIN_LEAVE, 0, 0, 0),
            WindowEvent::MouseInput { state, button, .. } => {
                let code = match button {
                    MouseButton::Left => 1,
                    MouseButton::Right => 2,
                    MouseButton::Middle => 3,
                    _ => return,
                };
                let kind = if state == ElementState::Pressed { WIN_POINTER } else { WIN_RELEASE };
                self.push(kind, code, self.pointer.0, self.pointer.1);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y.round() as i32,
                    MouseScrollDelta::PixelDelta(p) => {
                        self.wheel_rest += p.y;
                        let lines = (self.wheel_rest / 40.0).trunc();
                        self.wheel_rest -= lines * 40.0;
                        lines as i32
                    }
                };
                if lines != 0 {
                    self.push(WIN_WHEEL, 0, lines, 0);
                }
            }
            WindowEvent::ModifiersChanged(m) => self.mods = m.state(),
            WindowEvent::KeyboardInput { event, .. } => {
                let kind = if event.state == ElementState::Pressed { WIN_KEY } else { WIN_KEYUP };
                let Some(mut code) = key_code(&event.logical_key, event.text.as_deref()) else { return };
                let latin = latin(event.physical_key);
                // Ctrl+буква — управляющий код, как у окна FossAi.
                if self.mods.control_key() && code < WIN_KEY_NAMED {
                    if let Some(c) = char::from_u32(latin).filter(char::is_ascii_lowercase) {
                        code = c as u32 - 'a' as u32 + 1;
                    }
                }
                let mask = self.mask();
                self.push(kind, code, mask, latin as i32);
            }
            _ => {}
        }
    }
}

/// Окно OBSOS на winit.
struct DesktopWindow {
    event_loop: RefCell<EventLoop<()>>,
    handler: RefCell<Handler>,
    window: Rc<WinitWindow>,
    surface: RefCell<softbuffer::Surface<Rc<WinitWindow>, Rc<WinitWindow>>>,
    width: Cell<u32>,
    height: Cell<u32>,
    cursor: Cell<CursorShape>,
    closed: Cell<bool>,
}

impl DesktopWindow {
    fn open(title: &str, width: u32, height: u32) -> Option<DesktopWindow> {
        let mut event_loop = EventLoop::new().map_err(|e| println!("browser: event loop: {e}")).ok()?;
        let attributes = WindowAttributes::default().with_title(title).with_inner_size(PhysicalSize::new(width, height));
        let mut handler = Handler {
            attributes: Some(attributes),
            window: None,
            queue: VecDeque::new(),
            mods: ModifiersState::empty(),
            pointer: (0, 0),
            wheel_rest: 0.0,
        };
        // Окно появляется в `resumed` первого витка.
        for _ in 0..100 {
            let _ = event_loop.pump_app_events(Some(Duration::from_millis(10)), &mut handler);
            if handler.window.is_some() {
                break;
            }
        }
        let window = handler.window.clone()?;
        let context = softbuffer::Context::new(window.clone()).map_err(|e| println!("browser: softbuffer: {e}")).ok()?;
        let surface = softbuffer::Surface::new(&context, window.clone()).map_err(|e| println!("browser: surface: {e}")).ok()?;
        let size = window.inner_size();
        // Размер, который уже есть, — не событие.
        handler.queue.retain(|e| e.kind != WIN_RESIZE);
        println!("browser: window {}x{} (scale {})", size.width, size.height, window.scale_factor());
        let this = DesktopWindow {
            event_loop: RefCell::new(event_loop),
            handler: RefCell::new(handler),
            window,
            surface: RefCell::new(surface),
            width: Cell::new(size.width.max(1)),
            height: Cell::new(size.height.max(1)),
            cursor: Cell::new(CursorShape::Arrow),
            closed: Cell::new(false),
        };
        this.resize_surface();
        Some(this)
    }

    fn resize_surface(&self) -> bool {
        let (Some(w), Some(h)) = (NonZeroU32::new(self.width.get()), NonZeroU32::new(self.height.get())) else { return false };
        self.surface.borrow_mut().resize(w, h).is_ok()
    }

    fn pump(&self, timeout: Duration) {
        let mut handler = self.handler.borrow_mut();
        let status = self.event_loop.borrow_mut().pump_app_events(Some(timeout), &mut *handler);
        if let PumpStatus::Exit(_) = status {
            if !self.closed.replace(true) {
                handler.push(WIN_CLOSE, 0, 0, 0);
            }
        }
    }
}

impl HostWindow for DesktopWindow {
    fn width(&self) -> u32 {
        self.width.get()
    }

    fn height(&self) -> u32 {
        self.height.get()
    }

    fn swap_rb(&self) -> bool {
        // softbuffer: 0x00RRGGBB.
        false
    }

    fn next_event(&self) -> Option<WinEvent> {
        if self.handler.borrow().queue.is_empty() {
            self.pump(Duration::ZERO);
        }
        self.handler.borrow_mut().queue.pop_front()
    }

    fn resize(&self, width: u32, height: u32) -> bool {
        self.width.set(width.max(1));
        self.height.set(height.max(1));
        self.resize_surface()
    }

    fn set_cursor(&self, shape: CursorShape) {
        if self.cursor.replace(shape) != shape {
            self.window.set_cursor(match shape {
                CursorShape::Arrow => CursorIcon::Default,
                CursorShape::Text => CursorIcon::Text,
                CursorShape::Hand => CursorIcon::Pointer,
                CursorShape::Move => CursorIcon::Move,
                CursorShape::NotAllowed => CursorIcon::NotAllowed,
                CursorShape::ResizeEw => CursorIcon::EwResize,
                CursorShape::ResizeNs => CursorIcon::NsResize,
            });
        }
    }

    fn set_title(&self, title: &str) {
        self.window.set_title(title);
    }

    fn present(&self, draw: &mut dyn FnMut(&mut [u32])) {
        let mut surface = self.surface.borrow_mut();
        let Ok(mut buffer) = surface.buffer_mut() else { return };
        let len = self.width.get() as usize * self.height.get() as usize;
        if buffer.len() < len {
            return;
        }
        draw(&mut buffer[..len]);
        // softbuffer: старший байт — ноль (прозрачность окна не наша).
        for px in buffer.iter_mut() {
            *px &= 0x00ff_ffff;
        }
        self.window.pre_present_notify();
        let _ = buffer.present();
    }

    fn close(&self) {
        self.window.set_visible(false);
    }

    fn idle(&self, time: Duration) {
        // Ждать событий окна, а не просто спать: ввод приходит сразу.
        self.pump(time);
    }
}

fn main() {
    // Дом OBSOS (профили — `<дом>/.config/obsos`, загрузки — `<дом>/Downloads`):
    // на Windows `HOME` обычно нет — папка пользователя.
    if std::env::var_os("HOME").is_none() {
        if let Some(profile) = std::env::var_os("USERPROFILE") {
            // SAFETY (edition 2021: безопасно): до запуска любых потоков.
            std::env::set_var("HOME", profile);
        }
    }
    obsos_core::window::install(Box::new(platform::Desktop::new()));
    obsos_core::run(|title, width, height| DesktopWindow::open(title, width, height).map(|w| Rc::new(w) as Rc<Window>))
}
