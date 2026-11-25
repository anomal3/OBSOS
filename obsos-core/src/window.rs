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
/// * `WIN_WHEEL`: `x` — строки (вверх — больше нуля).
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
}

/// Окно, как его видит оболочка (`&Window`).
pub type Window = dyn HostWindow;

impl dyn HostWindow {
    /// Собрать кадр окна: копия кадра страницы в её прямоугольнике, холст
    /// egui поверх (панели непрозрачны, над страницей холст прозрачен).
    pub fn compose(&self, page: &PageFrame, at: (usize, usize), renderer: &mut EguiSoftwareRender) {
        let (ww, wh) = (self.width() as usize, self.height() as usize);
        self.present(&mut |surface: &mut [u32]| {
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
        });
    }
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
            for (px, s) in dst.iter_mut().zip(src.chunks_exact(4)) {
                let (b, g, r) = (s[0] as u32, s[1] as u32, s[2] as u32);
                *px = if swap_rb { 0xff00_0000 | (b << 16) | (g << 8) | r } else { 0xff00_0000 | (r << 16) | (g << 8) | b };
            }
        }
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
