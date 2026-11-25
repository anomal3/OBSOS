//! Платформа FossAi для OBSOS: окно стола (`freeos_window_*` из libfreeos),
//! буфер обмена, пояс, запуск программ (`SYS_SPAWN`), окно выбора файла
//! (`/bin/filedlg`) — договор `obsos_core::window`.

use std::cell::Cell;
use std::ffi::{CString, c_void};
use std::os::raw::{c_char, c_int};
use std::path::Path;

use obsos_core::window::{CursorShape, DialogAnswer, HostWindow, PendingDialog, Platform, WinEvent};

extern "C" {
    fn freeos_window_open(title: *const c_char, width: u32, height: u32, surface: *mut *mut c_void) -> i64;
    fn freeos_window_commit(id: i64, x: i32, y: i32, width: i32, height: i32) -> c_int;
    fn freeos_window_event(id: i64, event: *mut WinEvent) -> c_int;
    fn freeos_window_resize(id: i64, width: u32, height: u32) -> *mut c_void;
    fn freeos_window_style(id: i64, flags: u32) -> c_int;
    fn freeos_window_title(id: i64, title: *const c_char) -> c_int;
    fn freeos_window_close(id: i64) -> c_int;
    fn freeos_screen(width: *mut u32, height: *mut u32, pixel_format: *mut u32) -> c_int;
}

const WIN_STYLE_RESIZABLE: u32 = 1;
const PIXEL_RGB: u32 = 1;

/// Окно: поверхность (u32 на точку, строки сверху вниз) и её размер.
pub struct Window {
    id: i64,
    surface: Cell<*mut u32>,
    width: Cell<u32>,
    height: Cell<u32>,
    /// Красный в младшем байте (FREEOS_PIXEL_RGB): байты точки R,G,B,A;
    /// иначе B,G,R,A.
    swap_rb: bool,
    cursor: Cell<CursorShape>,
}

impl Window {
    pub fn open(title: &str, width: u32, height: u32) -> Option<Window> {
        let (mut sw, mut sh, mut fmt) = (0u32, 0u32, 0u32);
        // SAFETY: три выходных числа.
        unsafe { freeos_screen(&mut sw, &mut sh, &mut fmt) };
        let width = if sw > 0 { width.min(sw.saturating_sub(40)) } else { width };
        let height = if sh > 0 { height.min(sh.saturating_sub(80)) } else { height };
        let title = CString::new(title).ok()?;
        let mut surface: *mut c_void = std::ptr::null_mut();
        // SAFETY: заголовок — строка C, поверхность — выходной адрес.
        let id = unsafe { freeos_window_open(title.as_ptr(), width, height, &mut surface) };
        if id <= 0 || surface.is_null() {
            println!("browser: window open failed ({id})");
            return None;
        }
        // SAFETY: окно наше.
        unsafe { freeos_window_style(id, WIN_STYLE_RESIZABLE) };
        println!("browser: window {width}x{height}, screen {sw}x{sh}, pixel format {fmt}");
        Some(Window {
            id,
            surface: Cell::new(surface as *mut u32),
            width: Cell::new(width),
            height: Cell::new(height),
            swap_rb: fmt == PIXEL_RGB,
            cursor: Cell::new(CursorShape::Arrow),
        })
    }

    /// Поверхность целиком (строки подряд, без зазоров).
    #[allow(clippy::mut_from_ref)]
    fn pixels(&self) -> &mut [u32] {
        let len = self.width.get() as usize * self.height.get() as usize;
        // SAFETY: поверхность окна — width*height точек, отображена в нашу
        // память и живёт до следующего `resize`; ссылка дальше кадра не живёт.
        unsafe { std::slice::from_raw_parts_mut(self.surface.get(), len) }
    }
}

impl HostWindow for Window {
    fn width(&self) -> u32 {
        self.width.get()
    }

    fn height(&self) -> u32 {
        self.height.get()
    }

    fn swap_rb(&self) -> bool {
        self.swap_rb
    }

    fn next_event(&self) -> Option<WinEvent> {
        let mut ev = WinEvent::default();
        // SAFETY: окно наше, структура — по договору user-abi `WinEvent`.
        (unsafe { freeos_window_event(self.id, &mut ev) } == 1).then_some(ev)
    }

    fn resize(&self, width: u32, height: u32) -> bool {
        // SAFETY: окно наше; прежняя поверхность после вызова недействительна.
        let surface = unsafe { freeos_window_resize(self.id, width, height) };
        if surface.is_null() {
            return false;
        }
        self.surface.set(surface as *mut u32);
        self.width.set(width);
        self.height.set(height);
        true
    }

    /// Номер окна у ядра (владелец модального окна выбора файла).
    fn id(&self) -> i64 {
        self.id
    }

    fn set_cursor(&self, shape: CursorShape) {
        if self.cursor.get() != shape {
            self.cursor.set(shape);
            // SAFETY: окно наше, вызов без адресов.
            unsafe { syscall3(SYS_WINCURSOR, self.id as usize, shape as usize, 0) };
        }
    }

    fn set_title(&self, title: &str) {
        // Не длиннее 64 байт UTF-8 (MAX_TITLE), по границе символа.
        let mut cut = title.len().min(60);
        while !title.is_char_boundary(cut) {
            cut -= 1;
        }
        if let Ok(text) = CString::new(&title[..cut]) {
            // SAFETY: окно наше, строка C.
            unsafe { freeos_window_title(self.id, text.as_ptr()) };
        }
    }

    fn close(&self) {
        // SAFETY: окно наше.
        unsafe { freeos_window_close(self.id) };
    }

    fn present(&self, draw: &mut dyn FnMut(&mut [u32])) {
        draw(self.pixels());
        // SAFETY: окно наше; нулевой размер — всё окно.
        unsafe { freeos_window_commit(self.id, 0, 0, 0, 0) };
    }
}

const SYS_SPAWN: usize = 21;
const SYS_CLIPBOARD: usize = 85;
const CLIP_SET: usize = 1;
const CLIP_GET: usize = 2;
const SYS_WINCURSOR: usize = 89;
const SYS_TZINFO: usize = 90;
/// Наследовать консоль родителя (`FREEOS_SPAWN_INHERIT`).
const SPAWN_INHERIT: isize = -1;
/// Нет такого вызова (`ERR_NO_SYSCALL`: ядро старше браузера).
const ERR_NOSYS: isize = -2;

/// Ловушка ядра FreeOS, как `freeos_syscall` в libc/freeos/freeos-syscall.h:
/// номер и три аргумента.
///
/// # Safety
/// Аргументы-адреса должны быть верны для вызова `number`.
unsafe fn syscall3(number: usize, a0: usize, a1: usize, a2: usize) -> isize {
    let result: isize;
    // SAFETY: договор ловушки FreeOS; адреса проверяет вызывающий.
    unsafe {
        #[cfg(target_arch = "x86_64")]
        std::arch::asm!("int 0x80", inlateout("rax") number as isize => result,
            in("rdi") a0, in("rsi") a1, in("rdx") a2, options(nostack));
        #[cfg(target_arch = "aarch64")]
        std::arch::asm!("svc #0", in("x8") number, inlateout("x0") a0 as isize => result,
            in("x1") a1, in("x2") a2, options(nostack));
    }
    result
}

/// Запустить программу, не дожидаясь её. Строка разбирается тем же
/// разборщиком, что у оболочки (пробелы — разделители), поэтому `&` и `?` в
/// адресе не мешают — в отличие от `system()`, которая такие знаки
/// отвергает. Возвращает номер задачи.
pub fn spawn(command: &str) -> Result<i64, i64> {
    let bytes = command.as_bytes();
    // SAFETY: строка живёт до возврата из вызова.
    let result = unsafe { syscall3(SYS_SPAWN, bytes.as_ptr() as usize, bytes.len(), SPAWN_INHERIT as usize) };
    if result < 0 { Err(result as i64) } else { Ok(result as i64) }
}

/// Буфер обмена системы (общий с терминалом, Блокнотом, «Файлами»).
pub fn clipboard_set(text: &str) {
    // SAFETY: строка живёт до возврата.
    unsafe { syscall3(SYS_CLIPBOARD, CLIP_SET, text.as_ptr() as usize, text.len()) };
}

pub fn clipboard_get() -> Option<String> {
    let mut buf = vec![0u8; 4096];
    loop {
        // SAFETY: буфер наш, длина — его.
        let n = unsafe { syscall3(SYS_CLIPBOARD, CLIP_GET, buf.as_mut_ptr() as usize, buf.len()) };
        if n < 0 {
            return None;
        }
        let n = n as usize;
        if n <= buf.len() {
            buf.truncate(n);
            return String::from_utf8(buf).ok();
        }
        buf = vec![0u8; n];
    }
}

/// Смещение часового пояса системы от UTC в минутах (`None` — ядро старое).
pub fn timezone_minutes() -> Option<i32> {
    // SAFETY: вызов без адресов.
    let r = unsafe { syscall3(SYS_TZINFO, 0, 0, 0) };
    (r != ERR_NOSYS && r.abs() <= 18 * 60).then_some(r as i32)
}

const SYS_WRITE: usize = 1;
const SYS_READ: usize = 6;
const SYS_CLOSE: usize = 7;
const SYS_PIPE: usize = 39;
const SYS_LAUNCH: usize = 40;
const ERR_AGAIN: isize = -17;

/// Аргумент `SYS_LAUNCH` (user-abi `Launch`): команда и концы каналов.
#[repr(C)]
struct Launch {
    command: u64,
    command_len: u64,
    who: u64,
    stdin: i64,
    stdout: i64,
}

fn pipe() -> Option<(isize, isize)> {
    // SAFETY: вызов без адресов.
    let packed = unsafe { syscall3(SYS_PIPE, 0, 0, 0) };
    (packed >= 0).then(|| (packed >> 32, packed & 0xFFFF_FFFF))
}

fn close_fd(fd: isize) {
    // SAFETY: дескриптор наш.
    unsafe { syscall3(SYS_CLOSE, fd as usize, 0, 0) };
}

/// Окно выбора файла системы (`/bin/filedlg`, 20.11): отдельная программа,
/// модальная над окном браузера; запрос — строками `ключ\tзначение` в её
/// стандартный ввод, ответ — строка `\x1efiledlg-answer\tok\tПУТЬ` в выводе
/// (договор `user_progs::filedlg`). Каналы ядра неблокирующие — `poll` не
/// ждёт.
struct FileDialog {
    out: isize,
    pending: Vec<u8>,
}

impl FileDialog {
    /// «Сохранить как»: папка, предложенное имя, окно-владелец.
    fn save(owner: i64, title: &str, dir: &str, name: &str) -> Result<FileDialog, String> {
        let clean = |t: &str| t.replace(['\n', '\r', '\t'], " ");
        let request = format!(
            "mode\tsave\nowner\t{owner}\ntag\tobsos\ntitle\t{}\ndir\t{}\nname\t{}\nindex\t0\nextension\t0\n",
            clean(title),
            clean(dir),
            clean(name)
        );
        Self::start(&request)
    }

    /// «Выбор папки» (папка загрузок в настройках).
    fn folder(owner: i64, title: &str, dir: &str) -> Result<FileDialog, String> {
        let clean = |t: &str| t.replace(['\n', '\r', '\t'], " ");
        Self::start(&format!("mode\tfolder\nowner\t{owner}\ntag\tobsos\ntitle\t{}\ndir\t{}\n", clean(title), clean(dir)))
    }

    fn start(request: &str) -> Result<FileDialog, String> {
        let (in_read, in_write) = pipe().ok_or("канал не создаётся")?;
        let Some((out_read, out_write)) = pipe() else {
            close_fd(in_read);
            close_fd(in_write);
            return Err(String::from("канал не создаётся"));
        };
        let command = "/bin/filedlg --stdin";
        let launch = Launch {
            command: command.as_ptr() as u64,
            command_len: command.len() as u64,
            who: u64::MAX,
            stdin: in_read as i64,
            stdout: out_write as i64,
        };
        // SAFETY: структура и строка живут до возврата из вызова.
        let task = unsafe { syscall3(SYS_LAUNCH, &launch as *const Launch as usize, 0, 0) };
        // Свои копии концов, отданных окну, — закрыть: иначе конца файла не
        // увидит ни оно, ни мы.
        close_fd(in_read);
        close_fd(out_write);
        if task < 0 {
            close_fd(in_write);
            close_fd(out_read);
            return Err(format!("/bin/filedlg не запускается ({task})"));
        }
        let bytes = request.as_bytes();
        let mut sent = 0usize;
        let until = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while sent < bytes.len() && std::time::Instant::now() < until {
            // SAFETY: буфер наш.
            let wrote = unsafe { syscall3(SYS_WRITE, in_write as usize, bytes[sent..].as_ptr() as usize, bytes.len() - sent) };
            if wrote > 0 {
                sent += wrote as usize;
            } else if wrote == ERR_AGAIN {
                std::thread::sleep(std::time::Duration::from_millis(5));
            } else {
                break;
            }
        }
        close_fd(in_write);
        println!("browser: file dialog #{task} started");
        Ok(FileDialog { out: out_read, pending: Vec::new() })
    }

}

impl PendingDialog for FileDialog {
    /// Ответ, не дожидаясь: `None` — окно ещё открыто.
    fn poll(&mut self) -> Option<DialogAnswer> {
        let mut buf = [0u8; 1024];
        let ended = loop {
            // SAFETY: буфер наш.
            let got = unsafe { syscall3(SYS_READ, self.out as usize, buf.as_mut_ptr() as usize, buf.len()) };
            if got > 0 {
                self.pending.extend_from_slice(&buf[..got as usize]);
                continue;
            }
            break got == 0 || got != ERR_AGAIN;
        };
        while let Some(end) = self.pending.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=end).collect();
            let line = String::from_utf8_lossy(&line[..line.len() - 1]).into_owned();
            if let Some(rest) = line.strip_prefix("\x1efiledlg-answer\t") {
                return Some(match rest.split_once('\t') {
                    Some(("ok", path)) if !path.is_empty() => DialogAnswer::Chosen(path.to_string()),
                    _ => DialogAnswer::Cancelled,
                });
            } else if !line.is_empty() {
                println!("{line}");
            }
        }
        ended.then_some(DialogAnswer::Cancelled)
    }
}

impl Drop for FileDialog {
    fn drop(&mut self) {
        close_fd(self.out);
    }
}

pub const CHROMIUM: &str = "/opt/chromium/bin/chromium";

pub fn chromium_installed() -> bool {
    std::path::Path::new(CHROMIUM).exists()
}

/// Открыть адрес в Chromium (пакет со слоем Linux).
pub fn open_in_chromium(url: &str) -> Result<i64, i64> {
    let safe: String = url.chars().filter(|c| !c.is_whitespace()).collect();
    spawn(&format!("/opt/debian/bin/bash {CHROMIUM} {safe}"))
}

/// Службы FossAi для оболочки.
pub struct FossAi;

/// Где пакет `obsos` ставит программу (`/opt/obsos`).
const OBSOS: &str = "/opt/obsos/bin/obsos";

impl Platform for FossAi {
    fn clipboard_set(&self, text: &str) {
        clipboard_set(text)
    }

    fn clipboard_get(&self) -> Option<String> {
        clipboard_get()
    }

    fn timezone_minutes(&self) -> Option<i32> {
        timezone_minutes()
    }

    fn new_window(&self, args: &[&str]) -> Result<(), String> {
        let mut command = String::from(OBSOS);
        for a in args {
            command.push(' ');
            command.push_str(a);
        }
        spawn(&command).map(|_| ()).map_err(|e| format!("SYS_SPAWN {e}"))
    }

    fn show_in_folder(&self, path: &Path) -> Result<(), String> {
        spawn(&format!("/bin/files {}", path.display())).map(|_| ()).map_err(|e| format!("SYS_SPAWN {e}"))
    }

    fn install_package(&self, path: &Path) -> Option<Result<(), String>> {
        let shown = path.display().to_string();
        shown.to_ascii_lowercase().ends_with(".fpk").then(|| spawn(&format!("/bin/pkg install {shown}")).map(|_| ()).map_err(|e| format!("SYS_SPAWN {e}")))
    }

    fn chromium_installed(&self) -> bool {
        chromium_installed()
    }

    fn open_in_chromium(&self, url: &str) -> Result<i64, String> {
        open_in_chromium(url).map_err(|e| format!("SYS_SPAWN {e}"))
    }

    fn save_dialog(&self, owner: &obsos_core::window::Window, title: &str, dir: &str, name: &str) -> Result<obsos_core::window::FileDialog, String> {
        FileDialog::save(owner.id(), title, dir, name).map(|d| Box::new(d) as obsos_core::window::FileDialog)
    }

    fn folder_dialog(&self, owner: &obsos_core::window::Window, title: &str, dir: &str) -> Result<obsos_core::window::FileDialog, String> {
        FileDialog::folder(owner.id(), title, dir).map(|d| Box::new(d) as obsos_core::window::FileDialog)
    }

    fn notice_path(&self) -> String {
        String::from("/opt/obsos/share/NOTICE.md")
    }
}
