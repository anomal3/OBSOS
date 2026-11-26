//! Хром браузера: вкладки (слева, справа, сверху, снизу), панель адреса с
//! подсказками, главное меню, меню вкладки, значки и анимация загрузки.
//!
//! Геометрия левой боковой панели (`BRAND_H`, `NEW_TAB_H`, `TAB_ROW_H`) —
//! на неё целится сценарий `servo-window` FossAi.

use std::collections::HashSet;
use std::time::Instant;

use egui::{Align2, Color32, CornerRadius, FontId, Margin, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use crate::profile::{Look, TabsAt};
use crate::shell::{Action, Favicon, Infobar, Panel, Shell, TABSTRIP_H, TOOLBAR_H, TOOLBAR_H_COMPACT};
use crate::ui::{BRAND_H, NEW_TAB_H, Palette, TAB_ROW_H, clip, mix};

/// Сколько длится «полёт» файла к кнопке загрузок.
const FLIGHT: f32 = 0.75;
/// За сколько выезжает полоса загрузок.
pub const BAR_SLIDE: f32 = 0.28;

/// Состояние хрома между кадрами.
#[derive(Default)]
pub struct ChromeState {
    /// Открытое меню вкладки: номер и точка.
    tab_menu: Option<(usize, Pos2)>,
    /// Перетаскиваемая вкладка.
    drag: Option<usize>,
    suggestions: Vec<Suggestion>,
    selected: Option<usize>,
    /// Что набрано самим (стрелки подставляют в строку подсказки).
    typed: String,
    /// Список подсказок открыт.
    suggesting: bool,
    over_suggestions: bool,
    menu_button: Option<Rect>,
    download_button: Option<Rect>,
    /// Когда открыть список загрузок после новой загрузки.
    popup_at: Option<Instant>,
    /// Летящие к кнопке загрузок значки: когда и откуда.
    flights: Vec<(Instant, Pos2)>,
    known_downloads: Option<HashSet<u64>>,
    /// Когда появилась полоса загрузок.
    pub bar_since: Option<Instant>,
}

impl ChromeState {
    /// Вкладка, которую тащат мышью (номер с учётом перестановок).
    pub fn dragged_tab(&self) -> Option<usize> {
        self.drag
    }

    /// Список загрузок вот-вот откроется сам.
    pub fn popup_due(&self) -> bool {
        self.popup_at.is_some()
    }
}

#[derive(Clone)]
struct Suggestion {
    url: String,
    title: String,
    bookmark: bool,
}

// ── Значки ──────────────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
pub enum Icon {
    Back,
    Forward,
    Reload,
    Star(bool),
    Menu,
    Key,
    Plus,
    Close,
    /// Щит блокировки рекламы: включена (true) или выключена на сайте.
    Shield(bool),
    /// Режим чтения (раскрытая книга).
    Book,
    Download,
    Lock,
    Info,
    Search,
    Gear,
    Clock,
    Incognito,
    Sync,
    Devices,
    Code,
    Script,
    Person,
    Power,
    Find,
    External,
    /// Положение вкладок: где закрашенная полоса.
    Layout(TabsAt),
}

pub fn paint_icon(painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    let s = Stroke::new(1.7_f32, color);
    let c = rect.center();
    let r = 7.0;
    let p = |x: f32, y: f32| c + vec2(x, y);
    let line = |a: Pos2, b: Pos2| {
        painter.line_segment([a, b], s);
    };
    let path = |pts: Vec<Pos2>| {
        painter.add(egui::Shape::line(pts, s));
    };
    let closed = |pts: Vec<Pos2>| {
        painter.add(egui::Shape::closed_line(pts, s));
    };
    let arc = |center: Pos2, radius: f32, from: f32, to: f32| {
        let pts: Vec<Pos2> = (0..=24)
            .map(|i| {
                let a = from + (to - from) * i as f32 / 24.0;
                center + vec2(a.cos() * radius, a.sin() * radius)
            })
            .collect();
        painter.add(egui::Shape::line(pts, s));
    };
    match icon {
        Icon::Back | Icon::Forward => {
            let d = if matches!(icon, Icon::Back) { -1.0 } else { 1.0 };
            line(p(-r * d, 0.0), p(r * d, 0.0));
            path(vec![p(r * d * 0.15, -r * 0.8), p(r * d, 0.0), p(r * d * 0.15, r * 0.8)]);
        }
        Icon::Reload => {
            arc(c, r, -0.6, 4.4);
            let end = p(4.4f32.cos() * r, 4.4f32.sin() * r);
            line(end, end + vec2(4.5, -1.0));
            line(end, end + vec2(0.5, -4.5));
        }
        Icon::Sync => {
            arc(c, r, 3.5, 5.7);
            arc(c, r, 0.35, 2.55);
            path(vec![p(6.5, -7.0), p(6.0, -3.5), p(2.5, -4.0)]);
            path(vec![p(-6.5, 7.0), p(-6.0, 3.5), p(-2.5, 4.0)]);
        }
        Icon::Star(filled) => {
            let pts: Vec<Pos2> = (0..10)
                .map(|i| {
                    let a = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
                    let rr = if i % 2 == 0 { r + 1.0 } else { r * 0.45 };
                    c + vec2(a.cos() * rr, a.sin() * rr)
                })
                .collect();
            if filled {
                // Невыпуклую звезду закрашиваем лучами от центра.
                for i in 0..10 {
                    painter.add(egui::Shape::convex_polygon(vec![c, pts[i], pts[(i + 1) % 10]], color, Stroke::NONE));
                }
            } else {
                closed(pts);
            }
        }
        Icon::Menu => {
            for dy in [-5.5, 0.0, 5.5] {
                painter.circle_filled(p(0.0, dy), 1.8, color);
            }
        }
        Icon::Key => {
            painter.circle_stroke(p(-3.5, 0.0), 3.5, s);
            line(p(0.0, 0.0), p(r, 0.0));
            line(p(r - 1.0, 0.0), p(r - 1.0, 3.5));
            line(p(r - 4.0, 0.0), p(r - 4.0, 3.0));
        }
        Icon::Plus => {
            line(p(-6.0, 0.0), p(6.0, 0.0));
            line(p(0.0, -6.0), p(0.0, 6.0));
        }
        Icon::Shield(on) => {
            let pts = vec![p(0.0, -r), p(r * 0.85, -r * 0.6), p(r * 0.7, r * 0.3), p(0.0, r), p(-r * 0.7, r * 0.3), p(-r * 0.85, -r * 0.6)];
            if on {
                painter.add(egui::Shape::convex_polygon(pts, color.gamma_multiply(0.35), s));
            } else {
                closed(pts);
                line(p(-r, r), p(r, -r));
            }
        }
        Icon::Book => {
            line(p(0.0, -r * 0.6), p(0.0, r * 0.8));
            closed(vec![p(0.0, -r * 0.6), p(-r, -r * 0.8), p(-r, r * 0.6), p(0.0, r * 0.8)]);
            closed(vec![p(0.0, -r * 0.6), p(r, -r * 0.8), p(r, r * 0.6), p(0.0, r * 0.8)]);
        }
        Icon::Close => {
            let k = r * 0.6;
            line(p(-k, -k), p(k, k));
            line(p(-k, k), p(k, -k));
        }
        Icon::Download => {
            line(p(0.0, -7.0), p(0.0, 2.5));
            path(vec![p(-4.0, -1.5), p(0.0, 2.8), p(4.0, -1.5)]);
            path(vec![p(-7.0, 3.0), p(-7.0, 6.5), p(7.0, 6.5), p(7.0, 3.0)]);
        }
        Icon::Lock => {
            painter.rect_stroke(Rect::from_min_max(p(-5.0, -1.0), p(5.0, 6.0)), 1.5, s, StrokeKind::Middle);
            arc(p(0.0, -2.0), 3.2, std::f32::consts::PI, std::f32::consts::TAU);
        }
        Icon::Info => {
            painter.circle_stroke(c, 6.5, s);
            line(p(0.0, -0.5), p(0.0, 3.5));
            painter.circle_filled(p(0.0, -3.2), 1.2, color);
        }
        Icon::Search | Icon::Find => {
            painter.circle_stroke(p(-1.5, -1.5), 4.8, s);
            line(p(2.2, 2.2), p(6.5, 6.5));
        }
        Icon::Gear => {
            painter.circle_stroke(c, 5.0, s);
            painter.circle_stroke(c, 1.8, s);
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::FRAC_PI_4;
                line(c + vec2(a.cos() * 5.0, a.sin() * 5.0), c + vec2(a.cos() * 7.6, a.sin() * 7.6));
            }
        }
        Icon::Clock => {
            painter.circle_stroke(c, r, s);
            path(vec![p(0.0, -4.0), p(0.0, 0.0), p(3.5, 2.0)]);
        }
        Icon::Incognito => {
            line(p(-8.0, -1.0), p(8.0, -1.0));
            path(vec![p(-5.0, -1.0), p(-3.5, -6.5), p(3.5, -6.5), p(5.0, -1.0)]);
            painter.circle_stroke(p(-3.8, 3.5), 2.6, s);
            painter.circle_stroke(p(3.8, 3.5), 2.6, s);
            line(p(-1.2, 3.5), p(1.2, 3.5));
        }
        Icon::Devices => {
            painter.rect_stroke(Rect::from_min_max(p(-8.0, -6.0), p(4.0, 3.0)), 1.0, s, StrokeKind::Middle);
            line(p(-6.0, 6.0), p(1.0, 6.0));
            painter.rect_stroke(Rect::from_min_max(p(2.0, -2.0), p(8.0, 7.0)), 1.5, s, StrokeKind::Middle);
        }
        Icon::Code => {
            path(vec![p(-3.5, -5.0), p(-8.0, 0.0), p(-3.5, 5.0)]);
            path(vec![p(3.5, -5.0), p(8.0, 0.0), p(3.5, 5.0)]);
            line(p(1.5, -6.5), p(-1.5, 6.5));
        }
        Icon::Script => {
            closed(vec![p(-5.5, -7.0), p(2.5, -7.0), p(6.0, -3.5), p(6.0, 7.0), p(-5.5, 7.0)]);
            line(p(-2.5, -1.0), p(3.0, -1.0));
            line(p(-2.5, 3.0), p(3.0, 3.0));
        }
        Icon::Person => {
            painter.circle_stroke(p(0.0, -3.0), 3.2, s);
            arc(p(0.0, 8.0), 6.5, std::f32::consts::PI + 0.15, std::f32::consts::TAU - 0.15);
        }
        Icon::Power => {
            arc(c, 6.0, -1.0, 4.14);
            line(p(0.0, -8.0), p(0.0, -1.5));
        }
        Icon::External => {
            path(vec![p(-1.0, -6.0), p(-6.0, -6.0), p(-6.0, 6.0), p(6.0, 6.0), p(6.0, 1.0)]);
            line(p(0.0, 0.0), p(6.5, -6.5));
            path(vec![p(2.0, -6.5), p(6.5, -6.5), p(6.5, -2.0)]);
        }
        Icon::Layout(at) => {
            let frame = Rect::from_center_size(c, vec2(16.0, 13.0));
            painter.rect_stroke(frame, 2.0, s, StrokeKind::Middle);
            let bar = match at {
                TabsAt::Left => Rect::from_min_size(frame.min, vec2(5.0, frame.height())),
                TabsAt::Right => Rect::from_min_size(pos2(frame.max.x - 5.0, frame.min.y), vec2(5.0, frame.height())),
                TabsAt::Top => Rect::from_min_size(frame.min, vec2(frame.width(), 4.0)),
                TabsAt::Bottom => Rect::from_min_size(pos2(frame.min.x, frame.max.y - 4.0), vec2(frame.width(), 4.0)),
            };
            painter.rect_filled(bar, 1.0, color);
        }
    }
}

pub fn icon_button(ui: &mut Ui, icon: Icon, enabled: bool, tip: &str, p: &Palette) -> bool {
    icon_button_response(ui, icon, enabled, tip, p).clicked() && enabled
}

fn icon_button_response(ui: &mut Ui, icon: Icon, enabled: bool, tip: &str, p: &Palette) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(30.0, 28.0), if enabled { Sense::click() } else { Sense::hover() });
    if response.hovered() && enabled {
        let fill = if response.is_pointer_button_down_on() { mix(p.hover, p.text, 0.1) } else { p.hover };
        ui.painter().rect_filled(rect, 4.0, fill);
    }
    let color = if enabled { p.text } else { mix(p.text, p.toolbar, 0.65) };
    let color = if matches!(icon, Icon::Star(true)) { p.accent2 } else { color };
    paint_icon(ui.painter(), rect, icon, color);
    response.on_hover_text(tip)
}

/// Крутящаяся дуга загрузки.
fn spinner(painter: &egui::Painter, center: Pos2, radius: f32, time: f64, color: Color32) {
    let start = (time * 6.0) as f32;
    let pts: Vec<Pos2> = (0..=18)
        .map(|i| {
            let a = start + i as f32 * 0.24;
            center + vec2(a.cos() * radius, a.sin() * radius)
        })
        .collect();
    painter.add(egui::Shape::line(pts, Stroke::new(2.0_f32, color)));
}

// ── Вкладки ─────────────────────────────────────────────────────────────────

pub fn tabs(ui: &mut Ui, shell: &mut Shell, look: &Look, p: &Palette, actions: &mut Vec<Action>) {
    if !ui.input(|i| i.pointer.primary_down()) {
        shell.chrome.drag = None;
    }
    if shell.tabs.iter().any(|t| t.loading) {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
    }
    if look.tabs.is_side() {
        sidebar(ui, shell, look, p, actions);
    } else {
        tabstrip(ui, shell, look, p, actions);
    }
}

fn brand(painter: &egui::Painter, rect: Rect, p: &Palette) -> Rect {
    painter.circle_filled(pos2(rect.left() + 10.0, rect.center().y), 7.0, p.accent);
    painter.circle_filled(pos2(rect.left() + 20.0, rect.center().y - 5.0), 3.0, p.accent2);
    painter.text(pos2(rect.left() + 30.0, rect.center().y), Align2::LEFT_CENTER, "OBSOS", FontId::proportional(17.0), p.text)
}

fn sidebar(ui: &mut Ui, shell: &mut Shell, look: &Look, p: &Palette, actions: &mut Vec<Action>) {
    let panel = if look.tabs == TabsAt::Right { egui::Panel::right("obsos-tabs-right") } else { egui::Panel::left("obsos-tabs") };
    panel
        .exact_size(look.sidebar_width as f32)
        .resizable(false)
        .show_separator_line(false)
        .frame(egui::Frame::new().fill(p.sidebar).inner_margin(Margin::symmetric(8, 0)))
        .show_inside(ui, |ui| {
            // Прежние промежутки: на геометрию панели целится стенд FossAi
            // («Новая вкладка» и первая вкладка).
            ui.spacing_mut().item_spacing = vec2(8.0, 3.0);
            let width = ui.available_width();
            let (rect, _) = ui.allocate_exact_size(vec2(width, BRAND_H), Sense::hover());
            let name = brand(ui.painter(), rect, p);
            if shell.profile.private {
                // Окно инкогнито видно сразу: метка рядом с именем.
                let badge = Rect::from_min_size(pos2(name.right() + 8.0, rect.center().y - 10.0), vec2(80.0, 20.0));
                ui.painter().rect_filled(badge, 3.0, mix(p.accent, p.sidebar, 0.35));
                ui.painter().text(badge.center(), Align2::CENTER_CENTER, "инкогнито", FontId::proportional(11.5), Color32::WHITE);
            }
            let new_tab = egui::Button::new(RichText::new("+  Новая вкладка").color(p.text)).fill(mix(p.sidebar, p.text, 0.08)).corner_radius(4);
            if ui.add_sized([width, NEW_TAB_H], new_tab).on_hover_text("Ctrl+T").clicked() {
                actions.push(Action::NewTab);
            }
            ui.add_space(6.0);
            ui.spacing_mut().item_spacing.y = 2.0;
            let rows_h = ui.available_height() - 40.0;
            egui::ScrollArea::vertical().max_height(rows_h.max(60.0)).auto_shrink([false, false]).show(ui, |ui| {
                for i in 0..shell.tabs.len() {
                    tab(ui, shell, i, vec2(width, TAB_ROW_H), look, p, actions);
                }
            });
            // Внизу: синхронизация, положение вкладок, настройки.
            ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    if icon_button(ui, Icon::Gear, true, "Настройки (Ctrl+,)", p) {
                        actions.push(Action::OpenPage("settings"));
                    }
                    let other = if look.tabs == TabsAt::Left { TabsAt::Right } else { TabsAt::Left };
                    if icon_button(ui, Icon::Layout(TabsAt::Top), true, "Вкладки сверху", p) {
                        actions.push(Action::SetTabs(TabsAt::Top));
                    }
                    if icon_button(ui, Icon::Layout(other), true, if other == TabsAt::Right { "Вкладки справа" } else { "Вкладки слева" }, p) {
                        actions.push(Action::SetTabs(other));
                    }
                    let synced = shell.account().is_some();
                    let label = if shell.syncer.busy > 0 { "синхронизация…" } else if synced { "синхронизация" } else { "без синхронизации" };
                    if ui.add(egui::Button::new(RichText::new(label).size(11.5).color(p.dim)).frame(false).truncate()).clicked() {
                        actions.push(Action::Panel(Panel::Sync));
                    }
                });
            });
        });
}

fn tabstrip(ui: &mut Ui, shell: &mut Shell, look: &Look, p: &Palette, actions: &mut Vec<Action>) {
    let bottom = look.tabs == TabsAt::Bottom;
    let panel = if bottom { egui::Panel::bottom("obsos-strip-bottom") } else { egui::Panel::top("obsos-strip") };
    // Вкладки вплотную к панели адреса: активная сливается с ней.
    let margin = if bottom { Margin { left: 6, right: 6, top: 0, bottom: 4 } } else { Margin { left: 6, right: 6, top: 4, bottom: 0 } };
    panel
        .exact_size(TABSTRIP_H as f32)
        .resizable(false)
        .show_separator_line(false)
        .frame(egui::Frame::new().fill(p.sidebar).inner_margin(margin))
        .show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                if shell.profile.private {
                    let (badge, _) = ui.allocate_exact_size(vec2(80.0, 24.0), Sense::hover());
                    ui.painter().rect_filled(badge, 3.0, mix(p.accent, p.sidebar, 0.35));
                    ui.painter().text(badge.center(), Align2::CENTER_CENTER, "инкогнито", FontId::proportional(11.5), Color32::WHITE);
                    ui.add_space(6.0);
                }
                let n = shell.tabs.len().max(1) as f32;
                let width = ((ui.available_width() - 44.0) / n).clamp(36.0, 232.0);
                for i in 0..shell.tabs.len() {
                    tab(ui, shell, i, vec2(width, TABSTRIP_H as f32 - 4.0), look, p, actions);
                }
                ui.add_space(4.0);
                if icon_button(ui, Icon::Plus, true, "Новая вкладка (Ctrl+T)", p) {
                    actions.push(Action::NewTab);
                }
            });
        });
}

/// Одна вкладка (строка боковой панели или язычок полосы).
fn tab(ui: &mut Ui, shell: &mut Shell, i: usize, size: egui::Vec2, look: &Look, p: &Palette, actions: &mut Vec<Action>) {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let active = i == shell.active;
    let side = look.tabs.is_side();
    let time = ui.input(|input| input.time);
    // Значок страницы: текстура делается при первом рисовании.
    if let Some(Favicon::Pending(image)) = &shell.tabs[i].favicon {
        let texture = ui.ctx().load_texture(format!("favicon-{i}-{time}"), image.clone(), egui::TextureOptions::LINEAR);
        shell.tabs[i].favicon = Some(Favicon::Ready(texture));
    }
    let tab = &shell.tabs[i];
    let narrow = rect.width() < 70.0;
    let close = Rect::from_center_size(pos2(rect.right() - 14.0, rect.center().y), vec2(18.0, 18.0));
    let show_close = (!narrow && (response.hovered() || active)) || (narrow && active);
    let over_close = show_close && response.hover_pos().is_some_and(|q| close.contains(q));
    // Горизонтальная вкладка: активная — цвета панели адреса, остальные без
    // заливки и разделены чертой; боковая — строка с меткой акцента.
    let fill = if active {
        if side { p.active } else { p.toolbar }
    } else if response.hovered() {
        if side { p.hover } else { mix(p.sidebar, p.toolbar, 0.55) }
    } else {
        p.sidebar
    };
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    let radius: CornerRadius = match look.tabs {
        TabsAt::Top => CornerRadius { nw: 5, ne: 5, sw: 0, se: 0 },
        TabsAt::Bottom => CornerRadius { nw: 0, ne: 0, sw: 5, se: 5 },
        _ => CornerRadius::same(3),
    };
    painter.rect_filled(rect, radius, fill);
    if side && active {
        let x = if look.tabs == TabsAt::Right { rect.right() - 2.0 } else { rect.left() };
        painter.rect_filled(Rect::from_min_size(pos2(x, rect.top() + 6.0), vec2(2.0, rect.height() - 12.0)), 0.0, p.accent2);
    }
    if !side && !active && !response.hovered() && i + 1 != shell.active {
        painter.line_segment([pos2(rect.right() - 0.5, rect.top() + 8.0), pos2(rect.right() - 0.5, rect.bottom() - 8.0)], Stroke::new(1.0_f32, mix(p.sidebar, p.text, 0.18)));
    }
    let icon_at = Rect::from_center_size(pos2(rect.left() + 17.0, rect.center().y), vec2(16.0, 16.0));
    if tab.loading {
        spinner(&painter, icon_at.center(), 6.0, time, p.accent2);
    } else if let Some(Favicon::Ready(texture)) = &tab.favicon {
        painter.image(texture.id(), icon_at, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
    } else if tab.real_url.as_ref().is_some_and(|u| u.scheme() == "obsos") || tab.url.is_empty() {
        // Свои страницы — значок OBSOS.
        painter.circle_filled(icon_at.center() + vec2(-1.0, 1.0), 6.0, p.accent);
        painter.circle_filled(icon_at.center() + vec2(5.0, -4.0), 2.6, p.accent2);
    } else {
        let letter = tab.title.chars().find(|c| c.is_alphanumeric()).unwrap_or('•').to_uppercase().to_string();
        painter.circle_filled(icon_at.center(), 8.0, mix(p.sidebar, p.accent, 0.45));
        painter.text(icon_at.center(), Align2::CENTER_CENTER, letter, FontId::proportional(11.0), p.text);
    }
    if !narrow {
        let text_left = rect.left() + 32.0;
        let text_right = if show_close { close.left() - 4.0 } else { rect.right() - 8.0 };
        let color = if active { p.text } else { p.dim };
        let mut job = egui::text::LayoutJob::simple_singleline(tab.title.clone(), FontId::proportional(13.5), color);
        job.wrap.max_width = (text_right - text_left).max(8.0);
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        let galley = painter.layout_job(job);
        painter.galley(pos2(text_left, rect.center().y - galley.size().y / 2.0), galley, color);
    }
    if show_close {
        if over_close {
            painter.rect_filled(close, 3.0, mix(fill, p.text, 0.14));
        }
        paint_icon(&painter, close, Icon::Close, if over_close { p.text } else { p.dim });
    }
    // Перетаскивание: вкладка меняется местами с той, над которой курсор.
    if response.drag_started() {
        shell.chrome.drag = Some(i);
        actions.push(Action::Activate(i));
    }
    if let Some(from) = shell.chrome.drag {
        if from != i && ui.input(|input| input.pointer.hover_pos()).is_some_and(|q| rect.contains(q)) {
            let q = ui.input(|input| input.pointer.hover_pos()).unwrap_or(rect.center());
            let past = if side {
                (q.y > rect.center().y) == (i > from)
            } else {
                (q.x > rect.center().x) == (i > from)
            };
            if past {
                actions.push(Action::MoveTab(from, i));
                shell.chrome.drag = Some(i);
            }
        }
    }
    if response.clicked() {
        actions.push(if over_close { Action::Close(i) } else { Action::Activate(i) });
    }
    if response.middle_clicked() {
        actions.push(Action::Close(i));
    }
    if response.secondary_clicked() {
        shell.chrome.tab_menu = Some((i, response.interact_pointer_pos().unwrap_or(rect.center())));
    }
    if !response.dragged() {
        response.on_hover_text(shell.tabs[i].title.clone());
    }
}

// ── Всплывающие меню ────────────────────────────────────────────────────────

fn popup_frame(p: &Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(p.toolbar)
        .stroke(Stroke::new(1.0_f32, mix(p.toolbar, p.text, 0.18)))
        .corner_radius(4)
        .shadow(egui::epaint::Shadow { offset: [0, 4], blur: 14, spread: 0, color: Color32::from_black_alpha(80) })
        .inner_margin(Margin::same(4))
}

/// Строка меню: значок, название, сочетание клавиш.
fn item(ui: &mut Ui, icon: Option<Icon>, text: &str, keys: &str, p: &Palette) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, 3.0, mix(p.toolbar, p.text, 0.1));
    }
    if let Some(icon) = icon {
        paint_icon(ui.painter(), Rect::from_center_size(pos2(rect.left() + 18.0, rect.center().y), vec2(18.0, 18.0)), icon, p.dim);
    }
    ui.painter().text(pos2(rect.left() + 38.0, rect.center().y), Align2::LEFT_CENTER, text, FontId::proportional(13.5), p.text);
    ui.painter().text(pos2(rect.right() - 10.0, rect.center().y), Align2::RIGHT_CENTER, keys, FontId::proportional(11.5), p.dim);
    response.clicked()
}

fn separator(ui: &mut Ui, p: &Palette) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 7.0), Sense::hover());
    ui.painter().line_segment([pos2(rect.left() + 8.0, rect.center().y), pos2(rect.right() - 8.0, rect.center().y)], Stroke::new(1.0_f32, mix(p.sidebar, p.text, 0.14)));
}

/// Меню вкладки (правая кнопка).
pub fn tab_menu(ctx: &egui::Context, shell: &mut Shell, p: &Palette, actions: &mut Vec<Action>) {
    let Some((index, at)) = shell.chrome.tab_menu else { return };
    if index >= shell.tabs.len() {
        shell.chrome.tab_menu = None;
        return;
    }
    let look = shell.look().tabs;
    let screen = ctx.content_rect();
    let at = pos2(at.x.min(screen.right() - 270.0).max(4.0), at.y.min(screen.bottom() - 250.0).max(4.0));
    let mut done = false;
    let area = egui::Area::new(egui::Id::new("obsos-tab-menu")).order(egui::Order::Foreground).fixed_pos(at).show(ctx, |ui| {
        popup_frame(p).show(ui, |ui| {
            ui.set_width(246.0);
            ui.spacing_mut().item_spacing.y = 1.0;
            let mut choose = |action: Action| {
                actions.push(action);
                done = true;
            };
            if item(ui, Some(Icon::Plus), "Новая вкладка", "Ctrl+T", p) {
                choose(Action::NewTab);
            }
            if item(ui, Some(Icon::Reload), "Обновить", "F5", p) {
                choose(Action::ReloadTab(index));
            }
            if item(ui, None, "Дублировать", "", p) {
                choose(Action::Duplicate(index));
            }
            if item(ui, Some(Icon::Close), "Закрыть", "Ctrl+W", p) {
                choose(Action::Close(index));
            }
            if item(ui, None, "Закрыть остальные", "", p) {
                choose(Action::CloseOthers(index));
            }
            if item(ui, None, "Вернуть закрытую", "Ctrl+Shift+T", p) {
                choose(Action::Reopen);
            }
            separator(ui, p);
            ui.label(RichText::new("  Вкладки").size(11.5).color(p.dim));
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                for at in TabsAt::ALL {
                    let (rect, response) = ui.allocate_exact_size(vec2(56.0, 30.0), Sense::click());
                    if at == look || response.hovered() {
                        ui.painter().rect_filled(rect, 3.0, if at == look { p.active } else { p.hover });
                    }
                    paint_icon(ui.painter(), rect, Icon::Layout(at), if at == look { p.accent2 } else { p.dim });
                    if response.on_hover_text(at.title()).clicked() {
                        choose(Action::SetTabs(at));
                    }
                }
            });
        });
    });
    let outside = ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|q| !area.response.rect.contains(q)));
    if done || outside || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        shell.chrome.tab_menu = None;
    }
}

/// Главное меню под кнопкой «⋮».
pub fn main_menu(ctx: &egui::Context, shell: &mut Shell, p: &Palette, actions: &mut Vec<Action>) {
    let screen = ctx.content_rect();
    let button = shell.chrome.menu_button.unwrap_or(Rect::from_min_size(pos2(screen.right() - 40.0, 8.0), vec2(30.0, 28.0)));
    let top = button.bottom() + 4.0;
    let open = ctx.animate_bool_with_time(egui::Id::new("obsos-menu-open"), true, 0.14);
    // Не `clamp`: в узком окне нижняя граница больше верхней — паника.
    let right = button.right().min(screen.right() - 4.0).max(320.0);
    let at = pos2(right, top - 8.0 * (1.0 - open));
    let mut close = false;
    egui::Area::new(egui::Id::new("obsos-menu")).order(egui::Order::Foreground).pivot(Align2::RIGHT_TOP).fixed_pos(at).default_height(screen.height()).show(ctx, |ui| {
        ui.set_opacity(open);
        popup_frame(p).show(ui, |ui| {
            ui.set_width(296.0);
            // Полоска прокрутки не показывается: она подрезала правые кнопки
            // (в низком окне меню крутится колесом).
            egui::ScrollArea::vertical().scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).max_height((screen.bottom() - top - 24.0).max(200.0)).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                let mut run = |action: Action| {
                    actions.push(action);
                    close = true;
                };
                if item(ui, Some(Icon::Plus), "Новая вкладка", "Ctrl+T", p) {
                    run(Action::NewTab);
                }
                if shell.gpu_active && item(ui, Some(Icon::External), "Новое окно", "Ctrl+N", p) {
                    run(Action::NewWindow);
                }
                if item(ui, Some(Icon::Incognito), "Новое окно инкогнито", "Ctrl+Shift+N", p) {
                    run(Action::PrivateWindow);
                }
                separator(ui, p);
                if item(ui, Some(Icon::Clock), "История", "Ctrl+H", p) {
                    run(Action::OpenPage("history"));
                }
                if item(ui, Some(Icon::Download), "Загрузки", "Ctrl+J", p) {
                    run(Action::OpenPage("downloads"));
                }
                if item(ui, Some(Icon::Star(false)), "Закладки", "Ctrl+Shift+O", p) {
                    run(Action::OpenPage("bookmarks"));
                }
                let mut panel = None;
                for (target, icon, text) in [
                    (Panel::Passwords, Icon::Key, "Пароли"),
                    (Panel::RemoteTabs, Icon::Devices, "Вкладки других устройств"),
                    (Panel::Sync, Icon::Sync, "Синхронизация"),
                ] {
                    if item(ui, Some(icon), text, "", p) {
                        panel = Some(target);
                    }
                }
                separator(ui, p);
                let zoom = shell.tabs.get(shell.active).map(|t| t.zoom).unwrap_or(1.0);
                ui.horizontal(|ui| {
                    ui.add_space(38.0);
                    ui.label(RichText::new("Масштаб").size(13.5));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add_sized([30.0, 26.0], egui::Button::new("+")).clicked() {
                            actions.push(Action::Zoom(1.1));
                        }
                        if ui.add_sized([52.0, 26.0], egui::Button::new(format!("{:.0}%", zoom * 100.0))).on_hover_text("Сбросить (Ctrl+0)").clicked() {
                            actions.push(Action::Zoom(0.0));
                        }
                        if ui.add_sized([30.0, 26.0], egui::Button::new("−")).clicked() {
                            actions.push(Action::Zoom(1.0 / 1.1));
                        }
                    });
                });
                let current = shell.look().tabs;
                ui.horizontal(|ui| {
                    ui.add_space(38.0);
                    ui.label(RichText::new("Вкладки").size(13.5));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        for at in TabsAt::ALL.into_iter().rev() {
                            let (rect, response) = ui.allocate_exact_size(vec2(30.0, 28.0), Sense::click());
                            if at == current || response.hovered() {
                                ui.painter().rect_filled(rect, 3.0, if at == current { p.active } else { p.hover });
                            }
                            paint_icon(ui.painter(), rect, Icon::Layout(at), if at == current { p.accent2 } else { p.dim });
                            if response.on_hover_text(at.title()).clicked() {
                                actions.push(Action::SetTabs(at));
                            }
                        }
                    });
                });
                separator(ui, p);
                let mut run = |action: Action| {
                    actions.push(action);
                    close = true;
                };
                if item(ui, Some(Icon::Find), "Найти на странице", "Ctrl+F", p) {
                    run(Action::FindOpen);
                }
                if item(ui, Some(Icon::Book), "Режим чтения", "F9", p) {
                    run(Action::Reader);
                }
                if item(ui, None, "На весь экран", "F11", p) {
                    run(Action::Fullscreen);
                }
                if item(ui, Some(Icon::External), "Открыть страницу в Chromium", "", p) {
                    run(Action::Chromium(String::new()));
                }
                if item(ui, Some(Icon::Code), "Инструменты разработчика", "F12", p) {
                    panel = Some(Panel::DevTools);
                }
                if item(ui, Some(Icon::Script), "Скрипты и стили", "", p) {
                    panel = Some(Panel::Scripts);
                }
                if item(ui, Some(Icon::Person), "Профили", "", p) {
                    panel = Some(Panel::Profiles);
                }
                separator(ui, p);
                if item(ui, Some(Icon::Gear), "Настройки", "Ctrl+,", p) {
                    run(Action::OpenPage("settings"));
                }
                if item(ui, Some(Icon::Info), "О программе", "", p) {
                    run(Action::OpenPage("about"));
                }
                if item(ui, Some(Icon::Power), "Выход", "Ctrl+Shift+Q", p) {
                    run(Action::Quit);
                }
                if let Some(panel) = panel {
                    actions.push(Action::Panel(panel));
                }
                ui.add_space(2.0);
                ui.label(RichText::new(format!("   Профиль: {} · {}", shell.profile.name, shell.profile.settings.device_name)).size(11.0).color(p.dim));
            });
        });
    });
    if close {
        shell.panel = None;
    }
}

// ── Панель адреса ───────────────────────────────────────────────────────────

/// Подсказки: закладки и история, где есть все набранные слова; впереди —
/// совпадения с начала адреса сайта и частые.
fn suggest(shell: &Shell, text: &str) -> Vec<Suggestion> {
    let text = text.trim().to_lowercase();
    if text.is_empty() {
        return Vec::new();
    }
    let words: Vec<&str> = text.split_whitespace().collect();
    let bare = |url: &str| url.trim_start_matches("https://").trim_start_matches("http://").trim_start_matches("www.").to_lowercase();
    let score = |url: &str, title: &str, base: u64| -> Option<u64> {
        let (u, t) = (url.to_lowercase(), title.to_lowercase());
        if !words.iter().all(|w| u.contains(w) || t.contains(w)) {
            return None;
        }
        let mut value = base;
        if bare(url).starts_with(&text) {
            value += 10_000;
        }
        // Короткие адреса (корень сайта) — выше длинных.
        Some(value + 200u64.saturating_sub(url.len() as u64))
    };
    let mut found: Vec<(u64, Suggestion)> = Vec::new();
    for b in &shell.profile.bookmarks {
        if let Some(value) = score(&b.url, &b.title, 1_000) {
            found.push((value, Suggestion { url: b.url.clone(), title: b.title.clone(), bookmark: true }));
        }
    }
    for v in shell.profile.history.iter().take(3000) {
        if v.url.starts_with("obsos://") || v.url.starts_with("data:") {
            continue;
        }
        if let Some(value) = score(&v.url, &v.title, v.count.min(200) * 20) {
            if !found.iter().any(|(_, s)| s.url == v.url) {
                found.push((value, Suggestion { url: v.url.clone(), title: v.title.clone(), bookmark: false }));
            }
        }
    }
    found.sort_by(|a, b| b.0.cmp(&a.0));
    found.into_iter().take(7).map(|(_, s)| s).collect()
}

pub fn toolbar(ui: &mut Ui, shell: &mut Shell, look: &Look, p: &Palette, actions: &mut Vec<Action>) {
    let height = if look.compact { TOOLBAR_H_COMPACT } else { TOOLBAR_H } as f32;
    let address_id = egui::Id::new("obsos-address");
    let focused = ui.memory(|m| m.has_focus(address_id));
    // Стрелки по подсказкам — раньше, чем их заберёт поле ввода.
    if focused && shell.chrome.suggesting && !shell.chrome.suggestions.is_empty() {
        let (down, up) = ui.input_mut(|i| (i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown), i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)));
        if down || up {
            let n = shell.chrome.suggestions.len();
            let next = match (shell.chrome.selected, down) {
                (None, true) => Some(0),
                (None, false) => Some(n - 1),
                (Some(i), true) => (i + 1 < n).then_some(i + 1),
                (Some(i), false) => i.checked_sub(1),
            };
            shell.chrome.selected = next;
            shell.address = next.map(|i| shell.chrome.suggestions[i].url.clone()).unwrap_or_else(|| shell.chrome.typed.clone());
        }
    }
    let mut field = Rect::NOTHING;
    egui::Panel::top("obsos-bar")
        .exact_size(height)
        .resizable(false)
        .show_separator_line(false)
        .frame(egui::Frame::new().fill(p.toolbar).inner_margin(Margin::symmetric(8, if look.compact { 4 } else { 7 })))
        .show_inside(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let (back, forward) = shell.active_view().map(|w| (w.can_go_back(), w.can_go_forward())).unwrap_or((false, false));
                if icon_button(ui, Icon::Back, back, "Назад (Alt+←)", p) {
                    actions.push(Action::Back);
                }
                if icon_button(ui, Icon::Forward, forward, "Вперёд (Alt+→)", p) {
                    actions.push(Action::Forward);
                }
                if icon_button(ui, Icon::Reload, true, "Обновить (F5)", p) {
                    actions.push(Action::Reload);
                }
                ui.add_space(2.0);
                let url = shell.tabs.get(shell.active).and_then(|t| t.real_url.clone());
                let origin = url.as_ref().and_then(crate::vault::origin_of);
                let has_logins = origin.as_ref().is_some_and(|o| !shell.profile.logins_for(o).is_empty());
                let marked = url.as_ref().is_some_and(|u| shell.profile.is_bookmarked(u.as_str()));
                let web = url.as_ref().is_some_and(|u| u.scheme().starts_with("http"));
                let items = shell.delegate.downloads.snapshot();
                let has_downloads = !items.is_empty() || !shell.chrome.flights.is_empty();
                // Кнопки справа от поля: щит, загрузки, меню.
                let gap = ui.spacing().item_spacing.x;
                let right = (30.0 + gap) * (1.0 + web as u8 as f32 + has_downloads as u8 as f32) + 4.0;

                if shell.focus_address {
                    ui.memory_mut(|m| m.request_focus(address_id));
                    shell.focus_address = false;
                    // Как в Chromium: Ctrl+L / F6 / Alt+D выделяют весь адрес —
                    // набранное заменяет его, а не дописывается в конец.
                    let mut state = egui::text_edit::TextEditState::load(ui.ctx(), address_id).unwrap_or_default();
                    let len = shell.address.chars().count();
                    state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(len))));
                    state.store(ui.ctx(), address_id);
                }

                // Поле адреса со значком защиты слева и кнопками страницы справа.
                let (pill, _) = ui.allocate_exact_size(vec2((ui.available_width() - right).max(120.0), 30.0), Sense::hover());
                field = pill;
                let focused = ui.memory(|m| m.has_focus(address_id));
                ui.painter().rect_filled(pill, 4.0, p.field);
                let edge = if focused { p.accent } else { mix(p.toolbar, p.text, 0.14) };
                ui.painter().rect_stroke(pill, 4.0, Stroke::new(1.0_f32, edge), StrokeKind::Inside);
                let secure = url.as_ref().is_some_and(|u| u.scheme() == "https");
                let ours = url.as_ref().is_some_and(|u| u.scheme() == "obsos");
                let plain_http = url.as_ref().is_some_and(|u| u.scheme() == "http");
                let lead = Rect::from_center_size(pos2(pill.left() + 17.0, pill.center().y), vec2(20.0, 20.0));
                let warn = Color32::from_rgb(0xe0, 0x90, 0x40);
                if shell.address.is_empty() || focused {
                    paint_icon(ui.painter(), lead, Icon::Search, p.dim);
                } else if secure {
                    paint_icon(ui.painter(), lead, Icon::Lock, p.dim);
                    ui.interact(lead, ui.id().with("lock"), Sense::hover()).on_hover_text("Соединение защищено (HTTPS)");
                } else {
                    paint_icon(ui.painter(), lead, Icon::Info, if plain_http { warn } else { p.dim });
                    if plain_http {
                        ui.interact(lead, ui.id().with("lock"), Sense::hover()).on_hover_text("Соединение не защищено (HTTP)");
                    }
                }
                let mut buttons = Vec::new();
                if url.is_some() && !ours {
                    buttons.push((Icon::Star(marked), "Закладка (Ctrl+D)"));
                }
                if web {
                    buttons.push((Icon::Book, "Режим чтения (F9)"));
                }
                if has_logins {
                    buttons.push((Icon::Key, "Подставить сохранённый пароль"));
                }
                let zoom = shell.tabs.get(shell.active).map(|t| t.zoom).unwrap_or(1.0);
                let default_zoom = shell.profile.settings.page_zoom as f32 / 100.0;
                let mut inner_right = pill.right() - 6.0;
                for (icon, tip) in buttons {
                    let r = Rect::from_min_max(pos2(inner_right - 28.0, pill.top() + 2.0), pos2(inner_right, pill.bottom() - 2.0));
                    inner_right -= 29.0;
                    let response = ui.interact(r, ui.id().with(("page-button", tip)), Sense::click());
                    if response.hovered() {
                        ui.painter().rect_filled(r, 3.0, p.hover);
                    }
                    paint_icon(ui.painter(), r, icon, if matches!(icon, Icon::Star(true)) { p.accent2 } else { p.dim });
                    if response.on_hover_text(tip).clicked() {
                        match icon {
                            Icon::Star(_) => actions.push(Action::ToggleBookmark),
                            Icon::Book => actions.push(Action::Reader),
                            _ => {
                                if let Some(o) = &origin {
                                    shell.infobars.retain(|b| !matches!(b, Infobar::ChooseLogin(_)));
                                    shell.infobars.push(Infobar::ChooseLogin(o.clone()));
                                }
                            }
                        }
                    }
                }
                if (zoom - default_zoom).abs() > 0.005 {
                    let r = Rect::from_min_max(pos2(inner_right - 46.0, pill.top() + 4.0), pos2(inner_right - 2.0, pill.bottom() - 4.0));
                    inner_right -= 48.0;
                    let response = ui.interact(r, ui.id().with("zoom"), Sense::click());
                    ui.painter().rect_filled(r, 3.0, if response.hovered() { p.hover } else { mix(p.field, p.text, 0.08) });
                    ui.painter().text(r.center(), Align2::CENTER_CENTER, format!("{:.0}%", zoom * 100.0), FontId::proportional(11.5), p.text);
                    if response.on_hover_text("Вернуть обычный масштаб").clicked() {
                        actions.push(Action::Zoom(0.0));
                    }
                }
                let text_rect = Rect::from_min_max(pos2(pill.left() + 32.0, pill.top() + 3.0), pos2(inner_right - 2.0, pill.bottom() - 3.0));
                let hint = format!("Адрес или поиск в {}", shell.profile.settings.search.title());
                let edit = egui::TextEdit::singleline(&mut shell.address)
                    .id(address_id)
                    .hint_text(hint)
                    .frame(egui::Frame::NONE)
                    .font(FontId::proportional(14.0))
                    .margin(Margin::symmetric(2, 3))
                    .text_color(if plain_http && !focused { mix(p.text, warn, 0.45) } else { p.text })
                    .desired_width(text_rect.width());
                // Своя область поля: `put` сдвинул бы курсор панели, и кнопки
                // справа легли бы внутрь «таблетки».
                let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(text_rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
                let response = inner.add(edit);
                if response.changed() {
                    shell.chrome.typed = shell.address.clone();
                    shell.chrome.suggestions = suggest(shell, &shell.address);
                    shell.chrome.selected = None;
                    shell.chrome.suggesting = true;
                }
                if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    actions.push(Action::Go(shell.address.clone()));
                    shell.chrome.suggesting = false;
                } else if response.lost_focus() && !shell.chrome.over_suggestions {
                    shell.chrome.suggesting = false;
                }

                if web {
                    let host = url.as_ref().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default();
                    let (enabled, allowed) = {
                        let b = shell.delegate.blocker.borrow();
                        (b.enabled, b.allows(&host))
                    };
                    let count = shell.tabs.get(shell.active).map(|t| t.blocked).unwrap_or(0);
                    let tip = if !enabled {
                        String::from("Блокировка рекламы выключена (Настройки)")
                    } else if allowed {
                        format!("Реклама на {host} не блокируется — включить")
                    } else {
                        format!("Заблокировано рекламы и трекеров: {count}. Щелчок — не блокировать на {host}")
                    };
                    let shield = icon_button_response(ui, Icon::Shield(enabled && !allowed), enabled, &tip, p);
                    if enabled && !allowed && count > 0 {
                        let at = shield.rect.right_bottom() + vec2(-8.0, -6.0);
                        let text = if count > 99 { String::from("99+") } else { count.to_string() };
                        let galley = ui.painter().layout_no_wrap(text, FontId::proportional(9.5), Color32::WHITE);
                        let badge = Rect::from_center_size(at, galley.size() + vec2(6.0, 1.0));
                        ui.painter().rect_filled(badge, 2.0, p.accent);
                        ui.painter().galley(badge.center() - galley.size() / 2.0, galley, Color32::WHITE);
                    }
                    if shield.clicked() && enabled {
                        actions.push(Action::ToggleBlockSite);
                    }
                }
                if has_downloads {
                    download_button(ui, shell, &items, p, actions);
                } else {
                    shell.chrome.download_button = None;
                }
                let menu = icon_button_response(ui, Icon::Menu, true, "Меню (Alt+F)", p);
                shell.chrome.menu_button = Some(menu.rect);
                if shell.panel == Some(Panel::Menu) {
                    ui.painter().rect_stroke(menu.rect, 4.0, Stroke::new(1.0_f32, p.accent), StrokeKind::Inside);
                }
                if menu.clicked() {
                    actions.push(Action::Panel(Panel::Menu));
                }
            });
        });
    suggestions(ui.ctx(), shell, field, p, actions);
}

fn suggestions(ctx: &egui::Context, shell: &mut Shell, field: Rect, p: &Palette, actions: &mut Vec<Action>) {
    shell.chrome.over_suggestions = false;
    if !shell.chrome.suggesting || shell.chrome.suggestions.is_empty() || !field.is_positive() {
        return;
    }
    let mut chosen = None;
    let area = egui::Area::new(egui::Id::new("obsos-suggestions")).order(egui::Order::Foreground).fixed_pos(field.left_bottom() + vec2(0.0, 4.0)).show(ctx, |ui| {
        popup_frame(p).show(ui, |ui| {
            ui.set_width(field.width() - 12.0);
            ui.spacing_mut().item_spacing.y = 1.0;
            for (i, s) in shell.chrome.suggestions.iter().enumerate() {
                let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
                if response.hovered() || shell.chrome.selected == Some(i) {
                    ui.painter().rect_filled(rect, 3.0, if shell.chrome.selected == Some(i) { p.active } else { p.hover });
                }
                let icon = Rect::from_center_size(pos2(rect.left() + 18.0, rect.center().y), vec2(18.0, 18.0));
                paint_icon(ui.painter(), icon, if s.bookmark { Icon::Star(true) } else { Icon::Clock }, p.dim);
                let max = ((rect.width() - 60.0) / 7.2).max(10.0) as usize;
                let title = if s.title.is_empty() { s.url.clone() } else { s.title.clone() };
                let shown = clip(&title, max * 3 / 5);
                let at = ui.painter().text(pos2(rect.left() + 38.0, rect.center().y), Align2::LEFT_CENTER, shown, FontId::proportional(13.5), p.text);
                let url = s.url.trim_start_matches("https://").trim_start_matches("www.");
                let rest = ((rect.right() - at.right() - 24.0) / 7.0).max(0.0) as usize;
                if rest > 8 {
                    ui.painter().text(pos2(at.right() + 12.0, rect.center().y), Align2::LEFT_CENTER, clip(url, rest), FontId::proportional(12.0), p.accent2);
                }
                if response.clicked() {
                    chosen = Some(s.url.clone());
                }
            }
        });
    });
    shell.chrome.over_suggestions = ctx.input(|i| i.pointer.hover_pos()).is_some_and(|q| area.response.rect.contains(q));
    if let Some(url) = chosen {
        shell.address = url.clone();
        shell.chrome.suggesting = false;
        actions.push(Action::Go(url));
    }
}

// ── Загрузки ────────────────────────────────────────────────────────────────

/// Кнопка загрузок с кольцом общего хода.
fn download_button(ui: &mut Ui, shell: &mut Shell, items: &[crate::downloads::Item], p: &Palette, actions: &mut Vec<Action>) {
    let (rect, response) = ui.allocate_exact_size(vec2(30.0, 28.0), Sense::click());
    shell.chrome.download_button = Some(rect);
    if response.hovered() {
        ui.painter().rect_filled(rect, 4.0, p.hover);
    }
    let active: Vec<&crate::downloads::Item> = items.iter().filter(|d| d.active() && d.visible()).collect();
    // Толчок кнопки, когда значок долетел.
    let pulse = shell
        .chrome
        .flights
        .iter()
        .map(|(at, _)| at.elapsed().as_secs_f32() - FLIGHT)
        .filter(|t| (0.0..0.5).contains(t))
        .map(|t| 1.0 - t / 0.5)
        .fold(0.0f32, f32::max);
    if pulse > 0.0 {
        ui.painter().circle_stroke(rect.center(), 13.0 + 8.0 * (1.0 - pulse), Stroke::new(2.0_f32, p.accent2.gamma_multiply(pulse)));
    }
    let color = if active.is_empty() { p.text } else { p.accent2 };
    if active.is_empty() {
        paint_icon(ui.painter(), rect.expand(pulse * 2.0), Icon::Download, color);
    } else {
        let (got, total) = active.iter().fold((0u64, 0u64), |(g, t), d| (g + d.got, t + d.total.unwrap_or(0)));
        let center = rect.center();
        ui.painter().circle_stroke(center, 12.0, Stroke::new(2.0_f32, mix(p.toolbar, p.text, 0.18)));
        let time = ui.input(|i| i.time);
        if total > 0 && active.iter().all(|d| d.total.is_some()) {
            let fraction = (got as f32 / total as f32).clamp(0.02, 1.0);
            let pts: Vec<Pos2> = (0..=36)
                .map(|i| {
                    let a = -std::f32::consts::FRAC_PI_2 + i as f32 / 36.0 * fraction * std::f32::consts::TAU;
                    center + vec2(a.cos() * 12.0, a.sin() * 12.0)
                })
                .collect();
            ui.painter().add(egui::Shape::line(pts, Stroke::new(2.5_f32, p.accent2)));
        } else {
            spinner(ui.painter(), center, 12.0, time, p.accent2);
        }
        // Стрелка «капает» вниз, пока идёт загрузка.
        let drop = ((time * 1.6).fract() as f32 - 0.5) * 3.0;
        let c = center + vec2(0.0, drop);
        let s = Stroke::new(1.7_f32, color);
        ui.painter().line_segment([c + vec2(0.0, -5.0), c + vec2(0.0, 4.0)], s);
        ui.painter().add(egui::Shape::line(vec![c + vec2(-3.5, 0.5), c + vec2(0.0, 4.2), c + vec2(3.5, 0.5)], s));
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
    }
    if response.on_hover_text("Загрузки (Ctrl+J)").clicked() {
        actions.push(Action::Panel(Panel::Downloads));
    }
}

/// Новая загрузка: значок файла летит от указателя (или середины страницы) к
/// кнопке загрузок.
pub fn download_flights(ctx: &egui::Context, shell: &mut Shell, page: Rect, p: &Palette) {
    let items = shell.delegate.downloads.snapshot();
    let started: Vec<u64> = items.iter().filter(|d| d.visible()).map(|d| d.id).collect();
    match &mut shell.chrome.known_downloads {
        None => shell.chrome.known_downloads = Some(started.into_iter().collect()),
        Some(known) => {
            let fresh = started.iter().filter(|id| known.insert(**id)).count();
            if fresh > 0 {
                let from = ctx.input(|i| i.pointer.latest_pos()).filter(|q| page.contains(*q)).unwrap_or(page.center());
                shell.chrome.flights.push((Instant::now(), from));
                // Значок долетит — показать список (как в Edge). На FossAi
                // загрузки показывает полоса внизу окна.
                if !cfg!(target_os = "freeos") {
                    shell.chrome.popup_at = Some(Instant::now() + std::time::Duration::from_secs_f32(FLIGHT));
                }
                ctx.request_repaint();
                println!("browser: download animation");
            }
        }
    }
    if let Some(at) = shell.chrome.popup_at {
        if Instant::now() >= at {
            shell.chrome.popup_at = None;
            if shell.panel.is_none() && shell.modal.is_none() {
                shell.panel = Some(Panel::Downloads);
                ctx.request_repaint();
            }
        } else {
            ctx.request_repaint_after(at - Instant::now());
        }
    }
    shell.chrome.flights.retain(|(at, _)| at.elapsed().as_secs_f32() < FLIGHT + 0.6);
    if shell.chrome.flights.is_empty() {
        return;
    }
    ctx.request_repaint_after(std::time::Duration::from_millis(16));
    // Кнопки ещё нет в первом кадре — лететь в правый верхний угол.
    let screen = ctx.content_rect();
    let to = shell.chrome.download_button.map(|r| r.center()).unwrap_or(pos2(screen.right() - 55.0, 22.0));
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("obsos-flight")));
    for (at, from) in &shell.chrome.flights {
        let t = at.elapsed().as_secs_f32() / FLIGHT;
        if t >= 1.0 {
            continue;
        }
        // Сначала значок «всплывает», потом дугой уходит к кнопке.
        let pop = (t / 0.22).min(1.0);
        let fly = ((t - 0.22) / 0.78).max(0.0);
        let ease = fly * fly * (3.0 - 2.0 * fly);
        let control = pos2((from.x + to.x) / 2.0, from.y.min(to.y) - 90.0);
        let a = from.lerp(control, ease);
        let b = control.lerp(to, ease);
        let center = a.lerp(b, ease);
        let radius = 26.0 * pop * (1.0 - 0.62 * ease);
        painter.circle_filled(center, radius + 6.0, p.accent.gamma_multiply(0.25));
        painter.circle_filled(center, radius, p.accent);
        let k = radius / 26.0;
        let s = Stroke::new((3.5 * k).max(1.2), Color32::WHITE);
        painter.line_segment([center + vec2(0.0, -11.0 * k), center + vec2(0.0, 8.0 * k)], s);
        painter.add(egui::Shape::line(vec![center + vec2(-8.0 * k, 1.0 * k), center + vec2(0.0, 9.0 * k), center + vec2(8.0 * k, 1.0 * k)], s));
        painter.line_segment([center + vec2(-9.0 * k, 14.0 * k), center + vec2(9.0 * k, 14.0 * k)], s);
    }
}

/// Ширина списка загрузок под кнопкой.
const POPUP_W: f32 = 372.0;

/// Список последних загрузок под кнопкой: имя, ход, действия значками;
/// длинное обрезается, ничего не выходит за ширину.
pub fn downloads_popup(ctx: &egui::Context, shell: &mut Shell, p: &Palette, actions: &mut Vec<Action>) {
    use crate::downloads::State;
    let items = shell.delegate.downloads.snapshot();
    if items.iter().any(|d| d.active()) {
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
    let screen = ctx.content_rect();
    let button = shell.chrome.download_button.unwrap_or(Rect::from_min_size(pos2(screen.right() - 74.0, 40.0), vec2(30.0, 28.0)));
    let width = POPUP_W.min(screen.width() - 8.0);
    // Правый край — под кнопкой, но целиком в окне.
    let right = button.right().max(width + 4.0).min(screen.right() - 4.0);
    let mut close = false;
    let area = egui::Area::new(egui::Id::new("obsos-downloads-popup"))
        .order(egui::Order::Foreground)
        .pivot(Align2::RIGHT_TOP)
        .fixed_pos(pos2(right, button.bottom() + 6.0))
        .show(ctx, |ui| {
            popup_frame(p).show(ui, |ui| {
                ui.set_width(width - 10.0);
                ui.spacing_mut().item_spacing = vec2(4.0, 0.0);
                let (head, _) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::hover());
                ui.painter().text(pos2(head.left() + 10.0, head.center().y), Align2::LEFT_CENTER, "Загрузки", FontId::proportional(14.5), p.text);
                let folder = Rect::from_center_size(pos2(head.right() - 46.0, head.center().y), vec2(28.0, 26.0));
                if popup_button(ui, folder, "folder", "Открыть папку загрузок", p, |painter, color| crate::ui::paint_folder(painter, folder, color)) {
                    let dir = shell.delegate.downloads.dir.borrow().clone();
                    let _ = std::fs::create_dir_all(&dir);
                    crate::window::show_in_folder(&dir);
                }
                let cross = Rect::from_center_size(pos2(head.right() - 16.0, head.center().y), vec2(28.0, 26.0));
                if popup_button(ui, cross, "close", "Закрыть (Esc)", p, |painter, color| paint_icon(painter, cross, Icon::Close, color)) {
                    close = true;
                }
                separator(ui, p);
                if items.iter().all(|d| !d.visible()) {
                    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 48.0), Sense::hover());
                    ui.painter().text(r.center(), Align2::CENTER_CENTER, "Пока ничего не скачано", FontId::proportional(13.0), p.dim);
                }
                egui::ScrollArea::vertical().max_height((screen.bottom() - button.bottom() - 120.0).clamp(100.0, 380.0)).show(ui, |ui| {
                    for item in items.iter().rev().filter(|d| d.visible()).take(20) {
                        let (row, response) = ui.allocate_exact_size(vec2(ui.available_width(), 50.0), Sense::click());
                        if response.hovered() {
                            ui.painter().rect_filled(row, 3.0, mix(p.toolbar, p.text, 0.06));
                        }
                        let buttons = match item.state {
                            State::Done(_) => 2,
                            State::Failed(_) | State::Cancelled => 2,
                            _ => 1,
                        };
                        let text_right = row.right() - 10.0 - 30.0 * buttons as f32;
                        let line = |ui: &Ui, text: String, y: f32, size: f32, color: Color32| {
                            let mut job = egui::text::LayoutJob::simple_singleline(text, FontId::proportional(size), color);
                            job.wrap.max_width = (text_right - row.left() - 14.0).max(40.0);
                            job.wrap.max_rows = 1;
                            job.wrap.break_anywhere = true;
                            let galley = ui.painter().layout_job(job);
                            ui.painter().galley(pos2(row.left() + 10.0, y - galley.size().y / 2.0), galley, color);
                        };
                        let failed = matches!(item.state, State::Failed(_) | State::Cancelled);
                        line(ui, item.name.clone(), row.top() + 17.0, 13.5, p.text);
                        line(ui, crate::ui::download_status(item), row.top() + 35.0, 11.5, if failed { Color32::from_rgb(0xe0, 0x70, 0x60) } else { p.dim });
                        if item.active() {
                            let track = Rect::from_min_max(pos2(row.left() + 10.0, row.bottom() - 4.0), pos2(row.right() - 10.0, row.bottom() - 2.0));
                            ui.painter().rect_filled(track, 0.0, mix(p.toolbar, p.text, 0.14));
                            let (from, part) = match item.fraction() {
                                Some(f) => (0.0, f),
                                None => ((ui.input(|i| i.time) * 0.7).fract() as f32 * 0.75, 0.25),
                            };
                            ui.painter().rect_filled(Rect::from_min_size(pos2(track.left() + track.width() * from, track.top()), vec2(track.width() * part, 2.0)), 0.0, p.accent2);
                        }
                        let slot = |n: f32| Rect::from_center_size(pos2(row.right() - 18.0 - 30.0 * n, row.center().y), vec2(28.0, 28.0));
                        let (first, second) = (slot(0.0), slot(1.0));
                        match &item.state {
                            State::Done(path) => {
                                if popup_button(ui, first, ("show", item.id), "Показать в папке", p, |painter, color| crate::ui::paint_folder(painter, first, color)) {
                                    crate::ui::show_in_folder(path);
                                }
                                if popup_button(ui, second, ("remove", item.id), "Убрать из списка", p, |painter, color| paint_icon(painter, second, Icon::Close, color)) {
                                    shell.delegate.downloads.remove(item.id);
                                }
                                if response.clicked() {
                                    crate::ui::open_download(path, actions);
                                    close = true;
                                }
                            }
                            State::Failed(_) | State::Cancelled => {
                                if popup_button(ui, first, ("remove", item.id), "Убрать из списка", p, |painter, color| paint_icon(painter, first, Icon::Close, color)) {
                                    shell.delegate.downloads.remove(item.id);
                                }
                                if popup_button(ui, second, ("retry", item.id), "Повторить", p, |painter, color| paint_icon(painter, second, Icon::Reload, color)) {
                                    actions.push(Action::Download(item.url.clone()));
                                }
                            }
                            _ => {
                                if popup_button(ui, first, ("cancel", item.id), "Отменить", p, |painter, color| paint_icon(painter, first, Icon::Close, color)) {
                                    println!("browser: download cancel {}", crate::logs::text(&item.name));
                                    shell.delegate.downloads.cancel(item.id);
                                }
                            }
                        }
                    }
                });
                separator(ui, p);
                if item(ui, Some(Icon::Download), "Все загрузки", "", p) {
                    actions.push(Action::OpenPage("downloads"));
                    close = true;
                }
            });
        });
    // Щелчок мимо закрывает; по самой кнопке — она переключает сама.
    let outside = ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|q| !area.response.rect.contains(q) && !button.contains(q)));
    if close || outside {
        shell.panel = None;
    }
}

/// Кнопка-значок в заданном месте.
fn popup_button(ui: &mut Ui, rect: Rect, id: impl std::hash::Hash, tip: &str, p: &Palette, paint: impl FnOnce(&egui::Painter, Color32)) -> bool {
    let response = ui.interact(rect, ui.id().with(id), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, 3.0, mix(p.toolbar, p.text, 0.14));
    }
    paint(ui.painter(), if response.hovered() { p.text } else { p.dim });
    response.on_hover_text(tip).clicked()
}
