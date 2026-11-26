//! Где уходит время кадра: отрисовка страницы, перенос кадра в окно,
//! оболочка egui, вывод на экран. Печатается командой `perf`
//! (`automation.rs`) и раз в пять секунд при `OBSOS_PERF=1`.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
pub enum Stage {
    /// `webview.paint()` — рисование страницы WebRender.
    Paint = 0,
    /// Кадр из контекста рисования в память окна.
    Store = 1,
    /// Проход egui и растеризация оболочки.
    Ui = 2,
    /// Сборка кадра окна и показ.
    Compose = 3,
    /// `spin_event_loop` Servo.
    Spin = 4,
    /// Промежуток между соседними показанными кадрами подряд идущей
    /// анимации — по нему видны рывки.
    Gap = 5,
}

const NAMES: [&str; 6] = ["paint", "store", "ui", "compose", "spin", "gap"];

static COUNT: [AtomicU64; 6] = [const { AtomicU64::new(0) }; 6];
static TOTAL: [AtomicU64; 6] = [const { AtomicU64::new(0) }; 6];
static WORST: [AtomicU64; 6] = [const { AtomicU64::new(0) }; 6];
/// Промежутки длиннее 20 мс (видимый пропуск кадра на любом экране).
static LATE: AtomicU64 = AtomicU64::new(0);
static LAST_FRAME: Mutex<Option<Instant>> = Mutex::new(None);

/// Кадр показан. Паузы дольше 100 мс — это покой, а не рывок.
pub fn frame_shown() {
    let Ok(mut last) = LAST_FRAME.lock() else { return };
    if let Some(before) = last.replace(Instant::now()) {
        let gap = before.elapsed();
        if gap < Duration::from_millis(100) {
            add(Stage::Gap, before);
            if gap > Duration::from_millis(20) {
                LATE.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

pub fn add(stage: Stage, since: Instant) {
    let us = since.elapsed().as_micros() as u64;
    let i = stage as usize;
    COUNT[i].fetch_add(1, Ordering::Relaxed);
    TOTAL[i].fetch_add(us, Ordering::Relaxed);
    WORST[i].fetch_max(us, Ordering::Relaxed);
}

/// Строка «этап: раз / среднее / худшее» и обнуление.
pub fn report(seconds: f32) -> String {
    let mut out = format!("perf {seconds:.1}s:");
    for (i, name) in NAMES.iter().enumerate() {
        let n = COUNT[i].swap(0, Ordering::Relaxed);
        let total = TOTAL[i].swap(0, Ordering::Relaxed);
        let worst = WORST[i].swap(0, Ordering::Relaxed);
        if n > 0 {
            out.push_str(&format!(
                " | {name} {n}x avg {:.1}ms max {:.1}ms sum {:.0}ms",
                total as f64 / n as f64 / 1000.0,
                worst as f64 / 1000.0,
                total as f64 / 1000.0
            ));
        }
    }
    let late = LATE.swap(0, Ordering::Relaxed);
    if late > 0 {
        out.push_str(&format!(" | late {late}"));
    }
    out
}
