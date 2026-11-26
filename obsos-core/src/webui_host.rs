//! Оболочка со стороны страниц `obsos://settings|history|bookmarks|downloads`
//! (`webui.rs`): отдаёт им состояние и выполняет их команды.

use std::time::{Duration, Instant};

use browser_sync::json::{self, Value};
use servo::WebViewId;

use crate::downloads::{State, human_size, human_time};
use crate::profile::{Look, Rgb, Search, TabsAt, now_ms};
use crate::shell::{Action, Modal, Panel, Shell};
use crate::webui;
use crate::window::{self, Window};

/// Что уже отдано открытым страницам и когда проверять снова.
pub struct WebUi {
    sent: std::collections::HashMap<WebViewId, String>,
    next: Instant,
    /// Итог последнего обновления списков фильтров.
    pub filters_note: String,
    /// Что ищут на `obsos://history` (по всей истории, а не по присланному).
    history_query: String,
}

impl Default for WebUi {
    fn default() -> Self {
        WebUi { sent: Default::default(), next: Instant::now(), filters_note: String::new(), history_query: String::new() }
    }
}

fn s(text: impl Into<String>) -> Value {
    Value::Str(text.into())
}

fn b(v: bool) -> Value {
    Value::Bool(v)
}

fn n(v: impl std::fmt::Display) -> Value {
    Value::Num(v.to_string())
}

fn obj(fields: Vec<(&str, Value)>) -> Value {
    Value::Obj(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Rgb(m(a.0, b.0), m(a.1, b.1), m(a.2, b.2))
}

/// Цвета страниц управления из оформления оболочки: (имя переменной, цвет).
fn theme(look: &Look) -> Vec<(&'static str, String)> {
    let white = Rgb(255, 255, 255);
    let black = Rgb(0, 0, 0);
    let (card, field) = if look.dark { (mix(look.toolbar, look.text, 0.045), mix(look.toolbar, black, 0.3)) } else { (mix(look.toolbar, white, 0.7), white) };
    vec![
        ("bg", look.sidebar.hex()),
        ("panel", look.toolbar.hex()),
        ("card", card.hex()),
        ("border", mix(look.toolbar, look.text, 0.13).hex()),
        ("text", look.text.hex()),
        ("dim", mix(look.text, look.sidebar, 0.38).hex()),
        ("accent", look.accent.hex()),
        ("accent2", look.accent2.hex()),
        ("field", field.hex()),
    ]
}

/// Тот же набор строкой CSS — вставляется в HTML, чтобы страница не мигала
/// цветами по умолчанию до прихода состояния.
pub fn theme_css(look: &Look) -> String {
    let vars: String = theme(look).into_iter().map(|(k, v)| format!("--{k}:{v};")).collect();
    format!(":root{{{vars}}}")
}

fn host_of(url: &str) -> String {
    url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default()
}

/// Местные дата и время записи: («Сегодня»/«Вчера»/дата, «14:05»).
fn day_time(ms: u64) -> (String, String) {
    const MONTHS: [&str; 12] = ["января", "февраля", "марта", "апреля", "мая", "июня", "июля", "августа", "сентября", "октября", "ноября", "декабря"];
    let offset = window::platform().utc_offset_minutes() as i64 * 60;
    let local = |ms: u64| ((ms / 1000) as i64 + offset).max(0) as u64;
    let (y, mo, d, h, mi, _) = browser_sync::remote::civil(local(ms));
    let day_number = local(ms) / 86_400;
    let today = local(now_ms()) / 86_400;
    let day = if day_number == today {
        String::from("Сегодня")
    } else if day_number + 1 == today {
        String::from("Вчера")
    } else {
        format!("{d} {} {y}", MONTHS.get((mo as usize).wrapping_sub(1)).copied().unwrap_or(""))
    };
    (day, format!("{h:02}:{mi:02}"))
}

impl Shell {
    /// Состояние для страницы управления `host`.
    fn page_state(&mut self, host: &str) -> Value {
        let theme = obj(theme(self.look()).into_iter().map(|(k, v)| (k, s(v))).collect());
        match host {
            "history" => {
                let query = self.webui.history_query.to_lowercase();
                let items = self
                    .profile
                    .history
                    .iter()
                    .filter(|v| query.is_empty() || v.url.to_lowercase().contains(&query) || v.title.to_lowercase().contains(&query))
                    .take(2000)
                    .map(|v| {
                        let (day, time) = day_time(v.last_ms);
                        obj(vec![("url", s(&v.url)), ("title", s(&v.title)), ("host", s(host_of(&v.url))), ("day", s(day)), ("time", s(time))])
                    })
                    .collect();
                obj(vec![("theme", theme), ("items", Value::Arr(items))])
            }
            "bookmarks" => {
                let items = self.profile.bookmarks.iter().map(|m| obj(vec![("url", s(&m.url)), ("title", s(&m.title)), ("host", s(host_of(&m.url)))])).collect();
                obj(vec![("theme", theme), ("items", Value::Arr(items))])
            }
            "downloads" => {
                let items = self
                    .delegate
                    .downloads
                    .snapshot()
                    .iter()
                    .rev()
                    .map(|d| {
                        let percent = d.fraction().map(|f| (f * 100.0) as i64).unwrap_or(-1);
                        let (state, status) = match &d.state {
                            State::Running => {
                                let mut parts = vec![match d.total {
                                    Some(t) => format!("{} из {}", human_size(d.got), human_size(t)),
                                    None => human_size(d.got),
                                }];
                                if d.rate > 0.0 {
                                    parts.push(format!("{}/с", human_size(d.rate as u64)));
                                }
                                if let Some(eta) = d.eta() {
                                    parts.push(format!("ещё {}", human_time(eta)));
                                }
                                ("run", parts.join(" · "))
                            }
                            State::Asking => ("run", String::from("куда сохранить?")),
                            State::Probing | State::Page => ("run", String::from("проверка ссылки…")),
                            State::Done(path) => ("done", format!("готово · {} · {}", human_size(d.got), path.display())),
                            State::Failed(e) => ("fail", format!("ошибка: {e}")),
                            State::Cancelled => ("fail", String::from("отменено")),
                        };
                        obj(vec![("id", n(d.id)), ("title", s(&d.name)), ("url", s(&d.url)), ("state", s(state)), ("status", s(status)), ("percent", n(percent))])
                    })
                    .collect();
                let dir = self.delegate.downloads.dir.borrow().display().to_string();
                obj(vec![("theme", theme), ("items", Value::Arr(items)), ("dir", s(dir))])
            }
            _ => self.settings_state(theme),
        }
    }

    fn settings_state(&mut self, theme: Value) -> Value {
        let settings = &self.profile.settings;
        let look = &settings.look;
        let look_value = |l: &Look| {
            obj(vec![
                ("name", s(&l.name)),
                ("dark", b(l.dark)),
                ("tabs", s(l.tabs.id())),
                ("sidebar", s(l.sidebar.hex())),
                ("toolbar", s(l.toolbar.hex())),
                ("accent", s(l.accent.hex())),
                ("accent2", s(l.accent2.hex())),
                ("text", s(l.text.hex())),
                ("sidebar_width", n(l.sidebar_width)),
                ("accent_reach", s(l.accent_reach.to_string())),
                ("compact", b(l.compact)),
            ])
        };
        let (rules, blocked) = {
            let blocker = self.delegate.blocker.borrow();
            (blocker.filter_rules, blocker.blocked_total)
        };
        let filters = if self.webui.filters_note.is_empty() {
            format!("EasyList, EasyPrivacy, RuAdList: правил {rules} · заблокировано за сеанс: {blocked}")
        } else {
            self.webui.filters_note.clone()
        };
        let account = self.account().map(|a| format!("{} · {}", a.provider.title(), a.repo)).unwrap_or_default();
        let dir = self.delegate.downloads.dir.borrow().display().to_string();
        obj(vec![
            ("theme", theme),
            ("look", look_value(look)),
            ("looks", Value::Arr(settings.looks.iter().map(look_value).collect())),
            ("ui_scale", s(settings.ui_scale.to_string())),
            ("page_zoom", s(settings.page_zoom.to_string())),
            ("search", s(settings.search.id())),
            ("engines", Value::Arr(Search::ALL.iter().map(|e| Value::Arr(vec![s(e.id()), s(e.title())])).collect())),
            ("home", s(&settings.home)),
            ("restore_session", b(settings.restore_session)),
            ("smooth_scroll", b(settings.smooth_scroll)),
            ("gpu", b(settings.gpu)),
            ("gpu_available", b(self.gpu_available)),
            ("gpu_active", b(self.gpu_active)),
            ("renderer", s(&self.renderer_name)),
            ("block_ads", b(settings.block_ads)),
            ("filters", s(filters)),
            ("filters_busy", b(self.tools.filters_busy)),
            ("save_passwords", b(settings.save_passwords)),
            ("autofill", b(settings.autofill)),
            ("has_master", b(self.vault.as_ref().is_some_and(|v| v.has_master_password()))),
            ("history_days", n(settings.history_days)),
            ("download_dir", s(dir)),
            ("download_dir_custom", b(!settings.download_dir.is_empty())),
            ("download_ask", b(settings.download_ask)),
            ("sync_account", s(account)),
            ("sync_status", s(&self.sync.status)),
            ("sync_auto", b(settings.sync_auto)),
            ("sync_every_min", n(settings.sync_every_min)),
            ("sync_points_auto", b(settings.sync_points_auto)),
            ("device", s(&settings.device)),
            ("device_name", s(&settings.device_name)),
            ("profile", s(&self.profile.name)),
            ("profiles", Value::Arr(crate::profile::Profile::list().into_iter().map(s).collect())),
            ("scripts_dir", s(self.profile.dir.join("userscripts").display().to_string())),
            ("profile_dir", s(self.profile.dir.display().to_string())),
            ("version", s(env!("CARGO_PKG_VERSION"))),
        ])
    }

    /// Отдать состояние вкладке, если оно изменилось.
    fn push_state(&mut self, index: usize) {
        let Some(tab) = self.tabs.get(index) else { return };
        let webview = tab.webview.clone();
        let Some(url) = webview.url().filter(webui::is_control_page) else {
            self.webui.sent.remove(&webview.id());
            return;
        };
        let host = url.host_str().unwrap_or("").to_string();
        let text = self.page_state(&host).to_text();
        if self.webui.sent.get(&webview.id()) == Some(&text) {
            return;
        }
        // Проверка и внутри самой страницы: вкладка могла уже перейти на сайт,
        // а `webview.url()` об этом ещё не узнать.
        webview.evaluate_javascript(format!("location.protocol==='obsos:'&&window.obsos&&window.obsos.state({text})"), |_| {});
        self.webui.sent.insert(webview.id(), text);
    }

    /// Виток главного цикла: обновления списков и открытая страница
    /// управления (прогресс загрузок, итог синхронизации).
    pub fn poll_webui(&mut self) -> bool {
        let mut changed = false;
        let done = self.tools.filters.lock().ok().and_then(|mut f| f.take());
        if let Some(result) = done {
            self.tools.filters_busy = false;
            let note = match result {
                Ok(text) => format!("Списки обновлены: {} доменов", self.delegate.blocker.borrow_mut().store_filters(&text)),
                Err(e) => format!("Списки не скачались: {e}"),
            };
            self.webui.filters_note = note.clone();
            self.toast(note);
            changed = true;
        }
        if Instant::now() >= self.webui.next {
            self.webui.next = Instant::now() + Duration::from_millis(400);
            self.push_state(self.active);
        }
        changed
    }

    /// Скачать списки фильтров в фоне (ответ заберёт `poll_webui`).
    pub fn update_filters(&mut self) {
        if self.tools.filters_busy {
            return;
        }
        self.tools.filters_busy = true;
        self.webui.filters_note.clear();
        let slot = self.tools.filters.clone();
        let _ = std::thread::Builder::new().name(String::from("obsos-filters")).spawn(move || {
            use browser_sync::remote::{Http, Request};
            let mut all = String::new();
            let mut errors = Vec::new();
            for url in [
                "https://easylist.to/easylist/easylist.txt",
                "https://easylist.to/easylist/easyprivacy.txt",
                "https://easylist-downloads.adblockplus.org/advblock.txt",
            ] {
                match crate::net::Client.send(&Request::new("GET", url)) {
                    Ok(r) if r.status == 200 => all.push_str(&String::from_utf8_lossy(&r.body)),
                    Ok(r) => errors.push(format!("{url}: {}", r.status)),
                    Err(e) => errors.push(format!("{url}: {e}")),
                }
            }
            let result = if all.is_empty() { Err(errors.join("; ")) } else { Ok(all) };
            if let Ok(mut f) = slot.lock() {
                *f = Some(result);
            }
        });
    }

    /// Сообщение страницы управления.
    pub fn ui_message(&mut self, id: WebViewId, payload: &str, window: &Window) {
        let Some(index) = self.tabs.iter().position(|t| t.webview.id() == id) else { return };
        // Только настоящая страница управления с ключом этого запуска.
        let Some(url) = self.tabs[index].webview.url().filter(webui::is_control_page) else { return };
        let Ok(v) = json::parse(payload) else { return };
        if v.str_field("token") != Some(self.ui_token.as_str()) {
            println!("browser: webui bad token from {}", crate::logs::url(url.as_str()));
            return;
        }
        let field = |k: &str| v.str_field(k).unwrap_or("").to_string();
        match v.str_field("cmd") {
            Some("ready") => {
                self.webui.sent.remove(&id);
                // Только что открытая страница — поиск пуст.
                self.webui.history_query.clear();
            }
            Some("set") => {
                let (key, value) = (field("key"), field("value"));
                println!("browser: setting {key} = {}", crate::logs::scrub(&value));
                self.set_setting(&key, &value);
            }
            Some("act") => {
                let (name, arg) = (field("name"), field("arg"));
                println!("browser: webui {name}");
                self.ui_action(&name, &arg, window);
            }
            _ => return,
        }
        if let Some(i) = self.tabs.iter().position(|t| t.webview.id() == id) {
            self.push_state(i);
        }
    }

    pub fn set_setting(&mut self, key: &str, value: &str) {
        let on = value == "true";
        let number = value.trim().parse::<u32>().ok();
        let mut shared: Option<(&str, String)> = None;
        let s = &mut self.profile.settings;
        match key {
            "search" => {
                s.search = Search::from_id(value);
                shared = Some(("search", s.search.id().to_string()));
            }
            "home" => {
                s.home = if value.trim().is_empty() { String::from("obsos://newtab") } else { value.trim().to_string() };
                shared = Some(("home", s.home.clone()));
            }
            "restore_session" => s.restore_session = on,
            "smooth_scroll" => s.smooth_scroll = on,
            "gpu" => s.gpu = on,
            "block_ads" => {
                s.block_ads = on;
                self.delegate.blocker.borrow_mut().enabled = on;
            }
            "save_passwords" => s.save_passwords = on,
            "autofill" => s.autofill = on,
            "history_days" => s.history_days = number.map(|v| v.clamp(1, 3650)).unwrap_or(s.history_days),
            "download_dir" => s.download_dir = value.to_string(),
            "download_ask" => s.download_ask = on,
            "sync_auto" => s.sync_auto = on,
            "sync_every_min" => s.sync_every_min = number.map(|v| v.clamp(1, 24 * 60)).unwrap_or(s.sync_every_min),
            "sync_points_auto" => s.sync_points_auto = on,
            "device_name" if !value.trim().is_empty() => s.device_name = value.trim().to_string(),
            "tabs" => s.look.tabs = TabsAt::from_id(value).unwrap_or(s.look.tabs),
            "sidebar_width" => s.look.sidebar_width = number.map(|v| v.clamp(160, 420)).unwrap_or(s.look.sidebar_width),
            "sidebar" => s.look.sidebar = Rgb::parse(value).unwrap_or(s.look.sidebar),
            "toolbar" => s.look.toolbar = Rgb::parse(value).unwrap_or(s.look.toolbar),
            "text" => s.look.text = Rgb::parse(value).unwrap_or(s.look.text),
            // Тёмная или светлая основа: цвета заменяются, раскладка — нет.
            "base" if (value == "dark") != s.look.dark => {
                let base = if value == "dark" { Look::dark() } else { Look::light() };
                s.look = Look {
                    name: s.look.name.clone(),
                    tabs: s.look.tabs,
                    sidebar_width: s.look.sidebar_width,
                    compact: s.look.compact,
                    accent_reach: s.look.accent_reach,
                    ..base
                };
            }
            "accent" => s.look.accent = Rgb::parse(value).unwrap_or(s.look.accent),
            "accent2" => s.look.accent2 = Rgb::parse(value).unwrap_or(s.look.accent2),
            "accent_reach" => s.look.accent_reach = number.map(|v| v.min(2) as u8).unwrap_or(s.look.accent_reach),
            "compact" => s.look.compact = on,
            "ui_scale" => {
                s.ui_scale = match number {
                    Some(0) | None => 0,
                    Some(v) => v.clamp(75, 300),
                }
            }
            "page_zoom" => {
                s.page_zoom = number.map(|v| v.clamp(30, 300)).unwrap_or(100);
                let zoom = s.page_zoom as f32 / 100.0;
                for tab in &mut self.tabs {
                    tab.zoom = zoom;
                    tab.webview.set_page_zoom(zoom);
                }
            }
            _ => return,
        }
        self.profile.save_settings();
        if let Some((key, value)) = shared {
            self.profile.put_shared(key, &value);
        }
        self.apply_download_settings();
        self.update_page_data();
    }

    fn ui_action(&mut self, name: &str, arg: &str, window: &Window) {
        let id: Option<u64> = arg.parse().ok();
        let download_path = |shell: &Shell| {
            shell.delegate.downloads.snapshot().into_iter().find(|d| Some(d.id) == id).and_then(|d| match d.state {
                State::Done(path) => Some(path),
                _ => None,
            })
        };
        match name {
            "look" => {
                if let Some(mut look) = self.profile.settings.looks.iter().find(|l| l.name == arg).cloned() {
                    // Тема — это цвета; раскладка вкладок остаётся своей.
                    let now = &self.profile.settings.look;
                    look.tabs = now.tabs;
                    look.sidebar_width = now.sidebar_width;
                    look.compact = now.compact;
                    self.profile.settings.look = look;
                    self.profile.save_settings();
                    self.update_page_data();
                }
            }
            "save_look" if !arg.trim().is_empty() => {
                let mut look = self.profile.settings.look.clone();
                look.name = arg.trim().to_string();
                self.profile.save_look(&look);
                self.profile.settings.look = look;
                self.profile.save_settings();
                self.toast(format!("Тема «{}» сохранена", arg.trim()));
            }
            "delete_look" if self.profile.settings.looks.len() > 1 => {
                self.profile.delete_look(arg);
                self.toast(format!("Тема «{arg}» удалена"));
            }
            "search" => self.webui.history_query = arg.to_string(),
            "update_filters" => self.update_filters(),
            "set_master" => {
                if let Some(vault) = self.vault.as_mut() {
                    vault.set_master(arg);
                    self.toast(if arg.is_empty() { "Мастер-пароль снят" } else { "Мастер-пароль сохранён" });
                }
            }
            "clear_history" => self.modal = Some(Modal::Rollback(String::from("\u{0}history"))),
            "choose_dir" => self.choose_download_dir(window),
            "panel" => {
                self.panel = match arg {
                    "passwords" => Some(Panel::Passwords),
                    "sync" => Some(Panel::Sync),
                    "scripts" => Some(Panel::Scripts),
                    _ => None,
                };
                if arg == "sync" {
                    self.load_points();
                }
            }
            "sync_now" => self.sync_now(None),
            "profile" => {
                let name: String = arg.chars().filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_').collect();
                if !name.is_empty() {
                    window::new_window(&["--profile", &name]);
                }
            }
            "private" => window::new_window(&["--private"]),
            "scripts_dir" => {
                let dir = self.profile.dir.join("userscripts");
                let _ = std::fs::create_dir_all(&dir);
                window::show_in_folder(&dir);
            }
            "profile_dir" => window::show_in_folder(&self.profile.dir.clone()),
            "forget" => self.profile.forget(arg),
            "unbookmark" => {
                self.profile.remove_bookmark(arg);
                self.update_page_data();
            }
            "open" => {
                self.run(Action::OpenUrl(arg.to_string()), window);
            }
            "open_new" => {
                self.run(Action::NewTabUrl(arg.to_string()), window);
            }
            "downloads_dir" => {
                let dir = self.delegate.downloads.dir.borrow().clone();
                let _ = std::fs::create_dir_all(&dir);
                window::show_in_folder(&dir);
            }
            "dl_clear" => self.delegate.downloads.clear_finished(),
            "download" if arg.trim().starts_with("http") => self.download(arg.trim(), false),
            "dl_cancel" => {
                if let Some(id) = id {
                    self.delegate.downloads.cancel(id);
                }
            }
            "dl_remove" => {
                if let Some(id) = id {
                    self.delegate.downloads.remove(id);
                }
            }
            "dl_show" => {
                if let Some(path) = download_path(self) {
                    window::show_in_folder(&path);
                }
            }
            "dl_open" => {
                if let Some(path) = download_path(self) {
                    let mut actions = Vec::new();
                    crate::ui::open_download(&path, &mut actions);
                    for action in actions {
                        self.run(action, window);
                    }
                }
            }
            _ => {}
        }
    }
}
