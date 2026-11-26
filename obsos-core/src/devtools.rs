//! Инструменты разработчика OBSOS (F12, Ctrl+Shift+I; выбор элемента —
//! Ctrl+Shift+C): элементы, консоль, сеть, исходники, хранилище,
//! производительность. Встроены справа или снизу окна, как в Chromium.
//!
//! Страницу читает и меняет свой агент (`devtools_agent.js`), который
//! ставится в неё по требованию; запросы видит делегат Servo
//! (`load_web_resource`). Отладчика с точками останова здесь нет — для него
//! сервер инструментов Servo и Firefox (`--devtools`, настройки).

use std::cell::RefCell;
use std::collections::{BTreeSet, VecDeque};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use browser_sync::json::{self, Value};
use egui::{Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use servo::WebViewId;

use crate::chrome::Icon;
use crate::profile::TabsAt;
use crate::shell::{Note, Shell};

const MAX_ENTRIES: usize = 1500;
const AGENT: &str = include_str!("devtools_agent.js");

pub struct ConsoleEntry {
    pub webview: Option<WebViewId>,
    pub level: String,
    pub text: String,
    pub at: Instant,
    /// Сколько раз подряд повторилась.
    pub repeat: u32,
}

#[derive(Clone)]
pub struct NetEntry {
    pub webview: WebViewId,
    pub method: String,
    pub url: String,
    pub kind: String,
    pub main: bool,
    pub blocked: bool,
    pub at: Instant,
    pub headers: Vec<(String, String)>,
}

/// Журналы, которые пишет делегат Servo.
#[derive(Default)]
pub struct DevLog {
    pub console: VecDeque<ConsoleEntry>,
    pub network: VecDeque<NetEntry>,
}

impl DevLog {
    pub fn console(&mut self, webview: Option<WebViewId>, level: &str, text: String) {
        if let Some(last) = self.console.back_mut() {
            if last.webview == webview && last.level == level && last.text == text && level != "input" {
                last.repeat += 1;
                return;
            }
        }
        if self.console.len() >= MAX_ENTRIES {
            self.console.pop_front();
        }
        self.console.push_back(ConsoleEntry { webview, level: level.to_string(), text, at: Instant::now(), repeat: 1 });
    }

    pub fn request(&mut self, entry: NetEntry) {
        if self.network.len() >= MAX_ENTRIES {
            self.network.pop_front();
        }
        self.network.push_back(entry);
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum DevTab {
    Elements,
    // Открывается первой (как было; на это опирается стенд FossAi: F12 и
    // сразу набор команды).
    #[default]
    Console,
    Network,
    Sources,
    Storage,
    Performance,
}

type Slot<T> = Rc<RefCell<T>>;
/// Ответ повторного запроса деталей (фоновый поток).
type Fetched = Arc<Mutex<Option<(String, String)>>>;

/// Состояние панели (ответы агента приходят обратными вызовами).
pub struct DevUi {
    pub tab: DevTab,
    pub input: String,
    pub filter: String,
    pub level: u8,
    pub started: Instant,
    /// Отдать фокус строке ввода консоли (панель только что открыли).
    pub focus_input: bool,
    /// Ширина панели справа / высота снизу, меняется мышью.
    pub width: f32,
    pub height: f32,
    pub bottom: bool,
    /// Включить выбор элемента на странице (Ctrl+Shift+C).
    pub pick: bool,
    picking: bool,
    history: Vec<String>,
    history_at: usize,
    preserve: bool,
    page: Option<(WebViewId, String)>,
    // Элементы.
    tree: Slot<Option<Value>>,
    open: BTreeSet<u64>,
    selected: Option<u64>,
    details: Slot<Option<Value>>,
    side: u8,
    edit: Option<(u64, String, String)>,
    inline_edit: String,
    hovered: Option<u64>,
    scroll_to: bool,
    next_refresh: Instant,
    // Сеть.
    net_selected: Option<(String, Instant)>,
    net_kind: u8,
    net_tab: u8,
    response: Fetched,
    perf: Slot<Option<Value>>,
    // Исходники и хранилище.
    sources: Slot<Option<Value>>,
    source: Option<(String, u64, String)>,
    source_text: Fetched,
    storage: Slot<Option<Value>>,
    storage_new: (String, String),
    area: u8,
}

impl Default for DevUi {
    fn default() -> Self {
        DevUi {
            tab: DevTab::Console,
            input: String::new(),
            filter: String::new(),
            level: 0,
            started: Instant::now(),
            focus_input: true,
            width: 560.0,
            height: 320.0,
            bottom: false,
            pick: false,
            picking: false,
            history: Vec::new(),
            history_at: 0,
            preserve: false,
            page: None,
            tree: Rc::default(),
            open: BTreeSet::new(),
            selected: None,
            details: Rc::default(),
            side: 0,
            edit: None,
            inline_edit: String::new(),
            hovered: None,
            scroll_to: false,
            next_refresh: Instant::now(),
            net_selected: None,
            net_kind: 0,
            net_tab: 0,
            response: Arc::default(),
            perf: Rc::default(),
            sources: Rc::default(),
            source: None,
            source_text: Arc::default(),
            storage: Rc::default(),
            storage_new: (String::new(), String::new()),
            area: 0,
        }
    }
}

// ── Связь с агентом ─────────────────────────────────────────────────────────

fn quote(text: &str) -> String {
    let mut out = String::new();
    json::write_str(text, &mut out);
    out
}

/// Вызвать функцию агента и отдать разобранный JSON ответа.
fn call(shell: &Shell, expression: String, done: impl FnOnce(Value) + 'static) {
    let Some(w) = shell.active_view() else { return };
    let delegate = shell.delegate.clone();
    let script = format!("{AGENT}\nJSON.stringify(window.__obsosDT.{expression})");
    w.evaluate_javascript(script, move |result| {
        let value = match result {
            Ok(servo::JSValue::String(text)) => json::parse(&text).unwrap_or(Value::Null),
            _ => Value::Null,
        };
        done(value);
        delegate.note(Note::Redraw);
    });
}

fn into_slot(shell: &Shell, expression: String, slot: &Slot<Option<Value>>) {
    let slot = slot.clone();
    call(shell, expression, move |v| *slot.borrow_mut() = Some(v));
}

/// Значение JavaScript — читаемой строкой для консоли.
pub fn show_value(v: &servo::JSValue) -> String {
    fn walk(v: &servo::JSValue, depth: usize, out: &mut String) {
        use servo::JSValue as J;
        if out.len() > 8000 {
            return;
        }
        match v {
            J::Undefined => out.push_str("undefined"),
            J::Null => out.push_str("null"),
            J::Boolean(b) => out.push_str(if *b { "true" } else { "false" }),
            J::Number(n) => out.push_str(&format!("{n}")),
            J::String(s) if depth == 0 => out.push_str(&format!("\"{s}\"")),
            J::String(s) => out.push_str(&format!("\"{}\"", s.chars().take(200).collect::<String>())),
            J::Array(items) => {
                out.push_str(&format!("({}) [", items.len()));
                for (i, item) in items.iter().take(100).enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    walk(item, depth + 1, out);
                }
                out.push_str(if items.len() > 100 { ", …]" } else { "]" });
            }
            J::Object(map) => {
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                let pad = "  ".repeat(depth + 1);
                out.push('{');
                for key in keys.iter().take(100) {
                    out.push_str(&format!("\n{pad}{key}: "));
                    if depth < 3 {
                        walk(&map[*key], depth + 1, out);
                    } else {
                        out.push('…');
                    }
                }
                if !keys.is_empty() {
                    out.push('\n');
                    out.push_str(&"  ".repeat(depth));
                }
                out.push('}');
            }
            J::Element(_) => out.push_str("<элемент DOM> (смотрите вкладку «Элементы»)"),
            other => out.push_str(&format!("{other:?}")),
        }
    }
    let mut out = String::new();
    walk(v, 0, &mut out);
    out
}

fn num(v: &Value) -> f64 {
    match v {
        Value::Num(s) => s.parse().unwrap_or(0.0),
        _ => 0.0,
    }
}

fn field_num(v: &Value, key: &str) -> u64 {
    v.get(key).map(num).unwrap_or(0.0).max(0.0) as u64
}

fn array<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    match v.get(key) {
        Some(Value::Arr(a)) => a,
        _ => &[],
    }
}

fn pair(v: &Value) -> (&str, &str) {
    match v {
        Value::Arr(a) if a.len() >= 2 => match (&a[0], &a[1]) {
            (Value::Str(k), Value::Str(v)) => (k, v),
            _ => ("", ""),
        },
        _ => ("", ""),
    }
}

fn size_text(bytes: u64) -> String {
    if bytes == 0 { String::from("—") } else { crate::downloads::human_size(bytes) }
}

// ── Цвета ───────────────────────────────────────────────────────────────────

struct Colors {
    tag: Color32,
    attr: Color32,
    value: Color32,
    dim: Color32,
    error: Color32,
    warn: Color32,
    select: Color32,
    line: Color32,
}

fn colors(ui: &Ui) -> Colors {
    let v = ui.visuals();
    if v.dark_mode {
        Colors {
            tag: Color32::from_rgb(0x7c, 0xb7, 0xff),
            attr: Color32::from_rgb(0xf0, 0xb0, 0x70),
            value: Color32::from_rgb(0x9f, 0xd9, 0x8b),
            dim: Color32::from_rgb(0x8d, 0x93, 0xad),
            error: Color32::from_rgb(0xff, 0x8a, 0x80),
            warn: Color32::from_rgb(0xff, 0xd0, 0x70),
            select: v.selection.bg_fill.gamma_multiply(0.45),
            line: Color32::from_white_alpha(24),
        }
    } else {
        Colors {
            tag: Color32::from_rgb(0x1a, 0x4f, 0xb8),
            attr: Color32::from_rgb(0x9a, 0x4d, 0x00),
            value: Color32::from_rgb(0x1c, 0x6b, 0x2a),
            dim: Color32::from_rgb(0x6a, 0x70, 0x85),
            error: Color32::from_rgb(0xc0, 0x2a, 0x20),
            warn: Color32::from_rgb(0x8a, 0x5a, 0x00),
            select: v.selection.bg_fill.gamma_multiply(0.25),
            line: Color32::from_black_alpha(28),
        }
    }
}

fn mono(text: impl Into<String>) -> RichText {
    RichText::new(text).monospace().size(12.0)
}

// ── Панель ──────────────────────────────────────────────────────────────────

/// Другая вкладка или переход — всё прочитанное устарело.
fn follow_page(shell: &mut Shell) {
    let now_page = shell.tabs.get(shell.active).map(|t| (t.webview.id(), t.real_url.as_ref().map(|u| u.to_string()).unwrap_or_default()));
    if shell.dev.page != now_page {
        // Выбор элемента на прежней вкладке выключить там же.
        if shell.dev.picking {
            let previous = shell.dev.page.as_ref().map(|(id, _)| *id);
            if let Some(tab) = shell.tabs.iter().find(|t| Some(t.webview.id()) == previous) {
                tab.webview.evaluate_javascript("window.__obsosDT&&window.__obsosDT.pick(false)", |_| {});
            }
        }
        shell.dev.page = now_page;
        *shell.dev.tree.borrow_mut() = None;
        *shell.dev.details.borrow_mut() = None;
        *shell.dev.sources.borrow_mut() = None;
        *shell.dev.storage.borrow_mut() = None;
        *shell.dev.perf.borrow_mut() = None;
        shell.dev.selected = None;
        shell.dev.open.clear();
        shell.dev.source = None;
        shell.dev.picking = false;
    }
}

pub fn ui(ui: &mut Ui, shell: &mut Shell) {
    follow_page(shell);
    if std::mem::take(&mut shell.dev.pick) {
        shell.dev.tab = DevTab::Elements;
        shell.dev.picking = !shell.dev.picking;
        call(shell, format!("pick({})", shell.dev.picking), |_| {});
    }
    let c = colors(ui);
    ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
    // Шапка: выбор элемента, разделы (переносятся на узкой панели), справа —
    // стыковка и закрытие.
    ui.horizontal(|ui| {
        ui.spacing_mut().button_padding = vec2(7.0, 3.0);
        ui.spacing_mut().item_spacing.x = 2.0;
        let (rect, pick) = ui.allocate_exact_size(vec2(24.0, 22.0), Sense::click());
        if shell.dev.picking || pick.hovered() {
            ui.painter().rect_filled(rect, 5.0, if shell.dev.picking { c.select } else { c.line });
        }
        // Значок «прицел».
        let ink = Stroke::new(1.5_f32, if shell.dev.picking { c.tag } else { ui.visuals().text_color() });
        ui.painter().circle_stroke(rect.center(), 5.0, ink);
        for d in [vec2(8.5, 0.0), vec2(-8.5, 0.0), vec2(0.0, 8.5), vec2(0.0, -8.5)] {
            ui.painter().line_segment([rect.center() + d * 0.35, rect.center() + d], ink);
        }
        if pick.on_hover_text("Выбрать элемент на странице (Ctrl+Shift+C)").clicked() {
            shell.dev.pick = true;
            ui.ctx().request_repaint();
        }
        let controls = 56.0;
        ui.allocate_ui_with_layout(vec2((ui.available_width() - controls).max(80.0), 22.0), egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true), |ui| {
            for (tab, name) in [
                (DevTab::Elements, "Элементы"),
                (DevTab::Console, "Консоль"),
                (DevTab::Network, "Сеть"),
                (DevTab::Sources, "Исходники"),
                (DevTab::Storage, "Хранилище"),
                (DevTab::Performance, "Скорость"),
            ] {
                if ui.selectable_label(shell.dev.tab == tab, name).clicked() {
                    shell.dev.tab = tab;
                    shell.dev.filter.clear();
                    shell.dev.next_refresh = Instant::now();
                    if tab == DevTab::Console {
                        shell.dev.focus_input = true;
                    }
                }
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let button = |ui: &mut Ui, icon: Icon, tip: &str| {
                let (rect, response) = ui.allocate_exact_size(vec2(24.0, 22.0), Sense::click());
                if response.hovered() {
                    ui.painter().rect_filled(rect, 5.0, c.line);
                }
                crate::chrome::paint_icon(ui.painter(), rect, icon, ui.visuals().text_color());
                response.on_hover_text(tip).clicked()
            };
            if button(ui, Icon::Close, "Закрыть (F12)") {
                shell.panel = None;
                call(shell, String::from("highlight(0)"), |_| {});
            }
            let (icon, tip) = if shell.dev.bottom { (Icon::Layout(TabsAt::Right), "Закрепить справа") } else { (Icon::Layout(TabsAt::Bottom), "Закрепить снизу") };
            if button(ui, icon, tip) {
                shell.dev.bottom = !shell.dev.bottom;
            }
        });
    });
    ui.separator();
    match shell.dev.tab {
        DevTab::Elements => elements(ui, shell, &c),
        DevTab::Console => console(ui, shell, &c),
        DevTab::Network => network(ui, shell, &c),
        DevTab::Sources => sources(ui, shell, &c),
        DevTab::Storage => storage(ui, shell, &c),
        DevTab::Performance => performance(ui, shell, &c),
    }
}

/// Панель закрыли: выключить выбор элемента и подсветку, иначе следующий
/// щелчок по странице проглотится.
pub fn closed(shell: &mut Shell) {
    if shell.dev.picking || shell.dev.hovered.is_some() {
        shell.dev.picking = false;
        shell.dev.pick = false;
        shell.dev.hovered = None;
        call(shell, String::from("pick(false)"), |_| {});
    }
}

/// «Просмотреть код»: выбрать элемент в точке страницы (CSS-пиксели).
pub fn inspect_at(shell: &mut Shell, x: f32, y: f32) {
    shell.dev.tab = DevTab::Elements;
    // Сперва сбросить прочитанное о другой странице — иначе это сделает
    // первый кадр панели и сотрёт выбор.
    follow_page(shell);
    call(shell, format!("pickAt({x},{y})"), |_| {});
}

/// Сообщение агента (выбран элемент).
pub fn agent_message(shell: &mut Shell, payload: &str) {
    let Ok(v) = json::parse(payload) else { return };
    if v.str_field("t") == Some("picked") {
        shell.dev.picking = false;
        let id = field_num(&v, "i");
        shell.dev.open.extend(array(&v, "path").iter().map(|n| num(n) as u64));
        shell.dev.open.remove(&id);
        select(shell, id);
        shell.dev.scroll_to = true;
        shell.dev.next_refresh = Instant::now();
        println!("browser: devtools picked {id}");
    }
}

fn select(shell: &mut Shell, id: u64) {
    shell.dev.selected = Some(id);
    shell.dev.edit = None;
    *shell.dev.details.borrow_mut() = None;
    refresh_details(shell, id);
}

fn refresh_details(shell: &Shell, id: u64) {
    into_slot(shell, format!("details({id})"), &shell.dev.details);
}

fn refresh_tree(shell: &Shell) {
    let open: Vec<String> = shell.dev.open.iter().map(u64::to_string).collect();
    into_slot(shell, format!("tree([{}])", open.join(",")), &shell.dev.tree);
}

// ── Элементы ────────────────────────────────────────────────────────────────

struct Row {
    id: u64,
    depth: usize,
    job: egui::text::LayoutJob,
    expandable: bool,
    expanded: bool,
}

fn flatten(v: &Value, depth: usize, open: &BTreeSet<u64>, c: &Colors, text: Color32, rows: &mut Vec<Row>) {
    if rows.len() > 4000 {
        return;
    }
    let id = field_num(v, "i");
    let font = FontId::monospace(12.0);
    let mut job = egui::text::LayoutJob::default();
    let push = |job: &mut egui::text::LayoutJob, s: &str, color: Color32| job.append(s, 0.0, egui::TextFormat::simple(font.clone(), color));
    let kind = v.str_field("k").unwrap_or("");
    let mut expandable = false;
    match kind {
        "text" => push(&mut job, &format!("\"{}\"", v.str_field("x").unwrap_or("")), text),
        "comment" => push(&mut job, &format!("<!--{}-->", v.str_field("x").unwrap_or("")), c.dim),
        "shadow" => {
            push(&mut job, "#shadow-root", c.dim);
            expandable = true;
        }
        _ => {
            let tag = v.str_field("t").unwrap_or("?");
            push(&mut job, &format!("<{tag}"), c.tag);
            for a in array(v, "a") {
                let (k, val) = pair(a);
                push(&mut job, &format!(" {k}"), c.attr);
                if !val.is_empty() {
                    let short: String = val.chars().take(90).collect();
                    push(&mut job, "=", c.dim);
                    push(&mut job, &format!("\"{short}\""), c.value);
                }
            }
            push(&mut job, ">", c.tag);
            if let Some(x) = v.str_field("x") {
                push(&mut job, x, text);
                push(&mut job, &format!("</{tag}>"), c.tag);
            }
            expandable = field_num(v, "n") > 0;
        }
    }
    let children = array(v, "c");
    let expanded = expandable && !children.is_empty() && (open.contains(&id) || depth < 2 || kind == "shadow");
    if expandable && !expanded && kind == "element" {
        push(&mut job, &format!("…</{}>", v.str_field("t").unwrap_or("?")), c.dim);
    }
    // Строки не переносятся.
    job.wrap.max_width = f32::INFINITY;
    rows.push(Row { id, depth, job, expandable, expanded });
    if expanded {
        for child in children {
            flatten(child, depth + 1, open, c, text, rows);
        }
    }
}

fn elements(ui: &mut Ui, shell: &mut Shell, c: &Colors) {
    if Instant::now() >= shell.dev.next_refresh {
        shell.dev.next_refresh = Instant::now() + Duration::from_millis(1500);
        refresh_tree(shell);
    }
    ui.ctx().request_repaint_after(Duration::from_millis(1500));
    let text = ui.visuals().text_color();
    let total = ui.available_height();
    let wide = ui.available_width() > 760.0;
    let detail_h = if shell.dev.selected.is_some() && !wide { (total * 0.48).max(160.0) } else { 0.0 };
    let tree_w = if wide && shell.dev.selected.is_some() { ui.available_width() * 0.56 } else { ui.available_width() };
    ui.allocate_ui_with_layout(vec2(ui.available_width(), total), if wide { egui::Layout::left_to_right(egui::Align::Min) } else { egui::Layout::top_down(egui::Align::Min) }, |ui| {
        let mut rows = Vec::new();
        if let Some(tree) = shell.dev.tree.borrow().as_ref() {
            flatten(tree, 0, &shell.dev.open, c, text, &mut rows);
        }
        let mut hovered = None;
        let mut clicked = None;
        let mut toggle = None;
        let selected = shell.dev.selected;
        let scroll_to = std::mem::take(&mut shell.dev.scroll_to) && !rows.is_empty();
        if rows.is_empty() && shell.dev.selected.is_some() {
            shell.dev.scroll_to = true;
        }
        ui.allocate_ui(vec2(tree_w, total - detail_h), |ui| {
            egui::ScrollArea::both().id_salt("obsos-dom").auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                if rows.is_empty() {
                    ui.label("Читаю документ…");
                }
                for row in &rows {
                    let indent = row.depth as f32 * 13.0 + 16.0;
                    let galley = ui.painter().layout_job(row.job.clone());
                    let width = (indent + galley.size().x + 8.0).max(ui.available_width());
                    let (rect, response) = ui.allocate_exact_size(vec2(width, 17.0), Sense::click());
                    if selected == Some(row.id) {
                        ui.painter().rect_filled(rect, 3.0, c.select);
                        if scroll_to {
                            response.scroll_to_me(Some(egui::Align::Center));
                        }
                    } else if response.hovered() {
                        ui.painter().rect_filled(rect, 3.0, c.line);
                    }
                    let arrow = Rect::from_min_size(pos2(rect.left() + indent - 14.0, rect.top()), vec2(14.0, 17.0));
                    if row.expandable {
                        let m = arrow.center();
                        let pts = if row.expanded {
                            vec![m + vec2(-3.5, -2.0), m + vec2(3.5, -2.0), m + vec2(0.0, 3.0)]
                        } else {
                            vec![m + vec2(-2.0, -3.5), m + vec2(3.0, 0.0), m + vec2(-2.0, 3.5)]
                        };
                        ui.painter().add(egui::Shape::convex_polygon(pts, c.dim, Stroke::NONE));
                    }
                    ui.painter().galley(pos2(rect.left() + indent, rect.center().y - galley.size().y / 2.0), galley, text);
                    if response.hovered() {
                        hovered = Some(row.id);
                    }
                    if response.clicked() {
                        if row.expandable && response.interact_pointer_pos().is_some_and(|p| arrow.contains(p)) {
                            toggle = Some((row.id, row.expanded));
                        } else {
                            clicked = Some(row.id);
                        }
                    }
                    if response.double_clicked() && row.expandable {
                        toggle = Some((row.id, row.expanded));
                    }
                }
            });
        });
        if let Some((id, expanded)) = toggle {
            if expanded {
                shell.dev.open.remove(&id);
            } else {
                shell.dev.open.insert(id);
            }
            refresh_tree(shell);
        }
        if let Some(id) = clicked {
            select(shell, id);
        }
        if hovered != shell.dev.hovered && !shell.dev.picking {
            shell.dev.hovered = hovered;
            call(shell, format!("highlight({})", hovered.unwrap_or(0)), |_| {});
        }
        if shell.dev.selected.is_some() {
            ui.separator();
            ui.vertical(|ui| element_side(ui, shell, c));
        }
    });
}

fn element_side(ui: &mut Ui, shell: &mut Shell, c: &Colors) {
    let Some(id) = shell.dev.selected else { return };
    let details = shell.dev.details.borrow().clone();
    let Some(d) = details.filter(|d| *d != Value::Null) else {
        ui.label("Читаю элемент…");
        return;
    };
    ui.horizontal_wrapped(|ui| {
        ui.label(mono(d.str_field("selector").unwrap_or("")).color(c.tag).strong());
    });
    ui.horizontal(|ui| {
        for (i, name) in ["Стили", "Вычисленные", "Блок", "Атрибуты"].iter().enumerate() {
            if ui.selectable_label(shell.dev.side == i as u8, *name).clicked() {
                shell.dev.side = i as u8;
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("удалить").on_hover_text("Удалить элемент из документа").clicked() {
                call(shell, format!("remove({id})"), |_| {});
                shell.dev.selected = None;
                shell.dev.next_refresh = Instant::now() + Duration::from_millis(100);
            }
            if ui.small_button("копировать HTML").clicked() {
                call(shell, format!("outer({id})"), |v| {
                    if let Value::Str(html) = v {
                        crate::window::clipboard_set(&html);
                    }
                });
                shell.toast("HTML элемента скопирован");
            }
        });
    });
    egui::ScrollArea::vertical().id_salt("obsos-el-side").auto_shrink([false, false]).show(ui, |ui| match shell.dev.side {
        0 => {
            ui.label(mono("element.style {").color(c.dim));
            if shell.dev.edit.as_ref().is_none_or(|(i, k, _)| *i != id || k != "style") {
                shell.dev.inline_edit = d.str_field("inline").unwrap_or("").to_string();
            }
            let edit = ui.add(egui::TextEdit::multiline(&mut shell.dev.inline_edit).font(egui::TextStyle::Monospace).desired_rows(1).desired_width(f32::INFINITY).hint_text("свойство: значение;  (применяется при уходе из поля)"));
            if edit.has_focus() {
                shell.dev.edit = Some((id, String::from("style"), String::new()));
            }
            if edit.lost_focus() {
                shell.dev.edit = None;
                call(shell, format!("setAttr({id},'style',{})", quote(&shell.dev.inline_edit)), |_| {});
                refresh_details(shell, id);
            }
            ui.label(mono("}").color(c.dim));
            let rules = array(&d, "rules");
            if rules.is_empty() {
                ui.label(RichText::new("Правил из таблиц стилей нет").small());
            }
            for rule in rules {
                ui.add_space(4.0);
                let media = rule.str_field("media").unwrap_or("");
                if !media.is_empty() {
                    ui.label(mono(format!("@media {media}")).color(c.dim));
                }
                ui.horizontal(|ui| {
                    ui.label(mono(format!("{} {{", rule.str_field("s").unwrap_or(""))).color(c.tag));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(rule.str_field("from").unwrap_or("")).size(10.5).color(c.dim));
                    });
                });
                for declaration in rule.str_field("css").unwrap_or("").split(';').map(str::trim).filter(|s| !s.is_empty()) {
                    let (name, value) = declaration.split_once(':').unwrap_or((declaration, ""));
                    ui.horizontal_wrapped(|ui| {
                        ui.add_space(14.0);
                        ui.spacing_mut().item_spacing.x = 0.0;
                        ui.label(mono(name.trim()).color(c.attr));
                        ui.label(mono(format!(": {};", value.trim())));
                    });
                }
                ui.label(mono("}").color(c.dim));
            }
        }
        1 => {
            ui.add(egui::TextEdit::singleline(&mut shell.dev.filter).hint_text("фильтр свойств").desired_width(200.0));
            let f = shell.dev.filter.to_lowercase();
            egui::Grid::new("obsos-computed").striped(true).num_columns(2).show(ui, |ui| {
                for entry in array(&d, "computed") {
                    let (name, value) = pair(entry);
                    if !f.is_empty() && !name.contains(&f) {
                        continue;
                    }
                    ui.label(mono(name).color(c.attr));
                    ui.add(egui::Label::new(mono(value)).wrap());
                    ui.end_row();
                }
            });
        }
        2 => box_model(ui, &d, c),
        _ => {
            attributes(ui, shell, id, c);
        }
    });
}

fn attributes(ui: &mut Ui, shell: &mut Shell, id: u64, c: &Colors) {
    fn find(v: &Value, id: u64) -> Option<&Value> {
        if field_num(v, "i") == id {
            return Some(v);
        }
        array(v, "c").iter().find_map(|child| find(child, id))
    }
    let attrs: Vec<(String, String)> = shell
        .dev
        .tree
        .borrow()
        .as_ref()
        .and_then(|t| find(t, id).map(|n| array(n, "a").iter().map(|a| (pair(a).0.to_string(), pair(a).1.to_string())).collect()))
        .unwrap_or_default();
    let mut apply: Option<(String, Option<String>)> = None;
    for (name, value) in &attrs {
        ui.horizontal(|ui| {
            if ui.small_button("×").clicked() {
                apply = Some((name.clone(), None));
            }
            ui.label(mono(name).color(c.attr));
            let editing = shell.dev.edit.as_ref().is_some_and(|(i, k, _)| *i == id && k == name);
            let mut text = if editing { shell.dev.edit.as_ref().map(|e| e.2.clone()).unwrap_or_default() } else { value.clone() };
            let edit = ui.add(egui::TextEdit::singleline(&mut text).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY));
            if edit.has_focus() {
                shell.dev.edit = Some((id, name.clone(), text.clone()));
            }
            if edit.lost_focus() {
                shell.dev.edit = None;
                if text != *value {
                    apply = Some((name.clone(), Some(text)));
                }
            }
        });
    }
    ui.separator();
    ui.horizontal_wrapped(|ui| {
        ui.add(egui::TextEdit::singleline(&mut shell.dev.storage_new.0).hint_text("атрибут").desired_width(120.0));
        ui.add(egui::TextEdit::singleline(&mut shell.dev.storage_new.1).hint_text("значение").desired_width(180.0));
        if ui.button("Добавить").clicked() && !shell.dev.storage_new.0.trim().is_empty() {
            apply = Some((shell.dev.storage_new.0.trim().to_string(), Some(shell.dev.storage_new.1.clone())));
            shell.dev.storage_new = Default::default();
        }
    });
    if let Some((name, value)) = apply {
        let value = value.map(|v| quote(&v)).unwrap_or_else(|| String::from("null"));
        call(shell, format!("setAttr({id},{},{value})", quote(&name)), |_| {});
        shell.dev.next_refresh = Instant::now() + Duration::from_millis(100);
        refresh_details(shell, id);
    }
}

fn box_model(ui: &mut Ui, d: &Value, c: &Colors) {
    let four = |key: &str| -> [f64; 4] {
        let a = array(d, key);
        std::array::from_fn(|i| a.get(i).map(num).unwrap_or(0.0))
    };
    let rect: Vec<f64> = array(d, "rect").iter().map(num).collect();
    let (w, h) = (rect.get(2).copied().unwrap_or(0.0), rect.get(3).copied().unwrap_or(0.0));
    let (margin, border, padding) = (four("margin"), four("border"), four("padding"));
    let width = ui.available_width().min(380.0);
    let (area, _) = ui.allocate_exact_size(vec2(width, 230.0), Sense::hover());
    let painter = ui.painter();
    let mut r = area;
    let fmt = |v: f64| if v == 0.0 { String::from("–") } else { format!("{}", (v * 100.0).round() / 100.0) };
    for (name, values, color) in [
        ("margin", margin, Color32::from_rgb(0xb0, 0x84, 0x4e)),
        ("border", border, Color32::from_rgb(0xc8, 0xb0, 0x5a)),
        ("padding", padding, Color32::from_rgb(0x6f, 0x93, 0x5e)),
    ] {
        painter.rect_filled(r, 4.0, color.gamma_multiply(0.35));
        painter.rect_stroke(r, 4.0, Stroke::new(1.0_f32, color), StrokeKind::Inside);
        let text = ui.visuals().text_color();
        painter.text(r.left_top() + vec2(5.0, 3.0), Align2::LEFT_TOP, name, FontId::proportional(10.0), c.dim);
        painter.text(pos2(r.center().x, r.top() + 10.0), Align2::CENTER_CENTER, fmt(values[0]), FontId::monospace(11.0), text);
        painter.text(pos2(r.right() - 18.0, r.center().y), Align2::CENTER_CENTER, fmt(values[1]), FontId::monospace(11.0), text);
        painter.text(pos2(r.center().x, r.bottom() - 10.0), Align2::CENTER_CENTER, fmt(values[2]), FontId::monospace(11.0), text);
        painter.text(pos2(r.left() + 18.0, r.center().y), Align2::CENTER_CENTER, fmt(values[3]), FontId::monospace(11.0), text);
        r = Rect::from_min_max(r.min + vec2(36.0, 22.0), r.max - vec2(36.0, 22.0));
    }
    painter.rect_filled(r, 3.0, Color32::from_rgb(0x4f, 0x80, 0xb0).gamma_multiply(0.45));
    painter.text(r.center(), Align2::CENTER_CENTER, format!("{} × {}", fmt(w - border[1] - border[3] - padding[1] - padding[3]), fmt(h - border[0] - border[2] - padding[0] - padding[2])), FontId::monospace(11.5), ui.visuals().text_color());
    if rect.len() == 4 {
        ui.label(mono(format!("Положение в окне: x {} · y {} · размер {} × {}", rect[0], rect[1], rect[2], rect[3])));
    }
}

// ── Консоль ─────────────────────────────────────────────────────────────────

fn console(ui: &mut Ui, shell: &mut Shell, c: &Colors) {
    let active = shell.active_view().map(|w| w.id());
    ui.horizontal_wrapped(|ui| {
        if ui.small_button("Очистить").clicked() {
            shell.delegate.devlog.borrow_mut().console.clear();
        }
        ui.add(egui::TextEdit::singleline(&mut shell.dev.filter).hint_text("фильтр").desired_width(150.0));
        for (level, name) in [(0u8, "Все"), (1, "Ошибки"), (2, "Предупреждения"), (3, "Сообщения")] {
            if ui.selectable_label(shell.dev.level == level, name).clicked() {
                shell.dev.level = level;
            }
        }
    });
    let lines = shell.dev.input.matches('\n').count().min(6) as f32;
    let input_h = 30.0 + lines * 15.0;
    egui::ScrollArea::vertical().id_salt("obsos-console").max_height(ui.available_height() - input_h - 6.0).stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
        let log = shell.delegate.devlog.borrow();
        let f = shell.dev.filter.to_lowercase();
        let mut shown = 0;
        for e in log.console.iter().filter(|e| e.webview.is_none() || e.webview == active) {
            let error = e.level.contains("Error");
            let warn = e.level.contains("Warn");
            let own = e.level == "input" || e.level == "result";
            let keep = match shell.dev.level {
                1 => error,
                2 => warn,
                3 => !error && !warn,
                _ => true,
            };
            if !keep && !own || !f.is_empty() && !e.text.to_lowercase().contains(&f) {
                continue;
            }
            shown += 1;
            let color = if error {
                c.error
            } else if warn {
                c.warn
            } else if e.level == "result" {
                c.tag
            } else if e.level == "input" {
                c.dim
            } else {
                ui.visuals().text_color()
            };
            let fill = if error { c.error.gamma_multiply(0.09) } else if warn { c.warn.gamma_multiply(0.08) } else { Color32::TRANSPARENT };
            egui::Frame::new().fill(fill).inner_margin(egui::Margin::symmetric(4, 1)).show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    if e.repeat > 1 {
                        ui.label(RichText::new(format!("{}×", e.repeat)).size(10.5).color(c.dim));
                    }
                    let response = ui.add(egui::Label::new(mono(&e.text).color(color)).wrap().sense(Sense::click()));
                    if response.on_hover_text(format!("{:.1} с — щелчок копирует", e.at.saturating_duration_since(shell.dev.started).as_secs_f32())).clicked() {
                        crate::window::clipboard_set(&e.text);
                    }
                });
                ui.set_min_width(ui.available_width());
            });
        }
        if shown == 0 {
            ui.label(RichText::new("Сообщений нет").color(c.dim));
        }
    });
    let id = egui::Id::new("obsos-console-input");
    let focused = ui.memory(|m| m.has_focus(id));
    // Стрелки — по истории команд (пока строка одна).
    if focused && !shell.dev.input.contains('\n') && !shell.dev.history.is_empty() {
        let (up, down) = ui.input_mut(|i| (i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp), i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)));
        let n = shell.dev.history.len();
        if up {
            shell.dev.history_at = shell.dev.history_at.min(n).saturating_sub(1);
            shell.dev.input = shell.dev.history[shell.dev.history_at].clone();
        } else if down {
            shell.dev.history_at = (shell.dev.history_at + 1).min(n);
            shell.dev.input = shell.dev.history.get(shell.dev.history_at).cloned().unwrap_or_default();
        }
    }
    // Enter — выполнить, Shift+Enter — новая строка.
    let run = focused && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    ui.horizontal_top(|ui| {
        ui.label(mono("›").color(c.tag).size(15.0));
        let edit = ui.add(egui::TextEdit::multiline(&mut shell.dev.input).id(id).desired_rows(1).hint_text("JavaScript — Enter выполнить, Shift+Enter — строка").font(egui::TextStyle::Monospace).desired_width(ui.available_width()));
        if std::mem::take(&mut shell.dev.focus_input) {
            edit.request_focus();
        }
    });
    if run && !shell.dev.input.trim().is_empty() {
        let code = std::mem::take(&mut shell.dev.input);
        evaluate(shell, code);
        ui.memory_mut(|m| m.request_focus(id));
    }
}

/// Выполнить код консоли в странице.
pub fn evaluate(shell: &mut Shell, code: String) {
    let active = shell.active_view().map(|w| w.id());
    if shell.dev.history.last() != Some(&code) {
        shell.dev.history.push(code.clone());
    }
    shell.dev.history_at = shell.dev.history.len();
    shell.delegate.devlog.borrow_mut().console(active, "input", format!("› {code}"));
    if let Some(w) = shell.active_view() {
        let delegate = shell.delegate.clone();
        let id = w.id();
        w.evaluate_javascript(code, move |result| {
            let (level, text) = match result {
                Ok(v) => ("result", show_value(&v)),
                Err(e) => ("Error", format!("Ошибка: {e:?}")),
            };
            let short: String = text.chars().take(600).collect();
            if !crate::logs::private() {
                println!("browser: devtools ← {}", crate::logs::scrub(&short));
            }
            delegate.devlog.borrow_mut().console(Some(id), level, if level == "result" { format!("← {text}") } else { text });
            delegate.note(Note::Redraw);
        });
    }
}

// ── Сеть ────────────────────────────────────────────────────────────────────

const NET_KINDS: [&str; 6] = ["Все", "Документ", "JS", "CSS", "Картинки", "Fetch/XHR"];

fn net_kind(e: &NetEntry) -> u8 {
    if e.main {
        return 1;
    }
    match e.kind.as_str() {
        "Script" | "Worker" | "SharedWorker" | "ServiceWorker" => 2,
        "Style" => 3,
        "Image" => 4,
        "None" => 5,
        "Document" | "IFrame" | "Frame" => 1,
        _ => 0,
    }
}

fn short_name(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let name = path.trim_end_matches('/').rsplit('/').next().unwrap_or(path);
    let query = url.split_once('?').map(|(_, q)| format!("?{}", q.chars().take(24).collect::<String>())).unwrap_or_default();
    if url.starts_with("data:") {
        return String::from("data:…");
    }
    format!("{name}{query}")
}

fn network(ui: &mut Ui, shell: &mut Shell, c: &Colors) {
    let active = shell.active_view().map(|w| w.id());
    // Время и размер — из `performance` самой страницы.
    if Instant::now() >= shell.dev.next_refresh {
        shell.dev.next_refresh = Instant::now() + Duration::from_millis(1000);
        into_slot(shell, String::from("perf()"), &shell.dev.perf);
    }
    ui.ctx().request_repaint_after(Duration::from_millis(1000));
    ui.horizontal_wrapped(|ui| {
        if ui.small_button("Очистить").clicked() {
            shell.delegate.devlog.borrow_mut().network.clear();
            shell.dev.net_selected = None;
        }
        ui.checkbox(&mut shell.dev.preserve, "сохранять");
        ui.add(egui::TextEdit::singleline(&mut shell.dev.filter).hint_text("фильтр адресов").desired_width(150.0));
        for (i, name) in NET_KINDS.iter().enumerate() {
            if ui.selectable_label(shell.dev.net_kind == i as u8, *name).clicked() {
                shell.dev.net_kind = i as u8;
            }
        }
    });
    let f = shell.dev.filter.to_lowercase();
    let (rows, start): (Vec<NetEntry>, Option<Instant>) = {
        let log = shell.delegate.devlog.borrow();
        let mine: Vec<&NetEntry> = log.network.iter().filter(|e| Some(e.webview) == active).collect();
        // Без «сохранять» — только с последней загрузки документа.
        let from = if shell.dev.preserve { 0 } else { mine.iter().rposition(|e| e.main).unwrap_or(0) };
        let start = mine.get(from).map(|e| e.at);
        let rows = mine[from.min(mine.len())..]
            .iter()
            // `data:` — встроенные картинки, не сеть (видны по фильтру «data»).
            .filter(|e| !e.url.starts_with("data:") || f.starts_with("data"))
            .filter(|e| (shell.dev.net_kind == 0 || net_kind(e) == shell.dev.net_kind) && (f.is_empty() || e.url.to_lowercase().contains(&f)))
            .map(|e| (*e).clone())
            .collect();
        (rows, start)
    };
    let timing: std::collections::HashMap<String, (u64, u64)> = shell
        .dev
        .perf
        .borrow()
        .as_ref()
        .map(|p| {
            array(p, "res")
                .iter()
                .filter_map(|r| match r {
                    Value::Arr(a) if a.len() >= 5 => match &a[0] {
                        Value::Str(url) => Some((url.clone(), (num(&a[3]) as u64, num(&a[4]) as u64))),
                        _ => None,
                    },
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    let blocked = rows.iter().filter(|e| e.blocked).count();
    let bytes: u64 = rows.iter().filter_map(|e| timing.get(&e.url)).map(|t| t.1).sum();
    let selected = shell.dev.net_selected.clone();
    let detail_h = if selected.is_some() { ui.available_height() * 0.52 } else { 0.0 };
    let list_h = ui.available_height() - detail_h - 20.0;
    let width = ui.available_width();
    let columns = [width - 240.0, 64.0, 62.0, 56.0, 58.0];
    let header = |ui: &mut Ui, texts: [&str; 5], color: Color32, fill: Color32, sense: Sense| {
        let (rect, response) = ui.allocate_exact_size(vec2(width, 18.0), sense);
        if fill != Color32::TRANSPARENT {
            ui.painter().rect_filled(rect, 3.0, fill);
        } else if response.hovered() && sense == Sense::click() {
            ui.painter().rect_filled(rect, 3.0, c.line);
        }
        let mut x = rect.left() + 4.0;
        for (i, text) in texts.iter().enumerate() {
            let cell = Rect::from_min_size(pos2(x, rect.top()), vec2(columns[i].max(30.0) - 6.0, rect.height()));
            ui.painter().with_clip_rect(cell.intersect(ui.clip_rect())).text(pos2(x, rect.center().y), Align2::LEFT_CENTER, text, FontId::monospace(11.5), color);
            x += columns[i].max(30.0);
        }
        response
    };
    header(ui, ["Имя", "Метод", "Тип", "Время", "Размер"], c.dim, Color32::TRANSPARENT, Sense::hover());
    egui::ScrollArea::vertical().id_salt("obsos-net").max_height(list_h).auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for e in &rows {
            let kind = match net_kind(e) {
                1 => "документ",
                2 => "script",
                3 => "stylesheet",
                4 => "картинка",
                5 => "fetch",
                _ => "прочее",
            };
            let (time, size) = timing.get(&e.url).copied().unwrap_or((0, 0));
            let is = selected.as_ref().is_some_and(|(u, at)| *u == e.url && *at == e.at);
            let name = if e.blocked { format!("⊘ {}", short_name(&e.url)) } else { short_name(&e.url) };
            let time_text = if e.blocked { String::from("блок") } else if time > 0 { format!("{time} мс") } else { String::from("—") };
            let color = if e.blocked { c.error } else { ui.visuals().text_color() };
            let response = header(ui, [&name, &e.method, kind, &time_text, &size_text(size)], color, if is { c.select } else { Color32::TRANSPARENT }, Sense::click());
            if response.on_hover_text(&e.url).clicked() {
                shell.dev.net_selected = Some((e.url.clone(), e.at));
                *shell.dev.response.lock().expect("devtools response") = None;
                shell.dev.net_tab = 0;
            }
        }
    });
    let last = rows.last().zip(start).map(|(e, s)| e.at.saturating_duration_since(s).as_secs_f32()).unwrap_or(0.0);
    ui.label(RichText::new(format!("Запросов: {} · заблокировано: {blocked} · {} · за {last:.1} с", rows.len(), size_text(bytes))).size(11.0).color(c.dim));
    let Some((url, at)) = shell.dev.net_selected.clone() else { return };
    let Some(entry) = rows.iter().find(|e| e.url == url && e.at == at).cloned() else { return };
    ui.separator();
    ui.horizontal(|ui| {
        for (i, name) in ["Заголовки", "Ответ"].iter().enumerate() {
            if ui.selectable_label(shell.dev.net_tab == i as u8, *name).clicked() {
                shell.dev.net_tab = i as u8;
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("✕").clicked() {
                shell.dev.net_selected = None;
            }
            // Всё из запроса — в одинарных кавычках с экранированием: иначе
            // адрес сайта мог бы выполнить команду в терминале.
            for (label, powershell) in [("cURL (bash)", false), ("cURL (PowerShell)", true)] {
                if ui.small_button(label).clicked() {
                    crate::window::clipboard_set(&curl_command(&entry, powershell));
                    shell.toast("Команда cURL скопирована");
                }
            }
            if ui.small_button("копировать адрес").clicked() {
                crate::window::clipboard_set(&entry.url);
            }
        });
    });
    egui::ScrollArea::vertical().id_salt("obsos-net-detail").auto_shrink([false, false]).show(ui, |ui| {
        if shell.dev.net_tab == 0 {
            ui.add(egui::Label::new(mono(format!("{} {}", entry.method, entry.url))).wrap().selectable(true));
            ui.add_space(4.0);
            ui.label(RichText::new("Заголовки запроса (как их отдаёт Servo)").strong());
            for (k, v) in &entry.headers {
                ui.horizontal_wrapped(|ui| {
                    ui.label(mono(format!("{k}:")).color(c.attr));
                    ui.add(egui::Label::new(mono(v)).wrap());
                });
            }
        } else {
            let got = shell.dev.response.lock().ok().and_then(|r| r.clone());
            match got {
                Some((head, body)) => {
                    ui.label(RichText::new("Повторный запрос OBSOS (ответ странице Servo не показывает)").size(11.0).color(c.dim));
                    ui.add(egui::Label::new(mono(head).color(c.attr)).wrap());
                    ui.separator();
                    let mut text = body.as_str();
                    ui.add(egui::TextEdit::multiline(&mut text).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY));
                }
                None if entry.method == "GET" && entry.url.starts_with("http") => {
                    if ui.button("Запросить ещё раз и показать ответ").clicked() {
                        fetch(shell, &entry.url, shell.dev.response.clone());
                    }
                }
                None => {
                    ui.label("Ответ можно перезапросить только для GET по http(s).");
                }
            }
        }
    });
}

/// Строка в одинарных кавычках оболочки: bash или PowerShell (там кавычками
/// считаются и «типографские» ‘ ’ ‚ ‛ — их тоже удваивают).
fn shell_quote(text: &str, powershell: bool) -> String {
    let text = text.replace(['\r', '\n'], " ");
    let mut out = String::from("'");
    for c in text.chars() {
        match c {
            '\'' if !powershell => out.push_str("'\\''"),
            '\'' | '\u{2018}' | '\u{2019}' | '\u{201a}' | '\u{201b}' if powershell => {
                out.push(c);
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out.push('\'');
    out
}

fn curl_command(entry: &NetEntry, powershell: bool) -> String {
    // В PowerShell `curl` — это Invoke-WebRequest, нужна сама программа.
    let mut command = format!("{} {}", if powershell { "curl.exe" } else { "curl" }, shell_quote(&entry.url, powershell));
    if entry.method != "GET" {
        command.push_str(&format!(" -X {}", shell_quote(&entry.method, powershell)));
    }
    for (k, v) in &entry.headers {
        command.push_str(&format!(" -H {}", shell_quote(&format!("{k}: {v}"), powershell)));
    }
    command
}

#[cfg(test)]
mod tests {
    use super::shell_quote;

    #[test]
    fn quoting_keeps_commands_inside() {
        assert_eq!(shell_quote("https://e/a';calc;'", false), "'https://e/a'\\'';calc;'\\'''");
        assert_eq!(shell_quote("a'b\u{2019}c\nd", true), "'a''b\u{2019}\u{2019}c d'");
    }
}

/// GET своим клиентом (с куки страницы) в фоне.
fn fetch(shell: &Shell, url: &str, into: Fetched) {
    let Ok(parsed) = url::Url::parse(url) else { return };
    let cookie = shell.cookie_for(&parsed);
    let url = url.to_string();
    if let Ok(mut slot) = into.lock() {
        *slot = Some((String::new(), String::from("Загружаю…")));
    }
    let _ = std::thread::Builder::new().name(String::from("obsos-devfetch")).spawn(move || {
        use browser_sync::remote::{Http, Request};
        let mut request = Request::new("GET", &url);
        if let Some(cookie) = cookie {
            request.headers.push((String::from("Cookie"), cookie));
        }
        let result = match crate::net::Client.send(&request) {
            Ok(r) => {
                let mut head = format!("Статус {}", r.status);
                for (k, v) in &r.headers {
                    head.push_str(&format!("\n{k}: {v}"));
                }
                let text = match std::str::from_utf8(&r.body) {
                    Ok(t) => t.chars().take(200_000).collect(),
                    Err(_) => format!("(двоичные данные, {} байт)", r.body.len()),
                };
                (head, text)
            }
            Err(e) => (String::new(), format!("Не запросить: {e}")),
        };
        if let Ok(mut slot) = into.lock() {
            *slot = Some(result);
        }
    });
}

// ── Исходники ───────────────────────────────────────────────────────────────

fn sources(ui: &mut Ui, shell: &mut Shell, c: &Colors) {
    if shell.dev.sources.borrow().is_none() {
        *shell.dev.sources.borrow_mut() = Some(Value::Null);
        into_slot(shell, String::from("sources()"), &shell.dev.sources);
    }
    let list = shell.dev.sources.borrow().clone().unwrap_or(Value::Null);
    let Value::Arr(items) = list else {
        ui.label("Читаю список…");
        return;
    };
    let list_h = if shell.dev.source.is_some() { (ui.available_height() * 0.3).max(100.0) } else { ui.available_height() };
    egui::ScrollArea::vertical().id_salt("obsos-sources").max_height(list_h).auto_shrink([false, false]).show(ui, |ui| {
        for item in &items {
            let kind = item.str_field("k").unwrap_or("").to_string();
            let url = item.str_field("url").unwrap_or("").to_string();
            let index = field_num(item, "i");
            let label = match kind.as_str() {
                "doc" => format!("HTML  {url}"),
                _ if url.is_empty() => format!("{}   встроенный №{} ({} байт)", kind.to_uppercase(), index + 1, field_num(item, "size")),
                _ => format!("{}   {url}", kind.to_uppercase()),
            };
            let selected = shell.dev.source.as_ref().is_some_and(|(k, i, _)| *k == kind && *i == index);
            if ui.selectable_label(selected, mono(label)).clicked() {
                let external = kind != "doc" && url.starts_with("http");
                shell.dev.source = Some((kind.clone(), index, url.clone()));
                let slot = shell.dev.source_text.clone();
                if external {
                    fetch(shell, &url, slot);
                } else {
                    *slot.lock().expect("devtools source") = None;
                    call(shell, format!("source({},{index})", quote(&kind)), move |v| {
                        if let (Value::Str(text), Ok(mut s)) = (v, slot.lock()) {
                            *s = Some((String::new(), text));
                        }
                    });
                }
            }
        }
    });
    if shell.dev.source.is_none() {
        return;
    }
    ui.separator();
    let text = shell.dev.source_text.lock().ok().and_then(|s| s.clone()).map(|(_, t)| t).unwrap_or_else(|| String::from("Загружаю…"));
    ui.horizontal_wrapped(|ui| {
        ui.add(egui::TextEdit::singleline(&mut shell.dev.filter).hint_text("найти в тексте").desired_width(200.0));
        if ui.small_button("копировать всё").clicked() {
            crate::window::clipboard_set(&text);
        }
        ui.label(RichText::new(format!("{} символов", text.len())).size(11.0).color(c.dim));
    });
    // Минифицированный код — одна строка на мегабайт: режем на части.
    let mut lines: Vec<&str> = Vec::new();
    for line in text.lines() {
        let mut rest = line;
        while rest.len() > 400 {
            let mut cut = 400;
            while !rest.is_char_boundary(cut) {
                cut -= 1;
            }
            lines.push(&rest[..cut]);
            rest = &rest[cut..];
        }
        lines.push(rest);
    }
    let f = shell.dev.filter.to_lowercase();
    if !f.is_empty() {
        lines.retain(|l| l.to_lowercase().contains(&f));
    }
    egui::ScrollArea::both().id_salt("obsos-source").auto_shrink([false, false]).show_rows(ui, 15.0, lines.len(), |ui, range| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for i in range {
            ui.horizontal(|ui| {
                ui.label(mono(format!("{:>5}", i + 1)).color(c.dim));
                ui.add(egui::Label::new(mono(lines[i])).extend().selectable(true));
            });
        }
    });
}

// ── Хранилище ───────────────────────────────────────────────────────────────

fn storage(ui: &mut Ui, shell: &mut Shell, c: &Colors) {
    if shell.dev.storage.borrow().is_none() {
        *shell.dev.storage.borrow_mut() = Some(Value::Null);
        into_slot(shell, String::from("storage()"), &shell.dev.storage);
    }
    let mut reload = false;
    ui.horizontal_wrapped(|ui| {
        for (i, name) in ["Cookie", "localStorage", "sessionStorage"].iter().enumerate() {
            if ui.selectable_label(shell.dev.area == i as u8, *name).clicked() {
                shell.dev.area = i as u8;
            }
        }
        if ui.small_button("обновить").clicked() {
            reload = true;
        }
    });
    ui.separator();
    if shell.dev.area == 0 {
        let url = shell.tabs.get(shell.active).and_then(|t| t.real_url.clone());
        let cookies = url.as_ref().filter(|u| u.scheme().starts_with("http")).map(|u| shell.servo.site_data_manager().cookies_for_url(u.clone(), servo::CookieSource::HTTP)).unwrap_or_default();
        if cookies.is_empty() {
            ui.label(RichText::new("Кук для этой страницы нет").color(c.dim));
        }
        egui::ScrollArea::both().id_salt("obsos-cookies").auto_shrink([false, false]).show(ui, |ui| {
            egui::Grid::new("obsos-cookie-grid").striped(true).num_columns(4).show(ui, |ui| {
                for name in ["Имя", "Значение", "Домен и путь", "Защита"] {
                    ui.label(RichText::new(name).strong());
                }
                ui.end_row();
                for cookie in &cookies {
                    ui.label(mono(cookie.name()).color(c.attr));
                    let short: String = cookie.value().chars().take(48).collect();
                    if ui.add(egui::Label::new(mono(short)).sense(Sense::click())).on_hover_text("щелчок — скопировать значение").clicked() {
                        crate::window::clipboard_set(cookie.value());
                    }
                    ui.label(mono(format!("{}{}", cookie.domain().unwrap_or(""), cookie.path().unwrap_or(""))));
                    let mut flags = Vec::new();
                    if cookie.secure() == Some(true) {
                        flags.push("Secure");
                    }
                    if cookie.http_only() == Some(true) {
                        flags.push("HttpOnly");
                    }
                    ui.label(mono(flags.join(" ")).color(c.dim));
                    ui.end_row();
                }
            });
        });
    } else {
        let area = if shell.dev.area == 1 { "local" } else { "session" };
        let data = shell.dev.storage.borrow().clone().unwrap_or(Value::Null);
        let entries: Vec<(String, String)> = array(&data, area).iter().map(|e| (pair(e).0.to_string(), pair(e).1.to_string())).collect();
        let mut remove = None;
        ui.horizontal_wrapped(|ui| {
            ui.add(egui::TextEdit::singleline(&mut shell.dev.storage_new.0).hint_text("ключ").desired_width(140.0));
            ui.add(egui::TextEdit::singleline(&mut shell.dev.storage_new.1).hint_text("значение").desired_width(200.0));
            if ui.button("Записать").clicked() && !shell.dev.storage_new.0.is_empty() {
                call(shell, format!("setStorage('{area}',{},{})", quote(&shell.dev.storage_new.0), quote(&shell.dev.storage_new.1)), |_| {});
                shell.dev.storage_new = Default::default();
                reload = true;
            }
            if ui.button("Очистить всё").clicked() {
                call(shell, format!("clearStorage('{area}')"), |_| {});
                reload = true;
            }
        });
        if entries.is_empty() {
            ui.label(RichText::new("Пусто").color(c.dim));
        }
        egui::ScrollArea::vertical().id_salt("obsos-storage").auto_shrink([false, false]).show(ui, |ui| {
            for (key, value) in &entries {
                ui.horizontal(|ui| {
                    if ui.small_button("×").clicked() {
                        remove = Some(key.clone());
                    }
                    if ui.add(egui::Label::new(mono(key).color(c.attr)).sense(Sense::click())).on_hover_text("щелчок — изменить").clicked() {
                        shell.dev.storage_new = (key.clone(), value.clone());
                    }
                    ui.add(egui::Label::new(mono(value)).truncate());
                });
            }
        });
        if let Some(key) = remove {
            call(shell, format!("setStorage('{area}',{},null)", quote(&key)), |_| {});
            reload = true;
        }
    }
    if reload {
        into_slot(shell, String::from("storage()"), &shell.dev.storage);
    }
}

// ── Скорость ────────────────────────────────────────────────────────────────

fn performance(ui: &mut Ui, shell: &mut Shell, c: &Colors) {
    if Instant::now() >= shell.dev.next_refresh {
        shell.dev.next_refresh = Instant::now() + Duration::from_millis(1000);
        into_slot(shell, String::from("perf()"), &shell.dev.perf);
    }
    ui.ctx().request_repaint_after(Duration::from_millis(1000));
    let Some(p) = shell.dev.perf.borrow().clone().filter(|p| *p != Value::Null) else {
        ui.label("Читаю…");
        return;
    };
    ui.label(mono(format!("Состояние: {} · элементов DOM: {} · страница открыта {:.1} с", p.str_field("ready").unwrap_or(""), field_num(&p, "elements"), field_num(&p, "now") as f64 / 1000.0)));
    ui.label(mono(format!("Рисует: {}", shell.renderer_name)));
    let mut marks: Vec<(String, f64)> = Vec::new();
    for m in array(&p, "nav").iter().chain(array(&p, "paint")) {
        if let Value::Arr(a) = m {
            if let (Some(Value::Str(name)), Some(v)) = (a.first(), a.get(1)) {
                if num(v) > 0.0 {
                    marks.push((name.clone(), num(v)));
                }
            }
        }
    }
    egui::Grid::new("obsos-perf-marks").striped(true).num_columns(2).show(ui, |ui| {
        for (name, ms) in &marks {
            ui.label(mono(name));
            ui.label(mono(format!("{ms:.0} мс")).color(c.tag));
            ui.end_row();
        }
    });
    ui.add_space(6.0);
    ui.label(RichText::new("Загрузка ресурсов").strong());
    let rows: Vec<(String, f64, f64)> = array(&p, "res")
        .iter()
        .filter_map(|r| match r {
            Value::Arr(a) if a.len() >= 5 => match &a[0] {
                Value::Str(url) if !url.starts_with("data:") => Some((url.clone(), num(&a[2]), num(&a[3]))),
                _ => None,
            },
            _ => None,
        })
        .collect();
    let end = rows.iter().map(|r| r.1 + r.2).fold(1.0, f64::max);
    egui::ScrollArea::vertical().id_salt("obsos-waterfall").auto_shrink([false, false]).show_rows(ui, 16.0, rows.len(), |ui, range| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for i in range {
            let (url, start, duration) = &rows[i];
            let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), Sense::hover());
            let name_w = (rect.width() * 0.4).max(120.0);
            let name = Rect::from_min_size(rect.min, vec2(name_w - 6.0, rect.height()));
            ui.painter().with_clip_rect(name.intersect(ui.clip_rect())).text(pos2(rect.left(), rect.center().y), Align2::LEFT_CENTER, short_name(url), FontId::monospace(11.0), ui.visuals().text_color());
            let bar_w = rect.width() - name_w - 56.0;
            let x0: f32 = rect.left() + name_w + (*start / end) as f32 * bar_w;
            let w = ((*duration / end) as f32 * bar_w).max(2.0);
            ui.painter().rect_filled(Rect::from_min_size(Pos2::new(x0, rect.top() + 4.0), vec2(w, 8.0)), 2.0, c.tag);
            ui.painter().text(pos2(rect.right(), rect.center().y), Align2::RIGHT_CENTER, format!("{duration:.0} мс"), FontId::monospace(10.5), c.dim);
            response.on_hover_text(format!("{url}\nначало {start:.0} мс, длительность {duration:.0} мс"));
        }
    });
}
