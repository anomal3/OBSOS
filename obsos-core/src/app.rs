//! Главный цикл OBSOS: окно платформы (`crate::window`), Servo, оболочка.
//!
//! `obsos [--profile <имя>] [--sync-folder <папка>] [--private] [адрес]`
//!
//! Окно даёт платформа (стол FossAi, winit на Windows/Linux), страницу рисует swgl (программный OpenGL, `swgl_ctx`),
//! оболочку — egui прямо в пиксели окна (`ui.rs`). Вкладки — `WebView`
//! одного контекста swgl, видима только активная. Синхронизация (свой
//! репозиторий GitHub/GitLab/Bitbucket с тегами-точками, Яндекс Диск, Google
//! Диск, WebDAV, папка) — в отдельном потоке (`syncer.rs`), пароли — в
//! зашифрованном хранилище (`vault.rs`).
//!
//! Клавиши — раскладка Chromium (Ctrl+T/W/L/D/H/F/G/R, Ctrl+Shift+T/O/Q,
//! Ctrl+1..9, Ctrl+Tab, Alt+←/→/Home, F3/F5/F6/F10, Ctrl±/0) и своя
//! Ctrl+Shift+S — синхронизировать.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use crate::perf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui_software_backend::{ColorFieldOrder, EguiSoftwareRender};
use servo::protocol_handler::ProtocolRegistry;
use servo::{
    DevicePoint, DeviceVector2D, EventLoopWaker, InputEvent, Key, KeyState, KeyboardEvent, Modifiers, MouseButton,
    MouseButtonAction, MouseButtonEvent, MouseMoveEvent, NamedKey, Scroll, ServoBuilder,
    UserContentManager, UserScript,
    WebViewPoint, WebViewVector,
};

use crate::pages::{ObsosProtocol, PageData};
use crate::profile::{Look, Profile, TabsAt};
use crate::shell::{Action, Delegate, Modal, Panel, Shell, SyncUi, page_origin, page_size};
use crate::swgl_ctx::{PageContext, SwglRenderingContext};
use crate::syncer::Syncer;
use crate::vault::{OpenError, Vault};
use crate::window::*;
use crate::{blocker, devtools, downloads, shell, swgl_ctx, ui, window};

/// Будильник Servo: прерывает паузу главного цикла (`HostWindow::waker`).
/// Каждый запрос страницы к сети ждёт ответа оболочки (`load_web_resource`),
/// поэтому задержка здесь прямо видна в скорости загрузки.
#[derive(Clone)]
struct Waker(Option<Arc<dyn Fn() + Send + Sync>>);

impl EventLoopWaker for Waker {
    fn clone_box(&self) -> Box<dyn EventLoopWaker> {
        Box::new(self.clone())
    }

    fn wake(&self) {
        if let Some(wake) = &self.0 {
            wake();
        }
    }
}

// ── Клавиши окна → Servo и egui ─────────────────────────────────────────────

fn servo_key(code: u32) -> Option<Key> {
    if code >= WIN_KEY_NAMED {
        let named = match code - WIN_KEY_NAMED {
            1 => NamedKey::ArrowLeft,
            2 => NamedKey::ArrowRight,
            3 => NamedKey::ArrowUp,
            4 => NamedKey::ArrowDown,
            5 => NamedKey::Home,
            6 => NamedKey::End,
            7 => NamedKey::PageUp,
            8 => NamedKey::PageDown,
            9 => NamedKey::Delete,
            11 => NamedKey::Insert,
            24 => NamedKey::Shift,
            25 => NamedKey::Control,
            26 => NamedKey::Alt,
            n @ 12..=23 => match n - 12 {
                0 => NamedKey::F1,
                1 => NamedKey::F2,
                2 => NamedKey::F3,
                3 => NamedKey::F4,
                4 => NamedKey::F5,
                5 => NamedKey::F6,
                6 => NamedKey::F7,
                7 => NamedKey::F8,
                8 => NamedKey::F9,
                9 => NamedKey::F10,
                10 => NamedKey::F11,
                _ => NamedKey::F12,
            },
            _ => return None,
        };
        return Some(Key::Named(named));
    }
    Some(match code {
        8 => Key::Named(NamedKey::Backspace),
        9 => Key::Named(NamedKey::Tab),
        10 | 13 => Key::Named(NamedKey::Enter),
        27 => Key::Named(NamedKey::Escape),
        127 => Key::Named(NamedKey::Delete),
        c if c < 32 => return None,
        c => Key::Character(char::from_u32(c)?.to_string()),
    })
}

fn servo_modifiers(mask: i32) -> Modifiers {
    let mut m = Modifiers::empty();
    if mask & 1 != 0 {
        m |= Modifiers::SHIFT;
    }
    if mask & 2 != 0 {
        m |= Modifiers::CONTROL;
    }
    if mask & 4 != 0 {
        m |= Modifiers::ALT;
    }
    m
}

fn egui_modifiers(mask: i32) -> egui::Modifiers {
    let ctrl = mask & 2 != 0;
    egui::Modifiers { alt: mask & 4 != 0, ctrl, shift: mask & 1 != 0, mac_cmd: false, command: ctrl }
}

/// Буква клавиши: сама буква или управляющий код Ctrl+буква (1..=26).
fn letter_of(code: u32) -> Option<char> {
    match code {
        1..=26 => Some((b'a' + code as u8 - 1) as char),
        c => char::from_u32(c).filter(|c| c.is_ascii_alphabetic()).map(|c| c.to_ascii_lowercase()),
    }
}

fn egui_key(code: u32) -> Option<egui::Key> {
    use egui::Key as K;
    if code >= WIN_KEY_NAMED {
        return Some(match code - WIN_KEY_NAMED {
            1 => K::ArrowLeft,
            2 => K::ArrowRight,
            3 => K::ArrowUp,
            4 => K::ArrowDown,
            5 => K::Home,
            6 => K::End,
            7 => K::PageUp,
            8 => K::PageDown,
            9 => K::Delete,
            11 => K::Insert,
            16 => K::F5,
            _ => return None,
        });
    }
    match code {
        8 => Some(K::Backspace),
        9 => Some(K::Tab),
        10 | 13 => Some(K::Enter),
        27 => Some(K::Escape),
        127 => Some(K::Delete),
        c => egui::Key::from_name(&letter_of(c)?.to_ascii_uppercase().to_string()),
    }
}

/// Пользовательские скрипты и стили профиля (`userscripts/*.js|*.css`) —
/// замена расширениям — и свой скрипт OBSOS (пароли, MediaSource).
fn content_manager(servo: &servo::Servo, profile: &Profile) -> Rc<UserContentManager> {
    let manager = Rc::new(UserContentManager::new(servo));
    manager.add_script(Rc::new(UserScript::new(shell::PAGE_SCRIPT.to_string(), None)));
    // Белый фон корня, как у других браузеров: страница без своего фона и
    // короче окна иначе оставляла под собой куски предыдущей (плитки
    // WebRender не перерисовывались). Стиль пользователя — свой фон
    // страницы его перекрывает.
    if let Ok(url) = url::Url::parse("obsos://style/base.css") {
        manager.add_stylesheet(Rc::new(servo::user_contents::UserStyleSheet::new(String::from("html { background-color: #fff; }"), url)));
    }
    let dir = profile.dir.join("userscripts");
    let mut files: Vec<_> = std::fs::read_dir(&dir).map(|r| r.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    files.sort();
    let (mut scripts, mut styles) = (0, 0);
    for path in files {
        let Ok(source) = std::fs::read_to_string(&path) else { continue };
        match path.extension().and_then(|e| e.to_str()) {
            Some("js") => {
                manager.add_script(Rc::new(UserScript::new(source, Some(path.clone()))));
                scripts += 1;
            }
            Some("css") => {
                if let Ok(url) = url::Url::from_file_path(&path) {
                    manager.add_stylesheet(Rc::new(servo::user_contents::UserStyleSheet::new(source, url)));
                    styles += 1;
                }
            }
            _ => {}
        }
    }
    println!("browser: userscripts {scripts} js, {styles} css");
    manager
}

fn egui_cursor(icon: egui::CursorIcon) -> window::CursorShape {
    use egui::CursorIcon as C;
    use window::CursorShape as S;
    match icon {
        C::PointingHand => S::Hand,
        C::Text | C::VerticalText => S::Text,
        C::Move | C::Grab | C::Grabbing | C::AllScroll => S::Move,
        C::NotAllowed | C::NoDrop => S::NotAllowed,
        C::ResizeHorizontal | C::ResizeColumn | C::ResizeEast | C::ResizeWest => S::ResizeEw,
        C::ResizeVertical | C::ResizeRow | C::ResizeNorth | C::ResizeSouth => S::ResizeNs,
        _ => S::Arrow,
    }
}

/// Имя пояса для ICU по смещению: целые часы — `Etc/GMT-3` (знак у `Etc`
/// обратный: это Москва, UTC+3), иначе запись POSIX `<+0530>-5:30`.
fn tz_name(minutes: i32) -> String {
    if minutes == 0 {
        String::from("UTC")
    } else if minutes % 60 == 0 {
        format!("Etc/GMT{:+}", -minutes / 60)
    } else {
        let (sign, m) = if minutes > 0 { ('+', minutes) } else { ('-', -minutes) };
        let posix_sign = if minutes > 0 { '-' } else { '+' };
        format!("<{sign}{:02}{:02}>{posix_sign}{}:{:02}", m / 60, m % 60, m / 60, m % 60)
    }
}

/// Журнал Servo (`log`): предупреждения и ошибки — в журнал системы, по
/// ним видно, почему не загрузился файл страницы. Не больше 400 строк за
/// запуск — сайты бывают болтливы.
struct Logger;

static LOGGED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

impl log::Log for Logger {
    fn enabled(&self, meta: &log::Metadata) -> bool {
        // Сборка с инструментами Firefox предупреждает о каждом запросе без
        // метаданных; пустые события ввода — тоже шум.
        meta.level() <= log::Level::Warn && !matches!(meta.target(), "net::devtools::enabled" | "paint::webview_renderer")
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) || LOGGED.fetch_add(1, Ordering::Relaxed) >= 400 {
            return;
        }
        let text: String = crate::logs::scrub(&record.args().to_string()).chars().take(300).collect();
        println!("browser: log {} {}: {text}", record.level(), record.target());
    }

    fn flush(&self) {}
}

static LOGGER: Logger = Logger;

/// Метка «видеокарта ещё не показала ни кадра»: если драйвер уронит
/// программу (assert в surfman при `panic = "abort"`, сбой на первом кадре),
/// следующий запуск увидит её и выключит видеокарту в настройках.
const GPU_MARK: &str = "gpu.starting";

/// Окно на видеокарте (только Windows, `gpu_ctx`).
#[cfg(windows)]
type Gpu = crate::gpu_ctx::GpuOutput;
/// В прочих системах значения этого типа не бывает.
#[cfg(not(windows))]
enum Gpu {}

#[cfg(not(windows))]
impl Gpu {
    fn max_texture_side(&self) -> usize {
        match *self {}
    }
    fn add_window(&mut self, _: raw_window_handle::RawWindowHandle, _: dpi::PhysicalSize<u32>) -> Result<usize, String> {
        match *self {}
    }
    fn remove_window(&mut self, _: usize) {
        match *self {}
    }
    fn set_ui(&mut self, _: usize, _: Vec<egui::ClippedPrimitive>, _: &egui::TexturesDelta, _: f32) {
        match *self {}
    }
    fn frame(&mut self, _: usize, _: (u32, u32), _: (u32, u32), _: [u32; 4]) {
        match *self {}
    }
    fn snapshot(&mut self, _: usize, _: (u32, u32), _: (u32, u32), _: [u32; 4]) -> Vec<u32> {
        match *self {}
    }
    fn shutdown(&mut self) {
        match *self {}
    }
}

/// Вывод окна и контекст рисования страниц.
struct Screen {
    context: Rc<dyn PageContext>,
    /// `Some` — всё окно рисует видеокарта.
    gpu: Option<Gpu>,
    /// Такт экрана для плавной прокрутки (вместе с видеокартой).
    vsync: Option<Rc<dyn servo::RefreshDriver>>,
    renderer: String,
    /// Видеокарта пропущена из-за неудачного прошлого запуска.
    failed: bool,
}

/// Видеокартой (Windows, настройка «рисовать видеокартой», без ключа
/// `--swgl`), иначе программный swgl.
fn screen(window: &Window, size: dpi::PhysicalSize<u32>, wanted: bool, profile_dir: &std::path::Path) -> Screen {
    #[allow(unused_mut)]
    let mut failed = false;
    #[cfg(windows)]
    if let Some((display, handle)) = window.gl_handles().filter(|_| wanted && !std::env::args().any(|a| a == "--swgl")) {
        let mark = profile_dir.join(GPU_MARK);
        if mark.exists() {
            let _ = std::fs::remove_file(&mark);
            println!("browser: gpu skipped: previous start did not finish");
            failed = true;
        } else {
            let _ = std::fs::write(&mark, b"1");
            let hz = window.refresh_hz();
            let vsync: Rc<dyn servo::RefreshDriver> = crate::gpu_ctx::Vsync::new(hz);
            let window_size = dpi::PhysicalSize::new(window.width(), window.height());
            match crate::gpu_ctx::GpuOutput::new(display, handle, window_size, size, vsync.clone()) {
                Ok((gpu, context)) => {
                    let renderer = gpu.renderer_name();
                    println!("browser: screen {hz} Hz");
                    return Screen { context, gpu: Some(gpu), vsync: Some(vsync), renderer, failed: false };
                }
                Err(e) => {
                    let _ = std::fs::remove_file(&mark);
                    println!("browser: gpu unavailable: {e}");
                }
            }
        }
    }
    let _ = (window, wanted, profile_dir);
    Screen { context: SwglRenderingContext::new(size), gpu: None, vsync: None, renderer: swgl_ctx::renderer_name(), failed }
}

/// Убрать служебные `*-shm` баз SQLite под `dir`: данных в них нет (SQLite
/// восстанавливает их из журнала `-wal`). Только Windows: там файл, открытый
/// другим окном OBSOS, система удалить не даст — значит, удаляются лишь
/// брошенные. В прочих системах удаление открытого файла опасно.
fn remove_stale_shm(dir: &std::path::Path, depth: u32) {
    if !cfg!(windows) || depth > 6 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            remove_stale_shm(&path, depth + 1);
        } else if path.file_name().is_some_and(|n| n.to_string_lossy().ends_with(".sqlite-shm")) {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// Подключения инструментов разработчика Firefox (сервер включён ключом).
struct DevtoolsDelegate;

impl servo::ServoDelegate for DevtoolsDelegate {
    fn notify_devtools_server_started(&self, port: u16, _token: String) {
        println!("browser: devtools server on 127.0.0.1:{port} (Firefox: about:debugging)");
    }

    fn request_devtools_connection(&self, request: servo::AllowOrDenyRequest) {
        println!("browser: devtools client connected");
        request.allow();
    }
}

/// Масштаб интерфейса по настройке (проценты, 0 — как у экрана системы).
fn effective_scale(setting: u32, window: &Window) -> f32 {
    match setting {
        0 => window.scale_factor().clamp(0.75, 3.0),
        percent => percent as f32 / 100.0,
    }
}

/// Мобильный режим (узкий экран): мобильный User-Agent и масштаб страниц 2.
pub static MOBILE: AtomicBool = AtomicBool::new(false);

/// Палец на странице: где коснулся, где сейчас, ведёт ли (прокрутка).
struct Touch {
    start: (i32, i32),
    last: (i32, i32),
    dragging: bool,
}

/// Своё у каждого окна: интерфейс egui и состояние ввода.
struct View {
    /// Номер окна в оболочке (`windows.rs`).
    key: u32,
    window: Rc<Window>,
    /// Номер поверхности в `gpu_ctx`.
    surface: usize,
    ctx: egui::Context,
    look: Look,
    events: Vec<egui::Event>,
    /// Оболочку надо перерисовать.
    input: bool,
    mask: i32,
    mouse: DevicePoint,
    ui_due: Option<Instant>,
    ui_drawn: bool,
    over_ui: bool,
    /// Указатель над страницей (а не над оболочкой) — форму курсора выбирает
    /// страница; иначе — egui.
    over_page: bool,
    ui_cursor: egui::CursorIcon,
    /// Плавная прокрутка: сколько точек ещё прокрутить и когда был шаг.
    scroll_left: (f32, f32),
    scroll_at: Instant,
    scroll_where: DevicePoint,
    /// С видеокартой шаг прокрутки — по такту экрана: отметка «такт прошёл».
    tick: Arc<AtomicBool>,
    tick_asked: bool,
    was_fullscreen: bool,
    scale_setting: u32,
    touch: Option<Touch>,
}

impl View {
    fn new(key: u32, window: Rc<Window>, surface: usize, look: Look, scale_setting: u32) -> View {
        let ctx = egui::Context::default();
        ui::fonts(&ctx);
        ui::style(&ctx, &look);
        View {
            key,
            window,
            surface,
            ctx,
            look,
            events: Vec::new(),
            input: false,
            mask: 0,
            mouse: DevicePoint::new(0.0, 0.0),
            ui_due: Some(Instant::now()),
            ui_drawn: false,
            over_ui: false,
            over_page: false,
            ui_cursor: egui::CursorIcon::Default,
            scroll_left: (0.0, 0.0),
            scroll_at: Instant::now(),
            scroll_where: DevicePoint::new(0.0, 0.0),
            tick: Arc::new(AtomicBool::new(false)),
            tick_asked: false,
            was_fullscreen: false,
            scale_setting,
            touch: None,
        }
    }
}

/// Что сделать с окнами после витка.
enum Request {
    /// Закрыть окно (крестик).
    Close(u32),
    /// Оболочка окна сказала «хватит»: выход или не осталось вкладок.
    Refused(u32),
    Open,
    Move { from: u32, tab: usize, to: u32, at: usize },
    /// Вкладку отпустили вне полосы вкладок: точка экрана.
    TearOff { from: u32, tab: usize, at: (i32, i32) },
}

/// Точка окна у полосы вкладок (с запасом), куда вкладку можно положить.
fn near_tab_strip(look: &Look, window: &Window, x: i32, y: i32) -> bool {
    let (w, h) = (window.width() as i32, window.height() as i32);
    let slack = shell::px(28) as i32;
    if x < -slack || y < -slack || x > w + slack || y > h + slack {
        return false;
    }
    match look.tabs {
        TabsAt::Top => y <= shell::px(shell::TABSTRIP_H) as i32 + slack,
        TabsAt::Bottom => y >= h - shell::px(shell::TABSTRIP_H) as i32 - slack,
        TabsAt::Left => x <= shell::px(look.sidebar_width) as i32 + slack,
        TabsAt::Right => x >= w - shell::px(look.sidebar_width) as i32 - slack,
    }
}

/// Место в полосе вкладок по точке окна (повторяет раскладку `chrome.rs`).
fn drop_index(look: &Look, window: &Window, x: i32, y: i32, tabs: usize) -> usize {
    let k = shell::ui_scale();
    let at = if look.tabs.is_side() {
        let first = (ui::BRAND_H + ui::NEW_TAB_H + 9.0) * k;
        (y as f32 - first) / ((ui::TAB_ROW_H + 2.0) * k)
    } else {
        let width = ((window.width() as f32 / k - 56.0) / tabs.max(1) as f32).clamp(36.0, 232.0);
        (x as f32 / k - 6.0) / width
    };
    (at.round().max(0.0) as usize).min(tabs)
}

/// Новое окно (без вкладок) по образцу окна `like`.
fn open_view(shell: &mut Shell, views: &mut Vec<View>, order: &mut Vec<u32>, gpu: Option<&mut Gpu>, like: usize, at: Option<(i32, i32)>) -> Option<u32> {
    let gpu = gpu?;
    let source = &views[like];
    let (width, height) = (source.window.width(), source.window.height());
    let window = source.window.open_window("OBSOS — Браузер", width, height, at)?;
    let (_, handle) = window.gl_handles()?;
    let surface = match gpu.add_window(handle, dpi::PhysicalSize::new(window.width(), window.height())) {
        Ok(surface) => surface,
        Err(e) => {
            println!("browser: window: {e}");
            return None;
        }
    };
    shell.switch(source.key);
    let key = shell.add_window();
    let view = View::new(key, window, surface, shell.look().clone(), shell.profile.settings.ui_scale);
    shell.switch(key);
    shell::set_ui_scale(effective_scale(shell.profile.settings.ui_scale, &*view.window));
    shell.resize_all(&*view.window);
    views.push(view);
    order.retain(|k| *k != key);
    order.insert(0, key);
    println!("browser: window opened ({} open)", views.len());
    Some(key)
}

/// Закрыть окно `key` вместе с его вкладками.
fn close_view(shell: &mut Shell, views: &mut Vec<View>, order: &mut Vec<u32>, gpu: Option<&mut Gpu>, key: u32) {
    let Some(at) = views.iter().position(|v| v.key == key) else { return };
    shell.remove_window(key);
    let view = views.remove(at);
    if let Some(gpu) = gpu {
        // Поверхность — до уничтожения самого окна.
        gpu.remove_window(view.surface);
    }
    view.window.close();
    order.retain(|k| *k != key);
    if let Some(next) = order.first().copied().or_else(|| views.first().map(|v| v.key)) {
        shell.switch(next);
    }
    // Участки остальных окон сдвинулись.
    for view in views.iter_mut() {
        view.ui_due = Some(Instant::now());
    }
    println!("browser: window closed ({} open)", views.len());
}

/// Перенести вкладку между окнами без перезагрузки страницы.
fn move_tab(shell: &mut Shell, views: &mut Vec<View>, order: &mut Vec<u32>, gpu: Option<&mut Gpu>, from: u32, tab: usize, to: u32, at: usize) {
    if !shell.switch(from) || !shell.move_tab_to(tab, to, at) {
        return;
    }
    println!("browser: tab {} moved to window {to}", tab + 1);
    let left = shell.tabs.len();
    if left > 0 {
        let window_rc = views.iter().find(|v| v.key == from).map(|v| v.window.clone());
        if let Some(window) = window_rc {
            let next = if shell.active > tab || shell.active >= left { shell.active.saturating_sub(1) } else { shell.active };
            shell.activate(next.min(left - 1), &*window);
        }
    }
    shell.switch(to);
    if let Some(view) = views.iter_mut().find(|v| v.key == to) {
        shell.resize_all(&*view.window);
        shell.activate(shell.active, &*view.window);
        view.ui_due = Some(Instant::now());
        view.window.focus();
    }
    order.retain(|k| *k != to);
    order.insert(0, to);
    if left == 0 {
        close_view(shell, views, order, gpu, from);
        shell.switch(to);
    } else if let Some(view) = views.iter_mut().find(|v| v.key == from) {
        view.ui_due = Some(Instant::now());
    }
}

/// Открыть окно (заголовок, ширина, высота) и работать до его закрытия.
/// Ключи — из командной строки процесса. Службы платформы
/// (`window::install`) должны быть поставлены заранее.
pub fn run(open_window: impl FnOnce(&str, u32, u32) -> Option<Rc<Window>>) -> ! {
    let _ = log::set_logger(&LOGGER).map(|()| log::set_max_level(log::LevelFilter::Warn));
    // Дом должен быть записываемым (профиль, кеш и база Servo). На телефоне
    // корень — FAT32 только на чтение, а записываемый том в памяти ядро
    // монтирует в `/home`: туда, если в `HOME` писать нельзя.
    let writable = |dir: &std::path::Path| std::fs::create_dir_all(dir.join(".config")).is_ok();
    if cfg!(target_os = "freeos") && !std::env::var_os("HOME").map(std::path::PathBuf::from).is_some_and(|home| writable(&home)) && writable(std::path::Path::new("/home")) {
        println!("browser: home /home (HOME is not writable)");
        // SAFETY (edition 2021: безопасно): до запуска любых потоков.
        std::env::set_var("HOME", "/home");
    }
    let t0 = Instant::now();
    println!("browser: start");
    let mut args = std::env::args().skip(1).peekable();
    let (mut profile_name, mut first_url) = (String::from("default"), None);
    let mut sync_folder: Option<String> = None;
    let mut private = false;
    let mut control: Option<crate::automation::Control> = None;
    let mut size: Option<(u32, u32)> = None;
    while let Some(arg) = args.next() {
        if arg == "--private" {
            private = true;
        } else if arg == "--control" {
            // Команды из файла — для проверок без человека (`automation.rs`).
            control = args.next().map(|f| crate::automation::Control::new(f.into()));
        } else if arg == "--size" {
            size = args.next().and_then(|s| s.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?))));
        } else if arg == "--no-jit" || arg == "--no-offthread" || arg == "--swgl" || arg == "--console" {
            // Разбираются ниже или платформой.
        } else if arg == "--pref" {
            // Значение разбирается ниже, при настройке движка.
            args.next();
        } else if arg == "--devtools" {
            // Порт — необязательный: адресом он не считается.
            if args.peek().is_some_and(|a| a.parse::<u16>().is_ok()) {
                args.next();
            }
        } else if arg == "--profile" {
            profile_name = args.next().unwrap_or_default();
        } else if arg == "--sync-folder" {
            // Синхронизация в папку без панели — для стенда и для общей
            // папки (флешка, сетевой диск): площадка проверяется,
            // сохраняется в хранилище и включается, как из «Синхронизации».
            sync_folder = args.next();
        } else {
            first_url = Some(arg);
        }
    }
    let (width, height) = size.unwrap_or((1024, 700));
    let Some(window_rc) = open_window(if private { "OBSOS — Инкогнито" } else { "OBSOS — Браузер" }, width, height) else {
        std::process::exit(1);
    };
    let window: &Window = &*window_rc;
    // TMPDIR и прочие каталоги Servo — до запуска его потоков.
    let _ = swgl_ctx::prepare_dirs(None);
    // Часовой пояс для JavaScript (`Date`, `Intl`): SpiderMonkey берёт его из
    // `TZ`, а пояс знает ядро (SYS_TZINFO, «Параметры → Дата и время»).
    if std::env::var_os("TZ").is_none() {
        if let Some(minutes) = window::timezone_minutes() {
            let tz = tz_name(minutes);
            println!("browser: timezone {tz}");
            // SAFETY (edition 2021: безопасно): до запуска потоков Servo.
            std::env::set_var("TZ", tz);
        }
    }
    crate::logs::set_private(private);
    let profile = if private { Profile::open_private() } else { Profile::open(&profile_name) };
    let blocker = Rc::new(RefCell::new(blocker::Blocker::load(&profile.dir, profile.settings.block_ads)));
    let downloads = Rc::new(downloads::Downloads::new());
    println!("browser: profile {} ({}), device {}", profile.name, profile.dir.display(), profile.settings.device);
    let (vault, need_master) = match Vault::open(&profile.dir, None) {
        Ok(v) => (Some(v), false),
        Err(OpenError::NeedMaster) => (None, true),
        Err(OpenError::Broken(why)) => {
            println!("browser: vault broken: {why}");
            (None, false)
        }
    };

    // Окно — того же размера, что в прошлый раз (кроме явного `--size`).
    let geometry_file = profile.dir.join("window.txt");
    if size.is_none() {
        let saved = std::fs::read_to_string(&geometry_file).unwrap_or_default();
        let mut parts = saved.split_whitespace().filter_map(|p| p.parse::<u32>().ok());
        if let (Some(w), Some(h), Some(maximized)) = (parts.next(), parts.next(), parts.next()) {
            if (320..=16_000).contains(&w) && (240..=16_000).contains(&h) {
                window.set_geometry(w, h, maximized != 0);
            }
        }
    }
    // Масштаб оболочки и страниц — до первого расчёта геометрии.
    shell::set_ui_scale(effective_scale(profile.settings.ui_scale, window));
    let page = Rc::new(RefCell::new(PageFrame::default()));
    let Screen { context, mut gpu, vsync, renderer: renderer_name, failed: gpu_failed } =
        screen(window, page_size(&profile.settings.look, window.width(), window.height()), profile.settings.gpu, &profile.dir);
    let gpu_active = gpu.is_some();
    {
        let (page, swap_rb) = (page.clone(), window.swap_rb());
        context.set_present_hook(Box::new(move |frame, w, h| {
            if frame.is_empty() {
                page.borrow_mut().mark(w as usize, h as usize);
            } else {
                page.borrow_mut().store(frame, w, h, swap_rb);
            }
        }));
    }
    let _ = context.make_current();
    println!("browser: renderer {renderer_name}");

    let page_data = Arc::new(Mutex::new(PageData::default()));
    let mut protocols = ProtocolRegistry::with_internal_protocols();
    if protocols.register("obsos", ObsosProtocol { data: page_data.clone() }).is_err() {
        println!("browser: obsos:// not registered");
    }
    let opts = servo::Opts { config_dir: Some(profile.servo_dir()), ..servo::Opts::default() };
    // Узкий экран (телефон, 10.11): сайты отдают мобильную версию по
    // User-Agent, `<meta name=viewport>` учитывается, а страница рисуется
    // в масштабе 2 — 720 точек экрана это 360 CSS-пикселей, как у телефонов.
    let mut prefs = swgl_ctx::preferences();
    // Ключи для поиска сбоев (10.11, телефон вставал на m.vk.com):
    // `--no-jit` — JavaScript без JIT, `--no-offthread` — без фоновой
    // компиляции в отдельных потоках.
    if std::env::args().any(|a| a == "--no-jit") {
        prefs.js_disable_jit = true;
        prefs.js_baseline_jit_enabled = false;
        prefs.js_ion_enabled = false;
        println!("browser: JIT off (--no-jit)");
    }
    if std::env::args().any(|a| a == "--no-offthread") {
        prefs.js_offthread_compilation_enabled = false;
        println!("browser: off-thread JS compilation off (--no-offthread)");
    }
    // Телефонный режим — только на FossAi: узкое окно на настольной системе
    // телефоном его не делает (страницы становились вдвое крупнее).
    if cfg!(target_os = "freeos") && window.width() < 800 {
        MOBILE.store(true, Ordering::Relaxed);
        prefs.user_agent = String::from("Mozilla/5.0 (Android 10; Mobile; rv:153.0) Servo/0.8.0 Firefox/153.0");
        prefs.viewport_meta_enabled = true;
        println!("browser: mobile mode (narrow screen)");
    }
    // `--pref имя=значение` — любая настройка движка (поиск сбоев).
    let mut pairs = std::env::args();
    while let Some(arg) = pairs.next() {
        if arg != "--pref" {
            continue;
        }
        let Some((name, value)) = pairs.next().and_then(|p| p.split_once('=').map(|(n, v)| (n.to_string(), v.to_string()))) else { continue };
        use servo::PrefValue;
        let value = match (value.parse::<bool>(), value.parse::<i64>()) {
            (Ok(b), _) => PrefValue::Bool(b),
            (_, Ok(n)) => PrefValue::Int(n),
            _ => PrefValue::Str(value),
        };
        println!("browser: pref {name} = {value:?}");
        prefs.set_value(&name, value);
    }
    // Скорость загрузки: оставшиеся с прошлого запуска `*-shm` SQLite
    // (OBSOS выходит без разбора Servo) задерживали первое обращение каждого
    // сайта к localStorage/IndexedDB на ~1,6 с — страница всё это время
    // стояла (`localStorage.setItem`).
    remove_stale_shm(&profile.servo_dir(), 0);
    // Звонки и микрофон: движок спрашивает разрешение у оболочки.
    if window::platform().webrtc_available() {
        prefs.dom_webrtc_enabled = true;
        prefs.dom_webrtc_transceiver_enabled = true;
    }
    // Сервер инструментов разработчика Servo (Firefox: about:debugging) —
    // только по ключу `--devtools [порт]`, слушает свой компьютер.
    let devtools_port = std::env::args().skip_while(|a| a != "--devtools").nth(1).map(|p| p.parse::<u16>().unwrap_or(6080));
    let devtools_port = devtools_port.or_else(|| std::env::args().any(|a| a == "--devtools").then_some(6080));
    if let Some(port) = devtools_port {
        prefs.devtools_server_enabled = true;
        prefs.devtools_server_listen_address = format!("127.0.0.1:{port}");
    }
    let wake: Option<Arc<dyn Fn() + Send + Sync>> = window.waker().map(Arc::from);
    let servo = ServoBuilder::default()
        .opts(opts)
        .preferences(prefs)
        .protocol_registry(protocols)
        .event_loop_waker(Box::new(Waker(wake.clone())))
        .build();
    if devtools_port.is_some() {
        servo.set_delegate(Rc::new(DevtoolsDelegate));
    }
    // Маски CSS (`mask-image`): значки рисует наша правка вёрстки Servo
    // (patches/README.md). Переключателя в настройках Servo нет — это Stylo.
    // На FossAi не проверялось.
    if !cfg!(target_os = "freeos") {
        stylo_static_prefs::set_pref!("layout.css.masking.enabled", true);
    }
    let content = content_manager(&servo, &profile);
    let delegate = Rc::new(Delegate {
        notes: RefCell::new(Vec::new()),
        context: context.clone(),
        clipboard: Rc::new(RefCell::new(String::new())),
        content,
        page_cursor: std::cell::Cell::new(window::CursorShape::Arrow),
        blocker: blocker.clone(),
        downloads: downloads.clone(),
        devlog: RefCell::new(devtools::DevLog::default()),
        devtools_open: std::cell::Cell::new(false),
        page_opened: Default::default(),
        held_loads: RefCell::new(Vec::new()),
    });
    let draft_provider = vault.as_ref().and_then(|v| v.account().cloned());
    let mut shell = Shell {
        servo,
        context: context.clone(),
        delegate,
        tabs: Vec::new(),
        active: 0,
        address: String::new(),
        focus_address: false,
        opened: 0,
        profile,
        vault,
        page_data,
        panel: None,
        modal: need_master.then(|| Modal::Master { text: String::new(), error: String::new() }),
        infobars: Vec::new(),
        toasts: Vec::new(),
        filter: String::new(),
        syncer: Syncer::start(),
        sync: SyncUi {
            draft: draft_provider.unwrap_or_else(|| browser_sync::remote::Account::new(browser_sync::remote::Provider::GitHub)),
            label: String::new(),
            points: Vec::new(),
            status: String::new(),
            flow: None,
            sync_password: String::new(),
            // Первая автосинхронизация — через 10 с после запуска.
            next_auto: Instant::now() + Duration::from_secs(10),
            last_report: String::new(),
        },
        show_passwords: Default::default(),
        new_profile: String::new(),
        auth_tries: Vec::new(),
        find: Default::default(),
        tools: Default::default(),
        dev: Default::default(),
        closed: Vec::new(),
        quit: false,
        dl: Default::default(),
        webui: Default::default(),
        ui_token: crate::vault::random::<16>().iter().map(|b| format!("{b:02x}")).collect(),
        chrome: Default::default(),
        background_click: None,
        page_fullscreen: None,
        gpu_available: cfg!(windows),
        gpu_active,
        renderer_name,
        windows: Default::default(),
        frame_wanted: None,
        permissions: Vec::new(),
        granted: Default::default(),
    };
    if gpu_failed {
        shell.profile.settings.gpu = false;
        shell.profile.save_settings();
        shell.toast("Прошлый запуск с видеокартой не удался — выключено в «Настройки → Поиск и запуск»");
    }
    let mut gpu_mark = shell.gpu_active.then(|| shell.profile.dir.join(GPU_MARK));
    shell.apply_download_settings();
    if let Some(v) = &shell.vault {
        let key = v.sync_key;
        shell.profile.reload_logins(&key);
    }
    shell.update_page_data();

    // Первая вкладка: адрес из командной строки, иначе прошлый сеанс, иначе
    // домашняя страница.
    let session = shell.profile.settings.restore_session.then(|| shell.profile.load_session()).flatten();
    match (first_url.and_then(|a| shell.url_from_input(&a)), session) {
        (Some(url), _) => shell.new_tab(url, window),
        (None, Some((urls, active))) => {
            for u in &urls {
                if let Ok(url) = url::Url::parse(u) {
                    shell.new_tab(url, window);
                }
            }
            if shell.tabs.is_empty() {
                let home = shell.home_url();
                shell.new_tab(home, window);
            } else {
                shell.activate(active.min(shell.tabs.len() - 1), window);
            }
        }
        (None, None) => {
            let home = shell.home_url();
            shell.new_tab(home, window);
        }
    }
    println!("browser: servo up in {:?}", t0.elapsed());
    if let Some(dir) = sync_folder {
        let mut account = browser_sync::remote::Account::new(browser_sync::remote::Provider::Folder);
        account.repo = dir;
        shell.sync.draft = account;
        shell.check_account();
    }

    // Окно — своё состояние интерфейса и ввода; первое — то, что открыла
    // платформа.
    let mut views = vec![View::new(0, window_rc.clone(), 0, shell.look().clone(), shell.profile.settings.ui_scale)];
    // Узкий экран (телефон, 720 точек): вкладки сверху — боковая панель
    // съела бы треть ширины. Только на этот запуск, в профиль не пишется.
    if window.width() < 800 && shell.profile.settings.look.tabs.is_side() {
        shell.profile.settings.look.tabs = TabsAt::Top;
        shell.resize_all(window);
        views[0].look = shell.look().clone();
        ui::style(&views[0].ctx, &views[0].look);
    }
    let mut renderer = EguiSoftwareRender::new(if window.swap_rb() { ColorFieldOrder::Rgba } else { ColorFieldOrder::Bgra });
    let mut actions: Vec<Action> = Vec::new();
    let mut ui_frames = 0u64;
    let started = Instant::now();
    let mut last_session = Instant::now();
    // Палец на странице (мобильный режим): касание ведёт — прокрутка, нет —
    // по отпусканию это щелчок там, где коснулись.
    let mobile = MOBILE.load(Ordering::Relaxed);
    // Ввод проверочного сценария (`--control`) — тем же путём, что от окна.
    let mut scripted: std::collections::VecDeque<WinEvent> = Default::default();
    let perf_always = std::env::var_os("OBSOS_PERF").is_some();
    let mut perf_since = Instant::now();
    // Кадр анимации оболочки — не чаще кадра экрана.
    let ui_step = match gpu {
        Some(_) => Duration::from_secs(1) / window.refresh_hz().clamp(30, 500),
        None => Duration::from_millis(16),
    };
    // Номера окон от недавно получавших ввод; первое — «в фокусе».
    let mut order: Vec<u32> = vec![0];
    // Окно, которому идут команды и ввод сценария.
    let mut target: Option<u32> = None;

    'run: loop {
        let mut requests: Vec<Request> = Vec::new();
        for v in &views {
            if v.window.take_focused() {
                let key = v.key;
                order.retain(|k| *k != key);
                order.insert(0, key);
            }
        }
        let focus = order.first().copied().unwrap_or(0);
        let scripted_to = target.filter(|t| views.iter().any(|v| v.key == *t)).unwrap_or(focus);
        let mut command = None;
        if let Some(control) = control.as_mut() {
            shell.switch(scripted_to);
            let loading = shell.tabs.get(shell.active).is_some_and(|t| t.loading);
            command = control.poll(loading, &mut scripted);
        }
        if let Some(command) = command {
            shell.switch(scripted_to);
            let at = views.iter().position(|v| v.key == scripted_to).unwrap_or(0);
            let window_rc = views[at].window.clone();
            let window: &Window = &*window_rc;
            match command {
                crate::automation::Command::Go(url) => {
                    shell.run(Action::Go(url), window);
                    views[at].input = true;
                }
                crate::automation::Command::Eval(code) => {
                    if let Some(w) = shell.active_view() {
                        w.evaluate_javascript(code, |result| {
                            let text = match result {
                                Ok(v) => devtools::show_value(&v),
                                Err(e) => format!("error {e:?}"),
                            };
                            println!("browser: control eval {text}");
                        });
                    }
                }
                crate::automation::Command::Shot(path) => {
                    let (ww, wh) = (window.width() as usize, window.height() as usize);
                    let look = &views[at].look;
                    let origin = page_origin(look);
                    let pixels = match gpu.as_mut() {
                        Some(gpu) => gpu.snapshot(views[at].surface, (ww as u32, wh as u32), origin, shell.windows.region(scripted_to)),
                        None => {
                            let mut pixels = vec![0u32; ww * wh];
                            compose_into(&mut pixels, (ww, wh), &page.borrow(), (origin.0 as usize, origin.1 as usize), &mut renderer);
                            pixels
                        }
                    };
                    match crate::automation::save_bmp(&path, &pixels, ww, wh) {
                        Ok(()) => println!("browser: control shot {}", path.display()),
                        Err(e) => println!("browser: control shot failed: {e}"),
                    }
                }
                crate::automation::Command::Perf => {
                    println!("browser: {}", perf::report(perf_since.elapsed().as_secs_f32()));
                    perf_since = Instant::now();
                }
                crate::automation::Command::Window(n) => target = views.get(n).map(|v| v.key),
                crate::automation::Command::Windows => {
                    for (n, v) in views.iter().enumerate() {
                        shell.switch(v.key);
                        let tabs: Vec<&str> = shell.tabs.iter().map(|t| t.title.as_str()).collect();
                        println!("browser: control window {n}: {}x{} at {:?}, active {}, tabs {tabs:?}", v.window.width(), v.window.height(), v.window.inner_origin(), shell.active + 1);
                    }
                }
                crate::automation::Command::MoveTab(tab, n) => {
                    if let Some(to) = views.get(n).map(|v| v.key) {
                        requests.push(Request::Move { from: scripted_to, tab, to, at: usize::MAX });
                    }
                }
                crate::automation::Command::Quit => break 'run,
            }
        }

        // ── Ввод окон ──────────────────────────────────────────────────────
        for win in views.iter_mut() {
            let key = win.key;
            shell.switch(key);
            let window_rc = win.window.clone();
            let window: &Window = &*window_rc;
            let look = win.look.clone();
            let ctx = win.ctx.clone();
            let mut events = std::mem::take(&mut win.events);
            let mut input = std::mem::take(&mut win.input);
            let mut mask = win.mask;
            let mut mouse = win.mouse;
            let mut over_ui = win.over_ui;
            let mut over_page = win.over_page;
            let mut ui_due = win.ui_due;
            let mut scroll_left = win.scroll_left;
            let mut scroll_at = win.scroll_at;
            let mut scroll_where = win.scroll_where;
            let mut touch = win.touch.take();
            while let Some(ev) = (key == scripted_to).then(|| scripted.pop_front()).flatten().or_else(|| window.next_event()) {
                let k = shell::ui_scale();
                let (ox, oy, pw, ph) = shell::page_area(&look, window.width(), window.height());
                let in_page = |x: i32, y: i32| x >= ox as i32 && y >= oy as i32 && x < (ox + pw) as i32 && y < (oy + ph) as i32;
                // Точка страницы — в общем буфере, у окна там свой участок.
                let [rx, ry, ..] = shell.windows.region(key);
                let page_point = |x: i32, y: i32| DevicePoint::new((x - ox as i32) as f32 + rx as f32, (y - oy as i32) as f32 + ry as f32);
                // Точка окна → точка интерфейса (egui).
                let ui_pos = |x: i32, y: i32| egui::pos2(x as f32 / k, y as f32 / k);
                // Движение указателя по странице оболочку не меняет — перерисовывать
                // её на каждое не нужно.
                let was_over_page = over_page;
                let view = shell.active_view().cloned();
            let blocked = shell.modal.is_some();
            match ev.kind {
                WIN_CLOSE => requests.push(Request::Close(key)),
                WIN_RESIZE => {
                    let (w, h) = (ev.x.max(1) as u32, ev.y.max(1) as u32);
                    println!("browser: resize asked {w}x{h}");
                    if window.resize(w, h) {
                        shell.resize_all(window);
                        println!("browser: resized {w}x{h}");
                    } else {
                        println!("browser: resize to {w}x{h} failed: {}", std::io::Error::last_os_error());
                    }
                }
                WIN_MOVE => {
                    let pos = ui_pos(ev.x, ev.y);
                    events.push(egui::Event::PointerMoved(pos));
                    over_ui = ctx.layer_id_at(pos).is_some_and(|l| l.order != egui::Order::Background);
                    over_page = in_page(ev.x, ev.y) && !over_ui && !blocked && shell.chrome.dragged_tab().is_none();
                    if let Some(t) = touch.as_mut() {
                        if !t.dragging && ((ev.x - t.start.0).abs() > 12 || (ev.y - t.start.1).abs() > 12) {
                            t.dragging = true;
                        }
                        if t.dragging {
                            let delta = DeviceVector2D::new((t.last.0 - ev.x) as f32, (t.last.1 - ev.y) as f32);
                            t.last = (ev.x, ev.y);
                            if let Some(w) = &view {
                                w.notify_scroll_event(Scroll::Delta(WebViewVector::Device(delta)), WebViewPoint::Device(page_point(t.start.0, t.start.1)));
                            }
                        }
                    } else if over_page {
                        mouse = page_point(ev.x, ev.y);
                        if let Some(w) = &view {
                            w.notify_input_event(InputEvent::MouseMove(MouseMoveEvent::new(WebViewPoint::Device(mouse))));
                        }
                    } else if was_over_page {
                        // Ушёл со страницы на оболочку: наведение и подсказка
                        // ссылки гаснут.
                        shell.pointer_left_page();
                    }
                }
                WIN_LEAVE => {
                    events.push(egui::Event::PointerGone);
                    if std::mem::take(&mut over_page) {
                        shell.pointer_left_page();
                    }
                }
                WIN_POINTER | WIN_RELEASE if ev.code == 4 || ev.code == 5 => {
                    // Боковые кнопки мыши — назад и вперёд.
                    if ev.kind == WIN_RELEASE && !blocked {
                        shell.run(if ev.code == 4 { Action::Back } else { Action::Forward }, window);
                    }
                }
                WIN_POINTER | WIN_RELEASE => {
                    let pressed = ev.kind == WIN_POINTER;
                    let button = match ev.code {
                        2 => egui::PointerButton::Secondary,
                        3 => egui::PointerButton::Middle,
                        _ => egui::PointerButton::Primary,
                    };
                    let pos = ui_pos(ev.x, ev.y);
                    over_ui = ctx.layer_id_at(pos).is_some_and(|l| l.order != egui::Order::Background);
                    events.push(egui::Event::PointerMoved(pos));
                    events.push(egui::Event::PointerButton { pos, button, pressed, modifiers: egui_modifiers(mask) });
                    let dragged = shell.chrome.dragged_tab();
                    if let (false, 1, Some(tab)) = (pressed, ev.code, dragged) {
                        if gpu.is_some() && !near_tab_strip(&look, window, ev.x, ev.y) {
                            if let Some((x, y)) = window.inner_origin() {
                                requests.push(Request::TearOff { from: key, tab, at: (x + ev.x, y + ev.y) });
                            }
                        }
                    }
                    let to_page = in_page(ev.x, ev.y) && !over_ui && !blocked && dragged.is_none();
                    // Средняя кнопка и Ctrl+щелчок по ссылке: новую вкладку
                    // Servo открывает сам, а оболочка оставляет её в фоне.
                    if to_page && !pressed && (ev.code == 3 || (ev.code == 1 && mask & 2 != 0)) {
                        shell.background_click = Some(Instant::now());
                    }
                    if mobile && ev.code != 2 && (touch.is_some() || (pressed && to_page)) {
                        if pressed {
                            touch = Some(Touch { start: (ev.x, ev.y), last: (ev.x, ev.y), dragging: false });
                            shell.panel = None;
                        } else if let Some(t) = touch.take() {
                            if !t.dragging {
                                // Не вёл пальцем — щелчок там, где коснулся.
                                mouse = page_point(t.start.0, t.start.1);
                                if let Some(w) = &view {
                                    w.set_focused(true);
                                    let at = WebViewPoint::Device(mouse);
                                    w.notify_input_event(InputEvent::MouseMove(MouseMoveEvent::new(at)));
                                    w.notify_input_event(InputEvent::MouseButton(MouseButtonEvent::new(MouseButtonAction::Down, MouseButton::Primary, at)));
                                    w.notify_input_event(InputEvent::MouseButton(MouseButtonEvent::new(MouseButtonAction::Up, MouseButton::Primary, at)));
                                }
                                println!("browser: tap at {},{}", t.start.0, t.start.1);
                            }
                        }
                    } else if to_page {
                        mouse = page_point(ev.x, ev.y);
                        if let Some(w) = &view {
                            let button = match ev.code {
                                2 => MouseButton::Secondary,
                                3 => MouseButton::Auxiliary,
                                _ => MouseButton::Primary,
                            };
                            let action = if pressed { MouseButtonAction::Down } else { MouseButtonAction::Up };
                            if pressed {
                                w.set_focused(true);
                                // Щелчок по странице закрывает выдвижную панель
                                // (встроенные инструменты разработчика — нет).
                                if shell.panel != Some(Panel::DevTools) {
                                    shell.panel = None;
                                }
                            }
                            w.notify_input_event(InputEvent::MouseButton(MouseButtonEvent::new(action, button, WebViewPoint::Device(mouse))));
                        }
                    }
                    if pressed {
                        println!("browser: click at {},{}{}", ev.x, ev.y, if to_page { " (page)" } else { " (shell)" });
                    }
                }
                WIN_WHEEL => {
                    // `x` — строки по вертикали (вверх — больше нуля), `y` —
                    // по горизонтали.
                    println!("browser: wheel {} at {},{}", ev.x, mouse.x, mouse.y);
                    // Что зажато во время прокрутки: от окна, если оно знает
                    // (клавиши могли прийти не нам), иначе по клавишам.
                    let held = if ev.code & WIN_WHEEL_MODS != 0 { (ev.code & 7) as i32 } else { mask };
                    if over_ui || blocked {
                        events.push(egui::Event::MouseWheel {
                            unit: egui::MouseWheelUnit::Line,
                            delta: egui::vec2(ev.y as f32, ev.x as f32),
                            phase: egui::TouchPhase::Move,
                            modifiers: egui_modifiers(held),
                        });
                    } else if held & 2 != 0 {
                        // Ctrl+колесо — масштаб страницы.
                        if ev.x != 0 {
                            shell.run(Action::Zoom(if ev.x > 0 { 1.1 } else { 1.0 / 1.1 }), window);
                        }
                    } else {
                        // Щелчок колеса — 100 точек CSS, как в Chromium.
                        let step = 100.0 * shell::page_scale();
                        let (mut dx, mut dy) = (-(ev.y as f32) * step, -(ev.x as f32) * step);
                        if held & 1 != 0 && dx == 0.0 {
                            (dx, dy) = (dy, 0.0);
                        }
                        // Смена направления — не дожидаясь остатка.
                        if dx * scroll_left.0 < 0.0 {
                            scroll_left.0 = 0.0;
                        }
                        if dy * scroll_left.1 < 0.0 {
                            scroll_left.1 = 0.0;
                        }
                        if scroll_left == (0.0, 0.0) {
                            scroll_at = Instant::now() - Duration::from_millis(16);
                        }
                        scroll_left.0 += dx;
                        scroll_left.1 += dy;
                        scroll_where = mouse;
                    }
                }
                WIN_KEY | WIN_KEYUP => {
                    let pressed = ev.kind == WIN_KEY;
                    mask = ev.x;
                    let (shift, ctrl, alt) = (mask & 1 != 0, mask & 2 != 0, mask & 4 != 0);
                    // `y` — латинский знак физической клавиши (ядро, user-abi). По
                    // нему, а не по коду, узнаются сочетания: Ctrl+H и
                    // Ctrl+Backspace приходят одним кодом 8, а Ctrl+буква в
                    // русской раскладке — латинской клавишей.
                    let latin = char::from_u32(ev.y as u32).filter(|c| c.is_ascii_graphic()).map(|c| c.to_ascii_lowercase());
                    let letter = if ctrl || alt { latin.filter(|c| c.is_ascii_alphanumeric()) } else { None };
                    let named = |n: u32| ev.code == WIN_KEY_NAMED + n;
                    let tabs = shell.tabs.len().max(1);
                    // Раскладка Chromium (Linux).
                    let shortcut = if !pressed || blocked {
                        None
                    } else if ctrl && alt && letter == Some('r') {
                        Some(Action::Reader)
                    } else if ctrl {
                        match (letter.or(latin), shift) {
                            (Some('t'), false) => Some(Action::NewTab),
                            (Some('t'), true) => Some(Action::Reopen),
                            (Some('w'), false) => Some(Action::Close(shell.active)),
                            (Some('l' | 'k' | 'e'), false) => Some(Action::FocusAddress),
                            (Some('d'), false) => Some(Action::ToggleBookmark),
                            (Some('h'), false) => Some(Action::OpenPage("history")),
                            (Some(','), _) => Some(Action::OpenPage("settings")),
                            (Some('j'), false) => Some(Action::Panel(Panel::Downloads)),
                            (Some('c'), true) => Some(Action::Inspect),
                            (Some('n'), true) => Some(Action::PrivateWindow),
                            (Some('n'), false) => Some(Action::NewWindow),
                            (Some('i'), true) => Some(Action::Panel(Panel::DevTools)),
                            (Some('j'), true) => Some(Action::Console),
                            (Some('o' | 'b'), true) => Some(Action::Panel(Panel::Bookmarks)),
                            (Some('f'), false) => Some(Action::FindOpen),
                            (Some('g'), _) => Some(Action::Find { back: shift }),
                            (Some('r'), _) => Some(Action::Reload),
                            (Some('q'), true) => Some(Action::Quit),
                            (Some('s'), true) => Some(Action::SyncNow(None)),
                            (Some(d @ '1'..='8'), false) => {
                                let n = d as usize - '1' as usize;
                                (n < shell.tabs.len()).then_some(Action::Activate(n))
                            }
                            (Some('9'), false) => Some(Action::Activate(tabs - 1)),
                            (Some('=' | '+'), _) => Some(Action::Zoom(1.1)),
                            (Some('-'), _) => Some(Action::Zoom(1.0 / 1.1)),
                            (Some('0'), false) => Some(Action::Zoom(0.0)),
                            _ if ev.code == 9 || named(8) => {
                                // Ctrl+Tab / Ctrl+PgDn — следующая, с Shift и Ctrl+PgUp — предыдущая.
                                let back = (ev.code == 9 && shift) || named(7);
                                Some(Action::Activate(if back { (shell.active + tabs - 1) % tabs } else { (shell.active + 1) % tabs }))
                            }
                            _ if named(7) => Some(Action::Activate((shell.active + tabs - 1) % tabs)),
                            _ if named(15) => Some(Action::Close(shell.active)),
                            _ if named(9) && shift => Some(Action::OpenPage("history")),
                            _ => None,
                        }
                    } else if alt {
                        match letter {
                            Some('d') => Some(Action::FocusAddress),
                            Some('f' | 'e') => Some(Action::Panel(Panel::Menu)),
                            _ if named(1) => Some(Action::Back),
                            _ if named(2) => Some(Action::Forward),
                            _ if named(5) => Some(Action::Home),
                            _ => None,
                        }
                    } else if named(16) {
                        Some(Action::Reload)
                    } else if named(20) {
                        Some(Action::Reader)
                    } else if named(22) || (ev.code == 27 && shell::fullscreen()) {
                        Some(Action::Fullscreen)
                    } else if named(23) {
                        Some(Action::Panel(Panel::DevTools))
                    } else if named(17) {
                        Some(Action::FocusAddress)
                    } else if named(21) {
                        Some(Action::Panel(Panel::Menu))
                    } else if named(14) {
                        Some(if shell.find.open { Action::Find { back: shift } } else { Action::FindOpen })
                    } else if ev.code == 27 && shell.find.open {
                        Some(Action::FindClose)
                    } else if ev.code == 27 && shell.panel.is_some() {
                        shell.panel = None;
                        println!("browser: panel closed");
                        ui_due = Some(Instant::now());
                        None
                    } else {
                        None
                    };
                    if let Some(action) = shortcut {
                        println!("browser: shortcut {action:?}");
                        if !shell.run(action, window) {
                            requests.push(Request::Refused(key));
                        }
                        continue;
                    }
                    let to_shell = blocked || shell.focus_address || ctx.egui_wants_keyboard_input();
                    if to_shell {
                        // Буфер обмена OBSOS: Ctrl+V/C/X в поле оболочки.
                        if pressed && ctrl && letter == Some('v') {
                            let text = window::clipboard_get().unwrap_or_else(|| shell.delegate.clipboard.borrow().clone());
                            events.push(egui::Event::Paste(text));
                        } else if pressed && ctrl && letter == Some('c') {
                            events.push(egui::Event::Copy);
                        } else if pressed && ctrl && letter == Some('x') {
                            events.push(egui::Event::Cut);
                        } else if let Some(key) = letter.and_then(|l| egui::Key::from_name(&l.to_ascii_uppercase().to_string())).or_else(|| egui_key(ev.code)) {
                            events.push(egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: egui_modifiers(mask) });
                        }
                        let printable = ev.code < WIN_KEY_NAMED && ev.code >= 32 && ev.code != 127;
                        if pressed && printable && !ctrl && !alt {
                            if let Some(c) = char::from_u32(ev.code) {
                                events.push(egui::Event::Text(c.to_string()));
                            }
                        }
                    } else if let Some(w) = &view {
                        // Ctrl+буква — странице буквой (Ctrl+A, Ctrl+C в полях сайта).
                        let key = match letter {
                            Some(l) if ctrl => Some(Key::Character(l.to_string())),
                            _ => servo_key(ev.code),
                        };
                        if let Some(key) = key {
                            let state = if pressed { KeyState::Down } else { KeyState::Up };
                            let mut event = KeyboardEvent::from_state_and_key(state, key);
                            event.event.modifiers = servo_modifiers(mask);
                            w.notify_input_event(InputEvent::Keyboard(event));
                        }
                    }
                }
                _ => {}
            }
            if ev.kind != WIN_MOVE || !over_page || !was_over_page || touch.is_some() {
                input = true;
            }
            }
            win.events = events;
            win.input = input;
            win.mask = mask;
            win.mouse = mouse;
            win.over_ui = over_ui;
            win.over_page = over_page;
            win.ui_due = ui_due;
            win.scroll_left = scroll_left;
            win.scroll_at = scroll_at;
            win.scroll_where = scroll_where;
            win.touch = touch;
        }

        let spin_at = Instant::now();
        shell.servo.spin_event_loop();
        perf::add(perf::Stage::Spin, spin_at);

        // ── Плавная прокрутка ──────────────────────────────────────────────
        for win in views.iter_mut() {
            let moving = win.scroll_left != (0.0, 0.0);
            let due = match vsync.as_ref() {
                Some(vsync) if moving => {
                    let due = win.tick.swap(false, Ordering::Relaxed);
                    win.tick_asked &= !due;
                    if !win.tick_asked {
                        win.tick_asked = true;
                        let (tick, wake) = (win.tick.clone(), wake.clone());
                        vsync.observe_next_frame(Box::new(move || {
                            tick.store(true, Ordering::Relaxed);
                            if let Some(wake) = &wake {
                                wake();
                            }
                        }));
                    }
                    due
                }
                Some(_) => false,
                None => win.scroll_at.elapsed() >= Duration::from_millis(8),
            };
            if !(moving && due) {
                continue;
            }
            shell.switch(win.key);
            let dt = win.scroll_at.elapsed().as_secs_f32().min(0.05);
            win.scroll_at = Instant::now();
            let part = if shell.profile.settings.smooth_scroll { 1.0 - (-dt * 16.0).exp() } else { 1.0 };
            let step = |left: &mut f32| {
                let mut d = *left * part;
                if (*left - d).abs() < 1.5 {
                    d = *left;
                }
                *left -= d;
                d
            };
            let delta = DeviceVector2D::new(step(&mut win.scroll_left.0), step(&mut win.scroll_left.1));
            if let Some(w) = shell.active_view() {
                w.notify_scroll_event(Scroll::Delta(WebViewVector::Device(delta)), WebViewPoint::Device(win.scroll_where));
            }
        }

        // ── События Servo — каждому окну свои ──────────────────────────────
        let focus = order.first().copied().unwrap_or(0);
        let mut buckets: Vec<Vec<shell::Note>> = views.iter().map(|_| Vec::new()).collect();
        let focused_at = views.iter().position(|v| v.key == focus).unwrap_or(0);
        let notes = std::mem::take(&mut *shell.delegate.notes.borrow_mut());
        for note in notes {
            match note.view() {
                Some(id) => {
                    // Событие уже закрытой вкладки — никому.
                    if let Some(at) = shell.window_of(id).and_then(|k| views.iter().position(|v| v.key == k)) {
                        buckets[at].push(note);
                    }
                }
                None => match note {
                    shell::Note::HideControl(id) => buckets.iter_mut().for_each(|b| b.push(shell::Note::HideControl(id))),
                    shell::Note::Action(_) => buckets[focused_at].push(note),
                    _ => buckets.iter_mut().for_each(|b| b.push(shell::Note::Redraw)),
                },
            }
        }
        // Пришедшее во время разбора — на следующий виток.
        let mut later = Vec::new();
        for (win, bucket) in views.iter_mut().zip(buckets) {
            shell.switch(win.key);
            *shell.delegate.notes.borrow_mut() = bucket;
            let window_rc = win.window.clone();
            win.input |= shell.apply_notes(&*window_rc);
            later.append(&mut shell.delegate.notes.borrow_mut());
        }
        *shell.delegate.notes.borrow_mut() = later;
        shell.paint_pending();
        {
            let at = focused_at;
            shell.switch(views[at].key);
            let window_rc = views[at].window.clone();
            let window: &Window = &*window_rc;
            views[at].input |= shell.poll_sync(window);
            views[at].input |= shell.poll_downloads(window);
        }

        // ── Оболочка каждого окна ──────────────────────────────────────────
        let mut devtools_somewhere = false;
        for win in views.iter_mut() {
            let key = win.key;
            shell.switch(key);
            let window_rc = win.window.clone();
            let window: &Window = &*window_rc;
            win.input |= shell.poll_webui();
            // Настройка масштаба интерфейса поменялась.
            if shell.profile.settings.ui_scale != win.scale_setting {
                win.scale_setting = shell.profile.settings.ui_scale;
                shell::set_ui_scale(effective_scale(win.scale_setting, window));
                shell.resize_all(window);
                win.input = true;
            }
            if shell::fullscreen() != win.was_fullscreen {
                win.was_fullscreen = shell::fullscreen();
                window.set_fullscreen(win.was_fullscreen);
                shell.resize_all(window);
                win.input = true;
            }
            if std::mem::take(&mut win.input) {
                win.ui_due = Some(Instant::now());
            }
            win.ui_drawn = false;
            devtools_somewhere |= shell.panel == Some(Panel::DevTools);
            if !win.ui_due.is_some_and(|due| Instant::now() >= due) {
                continue;
            }
            let ui_at = Instant::now();
            let (ww, wh) = (window.width(), window.height());
            let k = shell::ui_scale();
            let mut raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(ww as f32 / k, wh as f32 / k))),
                time: Some(started.elapsed().as_secs_f64()),
                modifiers: egui_modifiers(win.mask),
                events: std::mem::take(&mut win.events),
                focused: true,
                max_texture_side: gpu.as_ref().map(|g| g.max_texture_side()),
                ..Default::default()
            };
            raw.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(k);
            let output = win.ctx.run_ui(raw, |ui| ui::shell_ui(ui, &mut shell, &mut actions));
            for command in &output.platform_output.commands {
                if let egui::OutputCommand::CopyText(text) = command {
                    *shell.delegate.clipboard.borrow_mut() = text.clone();
                    window::clipboard_set(text);
                }
            }
            win.ui_cursor = output.platform_output.cursor_icon;
            let primitives = win.ctx.tessellate(output.shapes, output.pixels_per_point);
            let count = primitives.len();
            match gpu.as_mut() {
                Some(gpu) => gpu.set_ui(win.surface, primitives, &output.textures_delta, output.pixels_per_point),
                None => renderer.render_to_canvas(ww as usize, wh as usize, &primitives, &output.textures_delta, output.pixels_per_point),
            }
            perf::add(perf::Stage::Ui, ui_at);
            let delay = output.viewport_output.values().map(|v| v.repaint_delay).min().unwrap_or(Duration::MAX);
            // Отсчёт от начала прохода: кадры анимации идут ровно, а не «такт плюс время рисования».
            win.ui_due = (delay < Duration::from_secs(3600)).then(|| ui_at + delay.max(ui_step));
            ui_frames += 1;
            if ui_frames <= 2 || ui_frames % 200 == 0 {
                println!("browser: ui frame {ui_frames} ({count} primitives)");
            }
            win.ui_drawn = true;
            for action in std::mem::take(&mut actions) {
                if !shell.run(action, window) {
                    requests.push(Request::Refused(key));
                }
                win.ui_due = Some(Instant::now());
            }
            // Инструменты разработчика открыли, закрыли или растянули — страница
            // уже или шире (встроены справа, как в Chromium).
            let dev_open = shell.panel == Some(Panel::DevTools);
            devtools_somewhere |= dev_open;
            if !dev_open {
                devtools::closed(&mut shell);
            }
            let points = |physical: u32| (physical as f32 / shell::ui_scale()) as u32;
            let want_dock = if dev_open && !shell::fullscreen() {
                if shell.dev.bottom {
                    (0, (shell.dev.height as u32).min(points(window.height()).saturating_sub(200)))
                } else {
                    ((shell.dev.width as u32).min(points(window.width()).saturating_sub(300)), 0)
                }
            } else {
                (0, 0)
            };
            if want_dock != (shell::dev_dock(), shell::dev_dock_bottom()) {
                shell::set_dev_dock(want_dock.0);
                shell::set_dev_dock_bottom(want_dock.1);
                shell.resize_all(window);
                win.ui_due = Some(Instant::now());
            }
            // Полоса загрузок внизу — только на FossAi (на неё целится стенд),
            // на настольных системах загрузки показывает список под кнопкой.
            // Сперва выезжает поверх страницы, затем страница становится на её
            // высоту ниже.
            let bar_wanted = cfg!(target_os = "freeos") && shell.dl.bar && !shell::fullscreen() && shell.delegate.downloads.snapshot().iter().any(|d| d.visible());
            if bar_wanted && shell.chrome.bar_since.is_none() {
                shell.chrome.bar_since = Some(Instant::now());
                win.ui_due = Some(Instant::now());
            } else if !bar_wanted {
                shell.chrome.bar_since = None;
            }
            let risen = shell.chrome.bar_since.is_some_and(|at| at.elapsed().as_secs_f32() >= crate::chrome::BAR_SLIDE);
            let want_bar = if risen { crate::shell::DOWNLOADS_BAR_H } else { 0 };
            if want_bar != crate::shell::downloads_bar() {
                crate::shell::set_downloads_bar(want_bar);
                shell.resize_all(window);
                println!("browser: downloads bar {}", if want_bar > 0 { "shown" } else { "hidden" });
                win.ui_due = Some(Instant::now());
            }
        }
        shell.delegate.devtools_open.set(devtools_somewhere);
        // Оформление поменялось (в любом окне) — всем окнам.
        for win in views.iter_mut() {
            if *shell.look() != win.look {
                shell.switch(win.key);
                let new = shell.look();
                let relayout = new.tabs != win.look.tabs || new.sidebar_width != win.look.sidebar_width || new.compact != win.look.compact;
                win.look = shell.look().clone();
                ui::style(&win.ctx, &win.look);
                shell.update_page_data();
                if relayout {
                    shell.resize_all(&*win.window);
                }
                win.ui_due = Some(Instant::now());
            }
        }

        // ── Кадры окон ─────────────────────────────────────────────────────
        let page_fresh = std::mem::take(&mut page.borrow_mut().fresh);
        let mut shown = false;
        for win in views.iter_mut() {
            win.window.set_cursor(if win.over_page { shell.delegate.page_cursor.get() } else { egui_cursor(win.ui_cursor) });
            let exposed = win.window.take_exposed();
            if !(page_fresh || win.ui_drawn || exposed) {
                continue;
            }
            shell.switch(win.key);
            let at = Instant::now();
            let (ox, oy) = page_origin(&win.look);
            match gpu.as_mut() {
                Some(gpu) => gpu.frame(win.surface, (win.window.width(), win.window.height()), (ox, oy), shell.windows.region(win.key)),
                None => win.window.compose(&page.borrow(), (ox as usize, oy as usize), &mut renderer),
            }
            perf::add(perf::Stage::Compose, at);
            shown = true;
        }
        if shown {
            perf::frame_shown();
        }
        // Видеокарта показала кадры — запуск удался.
        if page.borrow().count >= 2 {
            if let Some(mark) = gpu_mark.take() {
                let _ = std::fs::remove_file(mark);
            }
        }

        // ── Окна: открыть, закрыть, перенести вкладку ──────────────────────
        if std::mem::take(&mut shell.windows.wanted) {
            requests.push(Request::Open);
        }
        for request in requests {
            let focus = order.first().copied().unwrap_or(0);
            match request {
                Request::Refused(key) | Request::Close(key) => {
                    // Отказ оболочки — «выход» или «вкладок не осталось».
                    let refused = matches!(request, Request::Refused(_));
                    if !shell.switch(key) {
                        continue;
                    }
                    if views.len() == 1 || (refused && !shell.tabs.is_empty()) {
                        break 'run;
                    }
                    close_view(&mut shell, &mut views, &mut order, gpu.as_mut(), key);
                }
                Request::Open => {
                    let Some(from) = views.iter().position(|v| v.key == focus) else { continue };
                    let at = views[from].window.inner_origin().map(|(x, y)| (x + 40, y + 40));
                    if let Some(key) = open_view(&mut shell, &mut views, &mut order, gpu.as_mut(), from, at) {
                        shell.switch(key);
                        let window_rc = views[views.len() - 1].window.clone();
                        let home = shell.home_url();
                        shell.new_tab(home, &*window_rc);
                    }
                }
                Request::Move { from, tab, to, at } => {
                    move_tab(&mut shell, &mut views, &mut order, gpu.as_mut(), from, tab, to, at);
                }
                Request::TearOff { from, tab, at } => {
                    let Some(source) = views.iter().position(|v| v.key == from) else { continue };
                    // Над чьей полосой вкладок отпустили (из недавних окон —
                    // верхнее).
                    let under = order.iter().filter(|k| **k != from).find_map(|k| {
                        let v = views.iter().find(|v| v.key == *k)?;
                        let (x, y) = v.window.inner_origin()?;
                        let (x, y) = (at.0 - x, at.1 - y);
                        (x >= 0 && y >= 0 && x < v.window.width() as i32 && y < v.window.height() as i32).then_some((v, x, y))
                    });
                    let into = under.and_then(|(v, x, y)| {
                        shell.switch(v.key);
                        near_tab_strip(&v.look, &*v.window, x, y).then(|| (v.key, drop_index(&v.look, &*v.window, x, y, shell.tabs.len())))
                    });
                    shell.switch(from);
                    println!("browser: tab {} torn off at {},{} -> {:?}", tab + 1, at.0, at.1, into);
                    if let Some((to, place)) = into {
                        move_tab(&mut shell, &mut views, &mut order, gpu.as_mut(), from, tab, to, place);
                    } else if shell.tabs.len() > 1 {
                        let place = Some((at.0 - shell::px(120) as i32, at.1 - shell::px(30) as i32));
                        if let Some(to) = open_view(&mut shell, &mut views, &mut order, gpu.as_mut(), source, place) {
                            move_tab(&mut shell, &mut views, &mut order, gpu.as_mut(), from, tab, to, 0);
                        }
                    } else {
                        // Единственная вкладка — это просто перенос окна.
                        views[source].window.set_position(at.0 - shell::px(120) as i32, at.1 - shell::px(30) as i32);
                    }
                }
            }
        }

        if perf_always && perf_since.elapsed() > Duration::from_secs(5) {
            println!("browser: {}", perf::report(perf_since.elapsed().as_secs_f32()));
            perf_since = Instant::now();
        }
        if last_session.elapsed() > Duration::from_secs(20) {
            last_session = Instant::now();
            shell.profile.save_session(&shell.session_urls(), shell.active);
        }
        if shell.quit {
            break;
        }
        // Пауза прерывается и вводом, и будильником Servo.
        let scrolling = vsync.is_none() && views.iter().any(|v| v.scroll_left != (0.0, 0.0));
        let mut pause = Duration::from_millis(if page_fresh || scrolling { 1 } else { 5 });
        for due in views.iter().filter_map(|v| v.ui_due) {
            pause = pause.min(due.saturating_duration_since(Instant::now()));
        }
        window.idle(pause);
    }
    println!("browser: closing");
    // Закрыли сами раньше первого кадра — это не сбой видеокарты.
    if let Some(mark) = gpu_mark.take() {
        let _ = std::fs::remove_file(mark);
    }
    // Запоминается окно, с которым работали последним.
    let last = order.first().copied().unwrap_or(0);
    shell.switch(last);
    let window_rc = views.iter().find(|v| v.key == last).unwrap_or(&views[0]).window.clone();
    let window: &Window = &*window_rc;
    shell.profile.save_session(&shell.session_urls(), shell.active);
    shell.profile.save_settings();
    if let Some((w, h, maximized)) = window.geometry().filter(|_| !shell.profile.private && size.is_none() && !shell::fullscreen()) {
        // Развёрнутое окно: помним прежний обычный размер.
        let old = std::fs::read_to_string(&geometry_file).unwrap_or_default();
        let mut parts = old.split_whitespace().filter_map(|p| p.parse::<u32>().ok());
        let (w, h) = match (maximized, parts.next(), parts.next()) {
            (true, Some(ow), Some(oh)) => (ow, oh),
            _ => (w, h),
        };
        let _ = std::fs::write(&geometry_file, format!("{w} {h} {}", maximized as u32));
    }
    // Последняя синхронизация — не дольше 10 с, окно уже не ждёт.
    if shell.account().is_some() && shell.profile.settings.sync_auto {
        shell.sync_now(None);
        let until = Instant::now() + Duration::from_secs(10);
        while shell.syncer.busy > 0 && Instant::now() < until {
            let _ = shell.poll_sync(window);
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    let private_dir = shell.profile.private.then(|| shell.profile.dir.clone());
    if let Some(gpu) = gpu.as_mut() {
        gpu.shutdown();
    }
    for view in &views {
        view.window.close();
    }
    if let Some(dir) = private_dir {
        let _ = std::fs::remove_dir_all(dir);
    }
    // Всё своё уже сохранено. Разбор Servo на выходе (потоки, Service Worker)
    // мог висеть минутами — выходим сразу, память и потоки снимет ядро.
    println!("browser: closed");
    std::process::exit(0);
}
