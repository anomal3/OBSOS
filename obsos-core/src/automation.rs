//! Управление OBSOS из файла — для проверок без человека (`--control <файл>`;
//! без ключа ничего этого нет). Команды дописываются в файл построчно и
//! выполняются по очереди; ввод идёт тем же путём, что от окна (`WinEvent`).
//!
//! ```text
//! go <адрес>            перейти в текущей вкладке
//! click|rclick|mclick|ctrlclick <x> <y>
//! move <x> <y>          указатель
//! wheel|ctrlwheel <строки> [x y]  прокрутка (вверх — больше нуля)
//! key <имя> [ctrl] [shift] [alt]     enter esc tab f5 f12 left a 1 …
//! text <строка>         набрать
//! wait <мс>             пауза
//! waitload [мс]         дождаться конца загрузки (не дольше мс, 15000)
//! eval <js>             выполнить в странице, ответ — в журнал
//! shot <файл.bmp>       снимок окна
//! perf                  время кадров с прошлого `perf`
//! press|release <x> <y>   левая кнопка по частям (перетаскивание вместе с move;
//!                        точки могут быть за окном)
//! window <n>             команды дальше — этому окну (по порядку открытия)
//! windows                список окон и вкладок
//! movetab <i> <окно>     перенести вкладку (номера с нуля)
//! quit
//! ```

use std::collections::VecDeque;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::window::*;

pub enum Command {
    Go(String),
    Eval(String),
    Shot(PathBuf),
    Perf,
    /// Кому идут следующие команды: номер окна по порядку открытия.
    Window(usize),
    /// Список окон и их вкладок — в журнал.
    Windows,
    /// Перенести вкладку (номер с нуля) в окно.
    MoveTab(usize, usize),
    Quit,
}

pub struct Control {
    path: PathBuf,
    offset: u64,
    partial: String,
    lines: VecDeque<String>,
    next_read: Instant,
    wait_until: Option<Instant>,
    wait_load: Option<Instant>,
}

impl Control {
    pub fn new(path: PathBuf) -> Control {
        Control {
            path,
            offset: 0,
            partial: String::new(),
            lines: VecDeque::new(),
            next_read: Instant::now(),
            wait_until: None,
            wait_load: None,
        }
    }

    fn read_more(&mut self) {
        if Instant::now() < self.next_read {
            return;
        }
        self.next_read = Instant::now() + Duration::from_millis(100);
        let Ok(mut file) = std::fs::File::open(&self.path) else { return };
        if file.seek(SeekFrom::Start(self.offset)).is_err() {
            return;
        }
        let mut text = String::new();
        let Ok(n) = file.read_to_string(&mut text) else { return };
        self.offset += n as u64;
        self.partial.push_str(&text);
        while let Some(at) = self.partial.find('\n') {
            let line: String = self.partial.drain(..=at).collect();
            let line = line.trim().trim_start_matches('\u{feff}').to_string();
            if !line.is_empty() && !line.starts_with('#') {
                self.lines.push_back(line);
            }
        }
    }

    /// Следующая порция: события окна — в `events`, команда оболочке — в ответ.
    /// `loading` — идёт ли загрузка активной вкладки.
    pub fn poll(&mut self, loading: bool, events: &mut VecDeque<WinEvent>) -> Option<Command> {
        self.read_more();
        if let Some(until) = self.wait_until {
            if Instant::now() < until {
                return None;
            }
            self.wait_until = None;
        }
        if let Some(until) = self.wait_load {
            if loading && Instant::now() < until {
                return None;
            }
            println!("browser: control loaded{}", if loading { " (timeout)" } else { "" });
            self.wait_load = None;
        }
        let line = self.lines.pop_front()?;
        let (word, rest) = line.split_once(char::is_whitespace).unwrap_or((line.as_str(), ""));
        let rest = rest.trim();
        // Набираемое в журнал не пишется — там бывают пароли.
        if word == "text" {
            println!("browser: control text ({} chars)", rest.chars().count());
        } else {
            println!("browser: control {line}");
        }
        let numbers: Vec<i32> = rest.split_whitespace().filter_map(|s| s.parse().ok()).collect();
        let mut push = |kind: u32, code: u32, x: i32, y: i32| events.push_back(WinEvent { kind, code, x, y });
        match word {
            "go" => return Some(Command::Go(rest.to_string())),
            "eval" => return Some(Command::Eval(rest.to_string())),
            "shot" => return Some(Command::Shot(PathBuf::from(rest))),
            "perf" => return Some(Command::Perf),
            "window" if !numbers.is_empty() => return Some(Command::Window(numbers[0].max(0) as usize)),
            "windows" => return Some(Command::Windows),
            "movetab" if numbers.len() >= 2 => return Some(Command::MoveTab(numbers[0].max(0) as usize, numbers[1].max(0) as usize)),
            "press" | "release" if numbers.len() >= 2 => {
                let (x, y) = (numbers[0], numbers[1]);
                if word == "press" {
                    push(WIN_MOVE, 0, x, y);
                    push(WIN_POINTER, 1, x, y);
                } else {
                    push(WIN_RELEASE, 1, x, y);
                }
                self.wait_until = Some(Instant::now() + Duration::from_millis(150));
            }
            "quit" => return Some(Command::Quit),
            "click" | "rclick" | "mclick" | "ctrlclick" if numbers.len() >= 2 => {
                let (x, y) = (numbers[0], numbers[1]);
                let button = match word {
                    "rclick" => 2,
                    "mclick" => 3,
                    _ => 1,
                };
                if word == "ctrlclick" {
                    push(WIN_KEY, WIN_KEY_NAMED + 25, 2, 0);
                }
                push(WIN_MOVE, 0, x, y);
                push(WIN_POINTER, button, x, y);
                push(WIN_RELEASE, button, x, y);
                if word == "ctrlclick" {
                    push(WIN_KEYUP, WIN_KEY_NAMED + 25, 0, 0);
                }
                // Дать оболочке и странице обработать щелчок.
                self.wait_until = Some(Instant::now() + Duration::from_millis(150));
            }
            "move" if numbers.len() >= 2 => {
                push(WIN_MOVE, 0, numbers[0], numbers[1]);
                self.wait_until = Some(Instant::now() + Duration::from_millis(100));
            }
            "wheel" | "ctrlwheel" if !numbers.is_empty() => {
                if numbers.len() >= 3 {
                    push(WIN_MOVE, 0, numbers[1], numbers[2]);
                }
                if word == "ctrlwheel" {
                    push(WIN_KEY, WIN_KEY_NAMED + 25, 2, 0);
                }
                push(WIN_WHEEL, 0, numbers[0], 0);
                if word == "ctrlwheel" {
                    push(WIN_KEYUP, WIN_KEY_NAMED + 25, 0, 0);
                }
                self.wait_until = Some(Instant::now() + Duration::from_millis(100));
            }
            "key" => {
                let mut parts = rest.split_whitespace();
                let Some(name) = parts.next() else { return None };
                let mut mask = 0;
                for m in parts {
                    mask |= match m {
                        "shift" => 1,
                        "ctrl" => 2,
                        "alt" => 4,
                        _ => 0,
                    };
                }
                let (mut code, latin) = key_of(name)?;
                if mask & 2 != 0 && (latin as u8 as char).is_ascii_lowercase() && latin != 0 {
                    code = latin - 'a' as u32 + 1;
                }
                push(WIN_KEY, code, mask, latin as i32);
                push(WIN_KEYUP, code, mask, latin as i32);
                self.wait_until = Some(Instant::now() + Duration::from_millis(150));
            }
            "text" => {
                for c in rest.chars() {
                    let latin = if c.is_ascii() { c.to_ascii_lowercase() as i32 } else { 0 };
                    push(WIN_KEY, c as u32, 0, latin);
                    push(WIN_KEYUP, c as u32, 0, latin);
                }
                self.wait_until = Some(Instant::now() + Duration::from_millis(200));
            }
            "wait" => self.wait_until = Some(Instant::now() + Duration::from_millis(numbers.first().copied().unwrap_or(500).max(0) as u64)),
            "waitload" => {
                // Сперва дать загрузке начаться.
                self.wait_until = Some(Instant::now() + Duration::from_millis(400));
                self.wait_load = Some(Instant::now() + Duration::from_millis(numbers.first().copied().unwrap_or(15_000).max(0) as u64));
            }
            _ => println!("browser: control unknown command {line}"),
        }
        None
    }
}

/// Имя клавиши → (код `WinEvent`, латинский знак).
fn key_of(name: &str) -> Option<(u32, u32)> {
    let named = |n: u32| Some((WIN_KEY_NAMED + n, 0));
    match name {
        "left" => named(1),
        "right" => named(2),
        "up" => named(3),
        "down" => named(4),
        "home" => named(5),
        "end" => named(6),
        "pageup" => named(7),
        "pagedown" => named(8),
        "delete" => named(9),
        "enter" => Some((13, 0)),
        "esc" => Some((27, 0)),
        "tab" => Some((9, 0)),
        "backspace" => Some((8, 0)),
        "space" => Some((' ' as u32, ' ' as u32)),
        _ => {
            if let Some(n) = name.strip_prefix('f').and_then(|n| n.parse::<u32>().ok()).filter(|n| (1..=12).contains(n)) {
                return named(11 + n);
            }
            let c = name.chars().next()?;
            Some((c as u32, c as u32))
        }
    }
}

/// Снимок окна: 32-битный BMP (строки сверху вниз), точки `0x??RRGGBB`.
pub fn save_bmp(path: &Path, pixels: &[u32], width: usize, height: usize) -> std::io::Result<()> {
    let data = width * height * 4;
    let mut out = Vec::with_capacity(54 + data);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&((54 + data) as u32).to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(width as i32).to_le_bytes());
    out.extend_from_slice(&(-(height as i32)).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&[0; 24]);
    for px in pixels.iter().take(width * height) {
        out.extend_from_slice(&(px | 0xff00_0000).to_le_bytes());
    }
    std::fs::write(path, out)
}
