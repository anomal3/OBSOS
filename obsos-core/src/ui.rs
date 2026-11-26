//! Оболочка OBSOS на egui: тема, полосы уведомлений над страницей,
//! окна панелей (закладки, пароли, синхронизация, скрипты, профили),
//! инструменты разработчика, диалоги страницы
//! (`alert`/`confirm`/`prompt`, `<select>`, контекстное меню, цвет, файл,
//! HTTP-вход) и всплывающие сообщения. Вкладки, панель адреса и меню —
//! `chrome.rs`; настройки, история и прочие страницы — `webui.rs`.
//!
//! Раскладка по мотивам Zervo (© Goddv, MPL-2.0); код свой. Значки рисуются
//! линиями — в шрифтах egui нет стрелок.

use egui::{Align2, Color32, CornerRadius, FontId, Margin, Pos2, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};
use servo::{ContextMenuItem, RgbColor, SelectElementOptionOrOptgroup, SimpleDialog};

use crate::chrome::{self, Icon, icon_button, paint_icon};
use crate::profile::{Look, Rgb, TabsAt, now_ms};
use crate::shell::{Action, Infobar, Modal, Panel, Shell};
use crate::vault::Login;
use browser_sync::remote::{Provider, oauth};

// Геометрия боковой панели — на неё целится сценарий `servo-window`.
pub const BRAND_H: f32 = 34.0;
pub const NEW_TAB_H: f32 = 30.0;
pub const TAB_ROW_H: f32 = 30.0;
/// Ширина окна панели (пароли, синхронизация и др.).
const DIALOG_W: f32 = 560.0;

pub fn c(rgb: Rgb) -> Color32 {
    Color32::from_rgb(rgb.0, rgb.1, rgb.2)
}

pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

pub struct Palette {
    pub accent: Color32,
    pub accent2: Color32,
    pub sidebar: Color32,
    pub toolbar: Color32,
    pub text: Color32,
    pub dim: Color32,
    pub hover: Color32,
    pub active: Color32,
    pub field: Color32,
}

fn palette(look: &Look) -> Palette {
    let text = c(look.text);
    let sidebar = c(look.sidebar);
    let accent = c(look.accent);
    Palette {
        accent,
        accent2: c(look.accent2),
        sidebar,
        toolbar: c(look.toolbar),
        text,
        dim: mix(text, sidebar, 0.4),
        hover: mix(sidebar, text, 0.08),
        active: if look.accent_reach >= 2 { mix(sidebar, accent, 0.3) } else { mix(sidebar, text, 0.12) },
        field: mix(c(look.toolbar), if look.dark { Color32::BLACK } else { Color32::WHITE }, 0.35),
    }
}

pub fn style(ctx: &egui::Context, look: &Look) {
    let p = palette(look);
    let mut v = if look.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.panel_fill = p.toolbar;
    v.window_fill = p.sidebar;
    v.extreme_bg_color = p.field;
    v.override_text_color = Some(p.text);
    v.selection.bg_fill = p.accent;
    v.selection.stroke = Stroke::new(1.0_f32, Color32::WHITE);
    v.hyperlink_color = p.accent2;
    let button = if look.accent_reach >= 2 { mix(p.toolbar, p.accent, 0.45) } else { mix(p.toolbar, p.text, 0.1) };
    v.widgets.inactive.weak_bg_fill = button;
    v.widgets.inactive.bg_fill = button;
    v.widgets.hovered.weak_bg_fill = mix(button, p.text, 0.12);
    v.widgets.active.weak_bg_fill = p.accent;
    for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = CornerRadius::same(3);
        // Кнопки не «растут» при наведении — ровная сетка.
        w.expansion = 0.0;
    }
    v.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, mix(p.toolbar, p.text, 0.16));
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, mix(p.toolbar, p.text, 0.32));
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, mix(p.sidebar, p.text, 0.13));
    v.window_corner_radius = CornerRadius::same(6);
    v.menu_corner_radius = CornerRadius::same(4);
    v.window_stroke = Stroke::new(1.0_f32, mix(p.sidebar, p.text, 0.16));
    v.window_shadow = egui::epaint::Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(80) };
    v.popup_shadow = egui::epaint::Shadow { offset: [0, 3], blur: 10, spread: 0, color: Color32::from_black_alpha(60) };
    ctx.set_visuals(v);
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = vec2(8.0, 7.0);
        s.spacing.button_padding = vec2(12.0, 5.0);
        s.spacing.interact_size.y = 26.0;
        s.spacing.window_margin = Margin::same(18);
        // Мелкие подписи панелей — читаемыми.
        s.text_styles.insert(egui::TextStyle::Small, FontId::proportional(11.5));
    });
}

/// Шрифт системы вместо встроенного в egui (где платформа его даёт).
pub fn fonts(ctx: &egui::Context) {
    let Some(data) = crate::window::platform().ui_font() else { return };
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(String::from("system"), std::sync::Arc::new(egui::FontData::from_owned(data)));
    if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
        family.insert(0, String::from("system"));
    }
    ctx.set_fonts(fonts);
}

pub fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

fn when(ms: u64) -> String {
    let (y, mo, d, h, mi, _) = browser_sync::remote::civil(ms / 1000);
    let age = now_ms().saturating_sub(ms) / 1000;
    if age < 60 {
        String::from("только что")
    } else if age < 3600 {
        format!("{} мин назад", age / 60)
    } else {
        format!("{d:02}.{mo:02}.{y} {h:02}:{mi:02}")
    }
}

// ── Проход интерфейса ───────────────────────────────────────────────────────

pub fn shell_ui(ui: &mut Ui, shell: &mut Shell, actions: &mut Vec<Action>) {
    actions.clear();
    let look = shell.look().clone();
    let p = palette(&look);
    let full = crate::shell::fullscreen();
    if !full {
        // Порядок панелей: боковая — на всю высоту, горизонтальная полоса
        // сверху — над панелью адреса.
        chrome::tabs(ui, shell, &look, &p, actions);
        chrome::toolbar(ui, shell, &look, &p, actions);
    }
    let ctx = ui.ctx().clone();
    let screen = ctx.content_rect();
    let (left, top, right, bottom) = crate::shell::chrome_margins(&look);
    let page = Rect::from_min_max(
        pos2(left as f32, top as f32),
        pos2((screen.max.x - right as f32).max(left as f32 + 1.0), (screen.max.y - bottom as f32).max(top as f32 + 1.0)),
    );
    // Полоса загрузок: пока выезжает — лежит поверх страницы, потом страница
    // становится на неё ниже (`app.rs`).
    if !full && shell.chrome.bar_since.is_some() {
        let h = crate::shell::DOWNLOADS_BAR_H as f32;
        let floor = screen.max.y - if look.tabs == TabsAt::Bottom { crate::shell::TABSTRIP_H as f32 } else { 0.0 };
        let t = shell.chrome.bar_since.map(|at| at.elapsed().as_secs_f32() / chrome::BAR_SLIDE).unwrap_or(1.0).min(1.0);
        if t < 1.0 {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
        let rise = h * (1.0 - (1.0 - t) * (1.0 - t));
        downloads_bar(&ctx, shell, Rect::from_min_size(pos2(screen.min.x, floor - rise), vec2(screen.width(), h)), &p, actions);
    }
    overlays(&ctx, shell, page, &p, actions);
    match shell.panel {
        Some(Panel::Menu) => chrome::main_menu(&ctx, shell, &p, actions),
        Some(Panel::DevTools) => dev_panel(&ctx, shell, page, &p),
        Some(Panel::Downloads) => chrome::downloads_popup(&ctx, shell, &p, actions),
        Some(panel) => dialog(&ctx, shell, panel, &p, actions),
        None => {}
    }
    if shell.panel != Some(Panel::Menu) {
        ctx.animate_bool_with_time(egui::Id::new("obsos-menu-open"), false, 0.0);
    }
    if shell.find.open {
        find_bar(&ctx, shell, page, &p, actions);
    }
    chrome::tab_menu(&ctx, shell, &p, actions);
    if !full {
        chrome::download_flights(&ctx, shell, page, &p);
    }
    modal(&ctx, shell, page, &p, actions);
    toasts(&ctx, shell, page, &p);
}

/// Строка поиска на странице (Ctrl+F): справа сверху над страницей.
fn find_bar(ctx: &egui::Context, shell: &mut Shell, page: Rect, p: &Palette, actions: &mut Vec<Action>) {
    let id = egui::Id::new("obsos-find");
    egui::Area::new(egui::Id::new("obsos-find-bar"))
        .order(egui::Order::Foreground)
        .anchor(Align2::RIGHT_TOP, vec2(-(ctx.content_rect().max.x - page.right()) - 12.0, page.top() + 8.0))
        .show(ctx, |ui| {
            egui::Frame::new().fill(p.toolbar).corner_radius(4).inner_margin(Margin::symmetric(10, 6)).stroke(Stroke::new(1.0_f32, p.accent)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    let edit = ui.add(egui::TextEdit::singleline(&mut shell.find.text).id(id).hint_text("Найти на странице").desired_width(220.0));
                    if shell.find.focus {
                        edit.request_focus();
                        shell.find.focus = false;
                    }
                    let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if enter {
                        let back = ui.input(|i| i.modifiers.shift);
                        actions.push(Action::Find { back });
                        edit.request_focus();
                    }
                    let status = shell.find.status.borrow().clone();
                    ui.label(RichText::new(if status == "0/0" { String::from("нет") } else { status }).small());
                    if ui.small_button("Пред.").on_hover_text("Shift+Enter").clicked() {
                        actions.push(Action::Find { back: true });
                    }
                    if ui.small_button("След.").on_hover_text("Enter, Ctrl+G").clicked() {
                        actions.push(Action::Find { back: false });
                    }
                    if icon_button(ui, Icon::Close, true, "Закрыть (Esc)", p) {
                        actions.push(Action::FindClose);
                    }
                });
            });
        });
}

/// Полоса загрузки, подсказка ссылки, полосы уведомлений.
fn overlays(ctx: &egui::Context, shell: &mut Shell, page: Rect, p: &Palette, actions: &mut Vec<Action>) {
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Middle, egui::Id::new("obsos-overlay")));
    if let Some(tab) = shell.tabs.get(shell.active) {
        if tab.loading {
            let t = (ctx.input(|i| i.time) * 0.8).fract() as f32;
            let w = page.width() * 0.3;
            let x = page.left() + (page.width() + w) * t - w;
            painter.rect_filled(Rect::from_min_size(pos2(x.max(page.left()), page.top()), vec2(w, 3.0)), 0.0, p.accent2);
            // С видеокартой полоса едет каждый кадр экрана.
            ctx.request_repaint_after(std::time::Duration::from_millis(if shell.gpu_active { 1 } else { 60 }));
        }
        if let Some(status) = &tab.status {
            let text = clip(status, 90);
            let galley = painter.layout_no_wrap(text, FontId::proportional(12.0), p.text);
            let r = Rect::from_min_size(pos2(page.left() + 4.0, page.bottom() - galley.size().y - 10.0), galley.size() + vec2(12.0, 6.0));
            painter.rect_filled(r, 3.0, p.toolbar);
            painter.galley(r.min + vec2(6.0, 3.0), galley, p.text);
        }
    }
    if shell.infobars.is_empty() {
        return;
    }
    let bars = shell.infobars.clone();
    egui::Area::new(egui::Id::new("obsos-infobars")).order(egui::Order::Foreground).fixed_pos(page.min + vec2(8.0, 6.0)).show(ctx, |ui| {
        ui.set_width(page.width() - 16.0);
        for (index, bar) in bars.iter().enumerate() {
            egui::Frame::new().fill(p.toolbar).corner_radius(4).inner_margin(Margin::symmetric(12, 8)).stroke(Stroke::new(1.0_f32, p.accent)).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| match bar {
                    Infobar::SaveLogin(login) => {
                        ui.label(format!("Сохранить пароль для {} ({})?", login.origin, if login.username.is_empty() { "без имени" } else { &login.username }));
                        if ui.button("Сохранить").clicked() {
                            shell.save_login(login.clone());
                            shell.infobars.remove(index);
                        } else if ui.button("Никогда для этого сайта").clicked() {
                            shell.profile.put_shared(&format!("never/{}", login.origin), "1");
                            shell.infobars.remove(index);
                        } else if ui.button("Не сейчас").clicked() {
                            shell.infobars.remove(index);
                        }
                    }
                    Infobar::ChooseLogin(origin) => {
                        ui.label(format!("Войти на {origin} как:"));
                        let id = shell.active_view().map(|w| w.id());
                        for login in shell.profile.logins_for(origin) {
                            let name = if login.username.is_empty() { String::from("(без имени)") } else { login.username.clone() };
                            if ui.button(name).clicked() {
                                if let Some(id) = id {
                                    shell.fill(id, &login);
                                }
                                shell.infobars.retain(|b| !matches!(b, Infobar::ChooseLogin(_)));
                            }
                        }
                        if ui.button("Закрыть").clicked() {
                            shell.infobars.retain(|b| !matches!(b, Infobar::ChooseLogin(_)));
                        }
                    }
                    Infobar::Unsupported { url, what } => {
                        let explain = if what == "MediaSource" {
                            String::from("Сайт хочет потоковое видео (Media Source Extensions) — в движке Servo его нет.")
                        } else {
                            format!("Servo не умеет: {what}.")
                        };
                        ui.label(explain);
                        if ui.button("Открыть в Chromium").clicked() {
                            actions.push(Action::Chromium(url.clone()));
                        }
                        if ui.button("Подробнее").clicked() {
                            actions.push(Action::OpenUrl(crate::pages::unsupported_url(url, what).to_string()));
                        }
                        if ui.button("Скрыть").clicked() {
                            shell.infobars.retain(|b| !matches!(b, Infobar::Unsupported { .. }));
                        }
                    }
                });
            });
            if index >= shell.infobars.len() {
                break;
            }
        }
    });
}

fn toasts(ctx: &egui::Context, shell: &mut Shell, page: Rect, p: &Palette) {
    let now = std::time::Instant::now();
    shell.toasts.retain(|(_, until)| *until > now);
    if shell.toasts.is_empty() {
        return;
    }
    ctx.request_repaint_after(std::time::Duration::from_millis(500));
    // Снизу по середине страницы, над ней.
    egui::Area::new(egui::Id::new("obsos-toasts")).order(egui::Order::Tooltip).pivot(Align2::CENTER_BOTTOM).fixed_pos(pos2(page.center().x, page.bottom() - 20.0)).show(ctx, |ui| {
        ui.vertical_centered(|ui| {
            for (text, _) in &shell.toasts {
                egui::Frame::new()
                    .fill(p.toolbar)
                    .corner_radius(4)
                    .inner_margin(Margin::symmetric(18, 9))
                    .stroke(Stroke::new(1.0_f32, mix(p.toolbar, p.accent2, 0.6)))
                    .shadow(egui::epaint::Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(90) })
                    .show(ui, |ui| {
                        ui.set_max_width(page.width().min(460.0));
                        ui.label(text);
                    });
            }
        });
    });
}

// ── Панели ────────────────────────────────────────────────────────

/// Инструменты разработчика: встроены справа или снизу (страница на столько
/// меньше), край перетаскивается.
fn dev_panel(ctx: &egui::Context, shell: &mut Shell, page: Rect, p: &Palette) {
    let bottom = crate::shell::dev_dock_bottom() > 0;
    let rect = if bottom {
        Rect::from_min_size(page.left_bottom(), vec2(page.width(), crate::shell::dev_dock_bottom() as f32))
    } else {
        Rect::from_min_size(page.right_top(), vec2(crate::shell::dev_dock() as f32, page.height()))
    };
    if !rect.is_positive() {
        return;
    }
    egui::Area::new(egui::Id::new("obsos-devtools")).order(egui::Order::Foreground).fixed_pos(rect.min).show(ctx, |ui| {
        ui.painter().rect_filled(rect, 0.0, p.sidebar);
        ui.allocate_rect(rect, Sense::hover());
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(8.0, 4.0))).layout(egui::Layout::top_down(egui::Align::Min)));
        child.set_clip_rect(rect);
        crate::devtools::ui(&mut child, shell);
    });
    let grip = if bottom { Rect::from_min_size(rect.left_top() - vec2(0.0, 3.0), vec2(rect.width(), 7.0)) } else { Rect::from_min_size(rect.left_top() - vec2(3.0, 0.0), vec2(7.0, rect.height())) };
    let handle = egui::Area::new(egui::Id::new("obsos-dev-split")).order(egui::Order::Tooltip).fixed_pos(grip.min).show(ctx, |ui| {
        let (r, resp) = ui.allocate_exact_size(grip.size(), Sense::drag());
        let active = resp.hovered() || resp.dragged();
        if active {
            ui.ctx().set_cursor_icon(if bottom { egui::CursorIcon::ResizeVertical } else { egui::CursorIcon::ResizeHorizontal });
        }
        let line = if bottom { [pos2(r.left(), r.center().y), pos2(r.right(), r.center().y)] } else { [pos2(r.center().x, r.top()), pos2(r.center().x, r.bottom())] };
        ui.painter().line_segment(line, Stroke::new(if active { 3.0_f32 } else { 1.0 }, if active { p.accent } else { mix(p.sidebar, p.text, 0.22) }));
        resp.drag_delta()
    });
    let delta = handle.inner;
    let screen = ctx.content_rect();
    if bottom && delta.y != 0.0 {
        shell.dev.height = (shell.dev.height - delta.y).clamp(140.0, (screen.height() - 200.0).max(160.0));
    } else if !bottom && delta.x != 0.0 {
        shell.dev.width = (shell.dev.width - delta.x).clamp(300.0, (screen.width() - 300.0).max(320.0));
    }
}

/// Окно панели по центру: заголовок, крестик, содержимое с прокруткой; фон
/// затемнён, щелчок по нему закрывает.
fn dialog(ctx: &egui::Context, shell: &mut Shell, panel: Panel, p: &Palette, actions: &mut Vec<Action>) {
    let screen = ctx.content_rect();
    let shade = egui::Area::new(egui::Id::new("obsos-shade")).order(egui::Order::Middle).fixed_pos(screen.min).show(ctx, |ui| {
        let (r, response) = ui.allocate_exact_size(screen.size(), Sense::click());
        ui.painter().rect_filled(r, 0.0, Color32::from_black_alpha(110));
        response
    });
    // Щелчок, открывший окно из меню, фон ещё не получает — меню уже закрыто.
    if shade.inner.clicked() {
        shell.panel = None;
    }
    let width = DIALOG_W.min(screen.width() - 32.0).max(280.0);
    let height = (screen.height() - 120.0).clamp(200.0, 640.0);
    egui::Area::new(egui::Id::new("obsos-dialog"))
        .order(egui::Order::Foreground)
        .pivot(Align2::CENTER_TOP)
        .fixed_pos(pos2(screen.center().x, screen.top() + ((screen.height() - height) * 0.4).max(12.0)))
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(p.toolbar)
                .stroke(Stroke::new(1.0_f32, mix(p.toolbar, p.text, 0.2)))
                .corner_radius(6)
                .shadow(egui::epaint::Shadow { offset: [0, 8], blur: 28, spread: 0, color: Color32::from_black_alpha(110) })
                .inner_margin(Margin::same(16))
                .show(ui, |ui| {
                    ui.set_width(width - 32.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(panel_title(panel)).size(17.0).strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if icon_button(ui, Icon::Close, true, "Закрыть (Esc)", p) {
                                shell.panel = None;
                            }
                        });
                    });
                    ui.separator();
                    egui::ScrollArea::vertical().max_height(height - 80.0).min_scrolled_height(height - 80.0).auto_shrink([false, true]).show(ui, |ui| {
                        // Длинные строки переносятся, а не растягивают окно.
                        ui.set_max_width(width - 44.0);
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                        match panel {
                            Panel::Menu | Panel::DevTools | Panel::Downloads => {}
                            Panel::Bookmarks => bookmarks(ui, shell, actions),
                            Panel::Passwords => passwords(ui, shell),
                            Panel::RemoteTabs => remote_tabs(ui, shell, actions),
                            Panel::Sync => sync_panel(ui, shell, actions),
                            Panel::Scripts => scripts(ui, shell),
                            Panel::Profiles => profiles(ui, shell),
                        }
                    });
                });
        });
}

fn panel_title(panel: Panel) -> &'static str {
    match panel {
        Panel::Menu => "Меню",
        Panel::Bookmarks => "Закладки",
        Panel::Passwords => "Пароли",
        Panel::RemoteTabs => "Вкладки других устройств",
        Panel::Sync => "Синхронизация",
        Panel::Scripts => "Скрипты и стили",
        Panel::Profiles => "Профили",
        Panel::Downloads => "Загрузки",
        Panel::DevTools => "Инструменты разработчика",
    }
}

fn filter_box(ui: &mut Ui, shell: &mut Shell, hint: &str) -> String {
    ui.add(egui::TextEdit::singleline(&mut shell.filter).hint_text(hint).margin(Margin::symmetric(8, 6)).desired_width(f32::INFINITY));
    shell.filter.to_lowercase()
}

fn bookmarks(ui: &mut Ui, shell: &mut Shell, actions: &mut Vec<Action>) {
    if ui.button("☆ Добавить текущую страницу").clicked() {
        actions.push(Action::ToggleBookmark);
    }
    let f = filter_box(ui, shell, "Найти в закладках");
    let mut remove = None;
    for b in shell.profile.bookmarks.iter().filter(|b| f.is_empty() || b.url.to_lowercase().contains(&f) || b.title.to_lowercase().contains(&f)) {
        ui.horizontal_wrapped(|ui| {
            if ui.small_button("×").on_hover_text("Удалить").clicked() {
                remove = Some(b.url.clone());
            }
            let title = if b.title.is_empty() { &b.url } else { &b.title };
            if ui.link(clip(title, 40)).on_hover_text(&b.url).clicked() {
                actions.push(Action::OpenUrl(b.url.clone()));
            }
        });
    }
    if shell.profile.bookmarks.is_empty() {
        ui.label("Закладок пока нет — звёздочка в адресной строке или Ctrl+D.");
    }
    if let Some(url) = remove {
        shell.profile.remove_bookmark(&url);
        shell.update_page_data();
    }
}

fn passwords(ui: &mut Ui, shell: &mut Shell) {
    let Some(vault) = &shell.vault else {
        ui.label("Хранилище закрыто.");
        return;
    };
    ui.label(RichText::new(if vault.has_master_password() {
        "Хранилище защищено мастер-паролем."
    } else {
        "Без мастер-пароля пароли защищены ключом этого устройства (файлы профиля). Задать мастер-пароль — в «Настройках»."
    }).small());
    if shell.profile.foreign_logins > 0 {
        ui.label(RichText::new(format!(
            "Ещё {} пароль(ей) с других устройств зашифрованы другим паролем синхронизации — задайте такой же в «Синхронизации».",
            shell.profile.foreign_logins
        )).small());
    }
    ui.checkbox(&mut shell.profile.settings.save_passwords, "Предлагать сохранить пароль");
    ui.checkbox(&mut shell.profile.settings.autofill, "Подставлять сохранённые пароли");
    password_tools(ui, shell);
    ui.separator();
    let f = filter_box(ui, shell, "Найти сайт");
    let mut delete = None;
    let entries: Vec<(String, Login)> = shell.profile.logins.iter().map(|(k, l)| (k.clone(), l.clone())).collect();
    for (record, login) in entries.iter().filter(|(_, l)| f.is_empty() || l.origin.to_lowercase().contains(&f) || l.username.to_lowercase().contains(&f)) {
        ui.group(|ui| {
            ui.label(RichText::new(&login.origin).strong());
            ui.horizontal_wrapped(|ui| {
                ui.label(if login.username.is_empty() { "(без имени)" } else { &login.username });
                let shown = shell.show_passwords.contains(record);
                ui.label(if shown { login.password.clone() } else { "••••••••".into() });
                if ui.small_button(if shown { "скрыть" } else { "показать" }).clicked() {
                    if shown {
                        shell.show_passwords.remove(record);
                    } else {
                        shell.show_passwords.insert(record.clone());
                    }
                }
                if ui.small_button("копировать").clicked() {
                    *shell.delegate.clipboard.borrow_mut() = login.password.clone();
                    crate::window::clipboard_set(&login.password);
                }
                let leak = shell.tools.leaks.lock().ok().and_then(|l| l.get(record).cloned());
                match leak {
                    Some(text) => {
                        ui.label(RichText::new(text).small());
                    }
                    None => {
                        if ui.small_button("утечки?").on_hover_text("Проверить по базе Have I Been Pwned: уходят только 5 знаков SHA-1").clicked() {
                            check_leak(shell, record.clone(), login.password.clone());
                        }
                    }
                }
                if ui.small_button("удалить").clicked() {
                    delete = Some(record.clone());
                }
            });
        });
    }
    if shell.profile.logins.is_empty() {
        ui.label("Сохранённых паролей нет. OBSOS предложит сохранить пароль, когда вы войдёте на сайт.");
    }
    if let Some(record) = delete {
        shell.profile.delete_login(&record);
    }
}

/// Проверка утечки в фоне (сеть), ответ — в `tools.leaks`.
fn check_leak(shell: &mut Shell, record: String, password: String) {
    let leaks = shell.tools.leaks.clone();
    if let Ok(mut l) = leaks.lock() {
        l.insert(record.clone(), String::from("проверяю…"));
    }
    let _ = std::thread::Builder::new().name(String::from("obsos-leak")).spawn(move || {
        let text = match crate::tools::leak_count(&password) {
            Ok(0) => String::from("✓ в утечках не найден"),
            Ok(n) => format!("⚠ найден в утечках {n} раз — смените"),
            Err(e) => format!("не проверить: {e}"),
        };
        if let Ok(mut l) = leaks.lock() {
            l.insert(record, text);
        }
    });
}

/// Генератор паролей, импорт и экспорт паролей и закладок.
fn password_tools(ui: &mut Ui, shell: &mut Shell) {
    egui::CollapsingHeader::new("Сгенерировать пароль").show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.add(egui::Slider::new(&mut shell.tools.gen_len, 8..=64).text("знаков"));
            ui.checkbox(&mut shell.tools.gen_symbols, "знаки !#$");
        });
        ui.horizontal_wrapped(|ui| {
            if ui.button("Сгенерировать").clicked() {
                shell.tools.generated = crate::tools::generate_password(shell.tools.gen_len, shell.tools.gen_symbols);
            }
            if !shell.tools.generated.is_empty() && ui.button("Копировать").clicked() {
                crate::window::clipboard_set(&shell.tools.generated);
                *shell.delegate.clipboard.borrow_mut() = shell.tools.generated.clone();
                shell.toast("Пароль скопирован");
            }
        });
        if !shell.tools.generated.is_empty() {
            ui.label(RichText::new(&shell.tools.generated).monospace().strong());
        }
    });
    egui::CollapsingHeader::new("Импорт и экспорт").show(ui, |ui| {
        ui.label(RichText::new("Файл: пароли — CSV из Chrome/Firefox/Edge, закладки — HTML («Экспорт закладок»).").small());
        ui.add(egui::TextEdit::singleline(&mut shell.tools.import_path).desired_width(f32::INFINITY));
        ui.horizontal_wrapped(|ui| {
            if ui.button("Импорт паролей (CSV)").clicked() {
                let msg = match std::fs::read_to_string(shell.tools.import_path.trim()).map_err(|e| e.to_string()).and_then(|t| crate::tools::parse_passwords_csv(&t)) {
                    Ok(list) => {
                        let n = list.len();
                        for (origin, username, password) in list {
                            shell.save_login(Login { origin, username, password, updated_ms: now_ms() });
                        }
                        format!("Импортировано паролей: {n}")
                    }
                    Err(e) => format!("Не импортировать: {e}"),
                };
                shell.toast(&msg);
            }
            if ui.button("Импорт закладок (HTML)").clicked() {
                let msg = match std::fs::read_to_string(shell.tools.import_path.trim()) {
                    Ok(t) => {
                        let list = crate::tools::parse_bookmarks_html(&t);
                        let n = list.len();
                        for (url, title) in list {
                            if !shell.profile.is_bookmarked(&url) {
                                shell.profile.add_bookmark(&url, &title);
                            }
                        }
                        shell.update_page_data();
                        format!("Импортировано закладок: {n}")
                    }
                    Err(e) => format!("Не открыть файл: {e}"),
                };
                shell.toast(&msg);
            }
        });
        ui.horizontal_wrapped(|ui| {
            let dir = shell.delegate.downloads.dir.borrow().clone();
            if ui.button("Экспорт закладок").clicked() {
                let items: Vec<(String, String)> = shell.profile.bookmarks.iter().map(|b| (b.url.clone(), b.title.clone())).collect();
                let path = dir.join("obsos-bookmarks.html");
                let _ = std::fs::create_dir_all(&dir);
                let msg = match std::fs::write(&path, crate::tools::bookmarks_html(&items)) {
                    Ok(()) => format!("Закладки: {}", path.display()),
                    Err(e) => format!("Не записать: {e}"),
                };
                shell.toast(&msg);
            }
            if ui.button("Экспорт паролей (CSV, открытым текстом!)").clicked() {
                let items: Vec<(String, String, String)> = shell.profile.logins.values().map(|l| (l.origin.clone(), l.username.clone(), l.password.clone())).collect();
                let path = dir.join("obsos-passwords.csv");
                let _ = std::fs::create_dir_all(&dir);
                let msg = match std::fs::write(&path, crate::tools::passwords_csv(&items)) {
                    Ok(()) => format!("Пароли: {} — удалите файл после переноса", path.display()),
                    Err(e) => format!("Не записать: {e}"),
                };
                shell.toast(&msg);
            }
        });
    });
}

fn remote_tabs(ui: &mut Ui, shell: &mut Shell, actions: &mut Vec<Action>) {
    let devices = shell.profile.remote_tabs();
    if devices.is_empty() {
        ui.label("Других устройств пока нет: вкладки появятся после синхронизации с другого компьютера или телефона.");
    }
    for d in devices {
        ui.label(RichText::new(format!("{} · {}", d.device, when(d.hlc.wall))).strong());
        for t in &d.tabs {
            ui.horizontal_wrapped(|ui| {
                if ui.small_button("×").on_hover_text("Закрыть на том устройстве").clicked() {
                    let _ = shell.profile.replica.close_remote_tab(&d.device, &t.id, now_ms());
                }
                if ui.link(clip(if t.title.is_empty() { &t.url } else { &t.title }, 40)).on_hover_text(&t.url).clicked() {
                    actions.push(Action::NewTabUrl(t.url.clone()));
                }
            });
        }
        ui.separator();
    }
}

fn sync_panel(ui: &mut Ui, shell: &mut Shell, actions: &mut Vec<Action>) {
    let active = shell.account();
    match &active {
        Some(a) => {
            ui.label(RichText::new(format!("Включена: {} · {}", a.provider.title(), a.repo)).strong());
        }
        None => {
            ui.label("Синхронизация выключена. Выберите, где хранить закладки, историю, оформление и пароли:");
        }
    }
    if !shell.sync.status.is_empty() {
        ui.label(RichText::new(&shell.sync.status).small());
    }
    if active.is_some() {
        ui.horizontal_wrapped(|ui| {
            if ui.add_enabled(shell.syncer.busy == 0, egui::Button::new("Синхронизировать")).clicked() {
                actions.push(Action::SyncNow(None));
            }
            if ui.button("Выключить").clicked() {
                shell.sync_off();
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.add(egui::TextEdit::singleline(&mut shell.sync.label).hint_text("подпись точки").desired_width(170.0));
            if ui.add_enabled(shell.syncer.busy == 0, egui::Button::new("С меткой")).on_hover_text(if active.as_ref().is_some_and(|a| a.provider.is_git()) {
                "Синхронизировать и поставить тег obsos-дата-подпись: к нему можно откатиться"
            } else {
                "Синхронизировать и сохранить копию (points/…): к ней можно откатиться"
            }).clicked() {
                actions.push(Action::SyncNow(Some(std::mem::take(&mut shell.sync.label))));
            }
        });
        ui.checkbox(&mut shell.profile.settings.sync_auto, "Синхронизировать само");
        ui.horizontal_wrapped(|ui| {
            ui.label("каждые");
            ui.add(egui::DragValue::new(&mut shell.profile.settings.sync_every_min).range(1..=1440).suffix(" мин"));
        });
        ui.checkbox(&mut shell.profile.settings.sync_points_auto, "Ставить точку (тег) при каждой синхронизации");
        ui.separator();
        ui.label(RichText::new("Точки синхронизации").strong());
        if ui.small_button("обновить список").clicked() {
            shell.load_points();
        }
        let points = shell.sync.points.clone();
        if points.is_empty() {
            ui.label(RichText::new("Пока нет — «С меткой» создаёт точку.").small());
        }
        for point in points.iter().take(50) {
            ui.horizontal_wrapped(|ui| {
                if ui.small_button("откатить").on_hover_text("Вернуть закладки, историю, оформление и пароли к этой точке").clicked() {
                    shell.modal = Some(Modal::Rollback(point.name.clone()));
                }
                ui.label(&point.name);
                if !point.label.is_empty() {
                    ui.label(RichText::new(&point.label).small());
                }
            });
        }
        ui.separator();
    }
    ui.label(RichText::new("Площадка").strong());
    egui::ComboBox::from_id_salt("provider").selected_text(shell.sync.draft.provider.title()).width(260.0).show_ui(ui, |ui| {
        for provider in Provider::ALL {
            if ui.selectable_label(shell.sync.draft.provider == provider, provider.title()).clicked() {
                shell.select_provider(provider);
            }
        }
    });
    let provider = shell.sync.draft.provider;
    let draft = &mut shell.sync.draft;
    let field = |ui: &mut Ui, label: &str, value: &mut String, hint: &str, password: bool| {
        ui.label(RichText::new(label).small());
        ui.add(egui::TextEdit::singleline(value).hint_text(hint).password(password).desired_width(f32::INFINITY));
    };
    match provider {
        Provider::GitHub | Provider::GitLab | Provider::Bitbucket => {
            field(ui, "Репозиторий (создастся закрытым, если его нет)", &mut draft.repo, "имя/obsos-sync", false);
            field(ui, "Ветка", &mut draft.branch, "main", false);
            if provider == Provider::Bitbucket {
                field(ui, "Имя пользователя или почта (для пароля приложения)", &mut draft.user, "roman", false);
            }
            if provider == Provider::GitLab {
                field(ui, "Свой сервер GitLab (пусто — gitlab.com)", &mut draft.host, "https://gitlab.example.org", false);
            }
            field(ui, if provider == Provider::Bitbucket { "Пароль приложения или токен" } else { "Токен доступа" }, &mut draft.token, "", true);
        }
        Provider::YandexDisk => {
            field(ui, "Папка на Диске", &mut draft.repo, "OBSOS", false);
            field(ui, "Логин Яндекса", &mut draft.user, "roman@yandex.ru", false);
            field(ui, "Пароль приложения (раздел «Файлы»)", &mut draft.token, "", true);
        }
        Provider::WebDav => {
            field(ui, "Адрес WebDAV", &mut draft.host, "https://cloud.example.org/remote.php/dav/files/roman", false);
            field(ui, "Папка", &mut draft.repo, "OBSOS", false);
            field(ui, "Имя", &mut draft.user, "", false);
            field(ui, "Пароль приложения", &mut draft.token, "", true);
        }
        Provider::GoogleDrive => {
            field(ui, "Папка на Google Диске", &mut draft.repo, "OBSOS", false);
            ui.label(RichText::new("Google Диск — только вход через браузер: нужен OAuth-клиент типа «Телевизоры и устройства с ограниченным вводом».").small());
        }
        Provider::Folder => {
            field(ui, "Папка (флешка, сетевой диск)", &mut draft.repo, "/media/usb/OBSOS", false);
        }
    }
    if oauth::supported(provider) {
        egui::CollapsingHeader::new("Вход через браузер (OAuth Device Flow)").default_open(provider == Provider::GoogleDrive).show(ui, |ui| {
            field(ui, "client_id OAuth-приложения", &mut draft.client_id, "", false);
            if matches!(provider, Provider::GoogleDrive | Provider::YandexDisk) {
                field(ui, "client_secret", &mut draft.client_secret, "", true);
            }
        });
    }
    let token_page = provider.token_page();
    let can_device = oauth::supported(provider) && !shell.sync.draft.client_id.is_empty();
    ui.horizontal_wrapped(|ui| {
        if can_device && ui.button("Войти через браузер").clicked() {
            shell.login_start();
        }
        if !token_page.is_empty() && ui.button("Где взять токен").clicked() {
            actions.push(Action::NewTabUrl(token_page.to_string()));
        }
        if ui.button("Сохранить и включить").clicked() {
            shell.check_account();
        }
    });
    if let Some(flow) = &shell.sync.flow {
        ui.label(RichText::new(format!("Код: {} — введите его на {}", flow.code.user_code, flow.code.verification_uri)).strong());
    }
    ui.separator();
    ui.label(RichText::new("Пароль синхронизации паролей").strong());
    let shared = shell.vault.as_ref().is_some_and(|v| v.sync_key_shared);
    ui.label(RichText::new(if shared {
        "Задан: пароли расшифровываются на всех устройствах с тем же паролем."
    } else {
        "Не задан: пароли синхронизируются, но прочитать их может только это устройство. Задайте один и тот же пароль на всех устройствах."
    }).small());
    ui.horizontal_wrapped(|ui| {
        ui.add(egui::TextEdit::singleline(&mut shell.sync.sync_password).password(true).hint_text("пароль синхронизации").desired_width(200.0));
        if ui.button("Задать").clicked() {
            shell.set_sync_password();
        }
    });
    ui.label(RichText::new(format!("Это устройство: {} ({})", shell.profile.settings.device_name, shell.profile.settings.device)).small());
}

fn scripts(ui: &mut Ui, shell: &mut Shell) {
    let dir = shell.profile.dir.join("userscripts");
    ui.label(RichText::new(format!(
        "У Servo нет расширений. Вместо них OBSOS подключает к каждой странице ваши скрипты (*.js) и стили (*.css) из папки {} — изменения действуют на новых вкладках.",
        dir.display()
    )).small());
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&dir).map(|r| r.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    files.sort();
    if files.is_empty() {
        ui.label("Папка пуста.");
    }
    for path in files {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let enabled = name.ends_with(".js") || name.ends_with(".css");
        ui.horizontal_wrapped(|ui| {
            let mut on = enabled;
            if ui.checkbox(&mut on, &name).changed() {
                let target = if on { path.with_extension("") } else { path.with_extension(format!("{}.off", path.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default())) };
                let _ = std::fs::rename(&path, target);
            }
        });
    }
    if ui.button("Создать пример").clicked() {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join("example.css.off"), "/* Пример пользовательского стиля OBSOS: уберите .off из имени */\nbody { font-size: 105% !important; }\n");
        let _ = std::fs::write(dir.join("example.js.off"), "// Пример пользовательского скрипта OBSOS: уберите .off из имени\nconsole.log('OBSOS userscript on ' + location.host);\n");
    }
}

/// Открыть скачанный файл: текст и Markdown — страницей просмотра OBSOS,
/// картинки и HTML — вкладкой, пакет — `pkg install`, остальное — «Файлы»
/// с этим файлом (там своё «Открыть с помощью»).
pub fn open_download(path: &std::path::Path, actions: &mut Vec<Action>) {
    let shown = path.display().to_string();
    let lower = shown.to_ascii_lowercase();
    println!("browser: open download {}", crate::logs::text(&shown));
    if crate::pages::viewable_path(&shown) {
        actions.push(Action::NewTabUrl(crate::pages::view_url(&shown).to_string()));
    } else if [".html", ".htm", ".png", ".jpg", ".jpeg", ".gif", ".webp", ".svg", ".bmp"].iter().any(|e| lower.ends_with(e)) {
        let url = url::Url::from_file_path(path).map(|u| u.to_string()).unwrap_or_else(|()| format!("file://{shown}"));
        actions.push(Action::NewTabUrl(url));
    } else if let Some(result) = crate::window::platform().install_package(path) {
        if let Err(e) = result {
            println!("browser: install {shown} failed: {e}");
        }
    } else {
        show_in_folder(path);
    }
}

/// «Показать в папке»: «Файлы» открываются на папке с выделенным файлом.
pub fn show_in_folder(path: &std::path::Path) {
    println!("browser: show in folder {}", path.display());
    crate::window::show_in_folder(path);
}

/// Строка состояния загрузки: «37% · 12,3 МБ из 1,0 ГБ · 5,1 МБ/с · ещё 2 мин».
pub(crate) fn download_status(item: &crate::downloads::Item) -> String {
    use crate::downloads::{State, human_size, human_time};
    match &item.state {
        State::Running => {
            // Доля — первой: на узкой плашке хвост строки обрезается.
            let mut parts = Vec::new();
            if let Some(f) = item.fraction() {
                parts.push(format!("{:.0}%", f * 100.0));
            }
            parts.push(match item.total {
                Some(t) => format!("{} из {}", human_size(item.got), human_size(t)),
                None => human_size(item.got),
            });
            if item.rate > 0.0 {
                parts.push(format!("{}/с", human_size(item.rate as u64)));
            }
            if let Some(eta) = item.eta() {
                parts.push(format!("ещё {}", human_time(eta)));
            }
            parts.join(" · ")
        }
        State::Asking => String::from("куда сохранить?"),
        State::Probing | State::Page => String::from("проверка ссылки…"),
        State::Done(_) => format!("готово · {}", human_size(item.got)),
        State::Failed(e) => format!("ошибка: {e}"),
        State::Cancelled => String::from("отменено"),
    }
}

/// Полоса загрузок внизу окна (как в Chrome): последние загрузки плашками —
/// имя, ход, скорость; у идущей — отмена, у готовой — «Открыть» и «Показать
/// в папке»; справа «Все загрузки» и крестик.
fn downloads_bar(ctx: &egui::Context, shell: &mut Shell, rect: Rect, p: &Palette, actions: &mut Vec<Action>) {
    use crate::downloads::State;
    let items = shell.delegate.downloads.snapshot();
    if items.iter().any(|d| d.active()) {
        ctx.request_repaint_after(std::time::Duration::from_millis(300));
    }
    egui::Area::new(egui::Id::new("obsos-downloads-bar")).order(egui::Order::Foreground).fixed_pos(rect.min).show(ctx, |ui| {
        let (r, _) = ui.allocate_exact_size(rect.size(), Sense::hover());
        ui.painter().rect_filled(r, 0.0, p.toolbar);
        ui.painter().line_segment([r.left_top(), r.right_top()], Stroke::new(1.0_f32, mix(p.toolbar, p.text, 0.18)));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r.shrink2(vec2(8.0, 6.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
        let ui = &mut child;
        // Справа: крестик и «Все загрузки».
        let right_w = 150.0;
        let chips_w = (r.width() - 16.0 - right_w).max(200.0);
        let chip_w = 300.0_f32.min(chips_w);
        let fit = ((chips_w / (chip_w + 6.0)) as usize).max(1);
        for item in items.iter().rev().take(fit) {
            let (chip, resp) = ui.allocate_exact_size(vec2(chip_w, r.height() - 12.0), Sense::click());
            let hovered = resp.hovered();
            ui.painter().rect_filled(chip, 4.0, if hovered { p.hover } else { mix(p.toolbar, p.sidebar, 0.5) });
            ui.painter().rect_stroke(chip, 4.0, Stroke::new(1.0_f32, mix(p.toolbar, p.text, 0.12)), egui::StrokeKind::Inside);
            // Кольцо хода слева (как в Chrome): доля — дугой.
            let ring = pos2(chip.left() + 20.0, chip.center().y);
            let color = match item.state {
                State::Failed(_) | State::Cancelled => Color32::from_rgb(0xe0, 0x70, 0x60),
                State::Done(_) => p.accent2,
                _ => p.accent,
            };
            ui.painter().circle_stroke(ring, 11.0, Stroke::new(2.5_f32, mix(p.toolbar, p.text, 0.15)));
            let frac = match item.state {
                State::Done(_) => 1.0,
                _ => item.fraction().unwrap_or(0.0),
            };
            if frac > 0.0 {
                let n = (frac * 40.0).ceil().max(2.0) as usize;
                let pts: Vec<Pos2> = (0..=n)
                    .map(|i| {
                        let a = -std::f32::consts::FRAC_PI_2 + i as f32 / n as f32 * frac * std::f32::consts::TAU;
                        ring + vec2(a.cos() * 11.0, a.sin() * 11.0)
                    })
                    .collect();
                ui.painter().add(egui::Shape::line(pts, Stroke::new(2.5_f32, color)));
            } else if matches!(item.state, State::Running) {
                // Размер неизвестен — бегущая дуга.
                let t = ui.input(|i| i.time) as f32 * 4.0;
                let pts: Vec<Pos2> = (0..=12).map(|i| ring + vec2((t + i as f32 * 0.12).cos() * 11.0, (t + i as f32 * 0.12).sin() * 11.0)).collect();
                ui.painter().add(egui::Shape::line(pts, Stroke::new(2.5_f32, color)));
            }
            if let State::Done(_) = item.state {
                ui.painter().line_segment([ring + vec2(-5.0, 0.0), ring + vec2(-1.0, 4.0)], Stroke::new(2.0_f32, color));
                ui.painter().line_segment([ring + vec2(-1.0, 4.0), ring + vec2(5.0, -4.0)], Stroke::new(2.0_f32, color));
            }
            let text_x = chip.left() + 40.0;
            let name_w = chip.right() - text_x - 34.0;
            let name = clip(&item.name, (name_w / 7.5) as usize);
            ui.painter().text(pos2(text_x, chip.center().y - 8.0), Align2::LEFT_CENTER, name, FontId::proportional(13.0), p.text);
            let status = clip(&download_status(item), (name_w / 6.5) as usize);
            ui.painter().text(pos2(text_x, chip.center().y + 9.0), Align2::LEFT_CENTER, status, FontId::proportional(11.5), p.dim);
            // Кнопка справа на плашке: отмена у идущей, «Показать в папке» у готовой.
            let btn = Rect::from_center_size(pos2(chip.right() - 18.0, chip.center().y), vec2(26.0, 26.0));
            let btn_resp = ui.interact(btn, ui.id().with(("dl-btn", item.id)), Sense::click());
            if btn_resp.hovered() {
                ui.painter().rect_filled(btn, 3.0, mix(p.hover, p.text, 0.1));
            }
            match &item.state {
                State::Running | State::Asking => {
                    paint_icon(ui.painter(), btn, Icon::Close, p.text);
                    if btn_resp.on_hover_text("Отменить").clicked() {
                        println!("browser: download cancel {}", crate::logs::text(&item.name));
                        shell.delegate.downloads.cancel(item.id);
                    }
                }
                State::Done(path) => {
                    paint_folder(ui.painter(), btn, p.text);
                    if btn_resp.on_hover_text("Показать в папке").clicked() {
                        show_in_folder(path);
                    }
                }
                _ => {
                    paint_icon(ui.painter(), btn, Icon::Close, p.dim);
                    if btn_resp.on_hover_text("Убрать из списка").clicked() {
                        shell.delegate.downloads.remove(item.id);
                    }
                }
            }
            // Щелчок по плашке: готовую — открыть; иначе — панель загрузок.
            if resp.clicked() && !btn.contains(resp.interact_pointer_pos().unwrap_or(btn.center())) {
                match &item.state {
                    State::Done(path) => open_download(path, actions),
                    _ => actions.push(Action::Panel(Panel::Downloads)),
                }
            }
            ui.add_space(6.0);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if icon_button(ui, Icon::Close, true, "Скрыть полосу загрузок", p) {
                shell.dl.bar = false;
            }
            if ui.button("Все загрузки").clicked() {
                actions.push(Action::Panel(Panel::Downloads));
            }
        });
    });
}

/// Значок папки (кнопка «Показать в папке»).
pub(crate) fn paint_folder(painter: &egui::Painter, rect: Rect, color: Color32) {
    let c = rect.center();
    let s = Stroke::new(1.6_f32, color);
    let pts = vec![c + vec2(-7.0, -5.0), c + vec2(-2.0, -5.0), c + vec2(0.0, -3.0), c + vec2(7.0, -3.0), c + vec2(7.0, 5.0), c + vec2(-7.0, 5.0)];
    painter.add(egui::Shape::closed_line(pts, s));
}

fn profiles(ui: &mut Ui, shell: &mut Shell) {
    ui.label(RichText::new("У каждого профиля свои куки, закладки, пароли и синхронизация.").small());
    if shell.profile.private {
        ui.label(RichText::new("Это окно инкогнито: история, куки и вход забываются при закрытии.").strong());
    }
    if ui.button("Окно гостя (инкогнито, Ctrl+Shift+N)").clicked() {
        crate::window::new_window(&["--private"]);
    }
    for name in crate::profile::Profile::list() {
        ui.horizontal_wrapped(|ui| {
            ui.label(if name == shell.profile.name { format!("● {name} (открыт)") } else { name.clone() });
            if name != shell.profile.name && ui.small_button("открыть").clicked() {
                crate::window::new_window(&["--profile", &name]);
            }
        });
    }
    ui.horizontal_wrapped(|ui| {
        ui.add(egui::TextEdit::singleline(&mut shell.new_profile).hint_text("имя нового профиля").desired_width(180.0));
        if ui.button("Создать и открыть").clicked() && !shell.new_profile.trim().is_empty() {
            let name: String = shell.new_profile.chars().filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_').collect();
            crate::window::new_window(&["--profile", &name]);
            shell.new_profile.clear();
        }
    });
}

// ── Диалоги ─────────────────────────────────────────────────────────────────

/// Пункты контекстного меню, которые Servo отдаёт по-английски.
fn translate(label: &str) -> &str {
    match label {
        "Cut" => "Вырезать",
        "Copy" => "Копировать",
        "Paste" => "Вставить",
        "Select All" => "Выделить всё",
        "Back" => "Назад",
        "Forward" => "Вперёд",
        "Reload" => "Обновить",
        "Copy Link" | "Copy Link Address" => "Копировать адрес ссылки (из страницы)",
        "Open Link in New View" | "Open Link in New Tab" => "Открыть ссылку в новой вкладке (движок)",
        "Copy Image Link" | "Copy Image Address" => "Копировать адрес картинки (движок)",
        "Open Image in New View" | "Open Image in New Tab" => "Открыть картинку (движок)",
        "Undo" => "Отменить",
        "Redo" => "Повторить",
        "Delete" => "Удалить",
        other => other,
    }
}

fn modal(ctx: &egui::Context, shell: &mut Shell, page: Rect, p: &Palette, actions: &mut Vec<Action>) {
    let Some(m) = shell.modal.take() else { return };
    let mut put_back: Option<Modal> = None;
    // Esc закрывает меню и список страницы.
    if matches!(m, Modal::Select(..) | Modal::Context(..)) && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        match m {
            Modal::Select(select, _) => select.submit(),
            Modal::Context(menu, _) => menu.dismiss(),
            _ => {}
        }
        return;
    }
    // Затемнение страницы — щелчок мимо закрывает меню и списки.
    let backdrop = egui::Area::new(egui::Id::new("obsos-backdrop")).order(egui::Order::Foreground).fixed_pos(page.min).show(ctx, |ui| {
        let (r, resp) = ui.allocate_exact_size(page.size(), Sense::click());
        let dim = matches!(m, Modal::Select(..) | Modal::Context(..));
        if !dim {
            ui.painter().rect_filled(r, 0.0, Color32::from_black_alpha(90));
        }
        resp.clicked()
    });
    let center = page.center();
    let window = |title: &str| {
        egui::Window::new(title.to_string())
            .collapsible(false)
            .resizable(false)
            .order(egui::Order::Tooltip)
            .pivot(Align2::CENTER_CENTER)
            .fixed_pos(center)
            .frame(egui::Frame::window(&ctx.global_style()).fill(p.sidebar))
    };
    match m {
        Modal::Dialog(dialog, mut value) => {
            let mut done: Option<bool> = None;
            let title = match &dialog {
                SimpleDialog::Alert(_) => "Сообщение страницы",
                SimpleDialog::Confirm(_) => "Подтверждение",
                SimpleDialog::Prompt(_) => "Вопрос страницы",
            };
            window(title).show(ctx, |ui| {
                ui.set_max_width(420.0);
                ui.label(dialog.message());
                if matches!(dialog, SimpleDialog::Prompt(_)) {
                    let r = ui.add(egui::TextEdit::singleline(&mut value).desired_width(380.0));
                    r.request_focus();
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        done = Some(true);
                    }
                }
                ui.horizontal(|ui| {
                    if ui.button("OK").clicked() {
                        done = Some(true);
                    }
                    if !matches!(dialog, SimpleDialog::Alert(_)) && ui.button("Отмена").clicked() {
                        done = Some(false);
                    }
                });
            });
            match done {
                Some(true) => {
                    let mut dialog = dialog;
                    if let SimpleDialog::Prompt(p) = &mut dialog {
                        p.set_current_value(&value);
                    }
                    dialog.confirm();
                }
                Some(false) => dialog.dismiss(),
                None => put_back = Some(Modal::Dialog(dialog, value)),
            }
        }
        Modal::Auth { request, host, mut user, mut pass, mut remember } => {
            let mut done: Option<bool> = None;
            window("Вход на сайт").show(ctx, |ui| {
                ui.label(format!("Сайт {host} просит имя и пароль."));
                ui.add(egui::TextEdit::singleline(&mut user).hint_text("имя").desired_width(300.0));
                let r = ui.add(egui::TextEdit::singleline(&mut pass).password(true).hint_text("пароль").desired_width(300.0));
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    done = Some(true);
                }
                ui.checkbox(&mut remember, "Запомнить");
                ui.horizontal(|ui| {
                    if ui.button("Войти").clicked() {
                        done = Some(true);
                    }
                    if ui.button("Отмена").clicked() {
                        done = Some(false);
                    }
                });
            });
            match done {
                Some(true) => {
                    if remember {
                        if let Some(origin) = crate::vault::origin_of(request.url()) {
                            shell.save_login(Login { origin, username: user.clone(), password: pass.clone(), updated_ms: now_ms() });
                        }
                    }
                    request.authenticate(user, pass);
                }
                Some(false) => drop(request),
                None => put_back = Some(Modal::Auth { request, host, user, pass, remember }),
            }
        }
        Modal::Select(mut select, at) => {
            let mut chosen: Option<usize> = None;
            let current = select.selected_options();
            egui::Area::new(egui::Id::new("obsos-select")).order(egui::Order::Tooltip).fixed_pos(at).show(ctx, |ui| {
                egui::Frame::popup(&ctx.global_style()).fill(p.sidebar).show(ui, |ui| {
                    egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                        ui.set_min_width(180.0);
                        let mut option = |ui: &mut Ui, o: &servo::SelectElementOption| {
                            let r = ui.add_enabled(!o.is_disabled, egui::Button::selectable(current.contains(&o.id), &o.label));
                            if r.clicked() {
                                chosen = Some(o.id);
                            }
                        };
                        for item in select.options() {
                            match item {
                                SelectElementOptionOrOptgroup::Option(o) => option(ui, o),
                                SelectElementOptionOrOptgroup::Optgroup { label, options } => {
                                    ui.label(RichText::new(label).strong());
                                    for o in options {
                                        option(ui, o);
                                    }
                                }
                            }
                        }
                    });
                });
            });
            if let Some(id) = chosen {
                select.select(vec![id]);
                select.submit();
                println!("browser: select option {id}");
            } else if backdrop.inner {
                select.submit();
            } else {
                put_back = Some(Modal::Select(select, at));
            }
        }
        Modal::Context(menu, at) => {
            let mut chosen = None;
            let mut save: Option<String> = None;
            let mut extra: Option<Action> = None;
            let info = menu.element_info().clone();
            let link = info.link_url.filter(|u| matches!(u.scheme(), "http" | "https")).map(|u| u.to_string());
            let image = info.image_url.filter(|u| matches!(u.scheme(), "http" | "https")).map(|u| u.to_string());
            egui::Area::new(egui::Id::new("obsos-context")).order(egui::Order::Tooltip).fixed_pos(at).show(ctx, |ui| {
                egui::Frame::popup(&ctx.global_style()).fill(p.sidebar).show(ui, |ui| {
                    ui.set_min_width(200.0);
                    // Свои пункты (Servo их не знает): ссылка и картинка —
                    // в новую вкладку, копировать, сохранить как.
                    let entry = |ui: &mut Ui, text: &str| ui.add(egui::Button::new(text).frame(false)).clicked();
                    if let Some(url) = &link {
                        if entry(ui, "Открыть ссылку в новой вкладке") {
                            extra = Some(Action::OpenBackground(url.clone()));
                        }
                        if entry(ui, "Копировать адрес ссылки") {
                            crate::window::clipboard_set(url);
                            extra = Some(Action::Toast("Адрес ссылки скопирован"));
                        }
                        if entry(ui, "Сохранить ссылку как…") {
                            save = Some(url.clone());
                        }
                    }
                    if let Some(url) = &image {
                        if entry(ui, "Открыть картинку в новой вкладке") {
                            extra = Some(Action::OpenBackground(url.clone()));
                        }
                        if entry(ui, "Копировать адрес картинки") {
                            crate::window::clipboard_set(url);
                            extra = Some(Action::Toast("Адрес картинки скопирован"));
                        }
                        if entry(ui, "Сохранить картинку как…") {
                            save = Some(url.clone());
                        }
                    }
                    if link.is_some() || image.is_some() {
                        ui.separator();
                    }
                    for item in menu.items() {
                        match item {
                            ContextMenuItem::Item { label, action, enabled } => {
                                if ui.add_enabled(*enabled, egui::Button::new(translate(label)).frame(false)).clicked() {
                                    chosen = Some(*action);
                                }
                            }
                            ContextMenuItem::Separator => {
                                ui.separator();
                            }
                        }
                    }
                    ui.separator();
                    if ui.add(egui::Button::new("Просмотреть код").frame(false)).clicked() {
                        extra = Some(Action::InspectAt(at.x, at.y));
                    }
                });
            });
            if let Some(action) = extra {
                menu.dismiss();
                actions.push(action);
            } else if let Some(url) = save {
                println!("browser: context save as {}", crate::logs::url(&url));
                menu.dismiss();
                actions.push(Action::SaveAs(url));
            } else if let Some(action) = chosen {
                menu.select(action);
            } else if backdrop.inner {
                menu.dismiss();
            } else {
                put_back = Some(Modal::Context(menu, at));
            }
        }
        Modal::Color(mut picker, mut rgb) => {
            let mut done: Option<bool> = None;
            window("Цвет").show(ctx, |ui| {
                ui.color_edit_button_srgb(&mut rgb);
                ui.horizontal(|ui| {
                    if ui.button("Выбрать").clicked() {
                        done = Some(true);
                    }
                    if ui.button("Отмена").clicked() {
                        done = Some(false);
                    }
                });
            });
            match done {
                Some(true) => {
                    picker.select(Some(RgbColor { red: rgb[0], green: rgb[1], blue: rgb[2] }));
                    picker.submit();
                }
                Some(false) => picker.submit(),
                None => put_back = Some(Modal::Color(picker, rgb)),
            }
        }
        Modal::File(mut picker, mut path) => {
            let mut done: Option<bool> = None;
            window("Выбор файла").show(ctx, |ui| {
                ui.label("Путь к файлу:");
                let r = ui.add(egui::TextEdit::singleline(&mut path).desired_width(380.0));
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    done = Some(true);
                }
                let dir = std::path::Path::new(&path);
                let dir = if dir.is_dir() { dir.to_path_buf() } else { dir.parent().map(|d| d.to_path_buf()).unwrap_or_default() };
                egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                    let mut entries: Vec<_> = std::fs::read_dir(&dir).map(|r| r.flatten().map(|e| e.path()).collect()).unwrap_or_default();
                    entries.sort();
                    for e in entries.iter().take(200) {
                        let name = e.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        let label = if e.is_dir() { format!("{name}/") } else { name };
                        if ui.selectable_label(false, label).clicked() {
                            path = if e.is_dir() { format!("{}/", e.display()) } else { e.display().to_string() };
                        }
                    }
                });
                ui.horizontal(|ui| {
                    if ui.button("Открыть").clicked() {
                        done = Some(true);
                    }
                    if ui.button("Отмена").clicked() {
                        done = Some(false);
                    }
                });
            });
            match done {
                Some(true) => {
                    picker.select(&[std::path::PathBuf::from(path.clone())]);
                    picker.submit();
                }
                Some(false) => picker.dismiss(),
                None => put_back = Some(Modal::File(picker, path)),
            }
        }
        Modal::Master { mut text, error } => {
            let mut unlock = false;
            window("OBSOS — хранилище паролей").show(ctx, |ui| {
                ui.label("Введите мастер-пароль, чтобы открыть пароли и синхронизацию.");
                let r = ui.add(egui::TextEdit::singleline(&mut text).password(true).desired_width(300.0));
                r.request_focus();
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    unlock = true;
                }
                if !error.is_empty() {
                    ui.colored_label(Color32::from_rgb(0xe0, 0x60, 0x60), &error);
                }
                ui.horizontal(|ui| {
                    if ui.button("Открыть").clicked() {
                        unlock = true;
                    }
                });
            });
            if unlock {
                shell.unlock(&text);
            } else {
                put_back = Some(Modal::Master { text, error });
            }
        }
        Modal::Rollback(point) => {
            let mut done: Option<bool> = None;
            let history = point == "\u{0}history";
            window(if history { "Очистить историю" } else { "Откат к точке синхронизации" }).show(ctx, |ui| {
                ui.set_max_width(400.0);
                if history {
                    ui.label("Удалить всю историю на всех синхронизированных устройствах?");
                } else {
                    ui.label(format!(
                        "Вернуть закладки, историю, оформление и пароли к состоянию точки {point}? Изменения после неё уйдут на все устройства как новые — сама история площадки не переписывается, к нынешнему состоянию можно вернуться откатом к более поздней точке."
                    ));
                }
                ui.horizontal(|ui| {
                    if ui.button(if history { "Очистить" } else { "Откатить" }).clicked() {
                        done = Some(true);
                    }
                    if ui.button("Отмена").clicked() {
                        done = Some(false);
                    }
                });
            });
            match done {
                Some(true) if history => shell.profile.clear_history(),
                Some(true) => shell.rollback(point),
                Some(false) => {}
                None => put_back = Some(Modal::Rollback(point)),
            }
        }
    }
    let _ = actions;
    if shell.modal.is_none() {
        shell.modal = put_back;
    }
}
