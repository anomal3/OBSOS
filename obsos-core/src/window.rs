//! Договор с платформой: окно, в которое OBSOS рисует, и службы системы
//! (буфер обмена, окна выбора файла, запуск программ).
//!
//! OBSOS рисует сам и целиком программно: страницу — swgl (программный
//! OpenGL из Firefox), оболочку — egui прямо в пиксели. От платформы нужна
//! только поверхность окна (`u32` на точку, строки сверху вниз) и события в
//! кодировке окна FossAi (`WinEvent`, user-abi): так оболочка одинакова на
//! FossAi, Windows и Linux. Реализации — `obsos-fossai` (окно стола FossAi) и
//! `obsos-desktop` (winit + softbuffer).

use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use egui_software_backend::{BufferMutRef, EguiSoftwareRender};

/// Событие окна — как `WinEvent` user-abi FossAi.
///
/// * `WIN_KEY`/`WIN_KEYUP`: `code` — знак (Unicode) или `WIN_KEY_NAMED + n`
///   (1..4 стрелки ←→↑↓, 5 Home, 6 End, 7 PgUp, 8 PgDn, 9 Delete, 11 Insert,
///   12..23 F1..F12, 24 Shift, 25 Ctrl, 26 Alt); Ctrl+буква — управляющий код
///   1..26; `x` — модификаторы (1 Shift, 2 Ctrl, 4 Alt); `y` — латинский знак
///   физической клавиши (сочетания в любой раскладке).
/// * `WIN_POINTER`/`WIN_RELEASE`: `code` — кнопка (1 левая, 2 правая,
///   3 средняя), `x`,`y` — точка окна. `WIN_MOVE` — `x`,`y`.
/// * `WIN_WHEEL`: `x` — строки (вверх — больше нуля), `y` — по горизонтали;
///   `code` — `WIN_WHEEL_MODS | модификаторы`, если окно знает, что зажато в
///   момент прокрутки, иначе 0 (тогда — по последнему событию клавиши).
/// * `WIN_RESIZE`: `x`,`y` — новый размер (окно меняет поверхность по
///   `HostWindow::resize`).
#[repr(C)]
#[derive(Default, Clone, Copy, Debug)]
pub struct WinEvent {
    pub kind: u32,
    pub code: u32,
    pub x: i32,
    pub y: i32,
}

pub const WIN_KEY: u32 = 1;
pub const WIN_POINTER: u32 = 2;
pub const WIN_CLOSE: u32 = 3;
pub const WIN_MOVE: u32 = 4;
pub const WIN_LEAVE: u32 = 5;
pub const WIN_RELEASE: u32 = 6;
pub const WIN_RESIZE: u32 = 7;
pub const WIN_KEYUP: u32 = 9;
pub const WIN_WHEEL: u32 = 10;
pub const WIN_KEY_NAMED: u32 = 0x0100_0000;
pub const WIN_WHEEL_MODS: u32 = 0x100;

/// Форма указателя над окном (`WIN_CURSOR_*` FossAi).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CursorShape {
    Arrow = 0,
    Text = 1,
    Hand = 2,
    Move = 3,
    NotAllowed = 4,
    ResizeEw = 5,
    ResizeNs = 6,
}

/// Окно платформы.
pub trait HostWindow {
    fn width(&self) -> u32;
    fn height(&self) -> u32;
    /// Красный в младшем байте точки (байты R,G,B,A); иначе B,G,R,A
    /// (`0xAARRGGBB`, как у Windows и softbuffer).
    fn swap_rb(&self) -> bool;
    /// Следующее событие, не дожидаясь (`None` — очередь пуста).
    fn next_event(&self) -> Option<WinEvent>;
    /// Поменять размер поверхности (после `WIN_RESIZE`).
    fn resize(&self, width: u32, height: u32) -> bool;
    /// Номер окна у системы (владелец модальных окон); 0 — нет такого.
    fn id(&self) -> i64 {
        0
    }
    fn set_cursor(&self, shape: CursorShape);
    fn set_title(&self, title: &str);
    /// Дать нарисовать кадр на поверхности (`width × height` точек, строки
    /// подряд) и показать его.
    fn present(&self, draw: &mut dyn FnMut(&mut [u32]));
    fn close(&self);
    /// Пауза витка цикла, когда делать нечего.
    fn idle(&self, time: Duration) {
        std::thread::sleep(time);
    }
    /// Будильник главного цикла: прерывает `idle` из другого потока (Servo
    /// зовёт его, когда есть сообщения). `None` — окно так не умеет.
    fn waker(&self) -> Option<Box<dyn Fn() + Send + Sync>> {
        None
    }
    /// Физических точек в точке интерфейса по настройкам экрана системы.
    fn scale_factor(&self) -> f32 {
        1.0
    }
    /// Окно на весь экран без рамки (F11, видео).
    fn set_fullscreen(&self, _on: bool) {}
    /// Размер и «развёрнуто» — чтобы открыться так же в следующий раз.
    fn geometry(&self) -> Option<(u32, u32, bool)> {
        None
    }
    fn set_geometry(&self, _width: u32, _height: u32, _maximized: bool) {}
    /// Дескрипторы окна для рисования видеокартой; `None` — платформа этого
    /// не умеет (тогда всё рисуется программно через `present`).
    fn gl_handles(&self) -> Option<(raw_window_handle::RawDisplayHandle, raw_window_handle::RawWindowHandle)> {
        None
    }
    /// Частота экрана, на котором окно, Гц.
    fn refresh_hz(&self) -> u32 {
        60
    }
    /// Систему попросила перерисовать окно (развернули, открыли из-под
    /// других) с прошлого вызова.
    fn take_exposed(&self) -> bool {
        false
    }
    /// Ещё одно окно того же процесса в точке экрана `at` (`None` — где
    /// решит система). `None` в ответ — платформа так не умеет.
    fn open_window(&self, _title: &str, _width: u32, _height: u32, _at: Option<(i32, i32)>) -> Option<std::rc::Rc<Window>> {
        None
    }
    /// Левый верхний угол содержимого окна на экране.
    fn inner_origin(&self) -> Option<(i32, i32)> {
        None
    }
    /// Передвинуть окно так, чтобы угол его рамки оказался в точке экрана.
    fn set_position(&self, _x: i32, _y: i32) {}
    fn focus(&self) {}
    /// Окно получило ввод с прошлого вызова.
    fn take_focused(&self) -> bool {
        false
    }
}

/// Окно, как его видит оболочка (`&Window`).
pub type Window = dyn HostWindow;

impl dyn HostWindow {
    /// Собрать кадр окна: копия кадра страницы в её прямоугольнике, холст
    /// egui поверх (панели непрозрачны, над страницей холст прозрачен).
    pub fn compose(&self, page: &PageFrame, at: (usize, usize), renderer: &mut EguiSoftwareRender) {
        let size = (self.width() as usize, self.height() as usize);
        self.present(&mut |surface: &mut [u32]| compose_into(surface, size, page, at, renderer));
    }
}

/// То же в любой буфер `ww × wh` (снимок окна для проверок).
pub fn compose_into(surface: &mut [u32], (ww, wh): (usize, usize), page: &PageFrame, at: (usize, usize), renderer: &mut EguiSoftwareRender) {
    if surface.len() < ww * wh {
        return;
    }
    let (px, py) = (at.0.min(ww), at.1.min(wh));
    let (w, h) = ((ww - px).min(page.width), (wh - py).min(page.height));
    let blank = 0xffff_ffff;
    for y in py..wh {
        let row = &mut surface[y * ww..][..ww];
        if y - py < h {
            row[px..px + w].copy_from_slice(&page.pixels[(y - py) * page.width..][..w]);
            row[px + w..].fill(blank);
        } else {
            row[px..].fill(blank);
        }
    }
    let surface = &mut surface[..ww * wh];
    // SAFETY: [u8; 4] и u32 одного размера; выравнивание u8 не строже.
    let bytes = unsafe { std::slice::from_raw_parts_mut(surface.as_mut_ptr() as *mut [u8; 4], surface.len()) };
    let mut buffer = BufferMutRef::new(bytes, ww, wh);
    renderer.blit_canvas_to_buffer(&mut buffer);
}

/// Последний кадр страницы — уже в формате точек окна, строки сверху вниз.
#[derive(Default)]
pub struct PageFrame {
    pub pixels: Vec<u32>,
    pub width: usize,
    pub height: usize,
    pub fresh: bool,
    pub count: u64,
}

impl PageFrame {
    /// Кадр swgl (BGRA, строки снизу вверх).
    pub fn store(&mut self, frame: &[u8], fw: u32, fh: u32, swap_rb: bool) {
        let (w, h) = (fw as usize, fh as usize);
        self.pixels.resize(w * h, 0xffff_ffff);
        self.width = w;
        self.height = h;
        for y in 0..h {
            let src = &frame[(h - 1 - y) * w * 4..][..w * 4];
            let dst = &mut self.pixels[y * w..][..w];
            // Байты BGRA — это ровно `0xAARRGGBB` в памяти (little-endian):
            // перекладывать по каналам нужно только для окна с RGBA.
            if swap_rb {
                for (px, s) in dst.iter_mut().zip(src.chunks_exact(4)) {
                    let v = u32::from_le_bytes([s[0], s[1], s[2], s[3]]);
                    *px = 0xff00_0000 | (v & 0x0000_ff00) | ((v & 0xff) << 16) | ((v >> 16) & 0xff);
                }
            } else {
                for (px, s) in dst.iter_mut().zip(src.chunks_exact(4)) {
                    *px = 0xff00_0000 | u32::from_le_bytes([s[0], s[1], s[2], s[3]]);
                }
            }
        }
        self.mark(w, h);
    }

    /// Кадр остался на видеокарте (`gpu_ctx`): окну нужно только знать о нём.
    pub fn mark(&mut self, w: usize, h: usize) {
        self.width = w;
        self.height = h;
        self.fresh = true;
        self.count += 1;
        if self.count <= 3 || self.count % 100 == 0 {
            println!("browser: frame {} presented {w}x{h}", self.count);
        }
    }
}

/// Чем кончилось окно выбора файла.
pub enum DialogAnswer {
    Chosen(String),
    Cancelled,
}

/// Окно выбора файла или папки, открытое платформой; ответ — не дожидаясь.
pub trait PendingDialog {
    /// `None` — окно ещё открыто.
    fn poll(&mut self) -> Option<DialogAnswer>;
}

pub type FileDialog = Box<dyn PendingDialog>;

/// Службы системы.
pub trait Platform: Send + Sync {
    fn clipboard_set(&self, text: &str);
    fn clipboard_get(&self) -> Option<String>;
    /// Смещение пояса от UTC в минутах, если `TZ` надо ставить самим
    /// (FossAi); `None` — SpiderMonkey узнает пояс сам.
    fn timezone_minutes(&self) -> Option<i32> {
        None
    }
    /// Смещение местного времени от UTC для показа дат оболочкой.
    fn utc_offset_minutes(&self) -> i32 {
        self.timezone_minutes().unwrap_or(0)
    }
    /// Ещё одно окно OBSOS с ключами (`--private`, `--profile <имя>`).
    fn new_window(&self, args: &[&str]) -> Result<(), String>;
    /// Показать файл или папку в файловом менеджере системы.
    fn show_in_folder(&self, path: &Path) -> Result<(), String>;
    /// Поставить скачанный пакет системы (FossAi: `.fpk`); `false` — не
    /// наш формат, файл просто показывается в папке.
    fn install_package(&self, _path: &Path) -> Option<Result<(), String>> {
        None
    }
    /// Есть ли внешний браузер «Открыть в Chromium».
    fn chromium_installed(&self) -> bool;
    fn open_in_chromium(&self, url: &str) -> Result<i64, String>;
    /// «Сохранить как»: папка, предложенное имя.
    fn save_dialog(&self, owner: &Window, title: &str, dir: &str, name: &str) -> Result<FileDialog, String>;
    /// «Выбор папки».
    fn folder_dialog(&self, owner: &Window, title: &str, dir: &str) -> Result<FileDialog, String>;
    /// Файл с лицензиями (страница obsos://licenses).
    fn notice_path(&self) -> String;
    /// Шрифт интерфейса системы (TTF) для оболочки; `None` — встроенный.
    fn ui_font(&self) -> Option<Vec<u8>> {
        None
    }
}

static PLATFORM: OnceLock<Box<dyn Platform>> = OnceLock::new();

/// Поставить службы платформы — до `app::run`.
pub fn install(platform: Box<dyn Platform>) {
    let _ = PLATFORM.set(platform);
}

pub fn platform() -> &'static dyn Platform {
    PLATFORM.get().expect("obsos_core::window::install before run").as_ref()
}

pub fn clipboard_set(text: &str) {
    platform().clipboard_set(text)
}

pub fn clipboard_get() -> Option<String> {
    platform().clipboard_get()
}

pub fn timezone_minutes() -> Option<i32> {
    platform().timezone_minutes()
}

pub fn chromium_installed() -> bool {
    platform().chromium_installed()
}

pub fn open_in_chromium(url: &str) -> Result<i64, String> {
    platform().open_in_chromium(url)
}

pub fn new_window(args: &[&str]) {
    if let Err(e) = platform().new_window(args) {
        println!("browser: new window {args:?} failed: {e}");
    }
}

pub fn show_in_folder(path: &Path) {
    if let Err(e) = platform().show_in_folder(path) {
        println!("browser: show {} failed: {e}", path.display());
    }
}
