//! Состояние браузера: вкладки, события Servo, действия пользователя,
//! профиль, пароли, синхронизация. Рисует всё это `ui.rs`.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use browser_sync::json;
use browser_sync::remote::{Account, Provider, SyncPoint, oauth};
use servo::{
    AuthenticationRequest, ClipboardDelegate, ColorPicker, ConsoleLogLevel, ContextMenu, CreateNewWebViewRequest,
    EmbedderControl, EmbedderControlId, FilePicker, LoadStatus, NavigationRequest, RenderingContext, SelectElement,
    Servo, SimpleDialog, StringRequest, UserContentManager, WebView, WebViewBuilder, WebViewDelegate, WebViewId,
};

use crate::pages::{self, PageData};
use crate::profile::{self, Look, Profile, TabsAt, now_ms};
use crate::swgl_ctx::PageContext;
use crate::syncer::{Done, Job, Syncer};
use crate::vault::{self, Login, Vault};
use crate::window::{self, Window};

// ── Геометрия ───────────────────────────────────────────────────────────────
//
// Размеры оболочки — в точках интерфейса (как у egui); на экране точка
// интерфейса — `ui_scale()` физических. Прямоугольник страницы (`page_area`)
// — уже в физических точках окна.

/// Горизонтальная полоса вкладок (сверху или снизу).
pub const TABSTRIP_H: u32 = 36;
pub const TOOLBAR_H: u32 = 44;
pub const TOOLBAR_H_COMPACT: u32 = 38;

static SCALE: AtomicU32 = AtomicU32::new(0);

/// Физических точек в точке интерфейса.
pub fn ui_scale() -> f32 {
    match SCALE.load(Ordering::Relaxed) {
        0 => 1.0,
        bits => f32::from_bits(bits),
    }
}

pub fn set_ui_scale(scale: f32) {
    SCALE.store(scale.clamp(0.5, 4.0).to_bits(), Ordering::Relaxed);
}

/// Точки интерфейса → физические.
pub fn px(points: u32) -> u32 {
    (points as f32 * ui_scale()).round() as u32
}

/// Страница на весь экран: оболочки не видно (F11, видео).
static FULLSCREEN: AtomicBool = AtomicBool::new(false);

pub fn fullscreen() -> bool {
    FULLSCREEN.load(Ordering::Relaxed)
}

pub fn set_fullscreen(on: bool) {
    FULLSCREEN.store(on, Ordering::Relaxed);
}

/// Отступы оболочки от краёв окна в точках интерфейса: слева, сверху,
/// справа, снизу (с полосой загрузок и инструментами разработчика).
pub fn chrome_margins(look: &Look) -> (u32, u32, u32, u32) {
    if fullscreen() {
        return (0, 0, 0, 0);
    }
    let bar = if look.compact { TOOLBAR_H_COMPACT } else { TOOLBAR_H };
    let (left, top, right, bottom) = match look.tabs {
        TabsAt::Left => (look.sidebar_width, bar, 0, 0),
        TabsAt::Right => (0, bar, look.sidebar_width, 0),
        TabsAt::Top => (0, TABSTRIP_H + bar, 0, 0),
        TabsAt::Bottom => (0, bar, 0, TABSTRIP_H),
    };
    (left, top, right + dev_dock(), bottom + downloads_bar() + dev_dock_bottom())
}

/// Высота инструментов разработчика, закреплённых снизу.
static DEV_DOCK_BOTTOM: AtomicU32 = AtomicU32::new(0);

pub fn dev_dock_bottom() -> u32 {
    DEV_DOCK_BOTTOM.load(Ordering::Relaxed)
}

pub fn set_dev_dock_bottom(points: u32) {
    DEV_DOCK_BOTTOM.store(points, Ordering::Relaxed);
}

/// Прямоугольник страницы в окне `w × h`: x, y, ширина, высота (физические).
pub fn page_area(look: &Look, w: u32, h: u32) -> (u32, u32, u32, u32) {
    let (l, t, r, b) = chrome_margins(look);
    let (x, y) = (px(l).min(w), px(t).min(h));
    (x, y, w.saturating_sub(x + px(r)).max(1), h.saturating_sub(y + px(b)).max(1))
}

/// Масштаб страницы (`devicePixelRatio`): как у оболочки; на узком экране
/// телефона — 2.
pub fn page_scale() -> f32 {
    if crate::app::MOBILE.load(Ordering::Relaxed) { 2.0 } else { ui_scale() }
}

/// Где страница: левый верхний угол её прямоугольника в окне.
pub fn page_origin(look: &Look) -> (u32, u32) {
    let (l, t, _, _) = chrome_margins(look);
    (px(l), px(t))
}

pub fn page_size(look: &Look, w: u32, h: u32) -> dpi::PhysicalSize<u32> {
    let (_, _, w, h) = page_area(look, w, h);
    dpi::PhysicalSize::new(w, h)
}

/// Полоса загрузок внизу окна (как в Chrome): страница на столько ниже.
/// Ноль — полоса скрыта.
pub const DOWNLOADS_BAR_H: u32 = 52;
static DOWNLOADS_BAR: AtomicU32 = AtomicU32::new(0);

pub fn downloads_bar() -> u32 {
    DOWNLOADS_BAR.load(Ordering::Relaxed)
}

pub fn set_downloads_bar(points: u32) {
    DOWNLOADS_BAR.store(points, Ordering::Relaxed);
}

/// Ширина инструментов разработчика, встроенных справа (как в Chromium):
/// страница на столько уже. Ноль — закрыты.
static DEV_DOCK: AtomicU32 = AtomicU32::new(0);

pub fn dev_dock() -> u32 {
    DEV_DOCK.load(Ordering::Relaxed)
}

pub fn set_dev_dock(points: u32) {
    DEV_DOCK.store(points, Ordering::Relaxed);
}

/// Размеры оболочки текущего окна: масштаб, полный экран, инструменты
/// разработчика снизу, полоса загрузок, инструменты справа. У каждого окна
/// свои — при переключении они меняются вместе с его полями (`windows.rs`).
pub(crate) fn window_metrics() -> [u32; 5] {
    [
        SCALE.load(Ordering::Relaxed),
        FULLSCREEN.load(Ordering::Relaxed) as u32,
        DEV_DOCK_BOTTOM.load(Ordering::Relaxed),
        DOWNLOADS_BAR.load(Ordering::Relaxed),
        DEV_DOCK.load(Ordering::Relaxed),
    ]
}

pub(crate) fn set_window_metrics([scale, full, bottom, bar, dock]: [u32; 5]) {
    SCALE.store(scale, Ordering::Relaxed);
    FULLSCREEN.store(full != 0, Ordering::Relaxed);
    DEV_DOCK_BOTTOM.store(bottom, Ordering::Relaxed);
    DOWNLOADS_BAR.store(bar, Ordering::Relaxed);
    DEV_DOCK.store(dock, Ordering::Relaxed);
}

// ── События Servo → главный цикл ────────────────────────────────────────────

pub enum Note {
    Frame(WebViewId),
    Title(WebViewId, Option<String>),
    Url(WebViewId, url::Url),
    Load(WebViewId, LoadStatus),
    Status(WebViewId, Option<String>),
    History,
    /// Страница открыла вкладку (`window.open`): она и вкладка-родитель.
    Opened(WebView, WebViewId),
    Crashed(WebViewId, String),
    Control(WebViewId, EmbedderControl),
    HideControl(EmbedderControlId),
    Auth(WebViewId, AuthenticationRequest),
    /// Сообщение пользовательского скрипта OBSOS (`__obsos__:{…}`).
    Script(WebViewId, String),
    /// Команда со страницы `obsos://action/…` или подмена адреса.
    Action(Action),
    /// Заблокирован запрос рекламы/трекера на странице.
    Blocked(WebViewId),
    /// Сообщение страницы управления `obsos://settings` и подобных.
    WebUi(WebViewId, String),
    /// У страницы появился значок.
    Favicon(WebViewId),
    /// Страница просит весь экран (видео) или выходит из него.
    Fullscreen(WebViewId, bool),
    /// Сообщение агента инструментов разработчика.
    DevTools(WebViewId, String),
    /// Страница просит микрофон или камеру.
    Permission(WebViewId, servo::PermissionRequest),
    /// Только перерисовать оболочку (пришёл ответ страницы).
    Redraw,
}

pub struct Delegate {
    pub notes: RefCell<Vec<Note>>,
    pub context: Rc<dyn PageContext>,
    /// Запасной буфер обмена (если ядро без SYS_CLIPBOARD); обычно — общий
    /// буфер системы (`window::clipboard_*`).
    pub clipboard: Rc<RefCell<String>>,
    pub content: Rc<UserContentManager>,
    /// Форма указателя, которую просит страница (над ссылкой — рука).
    pub page_cursor: std::cell::Cell<window::CursorShape>,
    pub blocker: Rc<RefCell<crate::blocker::Blocker>>,
    pub downloads: Rc<crate::downloads::Downloads>,
    /// Консоль и сеть для инструментов разработчика (F12).
    pub devlog: RefCell<crate::devtools::DevLog>,
    /// Инструменты разработчика открыты — новые записи перерисовывают их.
    pub devtools_open: std::cell::Cell<bool>,
    /// Вкладки, открытые самими страницами (`window.open`).
    pub page_opened: RefCell<Vec<WebViewId>>,
    /// Запросы документов вкладок, ждущие проверки ссылки (файл или
    /// страница, `Shell::poll_downloads`).
    pub held_loads: RefCell<Vec<(WebViewId, servo::WebResourceLoad)>>,
}

impl Note {
    /// Вкладка, к которой относится событие, — по ней оно уходит в своё окно.
    pub fn view(&self) -> Option<WebViewId> {
        match self {
            Note::Frame(id)
            | Note::Title(id, _)
            | Note::Url(id, _)
            | Note::Load(id, _)
            | Note::Status(id, _)
            | Note::Opened(_, id)
            | Note::Crashed(id, _)
            | Note::Control(id, _)
            | Note::Auth(id, _)
            | Note::Script(id, _)
            | Note::Blocked(id)
            | Note::WebUi(id, _)
            | Note::Favicon(id)
            | Note::Fullscreen(id, _)
            | Note::Permission(id, _)
            | Note::DevTools(id, _) => Some(*id),
            Note::History | Note::HideControl(_) | Note::Action(_) | Note::Redraw => None,
        }
    }
}

impl Delegate {
    pub(crate) fn note(&self, note: Note) {
        self.notes.borrow_mut().push(note);
    }
}

const SCRIPT_PREFIX: &str = "__obsos__:";

/// A diagnostic line: to stdout (the terminal, scenarios) and to the kernel
/// log (25.11: OBSOS started from the desktop has no terminal, and Roman's
/// journal on the flash drive had not a single line of why web.pynop.online
/// did not log in). The kernel takes a stderr write that starts with
/// `kmsg: ` into its log (`user::syscall::write`) — one write per line.
pub fn klog(text: &str) {
    use std::io::Write;
    if crate::logs::private() {
        return;
    }
    let text = &crate::logs::scrub(text);
    println!("{text}");
    let short: String = text.chars().take(300).collect();
    let line = format!("kmsg: {}\n", short.replace('\n', " "));
    let _ = std::io::stderr().write_all(line.as_bytes());
}

impl WebViewDelegate for Delegate {
    fn request_permission(&self, webview: WebView, request: servo::PermissionRequest) {
        self.note(Note::Permission(webview.id(), request));
    }

    fn notify_new_frame_ready(&self, webview: WebView) {
        self.note(Note::Frame(webview.id()));
    }

    fn notify_load_status_changed(&self, webview: WebView, status: LoadStatus) {
        println!("browser: load status {status:?}");
        self.note(Note::Load(webview.id(), status));
    }

    fn notify_page_title_changed(&self, webview: WebView, title: Option<String>) {
        if !crate::logs::private() {
            println!("browser: title {title:?}");
        }
        self.note(Note::Title(webview.id(), title));
    }

    fn notify_url_changed(&self, webview: WebView, url: url::Url) {
        let shown = url.as_str();
        println!("browser: url {}", crate::logs::url(shown));
        // The journal gets the address without query and fragment: they may
        // carry tokens (`?access_token=`, OAuth `#code=`).
        if matches!(url.scheme(), "http" | "https") && !crate::logs::private() {
            let _ = std::io::Write::write_all(
                &mut std::io::stderr(),
                format!("kmsg: browser: page {}://{}{}\n", url.scheme(), url.host_str().unwrap_or(""), url.path()).as_bytes(),
            );
        }
        self.note(Note::Url(webview.id(), url));
    }

    fn notify_status_text_changed(&self, webview: WebView, status: Option<String>) {
        self.note(Note::Status(webview.id(), status));
    }

    fn notify_favicon_changed(&self, webview: WebView) {
        self.note(Note::Favicon(webview.id()));
    }

    fn notify_fullscreen_state_changed(&self, webview: WebView, on: bool) {
        self.note(Note::Fullscreen(webview.id(), on));
    }

    fn notify_history_changed(&self, _webview: WebView, _entries: Vec<url::Url>, _current: usize) {
        self.note(Note::History);
    }

    fn notify_crashed(&self, webview: WebView, reason: String, _backtrace: Option<String>) {
        klog(&format!("browser: page crashed: {reason}"));
        self.note(Note::Crashed(webview.id(), reason));
    }

    fn request_create_new(&self, parent: WebView, request: CreateNewWebViewRequest) {
        let webview = request
            .builder(self.context.clone() as Rc<dyn RenderingContext>)
            .delegate(parent.delegate())
            .clipboard_delegate(parent.clipboard_delegate())
            .user_content_manager(self.content.clone())
            .build();
        self.page_opened.borrow_mut().push(webview.id());
        self.note(Note::Opened(webview, parent.id()));
    }

    fn request_navigation(&self, webview: WebView, request: NavigationRequest) {
        let url = request.url.clone();
        // Переход затеяла веб-страница (в том числе открывшая новое окно):
        // служебные страницы с адресом в параметрах и локальные файлы ей
        // недоступны.
        let from_web = match webview.url() {
            Some(current) => !matches!(current.scheme(), "obsos" | "file" | "about" | "data"),
            None => self.page_opened.borrow().contains(&webview.id()),
        };
        let internal = url.scheme() == "file" || (url.scheme() == "obsos" && matches!(url.host_str(), Some("action" | "unsupported" | "crashed" | "view")));
        if from_web && internal {
            request.deny();
            return;
        }
        if url.scheme() == "obsos" && url.host_str() == Some("action") {
            // Команды оболочке — только со своих страниц (сайт мог бы так
            // открывать адреса во внешнем браузере без ведома пользователя).
            if !webview.url().is_some_and(|u| u.scheme() == "obsos") {
                request.deny();
                return;
            }
            let target = url.query_pairs().find(|(k, _)| k == "url").map(|(_, v)| v.into_owned()).unwrap_or_default();
            // Только веб-адреса: по `file:` и прочим система запускает
            // программы.
            let web = target.starts_with("http://") || target.starts_with("https://");
            let action = match url.path() {
                "/chromium" if web => Some(Action::Chromium(target)),
                "/force" if web => Some(Action::OpenUrl(target)),
                _ => None,
            };
            request.deny();
            if let Some(action) = action {
                self.note(Note::Action(action));
            }
            return;
        }
        if crate::downloads::looks_like_file(&url) {
            request.deny();
            self.note(Note::Action(Action::Download(url.to_string())));
            return;
        }
        if url.scheme() == "file" && pages::viewable_path(url.path()) {
            request.deny();
            self.note(Note::Action(Action::OpenUrl(pages::view_url(url.path()).to_string())));
            return;
        }
        request.allow();
    }

    fn notify_cursor_changed(&self, _webview: WebView, cursor: servo::Cursor) {
        use servo::Cursor as C;
        use window::CursorShape as S;
        self.page_cursor.set(match cursor {
            C::Pointer => S::Hand,
            C::Text | C::VerticalText => S::Text,
            C::Move | C::Grab | C::Grabbing | C::AllScroll => S::Move,
            C::NotAllowed | C::NoDrop => S::NotAllowed,
            C::EResize | C::WResize | C::EwResize | C::ColResize => S::ResizeEw,
            C::NResize | C::SResize | C::NsResize | C::RowResize => S::ResizeNs,
            _ => S::Arrow,
        });
    }

    /// Блокировка рекламы и трекеров: запрос к рекламному домену отменяется
    /// до сети. Сам документ страницы не трогаем.
    fn load_web_resource(&self, webview: WebView, load: servo::WebResourceLoad) {
        let request = load.request();
        let host = request.url.host_str().map(str::to_string).unwrap_or_default();
        let site = webview.url().and_then(|u| u.host_str().map(str::to_string));
        let blocked = !request.is_for_main_frame && !host.is_empty() && self.blocker.borrow().blocks(&host, site.as_deref());
        self.devlog.borrow_mut().request(crate::devtools::NetEntry {
            webview: webview.id(),
            method: request.method.to_string(),
            url: request.url.as_str().to_string(),
            kind: format!("{:?}", request.destination),
            main: request.is_for_main_frame,
            blocked,
            at: Instant::now(),
            headers: request.headers.iter().map(|(k, v)| (k.to_string(), String::from_utf8_lossy(v.as_bytes()).into_owned())).collect(),
        });
        if self.devtools_open.get() {
            self.note(Note::Redraw);
        }
        if blocked {
            let url = request.url.as_url().clone();
            let mut blocker = self.blocker.borrow_mut();
            blocker.blocked_total += 1;
            if blocker.blocked_total <= 3 || blocker.blocked_total % 50 == 0 {
                println!("browser: blocked {host} ({} total)", blocker.blocked_total);
            }
            drop(blocker);
            self.note(Note::Blocked(webview.id()));
            load.intercept(servo::WebResourceResponse::new(url)).cancel();
            return;
        }
        // Документ вкладки: файл это или страница, скажут заголовки ответа
        // (`Content-Disposition: attachment`, тип, который Servo не покажет), —
        // запрос ждёт проверки ссылки (`Shell::poll_downloads`).
        let url = request.url.as_url();
        if request.is_for_main_frame
            && !request.is_redirect
            && request.method.as_str() == "GET"
            && matches!(url.scheme(), "http" | "https")
            && crate::downloads::maybe_download(url)
        {
            self.held_loads.borrow_mut().push((webview.id(), load));
        }
    }

    fn request_authentication(&self, webview: WebView, request: AuthenticationRequest) {
        self.note(Note::Auth(webview.id(), request));
    }

    fn show_embedder_control(&self, webview: WebView, control: EmbedderControl) {
        self.note(Note::Control(webview.id(), control));
    }

    fn hide_embedder_control(&self, _webview: WebView, id: EmbedderControlId) {
        self.note(Note::HideControl(id));
    }

    fn show_console_message(&self, webview: WebView, level: ConsoleLogLevel, message: String) {
        if let Some(payload) = message.strip_prefix(SCRIPT_PREFIX) {
            self.note(Note::Script(webview.id(), payload.to_string()));
            return;
        }
        if let Some(payload) = message.strip_prefix(crate::webui::PREFIX) {
            self.note(Note::WebUi(webview.id(), payload.to_string()));
            return;
        }
        if let Some(payload) = message.strip_prefix("__obsos_dt__:") {
            self.note(Note::DevTools(webview.id(), payload.to_string()));
            return;
        }
        if self.devtools_open.get() {
            self.note(Note::Redraw);
        }
        let level = format!("{level:?}");
        // Ошибки страниц — и в журнал системы: по нему видно, на чём сайт
        // сломался, даже без открытой консоли.
        // 25.11: and into the kernel log (`klog`) — with the failed requests
        // and WebSocket closes the page script reports as warnings.
        if level.contains("Error") || level.contains("Warn") {
            let short: String = message.chars().take(400).collect();
            klog(&format!("browser: console {level}: {short}"));
        }
        self.devlog.borrow_mut().console(Some(webview.id()), &level, message);
    }
}

impl ClipboardDelegate for Delegate {
    fn clear(&self, _webview: WebView) {
        self.clipboard.borrow_mut().clear();
    }

    fn get_text(&self, _webview: WebView, request: StringRequest) {
        // Буфер обмена системы (общий с терминалом и Блокнотом); старое ядро
        // без него — свой.
        request.success(window::clipboard_get().unwrap_or_else(|| self.clipboard.borrow().clone()));
    }

    fn set_text(&self, _webview: WebView, text: String) {
        window::clipboard_set(&text);
        *self.clipboard.borrow_mut() = text;
    }
}

/// Пользовательский скрипт OBSOS в каждой странице: перехват отправки формы
/// с паролем (Servo не даёт оболочке события «форма отправлена»), сообщение
/// о поле пароля (для подстановки) и о том, что сайт полез в `MediaSource`
/// (потокового видео в Servo нет) — сообщения уходят в консоль, их ловит
/// `show_console_message`.
pub const PAGE_SCRIPT: &str = r#"(function(){
if (window.__obsos) return; window.__obsos = 1;
function send(o){ try { console.log('__obsos__:' + JSON.stringify(o)); } catch(e){} }
// Servo отдаёт потоковой компиляции wasm тело ответа ещё сжатым (gzip от
// сервера) — «failed to match magic number» у .NET/Blazor. arrayBuffer() даёт
// распакованное: компилируем из него.
try {
  if (window.WebAssembly) {
    var W = WebAssembly, body = function(src){ return Promise.resolve(src).then(function(r){ return r.arrayBuffer(); }); };
    W.instantiateStreaming = function(src, imports){ return body(src).then(function(b){ return W.instantiate(b, imports); }); };
    W.compileStreaming = function(src){ return body(src).then(function(b){ return W.compile(b); }); };
  }
} catch(e) {}
// Звонки: у RTCPeerConnection в Servo есть только старый addStream. Здесь то,
// чем пользуются сайты (Rupor: addTrack до начала звонка, e.streams[0],
// connectionState). Смена дорожки на ходу (камера, экран) не поддерживается.
try {
  var PC = window.RTCPeerConnection;
  if (PC && !PC.prototype.addTrack) {
    // Дорожки, уже отданные звонку, плееру или AudioContext: второго получателя
    // медиадвижок не выдерживает.
    var taken = new Set(), proto = PC.prototype, handlers = new WeakMap();
    var take = function(stream){ if (stream && stream.getTracks) stream.getTracks().forEach(function(t){ taken.add(t.id); }); };
    proto.addTrack = function(track){
      var list = this.__obsosSenders || (this.__obsosSenders = []);
      if (list.some(function(s){ return s.track === track; })) throw new DOMException('Track already added', 'InvalidAccessError');
      taken.add(track.id);
      this.addStream(new MediaStream([track]));
      var sender = { track: track,
        replaceTrack: function(){ return Promise.reject(new DOMException('replaceTrack is not supported', 'NotSupportedError')); },
        getParameters: function(){ return { encodings: [{}] }; },
        setParameters: function(){ return Promise.resolve(); } };
      list.push(sender);
      return sender;
    };
    proto.getSenders = function(){ return (this.__obsosSenders || []).slice(); };
    proto.getReceivers = function(){ return []; };
    proto.getTransceivers = function(){ return []; };
    proto.removeTrack = function(sender){ if (sender && sender.track) sender.track.enabled = false; };
    proto.getStats = function(){ return Promise.resolve(new Map()); };
    proto.restartIce = function(){};
    var states = { checking: 'connecting', connected: 'connected', completed: 'connected', disconnected: 'disconnected', failed: 'failed', closed: 'closed' };
    Object.defineProperty(proto, 'connectionState', { configurable: true, get: function(){ return states[this.iceConnectionState] || 'new'; } });
    Object.defineProperty(proto, 'onconnectionstatechange', { configurable: true,
      get: function(){ return handlers.get(this) || null; },
      set: function(f){ handlers.set(this, typeof f === 'function' ? f : null); } });
    var Wrapped = function RTCPeerConnection(config){
      var pc = new PC(config || {}), last = 'new';
      pc.addEventListener('iceconnectionstatechange', function(){
        var now = pc.connectionState;
        if (now === last || now === 'closed') return;
        last = now;
        var event = new Event('connectionstatechange');
        pc.dispatchEvent(event);
        var f = handlers.get(pc);
        if (f) f.call(pc, event);
      });
      // Этот слушатель стоит раньше `ontrack` страницы.
      pc.addEventListener('track', function(e){
        if (!e.streams) Object.defineProperty(e, 'streams', { value: [new MediaStream([e.track])] });
      });
      return pc;
    };
    Wrapped.prototype = proto;
    window.RTCPeerConnection = Wrapped;
    var source = Object.getOwnPropertyDescriptor(HTMLMediaElement.prototype, 'srcObject');
    if (source && source.set) Object.defineProperty(HTMLMediaElement.prototype, 'srcObject', { configurable: true, enumerable: source.enumerable,
      get: source.get, set: function(value){ take(value); source.set.call(this, value); } });
    if (window.AudioContext && AudioContext.prototype.createMediaStreamSource) {
      var make = AudioContext.prototype.createMediaStreamSource;
      AudioContext.prototype.createMediaStreamSource = function(stream){
        if (stream && stream.getTracks && stream.getTracks().some(function(t){ return taken.has(t.id); }))
          throw new DOMException('Stream is already in use', 'NotSupportedError');
        take(stream);
        return make.call(this, stream);
      };
    }
  }
} catch(e) {}
// Servo 0.8 не шлёт `change` текстовому полю, когда правку завершили (уход
// фокуса, Enter), и Enter в поле формы ничего не отправляет (нет неявной
// отправки). Blazor (`InputText` — `@bind` по `change`) так и не узнавал
// набранного: вход на web.pynop.online отвечал «поле обязательно» (25.11).
// Здесь — как в других браузерах: `change` при уходе фокуса и по Enter, если
// значение изменилось с момента фокуса; Enter — щелчок по кнопке формы по
// умолчанию (или отправка формы с одним полем), если сам Servo не отправил.
try {
  var base = new WeakMap(), lastSubmit = 0;
  var textual = function(t){ if (!t || !t.tagName) return false; if (t.tagName == 'TEXTAREA') return true; if (t.tagName != 'INPUT') return false;
    return ['text','password','email','search','tel','url','number',''].indexOf((t.type || 'text').toLowerCase()) >= 0; };
  var commit = function(t){ if (!textual(t) || !base.has(t) || base.get(t) === t.value) return; base.set(t, t.value);
    t.dispatchEvent(new Event('change', {bubbles: true})); };
  document.addEventListener('focusin', function(e){ if (textual(e.target)) base.set(e.target, e.target.value); }, true);
  document.addEventListener('change', function(e){ if (textual(e.target)) base.set(e.target, e.target.value); }, true);
  document.addEventListener('focusout', function(e){ commit(e.target); }, true);
  document.addEventListener('submit', function(){ lastSubmit = Date.now(); }, true);
  document.addEventListener('keydown', function(e){
    var t = e.target; if (e.key != 'Enter' || e.isComposing || !t || t.tagName != 'INPUT' || !textual(t)) return;
    commit(t);
    var f = t.form, at = Date.now(); if (!f) return;
    setTimeout(function(){
      if (e.defaultPrevented || lastSubmit >= at || !t.isConnected) return;
      var els = f.elements, btn = null, fields = 0;
      for (var i = 0; i < els.length; i++) { var x = els[i], y = (x.getAttribute('type') || '').toLowerCase();
        if (!btn && ((x.tagName == 'BUTTON' && (y == '' || y == 'submit')) || (x.tagName == 'INPUT' && (y == 'submit' || y == 'image')))) btn = x;
        if (x.tagName == 'INPUT' && textual(x)) fields++; }
      if (btn) { if (!btn.disabled) btn.click(); }
      else if (fields == 1) { if (f.requestSubmit) f.requestSubmit(); else f.submit(); }
    }, 0);
  }, false);
} catch(e) {}
// Журнал сети (25.11): неудавшиеся запросы fetch/XHR и закрытия WebSocket не
// по-хорошему — предупреждением в консоль, а оттуда в журнал ядра (`klog`).
// Адрес без запроса и якоря: в них бывают токены (`access_token=`). Не
// больше 40 строк на страницу.
try {
  var said = 0, bare = function(u){ try { var a = new URL(String(u), location.href); return a.origin + a.pathname; } catch(x){ return '?'; } };
  var warn = function(s){ if (said++ < 40) console.warn('net: ' + s); };
  if (window.fetch) { var F = window.fetch; window.fetch = function(input, init){
    var u = bare(input && input.url || input), m = (init && init.method) || (input && input.method) || 'GET';
    return F.apply(this, arguments).then(function(r){ if (r.status >= 400) warn(m + ' ' + u + ' -> ' + r.status); return r; },
      function(err){ warn(m + ' ' + u + ' failed: ' + err); throw err; }); }; }
  if (window.XMLHttpRequest) { var O = XMLHttpRequest.prototype.open; XMLHttpRequest.prototype.open = function(m, u){
    var x = this, mu = m + ' ' + bare(u); x.addEventListener('loadend', function(){ if (x.status == 0 || x.status >= 400) warn('XHR ' + mu + ' -> ' + x.status); });
    return O.apply(this, arguments); }; }
  if (window.WebSocket) { var WS = window.WebSocket; var Wrapped = function(u, p){
    var w = arguments.length > 1 ? new WS(u, p) : new WS(u), url = bare(u);
    w.addEventListener('error', function(){ warn('WebSocket ' + url + ' error'); });
    w.addEventListener('close', function(e){ if (e.code != 1000 && e.code != 1001) warn('WebSocket ' + url + ' closed ' + e.code + (e.reason ? ' ' + e.reason : '')); });
    return w; };
    Wrapped.prototype = WS.prototype; ['CONNECTING','OPEN','CLOSING','CLOSED'].forEach(function(k){ Wrapped[k] = WS[k]; });
    window.WebSocket = Wrapped; }
} catch(e) {}
window.addEventListener('error', function(e){ if (!(e instanceof ErrorEvent)) return; try { console.error('Uncaught ' + (e.message || e) + (e.filename ? ' @ ' + e.filename + ':' + e.lineno + ':' + e.colno : '')); } catch(x){} }, true);
window.addEventListener('unhandledrejection', function(e){ try { var r = e.reason; console.error('Unhandled rejection: ' + (r && (r.stack || r.message) || r)); } catch(x){} });
try {
  if (!('MediaSource' in window)) {
    var told = false;
    // Плашку — только если видео есть, а играть так и не начало: многие
    // плееры (vkvideo.ru) находят другой формат и играют без MediaSource.
    var stuck = function(root){
      var found = null;
      root.querySelectorAll('*').forEach(function(e){
        if (found === false) return;
        if (e.tagName === 'VIDEO') found = !(e.readyState >= 2 || e.currentTime > 0);
        else if (e.shadowRoot) { var inner = stuck(e.shadowRoot); if (inner !== null) found = inner; }
      });
      return found;
    };
    Object.defineProperty(window, 'MediaSource', { configurable: true, get: function(){
      if (!told) { told = true; setTimeout(function(){ if (stuck(document) === true) send({t:'unsupported', what:'MediaSource'}); }, 12000); } return undefined; } });
  }
} catch(e) {}
var last = '';
function grab(form){
  if (!form || !form.querySelector) return;
  var pw = form.querySelector('input[type=password]');
  if (!pw || !pw.value) return;
  var user = null, ins = form.querySelectorAll('input');
  for (var i = 0; i < ins.length; i++) {
    if (ins[i] === pw) break;
    var t = (ins[i].type || 'text').toLowerCase();
    if (t == 'text' || t == 'email' || t == 'tel' || t == '') user = ins[i];
  }
  var key = (user ? user.value : '') + '\u0000' + pw.value;
  if (key == last) return; last = key;
  send({t:'login', user: user ? user.value : '', pass: pw.value});
}
document.addEventListener('submit', function(e){ grab(e.target); }, true);
document.addEventListener('click', function(e){
  var b = e.target; while (b && b.tagName != 'BUTTON' && !(b.tagName == 'INPUT' && b.type == 'submit')) b = b.parentElement;
  if (b && b.form) grab(b.form);
}, true);
document.addEventListener('keydown', function(e){ if (e.key == 'Enter' && e.target && e.target.form) grab(e.target.form); }, true);
document.addEventListener('click', function(e){
  var a = e.target; while (a && a.tagName != 'A') a = a.parentElement;
  if (a && a.hasAttribute('download') && a.href && /^https?:/.test(a.href)) { e.preventDefault(); send({t:'download', url: a.href}); }
}, true);
// Два обхода пробелов движка, дорисовываются после загрузки и изменений:
// 1) SVG растеризует отдельный движок без CSS — `fill="var(--x)"` выходил
//    чёрным (github.com): подставляем вычисленные значения переменных;
// 2) `background-clip: text` Servo не разбирает — «градиентный текст»
//    (прозрачный цвет поверх градиента) был сплошным прямоугольником:
//    красим буквы первым цветом градиента.
try {
  var PAINT = ['fill', 'stroke', 'stop-color', 'flood-color'];
  var resolve = function(el, value, depth){
    var m = /^\s*var\(\s*(--[^,\s)]+)\s*(?:,\s*([\s\S]*))?\)\s*$/.exec(value);
    if (!m || depth > 4) return m ? '' : value;
    var got = getComputedStyle(el).getPropertyValue(m[1]).trim() || (m[2] || '').trim();
    return resolve(el, got, depth + 1);
  };
  var svgFix = function(){
    var list = document.querySelectorAll('svg[fill^="var("], svg [fill^="var("], svg [stroke^="var("], svg [stop-color^="var("], svg [flood-color^="var("]');
    var roots = [];
    for (var i = 0; i < list.length && i < 600; i++) {
      for (var k = 0; k < PAINT.length; k++) {
        var v = list[i].getAttribute(PAINT[k]);
        if (v && v.indexOf('var(') >= 0) { var to = resolve(list[i], v, 0); list[i].setAttribute(PAINT[k], to || 'none'); }
      }
      var root = list[i].closest ? list[i].closest('svg') : null;
      if (root && roots.indexOf(root) < 0) roots.push(root);
    }
    // Растр SVG Servo пересчитывает только при изменении самого <svg>; у
    // декоративных картинок фигуры, раскрашенные лишь классами CSS (растеризатор
    // их не видит), без этого заливаются чёрным.
    for (var r = 0; r < roots.length; r++) {
      if (roots[r].getAttribute('aria-hidden') == 'true') {
        var bare = roots[r].querySelectorAll('path[class]:not([fill]), rect[class]:not([fill])');
        for (var b = 0; b < bare.length; b++) bare[b].setAttribute('fill', 'none');
      }
      roots[r].setAttribute('data-obsos-svg', String(Date.now() % 100000));
    }
  };
  var textFix = function(){
    var list = document.querySelectorAll('h1,h2,h3,h4,h5,span,a,p,strong,em,b,div');
    for (var i = 0; i < list.length && i < 2500; i++) {
      var el = list[i];
      if (el.__obsosText || !el.firstChild) continue;
      var s = getComputedStyle(el), bg = s.backgroundImage;
      if (!bg || bg.indexOf('gradient(') < 0) continue;
      if (!(s.color === 'transparent' || /^rgba\([^)]*,\s*0\)$/.test(s.color)) || !el.textContent.trim()) continue;
      var stops = bg.match(/rgba?\([^)]*\)/g);
      if (!stops) continue;
      el.__obsosText = 1;
      el.style.setProperty('background-image', 'none', 'important');
      el.style.setProperty('color', stops[0], 'important');
    }
  };
  var fixLater = 0, fixed = 0;
  var fix = function(){ fixLater = 0; fixed++; try { svgFix(); textFix(); } catch(e) {} };
  var plan = function(){ if (!fixLater && fixed < 25) fixLater = setTimeout(fix, 300 + fixed * 120); };
  var watch = function(){
    fix();
    if (window.MutationObserver && document.body) new MutationObserver(function(records){
      for (var i = 0; i < records.length; i++) if (records[i].addedNodes.length) { plan(); return; }
    }).observe(document.body, {childList: true, subtree: true});
  };
  if (document.readyState == 'loading') document.addEventListener('DOMContentLoaded', watch); else watch();
  window.addEventListener('load', plan);
} catch(e) {}
function probe(){ if (document.querySelector('input[type=password]')) send({t:'pwfield'}); }
if (document.readyState == 'complete') probe(); else window.addEventListener('load', probe);
})();"#;

/// Скрипт подстановки входа (значения — строки JSON).
pub fn fill_script(user: &str, pass: &str) -> String {
    let quote = |s: &str| {
        let mut out = String::new();
        json::write_str(s, &mut out);
        out
    };
    format!(
        "(function(u,p){{var pw=document.querySelector('input[type=password]');if(!pw)return false;var f=pw.form||document;\
var ins=f.querySelectorAll('input'),user=null;for(var i=0;i<ins.length;i++){{if(ins[i]===pw)break;var t=(ins[i].type||'text').toLowerCase();\
if(t=='text'||t=='email'||t=='tel'||t=='')user=ins[i];}}function set(el,v){{el.focus();el.value=v;\
el.dispatchEvent(new Event('input',{{bubbles:true}}));el.dispatchEvent(new Event('change',{{bubbles:true}}));}}\
if(user&&u)set(user,u);set(pw,p);return true;}})({},{})",
        quote(user),
        quote(pass)
    )
}

// ── Оболочка ────────────────────────────────────────────────────────────────

pub struct Tab {
    pub webview: WebView,
    pub title: String,
    /// Адрес для показа (`obsos://newtab` — пусто).
    pub url: String,
    pub real_url: Option<url::Url>,
    pub loading: bool,
    pub status: Option<String>,
    pub zoom: f32,
    /// Сколько запросов рекламы/трекеров заблокировано на этой странице.
    pub blocked: u32,
    pub favicon: Option<Favicon>,
}

/// Значок вкладки: картинка от Servo, затем текстура egui (делается при
/// первом рисовании).
pub enum Favicon {
    Pending(egui::ColorImage),
    Ready(egui::TextureHandle),
}

pub(crate) fn color_image(image: &servo::Image) -> Option<egui::ColorImage> {
    use servo::PixelFormat as F;
    let (w, h) = (image.width as usize, image.height as usize);
    if w == 0 || h == 0 || w > 512 || h > 512 {
        return None;
    }
    let data = image.data();
    let bytes = match image.format {
        F::K8 => 1,
        F::KA8 => 2,
        F::RGB8 => 3,
        F::RGBA8 | F::BGRA8 => 4,
    };
    let mut pixels = Vec::with_capacity(w * h);
    for p in data.chunks_exact(bytes).take(w * h) {
        let (r, g, b, a) = match image.format {
            F::K8 => (p[0], p[0], p[0], 255),
            F::KA8 => (p[0], p[0], p[0], p[1]),
            F::RGB8 => (p[0], p[1], p[2], 255),
            F::RGBA8 => (p[0], p[1], p[2], p[3]),
            F::BGRA8 => (p[2], p[1], p[0], p[3]),
        };
        pixels.push(egui::Color32::from_rgba_unmultiplied(r, g, b, a));
    }
    (pixels.len() == w * h).then(|| egui::ColorImage { size: [w, h], source_size: egui::vec2(w as f32, h as f32), pixels })
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    NewTab,
    NewTabUrl(String),
    Activate(usize),
    Close(usize),
    Back,
    Forward,
    Reload,
    Go(String),
    OpenUrl(String),
    FocusAddress,
    ToggleBookmark,
    Chromium(String),
    Zoom(f32),
    Panel(Panel),
    SyncNow(Option<String>),
    /// Поиск на странице: открыть строку (Ctrl+F), следующее/предыдущее, закрыть.
    FindOpen,
    Find { back: bool },
    /// Скачать адрес в «Загрузки».
    Download(String),
    /// «Сохранить ссылку/картинку как…»: скачать, спросив, куда.
    SaveAs(String),
    /// Выбрать папку загрузок окном системы.
    ChooseDownloadDir,
    /// Блокировка рекламы на текущем сайте: выключить/включить (щит).
    ToggleBlockSite,
    /// Режим чтения: только текст статьи.
    Reader,
    /// Ещё одно окно того же профиля (Ctrl+N).
    NewWindow,
    /// Новое окно инкогнито (Ctrl+Shift+N): профиль во временном каталоге.
    PrivateWindow,
    /// Вернуть последнюю закрытую вкладку (Ctrl+Shift+T).
    Reopen,
    /// Домашняя страница в текущей вкладке (Alt+Home).
    Home,
    FindClose,
    Quit,
    /// Открыть `obsos://<имя>` (или перейти на вкладку, где он уже открыт).
    OpenPage(&'static str),
    /// Новая вкладка, не переключаясь на неё.
    OpenBackground(String),
    Duplicate(usize),
    CloseOthers(usize),
    ReloadTab(usize),
    /// Перетащить вкладку: откуда, куда.
    MoveTab(usize, usize),
    SetTabs(TabsAt),
    Fullscreen,
    /// Выбрать элемент страницы мышью (инструменты разработчика).
    Inspect,
    /// «Просмотреть код» элемента в точке окна (точки интерфейса).
    InspectAt(f32, f32),
    /// Инструменты разработчика сразу на консоли (Ctrl+Shift+J).
    Console,
    Toast(&'static str),
}

/// Состояние генератора паролей, проверки утечек, импорта и скачивания
/// списков фильтров (ответы фоновых потоков — в `Arc<Mutex>`).
pub struct ToolsUi {
    pub gen_len: usize,
    pub gen_symbols: bool,
    pub generated: String,
    pub import_path: String,
    /// Запись пароля → «не найден в утечках» / «найден N раз».
    pub leaks: Arc<Mutex<std::collections::BTreeMap<String, String>>>,
    /// Скачанный список EasyList (или ошибка), пока главный поток не забрал.
    pub filters: Arc<Mutex<Option<Result<String, String>>>>,
    pub filters_busy: bool,
}

impl Default for ToolsUi {
    fn default() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| String::from("/home"));
        ToolsUi {
            gen_len: 20,
            gen_symbols: true,
            generated: String::new(),
            import_path: format!("{home}/Downloads/"),
            leaks: Arc::default(),
            filters: Arc::default(),
            filters_busy: false,
        }
    }
}

/// Строка поиска на странице. В Servo своего поиска нет — ищет скрипт
/// (`find_script`): подсвечивает совпадения `<mark>` и прокручивает к текущему.
#[derive(Default)]
pub struct FindBar {
    pub open: bool,
    pub text: String,
    pub focus: bool,
    /// «3/12» — ответ скрипта (пишет обратный вызов `evaluate_javascript`).
    pub status: Rc<RefCell<String>>,
}

/// Скрипт поиска: `q` — строка (пусто — убрать подсветку), `back` — назад.
pub fn find_script(q: &str, back: bool) -> String {
    let mut quoted = String::new();
    json::write_str(q, &mut quoted);
    format!(
        r#"(function(q,back){{var s=window.__obsosFind;
if(!s||s.q!==q){{if(s){{s.marks.forEach(function(m){{var p=m.parentNode;if(p){{p.replaceChild(document.createTextNode(m.textContent),m);p.normalize();}}}});}}
s={{q:q,marks:[],i:-1}};window.__obsosFind=s;
if(q&&document.body){{var lq=q.toLowerCase(),w=document.createTreeWalker(document.body,NodeFilter.SHOW_TEXT,null),nodes=[],n;
while((n=w.nextNode())){{var t=n.parentNode&&n.parentNode.nodeName;if(t!=='SCRIPT'&&t!=='STYLE'&&t!=='TEXTAREA'&&n.nodeValue.toLowerCase().indexOf(lq)>=0)nodes.push(n);}}
nodes.forEach(function(n){{var v=n.nodeValue,lv=v.toLowerCase(),pos=0,i,f=document.createDocumentFragment();
while((i=lv.indexOf(lq,pos))>=0){{f.appendChild(document.createTextNode(v.slice(pos,i)));var m=document.createElement('mark');
m.style.background='#ffd54a';m.style.color='#000';m.textContent=v.slice(i,i+q.length);f.appendChild(m);s.marks.push(m);pos=i+q.length;}}
f.appendChild(document.createTextNode(v.slice(pos)));n.parentNode.replaceChild(f,n);}});}}}}
if(!s.marks.length)return q?'0/0':'';
if(s.i>=0)s.marks[s.i].style.background='#ffd54a';
s.i=back?(s.i<=0?s.marks.length-1:s.i-1):(s.i+1)%s.marks.length;
var m=s.marks[s.i];m.style.background='#ff8a3d';m.scrollIntoView({{block:'center'}});return (s.i+1)+'/'+s.marks.length;}})({quoted},{back})"#
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Menu,
    Bookmarks,
    Passwords,
    RemoteTabs,
    Sync,
    Scripts,
    Profiles,
    Downloads,
    DevTools,
}

pub enum Modal {
    Dialog(SimpleDialog, String),
    Auth { request: AuthenticationRequest, host: String, user: String, pass: String, remember: bool },
    Select(SelectElement, egui::Pos2),
    Context(ContextMenu, egui::Pos2),
    Color(ColorPicker, [u8; 3]),
    File(FilePicker, String),
    Master { text: String, error: String },
    Rollback(String),
}

impl Modal {
    fn control_id(&self) -> Option<EmbedderControlId> {
        match self {
            Modal::Select(c, _) => Some(c.id()),
            Modal::Context(c, _) => Some(c.id()),
            Modal::Color(c, _) => Some(c.id()),
            Modal::File(c, _) => Some(c.id()),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Infobar {
    SaveLogin(Login),
    ChooseLogin(String),
    Unsupported { url: String, what: String },
}

/// Вход через браузер (Device Flow), пока ждём подтверждения.
pub struct LoginFlow {
    pub code: oauth::DeviceCode,
    pub next_poll: Instant,
    pub interval: Duration,
    pub until: Instant,
}

pub struct SyncUi {
    /// Редактируемая учётная запись (форма в панели).
    pub draft: Account,
    pub label: String,
    pub points: Vec<SyncPoint>,
    pub status: String,
    pub flow: Option<LoginFlow>,
    pub sync_password: String,
    pub next_auto: Instant,
    pub last_report: String,
}

pub struct Shell {
    pub servo: Servo,
    pub context: Rc<dyn PageContext>,
    pub delegate: Rc<Delegate>,
    pub tabs: Vec<Tab>,
    pub active: usize,
    pub address: String,
    pub focus_address: bool,
    pub opened: usize,
    pub profile: Profile,
    pub vault: Option<Vault>,
    pub page_data: Arc<Mutex<PageData>>,
    pub panel: Option<Panel>,
    pub modal: Option<Modal>,
    pub infobars: Vec<Infobar>,
    pub toasts: Vec<(String, Instant)>,
    pub filter: String,
    pub syncer: Syncer,
    pub sync: SyncUi,
    pub show_passwords: BTreeSet<String>,
    pub new_profile: String,
    /// Сколько раз уже подставили сохранённый HTTP-вход для адреса (чтобы не
    /// зациклиться на неверном).
    pub auth_tries: Vec<String>,
    pub find: FindBar,
    pub tools: ToolsUi,
    pub dev: crate::devtools::DevUi,
    /// Закрытые вкладки — для Ctrl+Shift+T (последняя — в конце).
    pub closed: Vec<String>,
    pub quit: bool,
    pub dl: DownloadsUi,
    /// Вкладки, адресная строка, меню (`chrome.rs`).
    pub chrome: crate::chrome::ChromeState,
    /// Страницы управления `obsos://…` (`webui_host.rs`).
    pub webui: crate::webui_host::WebUi,
    /// Ключ этого запуска, с которым страницы управления присылают команды.
    pub ui_token: String,
    /// Последний щелчок «в новую вкладку» (средняя кнопка, Ctrl+щелчок).
    pub background_click: Option<Instant>,
    /// Вкладка, чья страница заняла весь экран (видео).
    pub page_fullscreen: Option<WebViewId>,
    /// Платформа умеет рисовать страницы видеокартой / сейчас так и рисует.
    pub gpu_available: bool,
    pub gpu_active: bool,
    pub renderer_name: String,
    /// Остальные окна (`windows.rs`).
    pub windows: crate::windows::Windows,
    /// Вкладка с новым кадром, ещё не нарисованным (`paint_pending`).
    pub frame_wanted: Option<WebView>,
    /// Вопросы страниц о микрофоне и камере без ответа.
    pub permissions: Vec<Permission>,
    /// Что уже разрешено сайтам до закрытия браузера: (источник, чему).
    pub granted: BTreeSet<(String, &'static str)>,
}

pub struct Permission {
    pub webview: WebViewId,
    origin: String,
    pub host: String,
    pub what: &'static str,
    request: servo::PermissionRequest,
}

/// Что показывает вкладка вместо адреса, ушедшего в загрузки (пока не
/// вернулась назад; у новой вкладки без истории — остаётся).
const DOWNLOAD_PAGE: &str = "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>Загрузка</title>\
<style>body{font-family:sans-serif;display:flex;align-items:center;justify-content:center;height:90vh;color:#667;background:#f4f6fb}</style></head>\
<body><p>Файл скачивается — ход внизу окна и в «Загрузках» (Ctrl+J).</p></body></html>";

/// Загрузки со стороны оболочки: ждущие проверки документы вкладок, окно
/// «Сохранить как», полоса загрузок.
#[derive(Default)]
pub struct DownloadsUi {
    /// Проверки ссылок: номер загрузки, вкладка, придержанный запрос, когда.
    probes: Vec<(u64, WebViewId, servo::WebResourceLoad, Instant, url::Url)>,
    /// О каких загрузках окно «Сохранить как» уже спрошено.
    asked: BTreeSet<u64>,
    /// Ждут окна «Сохранить как» (окно — одно за раз).
    queue: Vec<(u64, String)>,
    dialog: Option<(DialogFor, window::FileDialog)>,
    /// Вкладка ушла на адрес файла — вернуть её назад, когда Servo его покажет.
    leave: Vec<(WebViewId, url::Url, Instant)>,
    /// Полоса загрузок внизу показана (закрывается крестиком, снова — новой загрузкой).
    pub bar: bool,
}

enum DialogFor {
    Save(u64),
    Folder,
}

pub fn shown_url(url: &url::Url) -> String {
    match url.scheme() {
        "data" => String::new(),
        "obsos" if url.host_str() == Some("newtab") => String::new(),
        "obsos" if url.host_str() == Some("view") => {
            url.query_pairs().find(|(k, _)| k == "path").map(|(_, v)| v.into_owned()).unwrap_or_default()
        }
        _ => url.as_str().to_string(),
    }
}

impl Shell {
    pub fn look(&self) -> &Look {
        &self.profile.settings.look
    }

    pub fn active_view(&self) -> Option<&WebView> {
        self.tabs.get(self.active).map(|t| &t.webview)
    }

    pub fn pointer_left_page(&mut self) {
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.status = None;
            tab.webview.notify_input_event(servo::InputEvent::MouseLeftViewport(Default::default()));
        }
    }

    pub fn toast(&mut self, text: impl Into<String>) {
        let text = text.into();
        println!("browser: toast {}", crate::logs::scrub(&text));
        self.toasts.push((text, Instant::now() + Duration::from_secs(5)));
    }

    /// Что набрали в адресной строке → адрес.
    pub fn url_from_input(&self, text: &str) -> Option<url::Url> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        // Путь к файлу: `/…`, на Windows и `C:\…`.
        if text.starts_with('/') || (cfg!(windows) && std::path::Path::new(text).is_absolute()) {
            if pages::viewable_path(text) {
                return Some(pages::view_url(text));
            }
            return url::Url::from_file_path(text).ok().or_else(|| url::Url::parse(&format!("file://{text}")).ok());
        }
        if text.contains("://") || text.starts_with("data:") || text.starts_with("about:") || text.starts_with("obsos:") {
            return url::Url::parse(text).ok();
        }
        if !text.contains(' ') && (text.contains('.') || text.starts_with("localhost")) {
            return url::Url::parse(&format!("https://{text}")).ok();
        }
        Some(self.profile.settings.search.url(text))
    }

    pub fn home_url(&self) -> url::Url {
        url::Url::parse(&self.profile.settings.home).unwrap_or_else(|_| url::Url::parse("obsos://newtab").expect("newtab"))
    }

    pub fn update_page_data(&self) {
        let look = self.look();
        let search = self.profile.settings.search;
        let probe = search.url("");
        let field = probe.query_pairs().next().map(|(k, _)| k.into_owned()).unwrap_or_else(|| "q".into());
        let mut action = probe.clone();
        action.set_query(None);
        let data = PageData {
            bookmarks: self.profile.bookmarks.iter().map(|b| (b.url.clone(), b.title.clone())).collect(),
            search_action: action.to_string(),
            search_field: field,
            search_title: search.title().to_string(),
            dark: look.dark,
            accent: look.accent.hex(),
            accent2: look.accent2.hex(),
            chromium: window::chromium_installed(),
            notice_path: window::platform().notice_path(),
            token: self.ui_token.clone(),
            theme_css: crate::webui_host::theme_css(look),
        };
        if let Ok(mut shared) = self.page_data.lock() {
            if *shared == data {
                return;
            }
            *shared = data;
        }
        // Открытые «Новые вкладки» показывают старые плитки — перерисовать.
        for tab in &self.tabs {
            if tab.real_url.as_ref().is_some_and(|u| u.as_str().starts_with("obsos://newtab")) {
                tab.webview.reload();
            }
        }
    }

    // ── Вкладки ─────────────────────────────────────────────────────────

    fn open_tab(&mut self, webview: WebView, window: &Window) {
        self.opened += 1;
        webview.set_hidpi_scale_factor(euclid::Scale::new(page_scale()));
        let zoom = self.profile.settings.page_zoom as f32 / 100.0;
        if zoom != 1.0 {
            webview.set_page_zoom(zoom);
        }
        self.tabs.push(Tab {
            webview,
            title: String::from("Новая вкладка"),
            url: String::new(),
            real_url: None,
            loading: true,
            status: None,
            zoom,
            blocked: 0,
            favicon: None,
        });
        let index = self.tabs.len() - 1;
        println!("browser: tab {} opened ({} open)", self.opened, self.tabs.len());
        self.resize_all(window);
        self.activate(index, window);
    }

    pub fn new_tab(&mut self, url: url::Url, window: &Window) {
        let webview = WebViewBuilder::new(&self.servo, self.context.clone() as Rc<dyn RenderingContext>)
            .delegate(self.delegate.clone())
            .clipboard_delegate(self.delegate.clone())
            .user_content_manager(self.delegate.content.clone())
            .url(url)
            .build();
        self.open_tab(webview, window);
    }

    /// Выйти из полного экрана, занятого страницей (другая вкладка, переход).
    pub fn leave_page_fullscreen(&mut self) {
        let Some(id) = self.page_fullscreen.take() else { return };
        set_fullscreen(false);
        if let Some(tab) = self.tabs.iter().find(|t| t.webview.id() == id) {
            tab.webview.exit_fullscreen();
        }
    }

    pub fn activate(&mut self, index: usize, window: &Window) {
        if index >= self.tabs.len() {
            return;
        }
        if self.page_fullscreen.is_some_and(|id| id != self.tabs[index].webview.id()) {
            self.leave_page_fullscreen();
        }
        for (i, tab) in self.tabs.iter().enumerate() {
            if i != index {
                tab.webview.hide();
            }
        }
        self.active = index;
        let tab = &self.tabs[index];
        tab.webview.show();
        tab.webview.set_focused(true);
        tab.webview.paint();
        self.context.present();
        self.address = tab.url.clone();
        window.set_title(&format!("{} — {}", if self.profile.private { "OBSOS (инкогнито)" } else { "OBSOS" }, tab.title));
        self.infobars.clear();
        println!("browser: tab {} active", index + 1);
    }

    /// Закрыть вкладку; `false` — вкладок не осталось.
    pub fn close_tab(&mut self, index: usize, window: &Window) -> bool {
        if index >= self.tabs.len() {
            return true;
        }
        let tab = self.tabs.remove(index);
        // Вопрос закрытой вкладки — отказ (так отвечает и сам запрос при
        // уничтожении).
        let closed_view = tab.webview.id();
        self.permissions.retain(|p| p.webview != closed_view);
        if let Some(url) = tab.real_url {
            self.closed.push(url.to_string());
            if self.closed.len() > 25 {
                self.closed.remove(0);
            }
        }
        println!("browser: tab {} closed ({} open)", index + 1, self.tabs.len());
        if self.tabs.is_empty() {
            return false;
        }
        let next = if self.active > index || self.active >= self.tabs.len() { self.active.saturating_sub(1) } else { self.active };
        self.activate(next.min(self.tabs.len() - 1), window);
        true
    }

    fn tab_index(&self, id: WebViewId) -> Option<usize> {
        self.tabs.iter().position(|t| t.webview.id() == id)
    }

    pub fn resize_all(&mut self, window: &Window) {
        let size = page_size(self.look(), window.width(), window.height());
        // Холст swgl Servo меняет сам (`resize_rendering_context`) и, если
        // холст уже нужного размера, пропускает перестройку WebRender, —
        // поэтому трогать его здесь нельзя.
        for tab in &self.tabs {
            tab.webview.set_hidpi_scale_factor(euclid::Scale::new(page_scale()));
        }
        self.set_page_size(size);
    }

    pub fn load(&mut self, url: url::Url) {
        println!("browser: go {}", crate::logs::url(url.as_str()));
        // Адрес архива или программы из строки — сразу в загрузки: иначе
        // Servo сперва скачал бы его целиком, чтобы отказаться показывать.
        if crate::downloads::looks_like_file(&url) {
            self.download(url.as_str(), false);
            return;
        }
        // Меню и списки прежней страницы закрываются.
        match self.modal.take() {
            Some(Modal::Context(menu, _)) => menu.dismiss(),
            Some(Modal::Select(select, _)) => select.submit(),
            other => self.modal = other,
        }
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.url = shown_url(&url);
            tab.loading = true;
            tab.webview.load(url);
            tab.webview.set_focused(true);
        }
        self.infobars.clear();
    }

    // ── События Servo ───────────────────────────────────────────────────

    /// Разобрать накопленные события. Возвращает: менялась ли оболочка.
    pub fn apply_notes(&mut self, window: &Window) -> bool {
        let notes = std::mem::take(&mut *self.delegate.notes.borrow_mut());
        // Кадр страницы оболочку не меняет: анимация на странице не должна
        // перерисовывать egui. Несколько кадров за виток — одна отрисовка.
        let changed = notes.iter().any(|n| !matches!(n, Note::Frame(_)));
        // Вкладки, приславшие новый кадр (рисуется только активная).
        let mut frames: Vec<WebViewId> = Vec::new();
        for note in notes {
            match note {
                Note::Frame(id) => {
                    if !frames.contains(&id) {
                        frames.push(id);
                    }
                }
                Note::Title(id, title) => {
                    let active = self.active;
                    if let Some(i) = self.tab_index(id) {
                        let tab = &mut self.tabs[i];
                        tab.title = title.map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).unwrap_or_else(|| {
                            if tab.url.is_empty() { String::from("Новая вкладка") } else { tab.url.clone() }
                        });
                        if i == active {
                            window.set_title(&format!("{} — {}", if self.profile.private { "OBSOS (инкогнито)" } else { "OBSOS" }, tab.title));
                        }
                    }
                }
                Note::Url(id, url) => {
                    // Вкладка показала адрес, который ушёл в загрузки (ошибка
                    // отменённого документа) — назад, на страницу со ссылкой.
                    if let Some(at) = self.dl.leave.iter().position(|(w, u, _)| *w == id && *u == url) {
                        self.dl.leave.remove(at);
                        if let Some(i) = self.tab_index(id) {
                            let webview = self.tabs[i].webview.clone();
                            println!("browser: download page left");
                            if webview.can_go_back() {
                                webview.go_back(1);
                            } else {
                                webview.load(url::Url::parse("obsos://newtab").expect("newtab"));
                            }
                        }
                        continue;
                    }
                    let focused = self.focus_address;
                    let active = self.active;
                    let shown = shown_url(&url);
                    if let Some(i) = self.tab_index(id) {
                        self.tabs[i].url = shown.clone();
                        self.tabs[i].real_url = Some(url);
                        if i == active && !focused {
                            self.address = shown;
                        }
                    }
                }
                Note::Load(id, status) => {
                    if let Some(i) = self.tab_index(id) {
                        self.tabs[i].loading = status != LoadStatus::Complete;
                        if status == LoadStatus::Started {
                            self.tabs[i].blocked = 0;
                            self.tabs[i].favicon = None;
                            // Новый документ полный экран не наследует.
                            if self.page_fullscreen == Some(id) {
                                self.leave_page_fullscreen();
                            }
                        }
                        if status == LoadStatus::Complete {
                            let (url, title) = (self.tabs[i].real_url.clone(), self.tabs[i].title.clone());
                            if let Some(url) = url {
                                self.profile.visit(url.as_str(), &title);
                                if url.scheme().starts_with("http") && self.tabs[i].title == self.tabs[i].url {
                                    self.check_unknown_type(i);
                                }
                            }
                        }
                    }
                }
                Note::Status(id, status) => {
                    if let Some(i) = self.tab_index(id) {
                        self.tabs[i].status = status;
                    }
                }
                Note::History => {}
                Note::Opened(webview, _) => {
                    let back = self.active;
                    let background = self.background_click.take().is_some_and(|at| at.elapsed() < Duration::from_secs(3));
                    self.open_tab(webview, window);
                    if background {
                        self.activate(back, window);
                    }
                }
                Note::Crashed(id, reason) => {
                    if let Some(i) = self.tab_index(id) {
                        let url = self.tabs[i].real_url.as_ref().map(|u| u.to_string()).unwrap_or_default();
                        self.tabs[i].webview.load(pages::crashed_url(&url, &reason));
                    }
                }
                Note::Control(id, control) => self.show_control(id, control),
                Note::HideControl(id) => {
                    if self.modal.as_ref().and_then(Modal::control_id) == Some(id) {
                        self.modal = None;
                    }
                }
                Note::Auth(id, request) => self.auth_request(id, request),
                Note::Script(id, payload) => self.script_message(id, &payload),
                Note::Action(action) => {
                    self.run(action, window);
                }
                Note::Blocked(id) => {
                    if let Some(i) = self.tab_index(id) {
                        self.tabs[i].blocked += 1;
                    }
                }
                Note::WebUi(id, payload) => self.ui_message(id, &payload, window),
                Note::DevTools(id, payload) => {
                    if self.active_view().is_some_and(|w| w.id() == id) && self.panel == Some(Panel::DevTools) {
                        crate::devtools::agent_message(self, &payload);
                    }
                }
                Note::Permission(id, request) => self.permission_asked(id, request),
                Note::Redraw => {}
                Note::Favicon(id) => {
                    if let Some(i) = self.tab_index(id) {
                        let image = self.tabs[i].webview.favicon().and_then(|image| color_image(&image));
                        self.tabs[i].favicon = image.map(Favicon::Pending);
                    }
                }
                Note::Fullscreen(id, on) => {
                    if on && self.active_view().is_some_and(|w| w.id() == id) {
                        set_fullscreen(true);
                        self.panel = None;
                        self.page_fullscreen = Some(id);
                        // Кто занял экран и как выйти — чтобы страница не
                        // выдала себя за окно браузера.
                        let host = self.tabs[self.active].real_url.as_ref().and_then(|u| u.host_str()).unwrap_or("Страница").to_string();
                        self.toast(format!("{host} — на весь экран. Выход: Esc"));
                    } else if !on && self.page_fullscreen == Some(id) {
                        self.page_fullscreen = None;
                        set_fullscreen(false);
                    }
                }
            }
        }
        if self.active_view().is_some_and(|w| frames.contains(&w.id())) {
            self.frame_wanted = Some(self.tabs[self.active].webview.clone());
        }
        changed
    }

    /// Страница ждёт разрешения (её скрипт стоит до ответа).
    fn permission_asked(&mut self, id: WebViewId, request: servo::PermissionRequest) {
        let what = match request.feature() {
            servo::PermissionFeature::Microphone => "микрофону",
            servo::PermissionFeature::Camera => "камере",
            // Остальное (уведомления, местоположение…) пока не умеем — отказ.
            _ => return request.deny(),
        };
        let url = self.tab_index(id).and_then(|i| self.tabs[i].real_url.clone());
        let Some(url) = url else { return request.deny() };
        let origin = url.origin().ascii_serialization();
        if self.granted.contains(&(origin.clone(), what)) {
            return request.allow();
        }
        let host = url.host_str().unwrap_or("Страница").to_string();
        println!("browser: permission asked: {} {what}", crate::logs::url(url.as_str()));
        self.permissions.push(Permission { webview: id, origin, host, what, request });
    }

    /// Ответ на вопрос `index` из `permissions`. Разрешённое помнится для
    /// сайта до закрытия браузера.
    pub fn answer_permission(&mut self, index: usize, allow: bool) {
        if index >= self.permissions.len() {
            return;
        }
        let asked = self.permissions.remove(index);
        println!("browser: permission {}: {}", asked.what, if allow { "allowed" } else { "denied" });
        if allow {
            self.granted.insert((asked.origin, asked.what));
            asked.request.allow();
        } else {
            asked.request.deny();
        }
    }

    /// Нарисовать пришедшие кадры. Painter один на все окна, поэтому — раз
    /// за виток, после разбора событий каждого окна.
    pub fn paint_pending(&mut self) {
        let Some(webview) = self.frame_wanted.take() else { return };
        let at = Instant::now();
        webview.paint();
        crate::perf::add(crate::perf::Stage::Paint, at);
        let at = Instant::now();
        self.context.present();
        crate::perf::add(crate::perf::Stage::Store, at);
    }

    /// Servo показал свою «Unknown content type»: страница без заголовка.
    fn check_unknown_type(&mut self, index: usize) {
        let delegate = self.delegate.clone();
        let id = self.tabs[index].webview.id();
        self.tabs[index].webview.evaluate_javascript(
            "document.body && document.body.innerText.indexOf('Unknown content type') == 0 ? document.body.innerText : ''",
            move |result| {
                if let Ok(servo::JSValue::String(text)) = result {
                    if !text.is_empty() {
                        let what = text.trim_start_matches("Unknown content type").trim().trim_matches(|c| c == '(' || c == ')' || c == '.').to_string();
                        println!("browser: unknown type {what} — download");
                        delegate.note(Note::Script(id, String::from("{\"t\":\"fetch\"}")));
                    }
                }
            },
        );
    }

    /// Точка страницы (физическая, как отдаёт Servo) → точка интерфейса.
    fn ui_point(&self, x: i32, y: i32) -> egui::Pos2 {
        let (ox, oy) = page_origin(self.look());
        let k = ui_scale();
        egui::pos2((x + ox as i32) as f32 / k, (y + oy as i32) as f32 / k)
    }

    fn show_control(&mut self, id: WebViewId, control: EmbedderControl) {
        if self.active_view().is_none_or(|w| w.id() != id) {
            return;
        }
        self.modal = match control {
            EmbedderControl::SelectElement(select) => {
                let r = select.position();
                let at = self.ui_point(r.min.x, r.max.y);
                Some(Modal::Select(select, at))
            }
            EmbedderControl::ContextMenu(menu) => {
                let r = menu.position();
                let at = self.ui_point(r.min.x, r.min.y);
                Some(Modal::Context(menu, at))
            }
            EmbedderControl::ColorPicker(picker) => {
                let c = picker.current_color().map(|c| [c.red, c.green, c.blue]).unwrap_or([0, 0, 0]);
                Some(Modal::Color(picker, c))
            }
            EmbedderControl::FilePicker(picker) => {
                let home = std::env::var("HOME").unwrap_or_else(|_| "/home".into());
                Some(Modal::File(picker, format!("{home}/")))
            }
            EmbedderControl::SimpleDialog(dialog) => {
                let value = match &dialog {
                    SimpleDialog::Prompt(p) => p.current_value().to_string(),
                    _ => String::new(),
                };
                Some(Modal::Dialog(dialog, value))
            }
            // Экранной клавиатуры на ПК нет — поле и так получает клавиши.
            EmbedderControl::InputMethod(_) => return,
        };
        println!("browser: control shown");
    }

    fn auth_request(&mut self, id: WebViewId, request: AuthenticationRequest) {
        let url = request.url().clone();
        let host = url.host_str().unwrap_or("").to_string();
        let key = url.as_str().to_string();
        if let Some(origin) = vault::origin_of(&url) {
            if let Some(login) = self.profile.logins_for(&origin).into_iter().next() {
                if !self.auth_tries.contains(&key) {
                    self.auth_tries.push(key);
                    println!("browser: http auth from vault for {}", crate::logs::text(&host));
                    request.authenticate(login.username, login.password);
                    return;
                }
            }
        }
        let _ = id;
        self.modal = Some(Modal::Auth { request, host, user: String::new(), pass: String::new(), remember: true });
    }

    fn origin_of_tab(&self, id: WebViewId) -> Option<String> {
        let i = self.tab_index(id)?;
        vault::origin_of(self.tabs[i].real_url.as_ref()?)
    }

    fn script_message(&mut self, id: WebViewId, payload: &str) {
        let Ok(v) = json::parse(payload) else { return };
        match v.str_field("t") {
            Some("login") => {
                if !self.profile.settings.save_passwords {
                    return;
                }
                let Some(origin) = self.origin_of_tab(id) else { return };
                let username = v.str_field("user").unwrap_or("").to_string();
                let password = v.str_field("pass").unwrap_or("").to_string();
                if password.is_empty() || self.never_save(&origin) {
                    return;
                }
                let known = self.profile.logins_for(&origin).into_iter().any(|l| l.username == username && l.password == password);
                if !known {
                    println!("browser: login form submitted on {}", crate::logs::url(&origin));
                    self.infobars.retain(|b| !matches!(b, Infobar::SaveLogin(_)));
                    self.infobars.push(Infobar::SaveLogin(Login { origin, username, password, updated_ms: now_ms() }));
                }
            }
            Some("pwfield") => {
                let Some(origin) = self.origin_of_tab(id) else { return };
                let logins = self.profile.logins_for(&origin);
                if !self.profile.settings.autofill || logins.is_empty() {
                    return;
                }
                if logins.len() == 1 {
                    self.fill(id, &logins[0]);
                } else if !self.infobars.iter().any(|b| matches!(b, Infobar::ChooseLogin(_))) {
                    self.infobars.push(Infobar::ChooseLogin(origin));
                }
            }
            // Servo не показывает такой тип — скачать, а вкладку вернуть назад.
            Some("fetch") => {
                let Some(i) = self.tab_index(id) else { return };
                let Some(url) = self.tabs[i].real_url.clone() else { return };
                self.download(url.as_str(), false);
                let webview = self.tabs[i].webview.clone();
                if webview.can_go_back() {
                    webview.go_back(1);
                } else {
                    self.tabs[i].webview.load(url::Url::parse("obsos://newtab").expect("newtab"));
                }
            }
            // Ссылка `<a download>` на странице.
            Some("download") => {
                if let Some(url) = v.str_field("url") {
                    self.download(url, false);
                }
            }
            Some("unsupported") => {
                let what = v.str_field("what").unwrap_or("").to_string();
                let url = self.tab_index(id).and_then(|i| self.tabs[i].real_url.as_ref().map(|u| u.to_string())).unwrap_or_default();
                println!("browser: unsupported {} on {}", crate::logs::text(&what), crate::logs::url(&url));
                if !self.infobars.iter().any(|b| matches!(b, Infobar::Unsupported { .. })) {
                    self.infobars.push(Infobar::Unsupported { url, what });
                }
            }
            _ => {}
        }
    }

    pub fn fill(&mut self, id: WebViewId, login: &Login) {
        if let Some(i) = self.tab_index(id) {
            println!("browser: autofill {}", crate::logs::url(&login.origin));
            self.tabs[i].webview.evaluate_javascript(fill_script(&login.username, &login.password), |_| {});
        }
    }

    pub fn never_save(&mut self, origin: &str) -> bool {
        self.profile.replica_view_has_never(origin)
    }

    // ── Действия ────────────────────────────────────────────────────────

    /// Выполнить действие. `false` — закрыть окно.
    pub fn run(&mut self, action: Action, window: &Window) -> bool {
        match action {
            Action::NewTab => {
                let url = url::Url::parse("obsos://newtab").expect("newtab");
                self.new_tab(url, window);
                self.address.clear();
                self.focus_address = true;
            }
            Action::NewTabUrl(text) => {
                if let Some(url) = self.url_from_input(&text) {
                    self.new_tab(url, window);
                }
            }
            Action::Activate(i) => self.activate(i, window),
            Action::Close(i) => return self.close_tab(i, window),
            Action::Back => {
                if let Some(w) = self.active_view().filter(|w| w.can_go_back()) {
                    w.go_back(1);
                    println!("browser: back");
                }
            }
            Action::Forward => {
                if let Some(w) = self.active_view().filter(|w| w.can_go_forward()) {
                    w.go_forward(1);
                    println!("browser: forward");
                }
            }
            Action::Reload => {
                if let Some(w) = self.active_view() {
                    w.reload();
                    println!("browser: reload");
                }
            }
            Action::Go(text) => match self.url_from_input(&text) {
                Some(url) => self.load(url),
                None => println!("browser: go ignored (empty)"),
            },
            Action::OpenUrl(text) => {
                if let Ok(url) = url::Url::parse(&text) {
                    self.load(url);
                }
            }
            Action::FocusAddress => self.focus_address = true,
            Action::ToggleBookmark => {
                let Some(tab) = self.tabs.get(self.active) else { return true };
                let Some(url) = tab.real_url.as_ref().map(|u| u.to_string()) else { return true };
                if url.starts_with("obsos://newtab") {
                    return true;
                }
                let title = tab.title.clone();
                if self.profile.is_bookmarked(&url) {
                    self.profile.remove_bookmark(&url);
                    self.toast("Закладка удалена");
                } else {
                    self.profile.add_bookmark(&url, &title);
                    println!("browser: bookmark added {}", crate::logs::url(&url));
                    self.toast("Добавлено в закладки");
                }
                self.update_page_data();
            }
            Action::Chromium(url) => {
                let url = if url.is_empty() { self.tabs.get(self.active).and_then(|t| t.real_url.as_ref()).map(|u| u.to_string()).unwrap_or_default() } else { url };
                if !url.starts_with("http://") && !url.starts_with("https://") {
                    self.toast("Во внешнем браузере открываются только веб-страницы");
                } else if !window::chromium_installed() {
                    self.toast("Chromium не установлен — поставьте пакет chromium в «Пакетах»");
                } else {
                    match window::open_in_chromium(&url) {
                        Ok(task) if task > 0 => self.toast(format!("Открываю в Chromium (задача {task})")),
                        Ok(_) => self.toast("Открываю во внешнем браузере"),
                        Err(e) => self.toast(format!("Chromium не запустился: {e}")),
                    }
                }
            }
            Action::Zoom(factor) => {
                // «Сбросить» — к масштабу страниц из настроек.
                let usual = self.profile.settings.page_zoom as f32 / 100.0;
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.zoom = if factor == 0.0 { usual } else { (tab.zoom * factor).clamp(0.3, 3.0) };
                    tab.webview.set_page_zoom(tab.zoom);
                }
            }
            Action::Panel(panel) => {
                self.panel = if self.panel == Some(panel) { None } else { Some(panel) };
                if panel == Panel::DevTools {
                    self.dev.focus_input = true;
                }
                self.filter.clear();
                if panel == Panel::Sync && self.panel.is_some() {
                    self.load_points();
                }
            }
            Action::SyncNow(label) => self.sync_now(label),
            Action::Download(url) => self.download(&url, false),
            Action::SaveAs(url) => self.download(&url, true),
            Action::ChooseDownloadDir => self.choose_download_dir(window),
            Action::ToggleBlockSite => {
                let Some(host) = self.tabs.get(self.active).and_then(|t| t.real_url.as_ref()?.host_str().map(str::to_string)) else { return true };
                let allowed = self.delegate.blocker.borrow_mut().toggle_site(&host);
                self.toast(if allowed { "Блокировка рекламы на этом сайте выключена" } else { "Блокировка рекламы на этом сайте включена" });
                if let Some(tab) = self.tabs.get(self.active) {
                    tab.webview.reload();
                }
            }
            Action::Reader => {
                if let Some(w) = self.active_view() {
                    println!("browser: reader mode");
                    w.evaluate_javascript(pages::READER_SCRIPT, |_| {});
                }
            }
            Action::NewWindow => self.windows.wanted = true,
            Action::PrivateWindow => {
                window::new_window(&["--private"]);
            }
            Action::Reopen => {
                if let Some(url) = self.closed.pop() {
                    if let Ok(url) = url::Url::parse(&url) {
                        self.new_tab(url, window);
                    }
                }
            }
            Action::Home => {
                let home = self.home_url();
                self.load(home);
            }
            Action::FindOpen => {
                self.find.open = true;
                self.find.focus = true;
            }
            Action::Find { back } => self.find_step(back),
            Action::FindClose => {
                self.find.open = false;
                self.find.status.borrow_mut().clear();
                if let Some(w) = self.active_view() {
                    w.evaluate_javascript(find_script("", false), |_| {});
                }
            }
            Action::Quit => return false,
            Action::OpenPage(name) => {
                self.panel = None;
                let address = format!("obsos://{name}");
                let open = self.tabs.iter().position(|t| t.real_url.as_ref().is_some_and(|u| u.scheme() == "obsos" && u.host_str() == Some(name)));
                match (open, url::Url::parse(&address)) {
                    (Some(i), _) => self.activate(i, window),
                    (None, Ok(url)) => {
                        // Пустую «новую вкладку» занять, а не плодить вкладки.
                        let blank = self.tabs.get(self.active).is_some_and(|t| t.url.is_empty());
                        if blank {
                            self.load(url);
                        } else {
                            self.new_tab(url, window);
                        }
                    }
                    _ => {}
                }
                self.focus_address = false;
            }
            Action::OpenBackground(text) => {
                if let Some(url) = self.url_from_input(&text) {
                    let back = self.active;
                    self.new_tab(url, window);
                    self.activate(back, window);
                }
            }
            Action::Duplicate(i) => {
                if let Some(url) = self.tabs.get(i).and_then(|t| t.real_url.clone()) {
                    self.new_tab(url, window);
                }
            }
            Action::CloseOthers(keep) => {
                if keep < self.tabs.len() {
                    for i in (0..self.tabs.len()).rev() {
                        if i != keep {
                            let tab = self.tabs.remove(i);
                            self.closed.extend(tab.real_url.map(|u| u.to_string()));
                        }
                    }
                    println!("browser: other tabs closed");
                    self.activate(0, window);
                }
            }
            Action::ReloadTab(i) => {
                if let Some(tab) = self.tabs.get(i) {
                    tab.webview.reload();
                }
            }
            Action::MoveTab(from, to) => {
                if from < self.tabs.len() && to < self.tabs.len() && from != to {
                    let id = self.tabs[self.active].webview.id();
                    let tab = self.tabs.remove(from);
                    self.tabs.insert(to, tab);
                    self.active = self.tab_index(id).unwrap_or(0);
                }
            }
            Action::SetTabs(at) => {
                self.profile.settings.look.tabs = at;
                self.profile.save_settings();
                println!("browser: tabs {}", at.id());
            }
            Action::Fullscreen => {
                if self.page_fullscreen.is_some() {
                    self.leave_page_fullscreen();
                } else {
                    set_fullscreen(!fullscreen());
                    if fullscreen() {
                        self.panel = None;
                        self.toast("Полный экран — выход: Esc или F11");
                    }
                }
            }
            Action::Inspect => {
                self.panel = Some(Panel::DevTools);
                self.dev.pick = true;
            }
            Action::InspectAt(x, y) => {
                self.panel = Some(Panel::DevTools);
                // Точка интерфейса → CSS-пиксели страницы.
                let (ox, oy) = page_origin(self.look());
                let zoom = self.tabs.get(self.active).map(|t| t.zoom).unwrap_or(1.0);
                let k = ui_scale();
                let css = page_scale() * zoom;
                crate::devtools::inspect_at(self, (x * k - ox as f32) / css, (y * k - oy as f32) / css);
            }
            Action::Console => {
                self.panel = Some(Panel::DevTools);
                self.dev.tab = crate::devtools::DevTab::Console;
                self.dev.focus_input = true;
            }
            Action::Toast(text) => self.toast(text),
        }
        true
    }

    /// Куки Servo для адреса (`Cookie:` загрузки — файлы за входом на сайт).
    pub fn cookie_for(&self, url: &url::Url) -> Option<String> {
        let cookies = self.servo.site_data_manager().cookies_for_url(url.clone(), servo::CookieSource::HTTP);
        let text = cookies.iter().map(|c| format!("{}={}", c.name(), c.value())).collect::<Vec<_>>().join("; ");
        (!text.is_empty()).then_some(text)
    }

    pub fn download(&mut self, url: &str, ask: bool) {
        let Ok(parsed) = url::Url::parse(url) else { return };
        if !matches!(parsed.scheme(), "http" | "https") {
            self.toast("Скачать можно только адрес http(s)");
            return;
        }
        let referer = self.tabs.get(self.active).and_then(|t| t.real_url.as_ref()).map(|u| u.to_string());
        let request = crate::downloads::Request { url: url.to_string(), cookie: self.cookie_for(&parsed), referer, ask, name: None };
        self.delegate.downloads.start(request);
        self.dl.bar = true;
    }

    /// Папка загрузок из настроек (пусто — `~/Downloads`).
    pub fn apply_download_settings(&self) {
        let s = &self.profile.settings;
        let dir = if s.download_dir.trim().is_empty() { crate::downloads::default_dir() } else { std::path::PathBuf::from(s.download_dir.trim()) };
        *self.delegate.downloads.dir.borrow_mut() = dir;
        self.delegate.downloads.ask_always.set(s.download_ask);
    }

    /// Выбрать папку загрузок окном системы.
    pub fn choose_download_dir(&mut self, window: &Window) {
        if self.dl.dialog.is_some() {
            return;
        }
        let dir = self.delegate.downloads.dir.borrow().display().to_string();
        match window::platform().folder_dialog(window, "Папка для загрузок", &dir) {
            Ok(d) => self.dl.dialog = Some((DialogFor::Folder, d)),
            Err(e) => self.toast(format!("Окно выбора папки не открылось: {e}")),
        }
    }

    /// Загрузки на каждом витке цикла: проверки ссылок вкладок, окно
    /// «Сохранить как», уведомления о конце. `true` — оболочку перерисовать.
    pub fn poll_downloads(&mut self, window: &Window) -> bool {
        let mut changed = false;
        let downloads = self.delegate.downloads.clone();
        // Новые документы вкладок — проверить ссылку.
        let held = std::mem::take(&mut *self.delegate.held_loads.borrow_mut());
        for (webview, load) in held {
            let url = load.request().url.as_url().clone();
            let referer = load.request().referrer_url.as_ref().map(|u| u.as_str().to_string());
            let id = downloads.probe(url.as_str(), self.cookie_for(&url), referer);
            self.dl.probes.push((id, webview, load, Instant::now(), url));
        }
        // Решённые проверки: файл — документ отменить (качает проверка),
        // страница — отпустить к Servo. Не решилась за 6 с — тоже отпустить:
        // медленный сайт не должен висеть из-за проверки.
        let mut i = 0;
        while i < self.dl.probes.len() {
            let (id, _, _, at, _) = &self.dl.probes[i];
            let (id, at) = (*id, *at);
            let decided = downloads.probe_is_file(id);
            let late = decided.is_none() && at.elapsed() > Duration::from_secs(6);
            if decided.is_none() && !late {
                i += 1;
                continue;
            }
            let (id, webview, load, _, url) = self.dl.probes.remove(i);
            if decided == Some(true) {
                println!("browser: document {} is a download", crate::logs::url(url.as_str()));
                // Отменённый документ (`cancel`) оставлял навигацию Servo
                // висеть — следующие переходы вкладки не шли вовсе (20.11).
                // Поэтому ответ — короткая страница «файл скачивается», а
                // вкладка сразу уходит назад (`leave`, `Note::Url`).
                let mut headers = http::HeaderMap::new();
                headers.insert(http::header::CONTENT_TYPE, http::HeaderValue::from_static("text/html; charset=utf-8"));
                let mut answer = load.intercept(servo::WebResourceResponse::new(url.clone()).headers(headers));
                answer.send_body_data(DOWNLOAD_PAGE.as_bytes().to_vec());
                answer.finish();
                self.dl.leave.push((webview, url, Instant::now()));
                self.dl.bar = true;
                changed = true;
            } else {
                if late {
                    println!("browser: probe {} too slow — page", crate::logs::url(url.as_str()));
                    downloads.drop_probe(id);
                }
                drop(load);
            }
        }
        self.dl.leave.retain(|(_, _, at)| at.elapsed() < Duration::from_secs(20));
        for event in downloads.poll(&mut self.dl.asked) {
            match event {
                crate::downloads::Event::Ask { id, name } => self.dl.queue.push((id, name)),
                crate::downloads::Event::Finished { name, ok } => {
                    if ok {
                        // Открытый список загрузок говорит о том же сам.
                        if self.panel != Some(Panel::Downloads) && !self.chrome.popup_due() {
                            self.toast(format!("Загружено: {name}"));
                        }
                    }
                    changed = true;
                }
            }
        }
        // Окно «Сохранить как»: по одному.
        if self.dl.dialog.is_none() && !self.dl.queue.is_empty() {
            let (id, name) = self.dl.queue.remove(0);
            let dir = downloads.dir.borrow().clone();
            let _ = std::fs::create_dir_all(&dir);
            match window::platform().save_dialog(window, "Сохранить как", &dir.display().to_string(), &name) {
                Ok(d) => self.dl.dialog = Some((DialogFor::Save(id), d)),
                Err(e) => {
                    // Окна нет (старая система) — в папку загрузок.
                    println!("browser: save dialog failed: {e}");
                    downloads.answer(id, Some(crate::net::unique_path(&dir, &name)));
                }
            }
        }
        let answer = self.dl.dialog.as_mut().and_then(|(_, dialog)| dialog.poll());
        if let Some(answer) = answer {
            if let Some((purpose, _)) = self.dl.dialog.take() {
                let chosen = match answer {
                    window::DialogAnswer::Chosen(path) => Some(std::path::PathBuf::from(path)),
                    window::DialogAnswer::Cancelled => None,
                };
                match purpose {
                    DialogFor::Save(id) => {
                        println!("browser: save as {}", crate::logs::text(&chosen.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| String::from("cancelled"))));
                        downloads.answer(id, chosen);
                    }
                    DialogFor::Folder => {
                        if let Some(dir) = chosen {
                            println!("browser: download folder {}", dir.display());
                            self.profile.settings.download_dir = dir.display().to_string();
                            self.profile.save_settings();
                            self.apply_download_settings();
                        }
                    }
                }
                changed = true;
            }
        }
        changed
    }

    fn find_step(&mut self, back: bool) {
        let Some(w) = self.active_view() else { return };
        let status = self.find.status.clone();
        w.evaluate_javascript(find_script(&self.find.text, back), move |result| {
            let text = match result {
                Ok(servo::JSValue::String(t)) => t,
                _ => String::from("?"),
            };
            println!("browser: find {text}");
            *status.borrow_mut() = text;
        });
    }

    // ── Синхронизация ───────────────────────────────────────────────────

    pub fn account(&self) -> Option<Account> {
        self.vault.as_ref()?.account().cloned()
    }

    pub fn sync_now(&mut self, label: Option<String>) {
        let Some(account) = self.account() else {
            self.sync.status = String::from("Синхронизация не настроена");
            return;
        };
        let names: Vec<(String, String)> = self.tabs.iter().filter_map(|t| Some((t.real_url.as_ref()?.to_string(), t.title.clone()))).collect();
        self.profile.publish_tabs(&names);
        self.sync.status = String::from("Синхронизация…");
        println!("browser: sync start ({})", account.provider.title());
        self.syncer.send(Job::Sync {
            account,
            device: self.profile.settings.device.clone(),
            dir: self.profile.dir.clone(),
            point: label,
        });
    }

    pub fn load_points(&mut self) {
        if let Some(account) = self.account() {
            self.syncer.send(Job::Points { account });
        }
    }

    pub fn rollback(&mut self, point: String) {
        if let Some(account) = self.account() {
            self.sync.status = format!("Читаю точку {point}…");
            self.syncer.send(Job::FetchPoint { account, point });
        }
    }

    pub fn login_start(&mut self) {
        let account = self.sync.draft.clone();
        self.sync.status = String::from("Начинаю вход через браузер…");
        self.syncer.send(Job::LoginStart { account });
    }

    pub fn check_account(&mut self) {
        let account = self.sync.draft.clone();
        if let Err(e) = account.check() {
            self.sync.status = format!("Не хватает: {e}");
            return;
        }
        self.sync.status = String::from("Проверяю…");
        self.syncer.send(Job::Check { account });
    }

    /// Ответы потока синхронизации; плюс автосинхронизация и опрос входа.
    pub fn poll_sync(&mut self, window: &Window) -> bool {
        let mut changed = false;
        if let Some(flow) = &self.sync.flow {
            if Instant::now() >= flow.next_poll && self.syncer.busy == 0 {
                let device_code = flow.code.device_code.clone();
                if Instant::now() > flow.until {
                    self.sync.flow = None;
                    self.sync.status = String::from("Код входа истёк — начните заново");
                } else {
                    let account = self.sync.draft.clone();
                    self.syncer.send(Job::LoginPoll { account, device_code });
                    if let Some(f) = self.sync.flow.as_mut() {
                        f.next_poll = Instant::now() + f.interval;
                    }
                }
            }
        }
        let auto = self.profile.settings.sync_auto && self.account().is_some();
        if auto && Instant::now() >= self.sync.next_auto && self.syncer.busy == 0 {
            self.sync.next_auto = Instant::now() + Duration::from_secs(self.profile.settings.sync_every_min as u64 * 60);
            let label = self.profile.settings.sync_points_auto.then(String::new);
            self.sync_now(label);
        }
        while let Some(done) = self.syncer.poll() {
            changed = true;
            match done {
                Done::Synced(result, refreshed) => {
                    if let (Some(updated), Some(vault)) = (refreshed, self.vault.as_mut()) {
                        if let Some(i) = vault.active {
                            vault.accounts[i] = updated;
                            vault.save();
                        }
                    }
                    match result {
                        Ok(report) => {
                            self.profile.reload();
                            if let Some(v) = &self.vault {
                                let key = v.sync_key;
                                self.profile.reload_logins(&key);
                            }
                            self.update_page_data();
                            let point = report.point.as_ref().map(|p| format!(", точка {p}")).unwrap_or_default();
                            self.sync.last_report = format!(
                                "↑ {} ↓ {} · устройств: {}{point}",
                                report.pushed, report.pulled, report.devices
                            );
                            self.sync.status = format!("Синхронизировано: {}", self.sync.last_report);
                            println!("browser: sync ok pushed {} pulled {} devices {}{point}", report.pushed, report.pulled, report.devices);
                            if report.point.is_some() {
                                self.toast(format!("Точка синхронизации создана{point}"));
                                self.load_points();
                            }
                        }
                        Err(e) => {
                            self.sync.status = format!("Ошибка: {e}");
                            println!("browser: sync failed: {e}");
                        }
                    }
                }
                Done::Points(result) => match result {
                    Ok(points) => self.sync.points = points,
                    Err(e) => self.sync.status = format!("Точки: {e}"),
                },
                Done::Point(point, result) => match result {
                    Ok(then) => {
                        match browser_sync::sync::apply_rollback(&mut self.profile.replica, then, profile::SYNCED, now_ms()) {
                            Ok(r) => {
                                self.profile.reload();
                                if let Some(v) = &self.vault {
                                    let key = v.sync_key;
                                    self.profile.reload_logins(&key);
                                }
                                self.update_page_data();
                                self.sync.status = format!("Откат к {point}: возвращено {}, убрано {}", r.restored, r.removed);
                                println!("browser: rollback {point}: restored {} removed {}", r.restored, r.removed);
                                self.sync_now(None);
                            }
                            Err(e) => self.sync.status = format!("Откат: {e}"),
                        }
                    }
                    Err(e) => self.sync.status = format!("Откат: {e}"),
                },
                Done::LoginStarted(result) => match result {
                    Ok(code) => {
                        let interval = Duration::from_secs(code.interval.max(1));
                        self.sync.status = format!("Введите код {} на открывшейся странице", code.user_code);
                        if let Ok(url) = url::Url::parse(&code.verification_uri) {
                            self.new_tab(url, window);
                        }
                        self.sync.flow = Some(LoginFlow {
                            until: Instant::now() + Duration::from_secs(code.expires_in.max(60)),
                            next_poll: Instant::now() + interval,
                            interval,
                            code,
                        });
                    }
                    Err(e) => self.sync.status = format!("Вход: {e}"),
                },
                Done::LoginPolled(result) => match result {
                    Ok(oauth::Poll::Pending) => {}
                    Ok(oauth::Poll::SlowDown) => {
                        if let Some(f) = self.sync.flow.as_mut() {
                            f.interval += Duration::from_secs(5);
                        }
                    }
                    Ok(oauth::Poll::Done(tokens)) => {
                        self.sync.flow = None;
                        oauth::apply(&mut self.sync.draft, &tokens, now_ms());
                        self.sync.status = String::from("Вход выполнен — нажмите «Сохранить и включить»");
                    }
                    Ok(oauth::Poll::Failed(why)) => {
                        self.sync.flow = None;
                        self.sync.status = format!("Вход не удался: {why}");
                    }
                    Err(e) => self.sync.status = format!("Вход: {e}"),
                },
                Done::Checked(result) => match result {
                    Ok(()) => {
                        let draft = self.sync.draft.clone();
                        if let Some(vault) = self.vault.as_mut() {
                            match vault.accounts.iter().position(|a| a.provider == draft.provider) {
                                Some(i) => vault.accounts[i] = draft,
                                None => vault.accounts.push(draft),
                            }
                            vault.active = vault.accounts.iter().position(|a| a.provider == self.sync.draft.provider);
                            vault.save();
                        }
                        self.sync.status = String::from("Площадка готова, синхронизация включена");
                        println!("browser: sync account saved ({})", self.sync.draft.provider.title());
                        self.sync_now(None);
                    }
                    Err(e) => self.sync.status = format!("Проверка: {e}"),
                },
            }
        }
        changed
    }

    /// Выключить синхронизацию (учётная запись остаётся сохранённой).
    pub fn sync_off(&mut self) {
        if let Some(vault) = self.vault.as_mut() {
            vault.active = None;
            vault.save();
        }
        self.sync.status = String::from("Синхронизация выключена");
    }

    pub fn select_provider(&mut self, provider: Provider) {
        let saved = self.vault.as_ref().and_then(|v| v.accounts.iter().find(|a| a.provider == provider).cloned());
        self.sync.draft = saved.unwrap_or_else(|| Account::new(provider));
    }

    pub fn set_sync_password(&mut self) {
        let password = std::mem::take(&mut self.sync.sync_password);
        let Some(vault) = self.vault.as_mut() else { return };
        let old = vault.sync_key;
        vault.set_sync_password(&password);
        let new = vault.sync_key;
        self.profile.rekey_logins(&old, &new);
        self.toast(if password.is_empty() { "Пароли больше не расшифровываются на других устройствах" } else { "Пароль синхронизации задан — пароли перешифрованы" });
    }

    pub fn save_login(&mut self, login: Login) {
        if let Some(v) = &self.vault {
            let key = v.sync_key;
            self.profile.save_login(&key, login);
            self.toast("Пароль сохранён");
        }
    }

    /// Открыть хранилище мастер-паролем (окно спрашивает при запуске).
    pub fn unlock(&mut self, master: &str) {
        match Vault::open(&self.profile.dir, Some(master)) {
            Ok(vault) => {
                let key = vault.sync_key;
                self.profile.reload_logins(&key);
                self.vault = Some(vault);
                self.modal = None;
                println!("browser: vault unlocked");
            }
            Err(_) => self.modal = Some(Modal::Master { text: String::new(), error: String::from("Пароль не подошёл") }),
        }
    }

    /// Адреса вкладок всех окон, начиная с текущего (сеанс восстанавливается
    /// в одно окно).
    pub fn session_urls(&self) -> Vec<String> {
        self.all_tabs().filter_map(|t| t.real_url.as_ref().map(|u| u.to_string())).filter(|u| !u.starts_with("data:")).collect()
    }
}

impl Profile {
    /// Сайт в списке «никогда не сохранять пароль» (общие настройки).
    pub fn replica_view_has_never(&mut self, origin: &str) -> bool {
        self.replica.load(profile::SHARED).map(|v| v.state.get(&format!("never/{origin}")).is_some()).unwrap_or(false)
    }
}
