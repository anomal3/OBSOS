//! Рисование видеокартой (Windows): всё окно рисует видеокарта.
//!
//! Страницы всех окон — в один общий буфер («атлас»), у каждого окна свой
//! участок (`windows.rs`); оболочка — egui через OpenGL (`egui_glow`). Кадр
//! окна: копия участка буфера (`glBlitFramebuffer`) и поверх него оболочка,
//! показ — `SwapBuffers`. Кадр не копируется в память и не проходит через
//! процессор (прежний путь через `glReadPixels` и GDI давал рывки).
//!
//! Контекст OpenGL один на все окна: поверхности окон по очереди
//! привязываются к нему (surfman).

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{Sender, channel};
use std::time::{Duration, Instant};

use dpi::PhysicalSize;
use gleam::gl::{self, Gl};
use raw_window_handle::{DisplayHandle, RawDisplayHandle, RawWindowHandle, WindowHandle};
use servo::{DeviceIntRect, RefreshDriver, RenderingContext, RgbaImage, WindowRenderingContext};
use surfman::{Connection, Error, Surface, SurfaceAccess, SurfaceTexture, SurfaceType};

use crate::swgl_ctx::{PageContext, PresentHook};

// ── Такт кадров по экрану ───────────────────────────────────────────────────

#[link(name = "dwmapi")]
extern "system" {
    /// Ждёт следующей сборки кадра диспетчером окон — то есть такта экрана.
    fn DwmFlush() -> i32;
}

/// Такт анимаций и плавной прокрутки: по кадрам экрана, а не по таймеру
/// (таймер 60 Гц на экране 180 Гц давал неровные интервалы).
pub struct Vsync(Sender<Box<dyn Fn() + Send + 'static>>);

impl Vsync {
    pub fn new(hz: u32) -> Rc<Vsync> {
        let (tx, rx) = channel::<Box<dyn Fn() + Send + 'static>>();
        let period = Duration::from_secs(1) / hz.clamp(30, 500);
        let _ = std::thread::Builder::new().name(String::from("obsos-vsync")).spawn(move || {
            while let Ok(first) = rx.recv() {
                let started = Instant::now();
                // SAFETY: функция без параметров, звать можно из любого потока.
                let failed = unsafe { DwmFlush() } < 0;
                // Экран погашен или сборка кадров стоит: ответ приходит сразу.
                if failed || started.elapsed() < Duration::from_micros(300) {
                    std::thread::sleep(period);
                }
                let mut waiting = vec![first];
                waiting.extend(rx.try_iter());
                for callback in waiting {
                    callback();
                }
            }
        });
        Rc::new(Vsync(tx))
    }
}

impl RefreshDriver for Vsync {
    fn observe_next_frame(&self, callback: Box<dyn Fn() + Send + 'static>) {
        let _ = self.0.send(callback);
    }
}

// ── Буфер страниц ───────────────────────────────────────────────────────────

/// То, что видит Servo: свой буфер в контексте окна. Подключение и текстуры
/// из поверхностей — от контекста окна: по ним Servo рисует WebGL в своём
/// потоке и забирает кадр.
pub struct GpuPageContext {
    root: Rc<WindowRenderingContext>,
    gl: Rc<dyn Gl>,
    fbo: gl::GLuint,
    color: gl::GLuint,
    depth: gl::GLuint,
    size: Cell<PhysicalSize<u32>>,
    present_hook: RefCell<Option<PresentHook>>,
    refresh: Rc<dyn RefreshDriver>,
}

impl GpuPageContext {
    fn new(root: Rc<WindowRenderingContext>, size: PhysicalSize<u32>, refresh: Rc<dyn RefreshDriver>) -> Result<GpuPageContext, String> {
        let gl = root.gleam_gl_api();
        let fbo = gl.gen_framebuffers(1)[0];
        let buffers = gl.gen_renderbuffers(2);
        let this = GpuPageContext {
            root,
            gl,
            fbo,
            color: buffers[0],
            depth: buffers[1],
            size: Cell::new(size),
            present_hook: RefCell::new(None),
            refresh,
        };
        if !this.alloc(size) {
            return Err(String::from("page framebuffer incomplete"));
        }
        Ok(this)
    }

    fn alloc(&self, size: PhysicalSize<u32>) -> bool {
        let (w, h) = (size.width.max(1) as i32, size.height.max(1) as i32);
        let g = &self.gl;
        g.bind_framebuffer(gl::FRAMEBUFFER, self.fbo);
        g.bind_renderbuffer(gl::RENDERBUFFER, self.color);
        g.renderbuffer_storage(gl::RENDERBUFFER, gl::RGBA8, w, h);
        g.framebuffer_renderbuffer(gl::FRAMEBUFFER, gl::COLOR_ATTACHMENT0, gl::RENDERBUFFER, self.color);
        g.bind_renderbuffer(gl::RENDERBUFFER, self.depth);
        g.renderbuffer_storage(gl::RENDERBUFFER, gl::DEPTH24_STENCIL8, w, h);
        g.framebuffer_renderbuffer(gl::FRAMEBUFFER, gl::DEPTH_STENCIL_ATTACHMENT, gl::RENDERBUFFER, self.depth);
        g.bind_renderbuffer(gl::RENDERBUFFER, 0);
        let complete = g.check_frame_buffer_status(gl::FRAMEBUFFER) == gl::FRAMEBUFFER_COMPLETE;
        g.disable(gl::SCISSOR_TEST);
        g.clear_color(1.0, 1.0, 1.0, 1.0);
        g.clear(gl::COLOR_BUFFER_BIT);
        complete
    }
}

impl PageContext for GpuPageContext {
    fn set_present_hook(&self, hook: PresentHook) {
        *self.present_hook.borrow_mut() = Some(hook);
    }
}

impl RenderingContext for GpuPageContext {
    fn prepare_for_rendering(&self) {
        // WebRender берёт привязанный буфер за «экран».
        self.gl.bind_framebuffer(gl::FRAMEBUFFER, self.fbo);
    }

    fn read_to_image(&self, rect: DeviceIntRect) -> Option<RgbaImage> {
        self.make_current().ok()?;
        self.gl.bind_framebuffer(gl::FRAMEBUFFER, self.fbo);
        let (w, h) = (rect.width(), rect.height());
        if w <= 0 || h <= 0 {
            return None;
        }
        let y = self.size.get().height as i32 - rect.max.y;
        let pixels = self.gl.read_pixels(rect.min.x, y, w, h, gl::RGBA, gl::UNSIGNED_BYTE);
        let flipped: Vec<u8> = pixels.chunks_exact(w as usize * 4).rev().flatten().copied().collect();
        RgbaImage::from_raw(w as u32, h as u32, flipped)
    }

    fn size(&self) -> PhysicalSize<u32> {
        self.size.get()
    }

    fn resize(&self, size: PhysicalSize<u32>) {
        let size = PhysicalSize::new(size.width.max(1), size.height.max(1));
        if size == self.size.get() || self.make_current().is_err() {
            return;
        }
        self.size.set(size);
        if !self.alloc(size) {
            println!("browser: gpu: framebuffer {}x{} incomplete", size.width, size.height);
        }
    }

    fn present(&self) {
        // Пиксели остаются на видеокарте; окну нужно только знать о кадре.
        if let Some(hook) = self.present_hook.borrow().as_ref() {
            let size = self.size.get();
            hook(&[], size.width, size.height);
        }
    }

    fn make_current(&self) -> Result<(), Error> {
        self.root.make_current()
    }

    fn gleam_gl_api(&self) -> Rc<dyn Gl> {
        self.gl.clone()
    }

    fn glow_gl_api(&self) -> Arc<glow::Context> {
        self.root.glow_gl_api()
    }

    fn refresh_driver(&self) -> Option<Rc<dyn RefreshDriver>> {
        Some(self.refresh.clone())
    }

    fn connection(&self) -> Option<Connection> {
        self.root.connection()
    }

    fn create_texture(&self, surface: Surface) -> Option<(SurfaceTexture, u32, euclid::default::Size2D<i32>)> {
        self.root.create_texture(surface)
    }

    fn destroy_texture(&self, texture: SurfaceTexture) -> Option<Surface> {
        self.root.destroy_texture(texture)
    }
}

impl Drop for GpuPageContext {
    fn drop(&mut self) {
        if self.root.make_current().is_ok() {
            self.gl.delete_framebuffers(&[self.fbo]);
            self.gl.delete_renderbuffers(&[self.color, self.depth]);
        }
    }
}

// ── Окна ────────────────────────────────────────────────────────────────────

/// Что своё у окна: поверхность и кадр оболочки.
struct Screen {
    /// `None` — поверхность сейчас привязана к контексту.
    surface: Option<Surface>,
    painter: egui_glow::Painter,
    /// Последний кадр оболочки — повторяется под каждый новый кадр страницы.
    primitives: Vec<egui::ClippedPrimitive>,
    pixels_per_point: f32,
    /// Текстуры, освобождаемые вместе с заменой этого кадра оболочки.
    free: Vec<egui::TextureId>,
}

pub struct GpuOutput {
    root: Rc<WindowRenderingContext>,
    page: Rc<GpuPageContext>,
    /// По номеру окна; закрытые — `None`.
    screens: Vec<Option<Screen>>,
    /// Окно, чья поверхность привязана.
    bound: usize,
}

impl GpuOutput {
    /// Первое окно (номер 0) и общий буфер страниц.
    pub fn new(
        display: RawDisplayHandle,
        window: RawWindowHandle,
        window_size: PhysicalSize<u32>,
        page_size: PhysicalSize<u32>,
        refresh: Rc<dyn RefreshDriver>,
    ) -> Result<(GpuOutput, Rc<GpuPageContext>), String> {
        // SAFETY: дескрипторы даёт живое окно, оно переживает поверхность.
        let (display, handle) = unsafe { (DisplayHandle::borrow_raw(display), WindowHandle::borrow_raw(window)) };
        let root = Rc::new(WindowRenderingContext::new(display, handle, window_size).map_err(|e| format!("{e:?}"))?);
        root.make_current().map_err(|e| format!("{e:?}"))?;
        set_swap_interval(&root, 1);
        let page = Rc::new(GpuPageContext::new(root.clone(), PhysicalSize::new(page_size.width.max(1), page_size.height.max(1)), refresh)?);
        let painter = egui_glow::Painter::new(root.glow_gl_api(), "", None, false).map_err(|e| format!("{e}"))?;
        let first = Screen { surface: None, painter, primitives: Vec::new(), pixels_per_point: 1.0, free: Vec::new() };
        Ok((GpuOutput { root, page: page.clone(), screens: vec![Some(first)], bound: 0 }, page))
    }

    /// Ещё одно окно; номер — для остальных вызовов.
    pub fn add_window(&mut self, window: RawWindowHandle, size: PhysicalSize<u32>) -> Result<usize, String> {
        self.root.make_current().map_err(|e| format!("{e:?}"))?;
        let surface = {
            let (device, context) = self.root.surfman_details();
            // SAFETY: то же, что в `new`.
            let handle = unsafe { WindowHandle::borrow_raw(window) };
            let widget = device
                .connection()
                .create_native_widget_from_window_handle(handle, euclid::default::Size2D::new(size.width as i32, size.height as i32))
                .map_err(|e| format!("{e:?}"))?;
            device.create_surface(&context, SurfaceAccess::GPUOnly, SurfaceType::Widget { native_widget: widget }).map_err(|e| format!("{e:?}"))?
        };
        let painter = match egui_glow::Painter::new(self.root.glow_gl_api(), "", None, false) {
            Ok(painter) => painter,
            Err(e) => {
                let mut surface = surface;
                let (device, mut context) = self.root.surfman_details();
                let _ = device.destroy_surface(&mut context, &mut surface);
                return Err(format!("{e}"));
            }
        };
        self.screens.push(Some(Screen { surface: Some(surface), painter, primitives: Vec::new(), pixels_per_point: 1.0, free: Vec::new() }));
        Ok(self.screens.len() - 1)
    }

    /// Привязать поверхность окна `id` и сделать контекст текущим.
    fn bind(&mut self, id: usize) -> bool {
        if self.screens.get(id).is_none_or(Option::is_none) {
            return false;
        }
        if self.bound != id {
            let Some(incoming) = self.screens[id].as_mut().and_then(|s| s.surface.take()) else { return false };
            let outgoing = {
                let (device, mut context) = self.root.surfman_details();
                let outgoing = device.unbind_surface_from_context(&mut context).ok().flatten();
                if let Err((e, mut surface)) = device.bind_surface_to_context(&mut context, incoming) {
                    println!("browser: gpu: bind window {id}: {e:?}");
                    let _ = device.destroy_surface(&mut context, &mut surface);
                }
                outgoing
            };
            if let Some(Some(old)) = self.screens.get_mut(self.bound) {
                old.surface = outgoing;
            }
            self.bound = id;
        }
        self.root.make_current().is_ok()
    }

    /// Окно закрывается: убрать его поверхность (до уничтожения самого окна).
    pub fn remove_window(&mut self, id: usize) {
        let Some(other) = (0..self.screens.len()).find(|i| *i != id && self.screens[*i].is_some()) else { return };
        if !self.bind(other) {
            return;
        }
        let Some(mut screen) = self.screens.get_mut(id).and_then(Option::take) else { return };
        screen.painter.destroy();
        if let Some(mut surface) = screen.surface.take() {
            let (device, mut context) = self.root.surfman_details();
            let _ = device.destroy_surface(&mut context, &mut surface);
        }
    }

    pub fn renderer_name(&self) -> String {
        self.root.gleam_gl_api().get_string(gl::RENDERER)
    }

    pub fn max_texture_side(&self) -> usize {
        self.screens.iter().flatten().next().map_or(8192, |s| s.painter.max_texture_side())
    }

    /// Новый кадр оболочки окна `id`.
    pub fn set_ui(&mut self, id: usize, primitives: Vec<egui::ClippedPrimitive>, textures: &egui::TexturesDelta, pixels_per_point: f32) {
        let _ = self.root.make_current();
        let Some(Some(screen)) = self.screens.get_mut(id) else { return };
        for id in screen.free.drain(..) {
            screen.painter.free_texture(id);
        }
        for (id, delta) in &textures.set {
            screen.painter.set_texture(*id, delta);
        }
        screen.free = textures.free.clone();
        screen.primitives = primitives;
        screen.pixels_per_point = pixels_per_point;
    }

    /// Собрать кадр окна `size`: участок буфера `region` (x, y, ширина,
    /// высота; сверху вниз) — в точку `at` окна, оболочка поверх.
    fn draw(&mut self, id: usize, size: (u32, u32), at: (u32, u32), region: [u32; 4]) -> bool {
        use glow::HasContext as _;
        if !self.bind(id) {
            return false;
        }
        // Окон несколько — ждать развёртки в каждом значило бы делить частоту
        // между ними; кадры и так собирает диспетчер окон.
        set_swap_interval(&self.root, if self.screens.iter().flatten().count() > 1 { 0 } else { 1 });
        self.root.prepare_for_rendering();
        let glow = self.root.glow_gl_api();
        let atlas = self.page.size.get();
        let [rx, ry, rw, rh] = region;
        let (rw, rh) = (rw.min(atlas.width.saturating_sub(rx)), rh.min(atlas.height.saturating_sub(ry)));
        let gl = self.page.gl.clone();
        // SAFETY: контекст текущий.
        unsafe {
            glow.disable(glow::SCISSOR_TEST);
            glow.viewport(0, 0, size.0 as i32, size.1 as i32);
            glow.clear_color(1.0, 1.0, 1.0, 1.0);
            glow.clear(glow::COLOR_BUFFER_BIT);
        }
        if rw > 0 && rh > 0 {
            // У GL начало координат — снизу слева.
            let sy = atlas.height as i32 - (ry + rh) as i32;
            let ty = size.1 as i32 - (at.1 + rh) as i32;
            gl.bind_framebuffer(gl::READ_FRAMEBUFFER, self.page.fbo);
            gl.bind_framebuffer(gl::DRAW_FRAMEBUFFER, 0);
            gl.blit_framebuffer(rx as i32, sy, (rx + rw) as i32, sy + rh as i32, at.0 as i32, ty, (at.0 + rw) as i32, ty + rh as i32, gl::COLOR_BUFFER_BIT, gl::NEAREST);
            gl.bind_framebuffer(gl::FRAMEBUFFER, 0);
        }
        let Some(Some(screen)) = self.screens.get_mut(id) else { return false };
        screen.painter.paint_primitives([size.0, size.1], screen.pixels_per_point, &screen.primitives);
        true
    }

    /// Показать кадр окна.
    pub fn frame(&mut self, id: usize, size: (u32, u32), at: (u32, u32), region: [u32; 4]) {
        if self.draw(id, size, at, region) {
            self.root.present();
        }
    }

    /// Снимок окна для проверок: `0xAARRGGBB`, строки сверху вниз.
    pub fn snapshot(&mut self, id: usize, size: (u32, u32), at: (u32, u32), region: [u32; 4]) -> Vec<u32> {
        let (w, h) = (size.0 as usize, size.1 as usize);
        if !self.draw(id, size, at, region) {
            return vec![0; w * h];
        }
        let bytes = self.root.gleam_gl_api().read_pixels(0, 0, w as i32, h as i32, gl::BGRA, gl::UNSIGNED_BYTE);
        let mut out = Vec::with_capacity(w * h);
        for row in bytes.chunks_exact(w.max(1) * 4).rev() {
            out.extend(row.chunks_exact(4).map(|p| u32::from_le_bytes([p[0], p[1], p[2], p[3]])));
        }
        out
    }

    /// Отпустить окна перед выходом: дождаться видеокарты и разрушить
    /// поверхности. Выход с живой поверхностью однажды оставил процесс
    /// висеть в ядре (его не завершить и из диспетчера задач).
    pub fn shutdown(&mut self) {
        let _ = self.root.make_current();
        self.root.gleam_gl_api().finish();
        for screen in self.screens.iter_mut().filter_map(Option::as_mut) {
            screen.painter.destroy();
            if let Some(mut surface) = screen.surface.take() {
                let (device, mut context) = self.root.surfman_details();
                let _ = device.destroy_surface(&mut context, &mut surface);
            }
        }
        self.screens.clear();
        if let Err(e) = self.root.take_window() {
            println!("browser: gpu: release window: {e:?}");
        }
    }
}

/// `wglSwapIntervalEXT`: 1 — показывать по такту экрана, без разрывов.
fn set_swap_interval(window: &WindowRenderingContext, interval: i32) {
    let (device, context) = window.surfman_details();
    let address = device.get_proc_address(&context, "wglSwapIntervalEXT");
    if address.is_null() {
        return;
    }
    // SAFETY: это адрес `BOOL wglSwapIntervalEXT(int)` текущего контекста.
    let set: unsafe extern "system" fn(i32) -> i32 = unsafe { std::mem::transmute(address) };
    // SAFETY: то же.
    unsafe { set(interval) };
}
