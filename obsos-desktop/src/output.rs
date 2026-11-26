//! Куда идёт журнал `browser: …` на Windows, где у OBSOS нет окна консоли:
//! перенаправленный вывод (`obsos > журнал.txt`) остаётся как есть; с ключом
//! `--console` открывается консоль; иначе — файл
//! `%USERPROFILE%\.config\obsos\obsos.log` (прошлый запуск — `obsos.old.log`).

#[cfg(windows)]
pub fn setup() {
    use std::os::windows::io::IntoRawHandle;

    const STD_OUTPUT: u32 = -11i32 as u32;
    const STD_ERROR: u32 = -12i32 as u32;
    const PARENT: u32 = u32::MAX;

    #[link(name = "kernel32")]
    extern "system" {
        fn GetStdHandle(which: u32) -> isize;
        fn SetStdHandle(which: u32, handle: isize) -> i32;
        fn AttachConsole(process: u32) -> i32;
        fn AllocConsole() -> i32;
    }

    let set = |file: std::fs::File| {
        // Дескриптор живёт до конца программы.
        let handle = file.into_raw_handle() as isize;
        // SAFETY: действующий дескриптор открытого файла.
        unsafe {
            SetStdHandle(STD_OUTPUT, handle);
            SetStdHandle(STD_ERROR, handle);
        }
    };
    if std::env::args().any(|a| a == "--console") {
        // SAFETY: вызовы без параметров-указателей.
        unsafe {
            if AttachConsole(PARENT) == 0 {
                AllocConsole();
            }
        }
        if let Ok(console) = std::fs::OpenOptions::new().write(true).open("CONOUT$") {
            set(console);
        }
        return;
    }
    // SAFETY: то же.
    let current = unsafe { GetStdHandle(STD_OUTPUT) };
    if current != 0 && current != -1 {
        return;
    }
    let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) else { return };
    let dir = std::path::Path::new(&home).join(".config").join("obsos");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("obsos.log");
    let _ = std::fs::rename(&path, dir.join("obsos.old.log"));
    if let Ok(file) = std::fs::File::create(path) {
        set(file);
    }
}

#[cfg(not(windows))]
pub fn setup() {}
