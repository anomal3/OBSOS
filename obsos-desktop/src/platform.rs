//! Службы Windows и Linux для оболочки: буфер обмена (arboard), окна выбора
//! файла и папки (rfd, в своём потоке — цикл браузера не ждёт), «Показать
//! в папке» (Проводник / xdg-open), внешний браузер (браузер системы по
//! умолчанию), новое окно OBSOS (та же программа с ключами).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Mutex;

use obsos_core::window::{DialogAnswer, FileDialog, PendingDialog, Platform, Window};

pub struct Desktop {
    clipboard: Mutex<Option<arboard::Clipboard>>,
}

impl Desktop {
    pub fn new() -> Desktop {
        let clipboard = arboard::Clipboard::new().map_err(|e| println!("browser: clipboard: {e}")).ok();
        Desktop { clipboard: Mutex::new(clipboard) }
    }
}

/// Окно выбора, открытое в своём потоке.
struct ThreadDialog(Receiver<Option<PathBuf>>);

impl PendingDialog for ThreadDialog {
    fn poll(&mut self) -> Option<DialogAnswer> {
        match self.0.try_recv() {
            Ok(Some(path)) => Some(DialogAnswer::Chosen(path.display().to_string())),
            Ok(None) | Err(TryRecvError::Disconnected) => Some(DialogAnswer::Cancelled),
            Err(TryRecvError::Empty) => None,
        }
    }
}

fn dialog(run: impl FnOnce() -> Option<PathBuf> + Send + 'static) -> Result<FileDialog, String> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name(String::from("obsos-filedlg"))
        .spawn(move || {
            let _ = tx.send(run());
        })
        .map_err(|e| e.to_string())?;
    Ok(Box::new(ThreadDialog(rx)))
}

fn run_detached(command: &mut Command) -> Result<i64, String> {
    command.spawn().map(|c| c.id() as i64).map_err(|e| e.to_string())
}

impl Platform for Desktop {
    fn clipboard_set(&self, text: &str) {
        if let Some(c) = self.clipboard.lock().ok().as_mut().and_then(|c| c.as_mut()) {
            let _ = c.set_text(text.to_string());
        }
    }

    fn clipboard_get(&self) -> Option<String> {
        self.clipboard.lock().ok()?.as_mut()?.get_text().ok()
    }

    fn ui_font(&self) -> Option<Vec<u8>> {
        if !cfg!(windows) {
            return None;
        }
        let windows = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        std::fs::read(Path::new(&windows).join("Fonts").join("segoeui.ttf")).ok()
    }

    fn utc_offset_minutes(&self) -> i32 {
        chrono::Local::now().offset().local_minus_utc() / 60
    }

    fn new_window(&self, args: &[&str]) -> Result<(), String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        run_detached(Command::new(exe).args(args)).map(|_| ())
    }

    fn show_in_folder(&self, path: &Path) -> Result<(), String> {
        if cfg!(windows) {
            let mut c = Command::new("explorer");
            if path.is_file() {
                c.arg(format!("/select,{}", path.display()));
            } else {
                c.arg(path);
            }
            run_detached(&mut c).map(|_| ())
        } else {
            let dir = if path.is_file() { path.parent().unwrap_or(path) } else { path };
            run_detached(Command::new("xdg-open").arg(dir)).map(|_| ())
        }
    }

    fn chromium_installed(&self) -> bool {
        // «Открыть в Chromium» — браузер системы по умолчанию.
        true
    }

    fn open_in_chromium(&self, url: &str) -> Result<i64, String> {
        // Только веб: `rundll32 …FileProtocolHandler` по `file:` запустил бы
        // программу.
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(String::from("только веб-адреса"));
        }
        if cfg!(windows) {
            // Без cmd: знаки `&` в адресе не разбираются как команды.
            run_detached(Command::new("rundll32").arg("url.dll,FileProtocolHandler").arg(url)).map(|_| 0)
        } else {
            run_detached(Command::new("xdg-open").arg(url)).map(|_| 0)
        }
    }

    fn save_dialog(&self, _owner: &Window, title: &str, dir: &str, name: &str) -> Result<FileDialog, String> {
        let (title, dir, name) = (title.to_string(), dir.to_string(), name.to_string());
        dialog(move || rfd::FileDialog::new().set_title(title).set_directory(dir).set_file_name(name).save_file())
    }

    fn folder_dialog(&self, _owner: &Window, title: &str, dir: &str) -> Result<FileDialog, String> {
        let (title, dir) = (title.to_string(), dir.to_string());
        dialog(move || rfd::FileDialog::new().set_title(title).set_directory(dir).pick_folder())
    }

    fn notice_path(&self) -> String {
        // Рядом с программой (раздача), иначе — из исходников.
        let near = std::env::current_exe().ok().and_then(|e| Some(e.parent()?.join("NOTICE.md")));
        match near.filter(|p| p.is_file()) {
            Some(p) => p.display().to_string(),
            None => String::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../docs/NOTICE.md")),
        }
    }
}
