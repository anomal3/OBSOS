//! OBSOS для Windows и Linux: окно winit.
//!
//! Рисует всё `obsos-core`: видеокартой прямо в окно (Windows) или программно,
//! как на FossAi, а кадр тогда показывает softbuffer; здесь только окно, перевод событий winit в события окна FossAi
//! (`obsos_core::window::WinEvent`) и службы системы (`platform.rs`).
//!
//! `obsos [--profile <имя>] [--private] [--console] [адрес]`

// Без чёрного окна консоли рядом с браузером; журнал — в файл или туда, куда
// перенаправлен вывод (`output.rs`).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod output;
mod platform;

// Подсказка драйверам NVIDIA/AMD: рисовать на дискретной видеокарте.
surfman::declare_surfman!();

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::Duration;

use obsos_core::window::*;
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};
use winit::platform::pump_events::{EventLoopExtPumpEvents, PumpStatus};
use winit::raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use winit::window::{CursorIcon, Window as WinitWindow, WindowAttributes, WindowId};

/// Приёмник событий winit: события каждого окна копятся в своей очереди в
/// кодировке окна FossAi; окна создаются по запросу из цикла событий.
#[derive(Default)]
struct Handler {
    /// Какое окно создать.
    wanted: Option<WindowAttributes>,
    created: Option<Rc<WinitWindow>>,
    windows: HashMap<WindowId, Queue>,
}

impl Handler {
    fn create(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(attributes) = self.wanted.take() {
            match event_loop.create_window(attributes) {
                Ok(w) => {
                    self.windows.insert(w.id(), Queue::default());
                    self.created = Some(Rc::new(w));
                }
                Err(e) => println!("browser: window open failed: {e}"),
            }
        }
    }
}

/// Очередь и состояние ввода одного окна.
#[derive(Default)]
struct Queue {
    queue: VecDeque<WinEvent>,
    mods: ModifiersState,
    pointer: (i32, i32),
    /// Остаток прокрутки тачпада в точках (строка — 40 точек).
    wheel_rest: f64,
    /// Дробные строки колёс с мелким шагом (по вертикали и горизонтали).
    line_rest: (f32, f32),
    /// Окно надо перерисовать (развернули, вернули на передний план).
    exposed: bool,
    /// Окно получило ввод.
    focused: bool,
    /// Цикл событий закончился — `WIN_CLOSE` уже в очереди.
    closed: bool,
}

impl Queue {
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

/// Shift/Ctrl/Alt, зажатые прямо сейчас (маска `WinEvent`): Ctrl+колесо
/// должно работать, даже если нажатие клавиши досталось не нашему окну.
#[cfg(windows)]
fn held_modifiers() -> Option<i32> {
    #[link(name = "user32")]
    extern "system" {
        fn GetAsyncKeyState(key: i32) -> i16;
    }
    // SAFETY: функция только читает состояние клавиатуры.
    let down = |key: i32| unsafe { GetAsyncKeyState(key) } < 0;
    // VK_SHIFT, VK_CONTROL, VK_MENU.
    Some(down(0x10) as i32 | (down(0x11) as i32) << 1 | (down(0x12) as i32) << 2)
}

#[cfg(not(windows))]
fn held_modifiers() -> Option<i32> {
    None
}

impl ApplicationHandler for Handler {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.create(event_loop);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.create(event_loop);
    }

    fn window_event(&mut self, _event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if let Some(queue) = self.windows.get_mut(&id) {
            queue.event(event);
        }
    }
}

impl Queue {
    fn event(&mut self, event: WindowEvent) {
        if matches!(event, WindowEvent::Focused(true)) {
            self.focused = true;
        }
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
                    MouseButton::Back => 4,
                    MouseButton::Forward => 5,
                    _ => return,
                };
                let kind = if state == ElementState::Pressed { WIN_POINTER } else { WIN_RELEASE };
                self.push(kind, code, self.pointer.0, self.pointer.1);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let (lines, across) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => {
                        // Колёса с мелким шагом шлют доли строки — копим, а не
                        // отбрасываем.
                        self.line_rest.0 += y;
                        self.line_rest.1 += x;
                        let (lines, across) = (self.line_rest.0.trunc(), self.line_rest.1.trunc());
                        self.line_rest = (self.line_rest.0 - lines, self.line_rest.1 - across);
                        (lines as i32, across as i32)
                    }
                    MouseScrollDelta::PixelDelta(p) => {
                        self.wheel_rest += p.y;
                        let lines = (self.wheel_rest / 40.0).trunc();
                        self.wheel_rest -= lines * 40.0;
                        (lines as i32, (p.x / 40.0).round() as i32)
                    }
                };
                if lines != 0 || across != 0 {
                    let mods = WIN_WHEEL_MODS | held_modifiers().unwrap_or_else(|| self.mask()) as u32;
                    self.push(WIN_WHEEL, mods, lines, across);
                }
            }
            WindowEvent::ModifiersChanged(m) => self.mods = m.state(),
            WindowEvent::RedrawRequested | WindowEvent::Occluded(false) | WindowEvent::Focused(true) => self.exposed = true,
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

/// Значок окна и панели задач (assets/icons).
fn window_icon() -> Option<winit::window::Icon> {
    let decoder = png::Decoder::new(std::io::Cursor::new(&include_bytes!("../../assets/icons/obsos-64.png")[..]));
    let mut reader = decoder.read_info().ok()?;
    let mut data = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut data).ok()?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    data.truncate(info.buffer_size());
    winit::window::Icon::from_rgba(data, info.width, info.height).ok()
}

/// Общий на все окна цикл событий (winit даёт только один).
struct Shared {
    event_loop: RefCell<EventLoop<()>>,
    handler: RefCell<Handler>,
}

impl Shared {
    fn pump(&self, timeout: Duration) {
        let mut handler = self.handler.borrow_mut();
        let status = self.event_loop.borrow_mut().pump_app_events(Some(timeout), &mut *handler);
        if let PumpStatus::Exit(_) = status {
            for queue in handler.windows.values_mut() {
                if !std::mem::replace(&mut queue.closed, true) {
                    queue.push(WIN_CLOSE, 0, 0, 0);
                }
            }
        }
    }

    /// Создать окно: дождаться, пока цикл событий это сделает.
    fn create(&self, attributes: WindowAttributes) -> Option<Rc<WinitWindow>> {
        self.handler.borrow_mut().wanted = Some(attributes);
        for _ in 0..100 {
            self.pump(Duration::from_millis(10));
            if let Some(window) = self.handler.borrow_mut().created.take() {
                return Some(window);
            }
        }
        None
    }
}

/// Окно OBSOS на winit.
struct DesktopWindow {
    shared: Rc<Shared>,
    window: Rc<WinitWindow>,
    /// Только для программного вывода — создаётся при первом кадре: окну,
    /// в которое рисует видеокарта, он не нужен.
    surface: RefCell<Option<softbuffer::Surface<Rc<WinitWindow>, Rc<WinitWindow>>>>,
    width: Cell<u32>,
    height: Cell<u32>,
    cursor: Cell<CursorShape>,
}

impl DesktopWindow {
    /// Первое окно и общий цикл событий.
    fn open(title: &str, width: u32, height: u32) -> Option<DesktopWindow> {
        let event_loop = EventLoop::new().map_err(|e| println!("browser: event loop: {e}")).ok()?;
        let shared = Rc::new(Shared { event_loop: RefCell::new(event_loop), handler: Default::default() });
        DesktopWindow::on(shared, title, width, height, None)
    }

    /// Окно в общем цикле событий.
    fn on(shared: Rc<Shared>, title: &str, width: u32, height: u32, at: Option<(i32, i32)>) -> Option<DesktopWindow> {
        let mut attributes = WindowAttributes::default().with_title(title).with_inner_size(PhysicalSize::new(width, height)).with_window_icon(window_icon());
        if let Some((x, y)) = at {
            attributes = attributes.with_position(PhysicalPosition::new(x, y));
        }
        let window = shared.create(attributes)?;
        let size = window.inner_size();
        // Размер, который уже есть, — не событие.
        if let Some(queue) = shared.handler.borrow_mut().windows.get_mut(&window.id()) {
            queue.queue.retain(|e| e.kind != WIN_RESIZE);
        }
        println!("browser: window {}x{} (scale {})", size.width, size.height, window.scale_factor());
        Some(DesktopWindow {
            shared,
            window,
            surface: RefCell::new(None),
            width: Cell::new(size.width.max(1)),
            height: Cell::new(size.height.max(1)),
            cursor: Cell::new(CursorShape::Arrow),
        })
    }

    /// Очередь этого окна.
    fn queue<T>(&self, f: impl FnOnce(&mut Queue) -> T) -> Option<T> {
        self.shared.handler.borrow_mut().windows.get_mut(&self.window.id()).map(f)
    }

    fn resize_surface(&self) -> bool {
        let (Some(w), Some(h)) = (NonZeroU32::new(self.width.get()), NonZeroU32::new(self.height.get())) else { return false };
        match self.surface.borrow_mut().as_mut() {
            Some(surface) => surface.resize(w, h).is_ok(),
            None => true,
        }
    }

    fn make_surface(&self) {
        if self.surface.borrow().is_some() {
            return;
        }
        let made = softbuffer::Context::new(self.window.clone()).and_then(|context| softbuffer::Surface::new(&context, self.window.clone()));
        match made {
            Ok(surface) => {
                *self.surface.borrow_mut() = Some(surface);
                self.resize_surface();
            }
            Err(e) => println!("browser: softbuffer: {e}"),
        }
    }
}

impl Drop for DesktopWindow {
    fn drop(&mut self) {
        if let Ok(mut handler) = self.shared.handler.try_borrow_mut() {
            handler.windows.remove(&self.window.id());
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
        if self.queue(|q| q.queue.is_empty()) == Some(true) {
            self.shared.pump(Duration::ZERO);
        }
        self.queue(|q| q.queue.pop_front()).flatten()
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
        self.make_surface();
        let mut surface = self.surface.borrow_mut();
        let Some(surface) = surface.as_mut() else { return };
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
        self.shared.pump(time);
    }

    fn scale_factor(&self) -> f32 {
        self.window.scale_factor() as f32
    }

    fn geometry(&self) -> Option<(u32, u32, bool)> {
        let size = self.window.inner_size();
        (size.width > 0 && size.height > 0 && !self.window.is_minimized().unwrap_or(false)).then(|| (size.width, size.height, self.window.is_maximized()))
    }

    fn set_geometry(&self, width: u32, height: u32, maximized: bool) {
        // Ответ придёт событием `Resized` — поверхность поменяет цикл.
        let _ = self.window.request_inner_size(PhysicalSize::new(width, height));
        if maximized {
            self.window.set_maximized(true);
        }
    }

    fn set_fullscreen(&self, on: bool) {
        self.window.set_fullscreen(on.then(|| winit::window::Fullscreen::Borderless(None)));
    }

    fn gl_handles(&self) -> Option<(RawDisplayHandle, RawWindowHandle)> {
        Some((self.window.display_handle().ok()?.as_raw(), self.window.window_handle().ok()?.as_raw()))
    }

    fn refresh_hz(&self) -> u32 {
        let monitor = self.window.current_monitor().or_else(|| self.window.primary_monitor());
        monitor.and_then(|m| m.refresh_rate_millihertz()).map(|m| (m + 500) / 1000).filter(|hz| *hz >= 30).unwrap_or(60)
    }

    fn take_exposed(&self) -> bool {
        self.queue(|q| std::mem::take(&mut q.exposed)).unwrap_or(false)
    }

    fn open_window(&self, title: &str, width: u32, height: u32, at: Option<(i32, i32)>) -> Option<Rc<Window>> {
        DesktopWindow::on(self.shared.clone(), title, width, height, at).map(|w| Rc::new(w) as Rc<Window>)
    }

    fn inner_origin(&self) -> Option<(i32, i32)> {
        self.window.inner_position().ok().map(|p| (p.x, p.y))
    }

    fn set_position(&self, x: i32, y: i32) {
        self.window.set_outer_position(PhysicalPosition::new(x, y));
    }

    fn focus(&self) {
        self.window.focus_window();
    }

    fn take_focused(&self) -> bool {
        self.queue(|q| std::mem::take(&mut q.focused)).unwrap_or(false)
    }

    fn waker(&self) -> Option<Box<dyn Fn() + Send + Sync>> {
        // Пустое событие прерывает ожидание `pump_app_events`.
        let proxy = self.shared.event_loop.borrow().create_proxy();
        Some(Box::new(move || {
            let _ = proxy.send_event(());
        }))
    }
}

/// Паузы главного цикла и таймеры — с точностью 1 мс: по умолчанию Windows
/// округляет их до 15,6 мс, и анимации оболочки шли рывками.
#[cfg(windows)]
fn fine_timer() {
    #[link(name = "winmm")]
    extern "system" {
        fn timeBeginPeriod(period: u32) -> u32;
    }
    // SAFETY: просьба о точности таймера; система снимает её с процессом.
    unsafe { timeBeginPeriod(1) };
}

#[cfg(not(windows))]
fn fine_timer() {}

fn main() {
    // Дом OBSOS (профили — `<дом>/.config/obsos`, загрузки — `<дом>/Downloads`):
    // на Windows `HOME` обычно нет — папка пользователя.
    if std::env::var_os("HOME").is_none() {
        if let Some(profile) = std::env::var_os("USERPROFILE") {
            // SAFETY (edition 2021: безопасно): до запуска любых потоков.
            std::env::set_var("HOME", profile);
        }
    }
    // До первой строки журнала.
    output::setup();
    fine_timer();
    obsos_core::window::install(Box::new(platform::Desktop::new()));
    obsos_core::run(|title, width, height| DesktopWindow::open(title, width, height).map(|w| Rc::new(w) as Rc<Window>))
}
