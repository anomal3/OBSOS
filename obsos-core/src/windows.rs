//! Несколько окон одного браузера: общие профиль, загрузки, пароли и Servo, у
//! каждого окна свои вкладки и панели.
//!
//! Оболочка (`Shell`) всегда работает с одним, «текущим», окном: его поля
//! лежат прямо в ней, остальные окна отложены в `Windows::parked`
//! (`Shell::switch`).
//!
//! Страницы всех окон рисует один painter Servo в общий буфер («атлас»): у
//! каждого окна свой участок, окна — друг под другом. Поэтому вкладку можно
//! перенести в другое окно без перезагрузки: меняется только участок.

use std::collections::{BTreeSet, HashMap};
use std::time::Instant;

use servo::WebViewId;

use crate::shell::{Favicon, FindBar, Infobar, Modal, Panel, Shell, Tab};

/// Всё своё у окна, пока оно не текущее.
pub struct WindowState {
    pub key: u32,
    pub tabs: Vec<Tab>,
    pub active: usize,
    address: String,
    focus_address: bool,
    panel: Option<Panel>,
    modal: Option<Modal>,
    infobars: Vec<Infobar>,
    toasts: Vec<(String, Instant)>,
    filter: String,
    find: FindBar,
    dev: crate::devtools::DevUi,
    chrome: crate::chrome::ChromeState,
    show_passwords: BTreeSet<String>,
    new_profile: String,
    background_click: Option<Instant>,
    page_fullscreen: Option<WebViewId>,
    /// Размеры оболочки окна (`shell::window_metrics`).
    metrics: [u32; 5],
}

impl WindowState {
    fn new(key: u32, metrics: [u32; 5]) -> WindowState {
        WindowState {
            key,
            tabs: Vec::new(),
            active: 0,
            address: String::new(),
            focus_address: false,
            panel: None,
            modal: None,
            infobars: Vec::new(),
            toasts: Vec::new(),
            filter: String::new(),
            find: Default::default(),
            dev: Default::default(),
            chrome: Default::default(),
            show_passwords: Default::default(),
            new_profile: String::new(),
            background_click: None,
            page_fullscreen: None,
            metrics,
        }
    }
}

#[derive(Default)]
pub struct Windows {
    /// Номер текущего окна (первое — 0).
    pub key: u32,
    pub parked: Vec<WindowState>,
    last: u32,
    /// Размеры страниц окон в точках экрана; порядок — порядок участков.
    sizes: Vec<(u32, u32, u32)>,
    /// Просят новое окно (Ctrl+N).
    pub wanted: bool,
}

impl Windows {
    /// Участок общего буфера окна `key`: x, y, ширина, высота.
    pub fn region(&self, key: u32) -> [u32; 4] {
        let mut y = 0;
        for &(k, w, h) in &self.sizes {
            if k == key {
                return [0, y, w, h];
            }
            y += h;
        }
        [0, 0, 1, 1]
    }

    fn atlas(&self) -> dpi::PhysicalSize<u32> {
        let width = self.sizes.iter().map(|s| s.1).max().unwrap_or(1);
        let height = self.sizes.iter().map(|s| s.2).sum::<u32>();
        dpi::PhysicalSize::new(width.max(1), height.max(1))
    }
}

impl Shell {
    /// Сделать окно `key` текущим.
    pub fn switch(&mut self, key: u32) -> bool {
        if self.windows.key == key {
            return true;
        }
        let Some(at) = self.windows.parked.iter().position(|w| w.key == key) else { return false };
        let mut state = self.windows.parked.swap_remove(at);
        self.exchange(&mut state);
        self.windows.parked.push(state);
        true
    }

    /// Поменять местами поля текущего окна и `state`. Разбор по всем полям —
    /// чтобы новое поле окна нельзя было забыть.
    fn exchange(&mut self, state: &mut WindowState) {
        use std::mem::swap;
        let WindowState { key, tabs, active, address, focus_address, panel, modal, infobars, toasts, filter, find, dev, chrome, show_passwords, new_profile, background_click, page_fullscreen, metrics } = state;
        swap(&mut self.windows.key, key);
        swap(&mut self.tabs, tabs);
        swap(&mut self.active, active);
        swap(&mut self.address, address);
        swap(&mut self.focus_address, focus_address);
        swap(&mut self.panel, panel);
        swap(&mut self.modal, modal);
        swap(&mut self.infobars, infobars);
        swap(&mut self.toasts, toasts);
        swap(&mut self.filter, filter);
        swap(&mut self.find, find);
        swap(&mut self.dev, dev);
        swap(&mut self.chrome, chrome);
        swap(&mut self.show_passwords, show_passwords);
        swap(&mut self.new_profile, new_profile);
        swap(&mut self.background_click, background_click);
        swap(&mut self.page_fullscreen, page_fullscreen);
        let mine = crate::shell::window_metrics();
        crate::shell::set_window_metrics(*metrics);
        *metrics = mine;
    }

    /// Новое пустое окно (не текущее); масштаб — как у текущего.
    pub fn add_window(&mut self) -> u32 {
        self.windows.last += 1;
        let key = self.windows.last;
        let scale = crate::shell::window_metrics()[0];
        self.windows.parked.push(WindowState::new(key, [scale, 0, 0, 0, 0]));
        key
    }

    /// Закрыть окно `key` вместе с вкладками (адреса — в «закрытые»). Если оно
    /// было текущим, текущим становится другое.
    pub fn remove_window(&mut self, key: u32) {
        if self.windows.key == key {
            let Some(other) = self.windows.parked.first().map(|w| w.key) else { return };
            self.switch(other);
        }
        if let Some(at) = self.windows.parked.iter().position(|w| w.key == key) {
            let state = self.windows.parked.swap_remove(at);
            for tab in state.tabs {
                let closed = tab.webview.id();
                self.permissions.retain(|p| p.webview != closed);
                if let Some(url) = tab.real_url {
                    self.closed.push(url.to_string());
                }
            }
        }
        self.windows.sizes.retain(|s| s.0 != key);
        self.relayout();
    }

    /// Окно, где живёт вкладка.
    pub fn window_of(&self, id: WebViewId) -> Option<u32> {
        if self.tabs.iter().any(|t| t.webview.id() == id) {
            return Some(self.windows.key);
        }
        self.windows.parked.iter().find(|w| w.tabs.iter().any(|t| t.webview.id() == id)).map(|w| w.key)
    }

    /// Все вкладки всех окон.
    pub fn all_tabs(&self) -> impl Iterator<Item = &Tab> {
        self.tabs.iter().chain(self.windows.parked.iter().flat_map(|w| w.tabs.iter()))
    }

    /// Размер страницы текущего окна поменялся.
    pub fn set_page_size(&mut self, size: dpi::PhysicalSize<u32>) {
        let key = self.windows.key;
        match self.windows.sizes.iter_mut().find(|s| s.0 == key) {
            Some(slot) => (slot.1, slot.2) = (size.width, size.height),
            None => self.windows.sizes.push((key, size.width, size.height)),
        }
        self.relayout();
    }

    /// Раздать вкладкам участки общего буфера и подогнать его размер.
    pub fn relayout(&mut self) {
        let mut viewports = HashMap::new();
        if self.windows.sizes.len() > 1 {
            let current = std::iter::once((self.windows.key, &self.tabs));
            for (key, tabs) in current.chain(self.windows.parked.iter().map(|w| (w.key, &w.tabs))) {
                let [x, y, w, h] = self.windows.region(key);
                for tab in tabs {
                    viewports.insert(tab.webview.id(), [x as f32, y as f32, w as f32, h as f32]);
                }
            }
        }
        if let Ok(mut shared) = ::paint::obsos::VIEWPORTS.lock() {
            *shared = (!viewports.is_empty()).then_some(viewports);
        }
        // Painter один на всех: хватает одного вызова.
        let atlas = self.windows.atlas();
        if let Some(tab) = self.all_tabs().next() {
            tab.webview.resize(atlas);
        }
    }

    /// Перенести вкладку `index` текущего окна в окно `to` (на место `at`) без
    /// перезагрузки. Текущим остаётся прежнее окно; в целевом вкладка
    /// становится активной, когда оно само её покажет (`Shell::activate`).
    /// `false` — переносить нечего.
    pub fn move_tab_to(&mut self, index: usize, to: u32, at: usize) -> bool {
        if index >= self.tabs.len() || to == self.windows.key {
            return false;
        }
        let Some(target) = self.windows.parked.iter().position(|w| w.key == to) else { return false };
        let mut tab = self.tabs.remove(index);
        // Меню и диалоги страницы остаются от прежнего окна — закрыть.
        if self.page_fullscreen == Some(tab.webview.id()) {
            self.leave_page_fullscreen();
        }
        self.modal = None;
        self.infobars.clear();
        self.chrome = Default::default();
        // Значок — текстура интерфейса прежнего окна: взять заново.
        tab.favicon = tab.webview.favicon().and_then(|image| crate::shell::color_image(&image)).map(Favicon::Pending);
        let state = &mut self.windows.parked[target];
        let at = at.min(state.tabs.len());
        state.tabs.insert(at, tab);
        state.active = at;
        true
    }
}
