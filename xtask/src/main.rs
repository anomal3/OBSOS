//! Сборочные задачи OBSOS.
//!
//! `cargo xtask vendor` — копии крейтов Servo 0.8 с правками OBSOS в
//! `vendor/` (на них ссылается `[patch.crates-io]` корневого Cargo.toml).
//! Крейт берётся из кеша cargo или с static.crates.io, хеш SHA-256
//! сверяется, архив распаковывается и правится заменами строк — каждая
//! замена обязана найтись ровно столько раз, сколько записано (новая версия
//! Servo — правки надо пересмотреть, а не наложить вслепую).
//!
//! Копия пересоздаётся, только если правки поменялись (метка
//! `.obsos-patch` с хешем правок): иначе cargo пересобрал бы servo-script и
//! всё, что выше, из-за новых дат файлов.
//!
//! Правки — `patches/README.md`. Закрытые `cfg(target_os = "freeos")`
//! действуют только на FossAi, остальные — везде.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use sha2::{Digest, Sha256};

/// Замена: файл в крейте, что было, что стало, сколько раз.
struct Edit {
    file: &'static str,
    old: &'static str,
    new: &'static str,
    count: usize,
}

struct Krate {
    name: &'static str,
    version: &'static str,
    /// SHA-256 файла `.crate` (static.crates.io, 25.11.2026).
    sha256: &'static str,
    edits: &'static [Edit],
    /// Файлы из `patches/` в крейт: (откуда, куда).
    files: &'static [(&'static str, &'static str)],
}

const SCRIPT_EDITS: &[Edit] = &[
    Edit {
        file: "dom/webrtc/rtcdatachannel.rs",
        old: "        let servo_media_id = servo_media_id.unwrap_or(\n            controller\n                .as_ref()\n                .unwrap()\n                .create_data_channel(init)\n                .expect(\"Expected data channel id\"),\n        );\n",
        new: "        // __obsos__: было `unwrap_or`, и канал создавался даже для чужого: обе\n        // стороны порождали их по кругу (тысячи потоков, звук замирал).\n        let servo_media_id = servo_media_id.unwrap_or_else(|| {\n            controller\n                .as_ref()\n                .unwrap()\n                .create_data_channel(init)\n                .expect(\"Expected data channel id\")\n        });\n",
        count: 1,
    },
    // Звонки: TURN из iceServers, enabled/stop у дорожек.
    Edit {
        file: "dom/webrtc/rtcpeerconnection.rs",
        old: "        if let Some(ref servers) = config.iceServers &&\n            let Some(server) = servers.first()\n        {\n            let server = match server.urls {\n                StringOrStringSequence::String(ref s) => Some(s.clone()),\n                StringOrStringSequence::StringSequence(ref s) => s.first().cloned(),\n            };\n            if let Some(server) = server {\n                let policy = match config.bundlePolicy {\n                    RTCBundlePolicy::Balanced => BundlePolicy::Balanced,\n                    RTCBundlePolicy::Max_compat => BundlePolicy::MaxCompat,\n                    RTCBundlePolicy::Max_bundle => BundlePolicy::MaxBundle,\n                };\n                this.controller\n                    .borrow()\n                    .as_ref()\n                    .unwrap()\n                    .configure(String::from(server), policy);\n            }\n        }\n",
        new: "        // __obsos__: все серверы (с логином и паролем TURN), а не только первый\n        // адрес в роли STUN (patches/README.md).\n        let mut lines: Vec<String> = Vec::new();\n        if let Some(ref servers) = config.iceServers {\n            for server in servers.iter() {\n                let urls: Vec<String> = match server.urls {\n                    StringOrStringSequence::String(ref s) => vec![String::from(s.clone())],\n                    StringOrStringSequence::StringSequence(ref s) => {\n                        s.iter().map(|s| String::from(s.clone())).collect()\n                    },\n                };\n                let username = server.username.clone().map(String::from).unwrap_or_default();\n                let credential = server.credential.clone().map(String::from).unwrap_or_default();\n                for url in urls {\n                    lines.extend(obsos_ice_line(&url, &username, &credential));\n                }\n            }\n        }\n        if matches!(config.iceTransportPolicy, crate::dom::bindings::codegen::Bindings::RTCPeerConnectionBinding::RTCIceTransportPolicy::Relay) {\n            lines.push(\"relay\".to_owned());\n        }\n        if !lines.is_empty() {\n            let policy = match config.bundlePolicy {\n                RTCBundlePolicy::Balanced => BundlePolicy::Balanced,\n                RTCBundlePolicy::Max_compat => BundlePolicy::MaxCompat,\n                RTCBundlePolicy::Max_bundle => BundlePolicy::MaxBundle,\n            };\n            this.controller\n                .borrow()\n                .as_ref()\n                .unwrap()\n                .configure(lines.join(\"\\n\"), policy);\n        }\n",
        count: 1,
    },
    Edit {
        file: "dom/webrtc/rtcpeerconnection.rs",
        old: "impl Convert<RTCSessionDescriptionInit> for SessionDescription {\n",
        new: "/// __obsos__: `stun:host:port` → `stun://host:port`, `turn:host?transport=tcp`\n/// с логином и паролем → `turn://логин:пароль@host?transport=tcp`. GStreamer\n/// делит логин и пароль по первому «:» и лишь потом раскодирует %XX.\nfn obsos_ice_line(url: &str, username: &str, credential: &str) -> Option<String> {\n    fn escape(text: &str) -> String {\n        text.bytes()\n            .map(|b| {\n                if b.is_ascii_alphanumeric() || b\"-._~\".contains(&b) {\n                    char::from(b).to_string()\n                } else {\n                    format!(\"%{b:02X}\")\n                }\n            })\n            .collect()\n    }\n    let (scheme, rest) = url.split_once(':')?;\n    let rest = rest.trim_start_matches('/');\n    match scheme {\n        \"stun\" => Some(format!(\"stun://{rest}\")),\n        \"turn\" | \"turns\" if !username.is_empty() => Some(format!(\n            \"{scheme}://{}:{}@{rest}\",\n            escape(username),\n            escape(credential)\n        )),\n        _ => None,\n    }\n}\n\nimpl Convert<RTCSessionDescriptionInit> for SessionDescription {\n",
        count: 1,
    },
    Edit {
        file: "dom/media/mediastreamtrack.rs",
        old: "use dom_struct::dom_struct;\n",
        new: "use std::cell::Cell;\n\nuse dom_struct::dom_struct;\n",
        count: 1,
    },
    Edit {
        file: "dom/media/mediastreamtrack.rs",
        old: "    #[no_trace]\n    ty: MediaStreamType,\n}\n",
        new: "    #[no_trace]\n    ty: MediaStreamType,\n    // __obsos__\n    enabled: Cell<bool>,\n}\n",
        count: 1,
    },
    Edit {
        file: "dom/media/mediastreamtrack.rs",
        old: "            id,\n            ty,\n        }\n    }\n",
        new: "            id,\n            ty,\n            enabled: Cell::new(true),\n        }\n    }\n\n    /// __obsos__: сам поток в медиадвижке.\n    fn with_stream(&self, f: impl FnOnce(&mut dyn servo_media::streams::MediaStream)) {\n        if let Some(stream) = servo_media::streams::registry::get_stream(&self.id) &&\n            let Ok(mut stream) = stream.lock()\n        {\n            f(&mut *stream)\n        }\n    }\n",
        count: 1,
    },
    Edit {
        file: "dom/media/mediastreamtrack.rs",
        old: "    /// <https://w3c.github.io/mediacapture-main/#dom-mediastreamtrack-clone>\n",
        new: "    // __obsos__: <https://w3c.github.io/mediacapture-main/#dom-mediastreamtrack-enabled>\n    fn Enabled(&self) -> bool {\n        self.enabled.get()\n    }\n\n    fn SetEnabled(&self, value: bool) {\n        self.enabled.set(value);\n        self.with_stream(|stream| stream.set_enabled(value));\n    }\n\n    // __obsos__: <https://w3c.github.io/mediacapture-main/#dom-mediastreamtrack-stop>\n    fn Stop(&self) {\n        self.with_stream(|stream| stream.stop());\n    }\n\n    /// <https://w3c.github.io/mediacapture-main/#dom-mediastreamtrack-clone>\n",
        count: 1,
    },
    // Микрофон и камера — только с разрешения (Servo не спрашивал вовсе).
    Edit {
        file: "dom/media/mediadevices.rs",
        old: "        let p = Promise::new_in_realm(cx);\n        let media = ServoMedia::get();\n        let stream = MediaStream::new(cx, &self.global());\n",
        new: "        let p = Promise::new_in_realm(cx);\n        // __obsos__: разрешение спрашивается у браузера (patches/README.md):\n        // без этого любой сайт получал бы микрофон и камеру молча.\n        for (wanted, name) in [\n            (convert_constraints(&constraints.audio).is_some(), crate::dom::bindings::codegen::Bindings::PermissionStatusBinding::PermissionName::Microphone),\n            (convert_constraints(&constraints.video).is_some(), crate::dom::bindings::codegen::Bindings::PermissionStatusBinding::PermissionName::Camera),\n        ] {\n            if wanted && crate::dom::permission::permissions::request_permission_to_use(name, &self.global()) != crate::dom::bindings::codegen::Bindings::PermissionStatusBinding::PermissionState::Granted {\n                p.reject_error(cx, crate::dom::bindings::error::Error::NotAllowed(None));\n                return p;\n            }\n        }\n        let media = ServoMedia::get();\n        let stream = MediaStream::new(cx, &self.global());\n",
        count: 1,
    },
    Edit {
        file: "dom/media/mediadevices.rs",
        old: "        // Step 2.5\n        let media = ServoMedia::get();\n",
        new: "        // __obsos__: названия устройств — только после разрешения.\n        let allowed = [crate::dom::bindings::codegen::Bindings::PermissionStatusBinding::PermissionName::Microphone, crate::dom::bindings::codegen::Bindings::PermissionStatusBinding::PermissionName::Camera]\n            .into_iter()\n            .any(|name| crate::dom::permission::permissions::descriptor_permission_state(name, Some(&self.global())) == crate::dom::bindings::codegen::Bindings::PermissionStatusBinding::PermissionState::Granted);\n\n        // Step 2.5\n        let media = ServoMedia::get();\n",
        count: 1,
    },
    Edit {
        file: "dom/media/mediadevices.rs",
        old: "                            &device.label,\n",
        new: "                            if allowed { device.label.as_str() } else { \"\" },\n",
        count: 1,
    },
    // `navigator.platform` на FossAi — как у Linux («Linux»): сайты
    // определяют по нему платформу, и незнакомое имя только мешает им.
    Edit {
        file: "dom/navigator/navigatorinfo.rs",
        old: "#[cfg(any(target_os = \"android\", target_os = \"linux\", target_os = \"freebsd\"))]\npub(crate) fn Platform() -> DOMString {",
        new: "// __obsos__ FossAi: как Linux — так сайтам понятнее.\n#[cfg(any(target_os = \"android\", target_os = \"linux\", target_os = \"freebsd\", target_os = \"freeos\"))]\npub(crate) fn Platform() -> DOMString {",
        count: 1,
    },
    // Потоковая компиляция WebAssembly (`compileStreaming`,
    // `instantiateStreaming`) теряет начало тела: куски ответа, пришедшие до
    // вызова compileStreaming, уходят в обычный поток тела, а компилятор
    // получает файл с середины («failed to match magic number» — .NET/Blazor
    // на web.pynop.online, 09.11.2026). Без обработчика потока SpiderMonkey
    // не заводит эти функции, и загрузчики берут `arrayBuffer()`. Везде.
    Edit {
        file: "runtime/script_runtime.rs",
        old: "            InitConsumeStreamCallback(cx, Some(consume_stream), Some(report_stream_error));",
        new: "            // __obsos__: без потоковой компиляции wasm (patches/README.md).\n            let _ = (consume_stream, report_stream_error);",
        count: 1,
    },
    // `<style>`, добавленный в shadow root после элементов, не действовал:
    // набор стилей помечался изменённым, но никто не просил его пересчитать
    // (плеер vkvideo.ru и другие веб-компоненты — без оформления,
    // 26.11.2026). Везде.
    Edit {
        file: "dom/shadowroot/shadowroot.rs",
        old: "        let stylesheets = &mut self.author_styles.borrow_mut().stylesheets;\n\n        // FIXME(stevennovaryo)",
        new: "        // __obsos__: заём отпускается до пометки (patches/README.md).\n        let mut styles = self.author_styles.borrow_mut();\n        let stylesheets = &mut styles.stylesheets;\n\n        // FIXME(stevennovaryo)",
        count: 1,
    },
    Edit {
        file: "dom/shadowroot/shadowroot.rs",
        old: "        DocumentOrShadowRoot::add_stylesheet(\n            StylesheetSource::Element(Dom::from_ref(owner_node)),\n            StylesheetSetRef::Author(stylesheets),\n            sheet,\n            insertion_point,\n            self.document.style_shared_author_lock(),\n        );\n",
        new: "        DocumentOrShadowRoot::add_stylesheet(\n            StylesheetSource::Element(Dom::from_ref(owner_node)),\n            StylesheetSetRef::Author(stylesheets),\n            sheet,\n            insertion_point,\n            self.document.style_shared_author_lock(),\n        );\n        // __obsos__: пересчитать стили shadow root.\n        drop(styles);\n        self.invalidate_stylesheets(no_gc);\n",
        count: 1,
    },
    // IndexedDB: завершение транзакции могло встать в очередь дважды (отказ и
    // запроса, и фиксации), второе роняло браузер паникой
    // «clear_upgrade_transaction called but no upgrade transaction is set»
    // (вход в Rupor на FossAi, 26.11.2026). Везде.
    Edit {
        file: "dom/indexeddb/idbtransaction.rs",
        old: "            .queue(task!(send_abort_notification: move |cx| {\n                let this = this.root();\n",
        new: "            .queue(task!(send_abort_notification: move |cx| {\n                let this = this.root();\n                // __obsos__: уже завершена (patches/README.md).\n                if this.finished.get() {\n                    return;\n                }\n",
        count: 1,
    },
    Edit {
        file: "dom/indexeddb/idbtransaction.rs",
        old: "            task!(send_complete_notification: move |cx| {\n                let this = this.root();\n",
        new: "            task!(send_complete_notification: move |cx| {\n                let this = this.root();\n                // __obsos__: уже завершена (patches/README.md).\n                if this.finished.get() {\n                    return;\n                }\n",
        count: 1,
    },
    Edit {
        file: "dom/indexeddb/idbdatabase.rs",
        old: "        let current = self\n            .upgrade_transaction\n            .get()\n            .expect(\"clear_upgrade_transaction called but no upgrade transaction is set\");\n\n        debug_assert!(\n            &*current == transaction,\n            \"clear_upgrade_transaction called with non-current transaction\"\n        );\n\n        self.upgrade_transaction.set(None);\n",
        new: "        // __obsos__: без паники (patches/README.md).\n        if self.upgrade_transaction.get().is_some_and(|current| &*current == transaction) {\n            self.upgrade_transaction.set(None);\n        }\n",
        count: 1,
    },
];

const NET_EDITS: &[Edit] = &[
    // FossAi: только HTTP/1.1. По HTTP/2 к одному CDN (github.githubassets.com)
    // часть потоков на FossAi повисала: таблицы стилей оставались пустыми
    // (09.11.2026). HTTP/1.1 — до шести соединений на сайт.
    Edit {
        file: "connector.rs",
        old: "        .enable_http1()\n        .enable_http2()\n        .wrap_connector(ProxyConnector::new());\n",
        new: "        .enable_http1();\n    // __obsos__ FossAi: без HTTP/2 (patches/README.md).\n    #[cfg(not(target_os = \"freeos\"))]\n    let connector = connector.enable_http2();\n    let connector = connector.wrap_connector(ProxyConnector::new());\n",
        count: 1,
    },
];

const PAINT_EDITS: &[Edit] = &[
    // Несколько окон на одном painter: буфер общий («атлас»), у каждой
    // вкладки свой участок — его задаёт приложение. Без записи в списке
    // вкладка, как и раньше, занимает весь буфер. Везде.
    Edit {
        file: "lib.rs",
        old: "mod painter;\n",
        new: "mod painter;\n/// __obsos__: участки общего буфера для вкладок нескольких окон\n/// (patches/README.md).\npub mod obsos {\n    use std::collections::HashMap;\n    use std::sync::Mutex;\n\n    use servo_base::id::WebViewId;\n\n    /// Вкладка → (x, y, ширина, высота) в точках буфера.\n    pub static VIEWPORTS: Mutex<Option<HashMap<WebViewId, [f32; 4]>>> = Mutex::new(None);\n}\n",
        count: 1,
    },
    Edit {
        file: "painter.rs",
        old: "        if self.rendering_context.size() == new_size {\n            return;\n        }\n\n        if let Err(error) = self.rendering_context.make_current() {\n            error!(\"Failed to make the rendering context current: {error:?}\");\n        }\n        self.rendering_context.resize(new_size);\n\n        let new_size = Size2D::new(new_size.width as f32, new_size.height as f32);\n        let new_viewport_rect = Rect::from(new_size).to_box2d();\n        for webview_renderer in self.webview_renderers.values_mut() {\n            webview_renderer.set_rect(new_viewport_rect);\n            webview_renderer.webview.notify_viewport_updated();\n        }\n",
        new: "        // __obsos__: у вкладки может быть свой участок буфера, и он меняется\n        // без изменения размера буфера (patches/README.md).\n        let size_changed = self.rendering_context.size() != new_size;\n        if size_changed {\n            if let Err(error) = self.rendering_context.make_current() {\n                error!(\"Failed to make the rendering context current: {error:?}\");\n            }\n            self.rendering_context.resize(new_size);\n        }\n\n        let new_size = Size2D::new(new_size.width as f32, new_size.height as f32);\n        let new_viewport_rect = Rect::from(new_size).to_box2d();\n        let mut moved = false;\n        {\n            let viewports = crate::obsos::VIEWPORTS.lock().ok();\n            for (id, webview_renderer) in self.webview_renderers.iter_mut() {\n                let rect = viewports.as_ref().and_then(|v| v.as_ref()?.get(id)).map_or(new_viewport_rect, |r| {\n                    Rect::new(Point2D::new(r[0], r[1]), Size2D::new(r[2], r[3])).to_box2d()\n                });\n                if webview_renderer.set_rect(rect) || size_changed {\n                    webview_renderer.webview.notify_viewport_updated();\n                    moved = true;\n                }\n            }\n        }\n        if !size_changed && !moved {\n            return;\n        }\n",
        count: 1,
    },
    // Для swgl (программный OpenGL): WebRender очищает кэш картинок
    // «квадратами» с тестом глубины ALWAYS, а swgl умеет только LESS/LEQUAL
    // (assert в DepthFunc, gl.cc:1390 — падение на первом кадре) — очистка
    // обычным glClear по ножницам. С видеокартой — как в самом Servo.
    Edit {
        file: "painter.rs",
        old: "                shared_font_namespace: Some(painter_id.into()),\n                ..Default::default()",
        new: "                shared_font_namespace: Some(painter_id.into()),\n                // __obsos__: swgl не умеет DepthFunc(ALWAYS).\n                clear_caches_with_quads: !webrender_gl.get_string(RENDERER).starts_with(\"Software WebRender\"),\n                ..Default::default()",
        count: 1,
    },
    // Варианта шейдеров DITHERING в swgl нет («unknown shader
    // ps_quad_gradient DITHERING», assert в BindAttribLocation).
    Edit {
        file: "painter.rs",
        old: "                enable_dithering: true,",
        new: "                enable_dithering: !webrender_gl.get_string(RENDERER).starts_with(\"Software WebRender\"), // __obsos__: swgl",
        count: 1,
    },
];

const STORAGE_EDITS: &[Edit] = &[
    // FossAi: localStorage и IndexedDB — SQLite в режиме WAL, а WAL берёт
    // общую память файла `-shm` (mmap MAP_SHARED + блокировки байтов),
    // которой на FossAi нет: SQLITE_IOERR_SHMOPEN на каждый доступ,
    // localStorage не хранил ничего (25.11.2026, hw-log19). Журнал TRUNCATE;
    // исключительный режим блокировки перед ним — чтобы база, уже
    // переведённая в WAL, открылась без -shm.
    Edit {
        file: "shared/mod.rs",
        old: "pub const DB_INIT_PRAGMAS: [&str; 2] =\n    [\"PRAGMA journal_mode = WAL;\", \"PRAGMA encoding = 'UTF-16';\"];",
        new: "#[cfg(not(target_os = \"freeos\"))]\npub const DB_INIT_PRAGMAS: [&str; 2] =\n    [\"PRAGMA journal_mode = WAL;\", \"PRAGMA encoding = 'UTF-16';\"];\n// __obsos__ FossAi: без WAL и общей памяти -shm (patches/README.md).\n#[cfg(target_os = \"freeos\")]\npub const DB_INIT_PRAGMAS: [&str; 3] = [\n    \"PRAGMA locking_mode = EXCLUSIVE;\",\n    \"PRAGMA journal_mode = TRUNCATE;\",\n    \"PRAGMA encoding = 'UTF-16';\",\n];",
        count: 1,
    },
];

// Шрифты на FossAi: FreeType (как Linux/Android), список шрифтов — свой
// (patches/fonts/font_list.rs: обход /usr/share/fonts и таблицы name/OS2
// через skrifa, без Fontconfig).
const FONTS_TRAITS_EDITS: &[Edit] = &[Edit {
    file: "font_identifier.rs",
    old: "#[cfg(any(target_os = \"linux\", target_os = \"android\", target_os = \"freebsd\"))]\nmod platform {",
    new: "// __obsos__ FossAi: FreeType, как Linux.\n#[cfg(any(target_os = \"linux\", target_os = \"android\", target_os = \"freebsd\", target_os = \"freeos\"))]\nmod platform {",
    count: 1,
}];

const FONTS_EDITS: &[Edit] = &[
    Edit {
        file: "Cargo.toml",
        old: "target_os = \"linux\", target_os = \"android\", target_os = \"freebsd\"))'.dependencies.",
        new: "target_os = \"linux\", target_os = \"android\", target_os = \"freebsd\", target_os = \"freeos\"))'.dependencies.",
        count: 3,
    },
    Edit {
        file: "platform/mod.rs",
        old: "#[cfg(any(target_os = \"linux\", target_os = \"android\", target_os = \"freebsd\"))]",
        new: "#[cfg(any(target_os = \"linux\", target_os = \"android\", target_os = \"freebsd\", target_os = \"freeos\"))]",
        count: 2,
    },
    Edit {
        file: "platform/freetype/mod.rs",
        old: "mod library_handle;",
        new: "// __obsos__ FossAi: свой список шрифтов — обход каталогов, без Fontconfig.\n#[cfg(target_os = \"freeos\")]\nmod freeos {\n    pub mod font_list;\n}\n#[cfg(target_os = \"freeos\")]\npub use self::freeos::font_list;\n\nmod library_handle;",
        count: 1,
    },
];

// `mask-image` (значки): Servo не рисует маски — вместо значков Википедии
// были цветные квадраты (26.11.2026). Везде.
const LAYOUT_EDITS: &[Edit] = &[
    Edit {
        file: "display_list/background.rs",
        old: "/// Abstract over the horizontal or vertical dimension\n",
        new: "// __obsos__: раскладка картинки маски — по тем же правилам, что фоновой\n// (patches/README.md).\npub(super) fn layout_mask_tile(\n    area: units::LayoutRect,\n    natural_sizes: NaturalSizes,\n    size: &Size,\n    repeat: RepeatXY,\n    position_x: &LengthPercentage,\n    position_y: &LengthPercentage,\n) -> Option<(units::LayoutRect, units::LayoutSize, units::LayoutSize, bool)> {\n    let fit = |contain: bool| {\n        let mut tile = area.size();\n        if let Some(ratio) = natural_sizes.ratio {\n            let area_ratio = area.size().width / area.size().height;\n            let fit_width = if contain { area_ratio <= ratio } else { area_ratio > ratio };\n            if fit_width {\n                tile.height = tile.width / ratio\n            } else {\n                tile.width = tile.height * ratio\n            }\n        }\n        tile\n    };\n    let mut tile_size = match size {\n        Size::Contain => fit(true),\n        Size::Cover => fit(false),\n        Size::ExplicitSize { width, height } => {\n            let mut width = width.non_auto().map(|lp| lp.0.to_used_value(Au::from_f32_px(area.size().width)));\n            let mut height = height.non_auto().map(|lp| lp.0.to_used_value(Au::from_f32_px(area.size().height)));\n            if width.is_none() && height.is_none() {\n                width = natural_sizes.width;\n                height = natural_sizes.height;\n            }\n            match (width, height) {\n                (Some(w), Some(h)) => units::LayoutSize::new(w.to_f32_px(), h.to_f32_px()),\n                (Some(w), None) => units::LayoutSize::new(w.to_f32_px(), natural_sizes.ratio.map_or(area.size().height, |r| w.to_f32_px() / r)),\n                (None, Some(h)) => units::LayoutSize::new(natural_sizes.ratio.map_or(area.size().width, |r| h.to_f32_px() * r), h.to_f32_px()),\n                (None, None) => fit(true),\n            }\n        },\n    };\n    if !(tile_size.width > 0.0 && tile_size.height > 0.0) {\n        return None;\n    }\n    let RepeatXY(repeat_x, repeat_y) = repeat;\n    let x = layout_1d(&mut tile_size.width, repeat_x, position_x, 0.0, area.size().width, area.size().width);\n    let y = layout_1d(&mut tile_size.height, repeat_y, position_y, 0.0, area.size().height, area.size().height);\n    let bounds = units::LayoutRect::from_origin_and_size(\n        area.min + Vector2D::new(x.bounds_origin, y.bounds_origin),\n        Size2D::new(x.bounds_size, y.bounds_size),\n    );\n    Some((bounds, tile_size, units::LayoutSize::new(x.tile_spacing, y.tile_spacing), x.repeat || y.repeat))\n}\n\n/// Abstract over the horizontal or vertical dimension\n",
        count: 1,
    },
    Edit {
        file: "display_list/mod.rs",
        old: "        let b = painter.style.get_background();\n        let background_color = painter.style.resolve_color(&b.background_color);\n",
        new: "        // __obsos__: значок маской (patches/README.md).\n        if painter.painting_area_override.is_none() &&\n            painter.positioning_area_override.is_none() &&\n            self.build_masked_icon(builder, state, painter)\n        {\n            return;\n        }\n        let b = painter.style.get_background();\n        let background_color = painter.style.resolve_color(&b.background_color);\n",
        count: 1,
    },
    Edit {
        file: "display_list/mod.rs",
        old: "    fn build_background_image(\n        &self,\n",
        new: "    /// __obsos__: `mask-image` для значков — пустой элемент со сплошным цветом\n    /// и картинкой-маской (Википедия, многие наборы значков). Маски поверх\n    /// содержимого и градиентные пока не поддержаны (patches/README.md).\n    fn build_masked_icon(\n        &mut self,\n        builder: &mut DisplayListBuilder,\n        state: &TraversalState,\n        painter: &BackgroundPainter,\n    ) -> bool {\n        use style::values::computed::image::Image;\n\n        let style = painter.style;\n        let svg = style.get_svg();\n        let Some(mask) = svg.mask_image.0.first().filter(|image| matches!(image, Image::Url(_))) else {\n            return false;\n        };\n        if !self.fragment.children.is_empty() {\n            return false;\n        }\n        let b = style.get_background();\n        if b.background_image.0.iter().any(|image| !matches!(image, Image::None)) {\n            return false;\n        }\n        let color = rgba(style.resolve_color(&b.background_color));\n        if color.a <= 0.0 {\n            return false;\n        }\n        let node = self.fragment.base.tag.map(|tag| tag.node);\n        // Картинка ещё грузится или не загрузилась — не рисуем ничего (а не\n        // цветной квадрат).\n        let Ok(ResolvedImage::Image { image, size, .. }) = builder.image_resolver.resolve_image(node, mask) else {\n            return true;\n        };\n        let area = self.border_rect;\n        let Some((bounds, tile, spacing, repeat)) = background::layout_mask_tile(\n            area,\n            NaturalSizes::from_width_and_height(size.width, size.height),\n            background::get_cyclic(&svg.mask_size.0, 0),\n            background::get_cyclic(&svg.mask_repeat.0, 0).clone(),\n            background::get_cyclic(&svg.mask_position_x.0, 0),\n            background::get_cyclic(&svg.mask_position_y.0, 0),\n        ) else {\n            return true;\n        };\n        let scale = builder.device_pixel_ratio.get();\n        let pixels = Size2D::new(tile.width * scale, tile.height * scale).to_i32();\n        let Some(image_key) = builder.image_resolver.image_key_from_cached_image(&image, pixels, node) else {\n            return true;\n        };\n        let mut common = builder.common_properties(state, area, style);\n        if let Some(clip_chain_id) = self.border_edge_clip(builder, state, false) {\n            common.clip_chain_id = clip_chain_id;\n        }\n        // Цвет — фона элемента, прозрачность — картинки маски.\n        #[rustfmt::skip]\n        let matrix = [\n            0.0, 0.0, 0.0, 0.0,\n            0.0, 0.0, 0.0, 0.0,\n            0.0, 0.0, 0.0, 0.0,\n            0.0, 0.0, 0.0, color.a,\n            color.r, color.g, color.b, 0.0,\n        ];\n        let spatial_id = builder.spatial_id(state.spatial_id);\n        builder.wr().push_stacking_context(\n            spatial_id,\n            PrimitiveFlags::default(),\n            None,\n            wr::TransformStyle::Flat,\n            MixBlendMode::Normal,\n            &[wr::FilterOp::ColorMatrix(matrix)],\n            &[],\n            wr::RasterSpace::Screen,\n            wr::StackingContextFlags::empty(),\n            None,\n        );\n        let rendering = style.get_inherited_box().image_rendering.to_webrender();\n        if repeat {\n            builder.wr().push_repeating_image(&common, bounds, tile, spacing, rendering, wr::AlphaType::PremultipliedAlpha, image_key, wr::ColorF::WHITE);\n        } else {\n            builder.wr().push_image(&common, bounds, rendering, wr::AlphaType::PremultipliedAlpha, image_key, wr::ColorF::WHITE);\n        }\n        builder.wr().pop_stacking_context();\n        builder.mark_is_paintable();\n        true\n    }\n\n    fn build_background_image(\n        &self,\n",
        count: 1,
    },
];

// WebGL на Windows: устройство surfman в потоке WebGL создавалось без
// текущего контекста OpenGL, и драйвер NVIDIA падал (26.11.2026).
// Дальше всплыли ещё две ошибки того же слоя: DC скрытого окна освобождался
// сразу (контекст терялся) и блокировка общей текстуры шла без контекста.
const SURFMAN_EDITS: &[Edit] = &[
    Edit {
        file: "src/wgl/device.rs",
        old: "pub(crate) struct HiddenWindow {\n",
        new: "/// __obsos__: временный контекст OpenGL на невидимом окне, если у потока ещё\n/// нет своего. `wglDXOpenDeviceNV` без текущего контекста роняет драйвер\n/// NVIDIA (так создаётся устройство в потоке WebGL).\nstruct TemporaryContext {\n    context: winapi::shared::windef::HGLRC,\n    _window: Option<HiddenWindow>,\n}\n\nimpl TemporaryContext {\n    unsafe fn new() -> TemporaryContext {\n        if !wglGetCurrentContext().is_null() {\n            return TemporaryContext { context: ptr::null_mut(), _window: None };\n        }\n        let window = HiddenWindow::new();\n        let context = {\n            let dc = window.get_dc();\n            let mut descriptor: PIXELFORMATDESCRIPTOR = mem::zeroed();\n            descriptor.nSize = mem::size_of::<PIXELFORMATDESCRIPTOR>() as u16;\n            descriptor.nVersion = 1;\n            descriptor.dwFlags = wingdi::PFD_DRAW_TO_WINDOW | wingdi::PFD_SUPPORT_OPENGL | wingdi::PFD_DOUBLEBUFFER;\n            descriptor.iPixelType = wingdi::PFD_TYPE_RGBA;\n            descriptor.cColorBits = 32;\n            let format = wingdi::ChoosePixelFormat(dc.dc, &descriptor);\n            wingdi::SetPixelFormat(dc.dc, format, &descriptor);\n            let context = wingdi::wglCreateContext(dc.dc);\n            wglMakeCurrent(dc.dc, context);\n            context\n        };\n        TemporaryContext { context, _window: Some(window) }\n    }\n}\n\nimpl Drop for TemporaryContext {\n    fn drop(&mut self) {\n        if !self.context.is_null() {\n            unsafe {\n                wglMakeCurrent(ptr::null_mut(), ptr::null_mut());\n                wglDeleteContext(self.context);\n            }\n        }\n    }\n}\n\npub(crate) struct HiddenWindow {\n",
        count: 1,
    },
    Edit {
        file: "src/wgl/device.rs",
        old: "            let gl_dx_interop_device =\n                (dx_interop_functions.DXOpenDeviceNV)(d3d11_device.as_raw() as *mut c_void);\n",
        new: "            // __obsos__: расширению нужен текущий контекст (patches/README.md).\n            let temporary = TemporaryContext::new();\n            let gl_dx_interop_device =\n                (dx_interop_functions.DXOpenDeviceNV)(d3d11_device.as_raw() as *mut c_void);\n            drop(temporary);\n",
        count: 1,
    },
    Edit {
        file: "src/wgl/device.rs",
        old: "    window: HWND,\n    join_handle: Option<JoinHandle<()>>,\n",
        new: "    window: HWND,\n    // __obsos__\n    dc: HDC,\n    join_handle: Option<JoinHandle<()>>,\n",
        count: 1,
    },
    Edit {
        file: "src/wgl/device.rs",
        old: "        HiddenWindow {\n            window,\n            join_handle",
        new: "        // __obsos__: один DC на всю жизнь окна. Освобождённый сразу после\n        // wglMakeCurrent становился недействительным, вернуть прежний контекст\n        // было нечем, и поток оставался без OpenGL.\n        let dc = unsafe { winuser::GetDC(window) };\n        HiddenWindow {\n            window,\n            dc,\n            join_handle",
        count: 1,
    },
    Edit {
        file: "src/wgl/device.rs",
        old: "unsafe { DCGuard::new(winuser::GetDC(self.window), Some(self.window)) }",
        new: "DCGuard::new(self.dc, None)",
        count: 1,
    },
    Edit {
        file: "src/wgl/device.rs",
        old: "        self.lock_surface(&surface);\n        context.framebuffer",
        new: "        // __obsos__: расширению нужен текущий контекст поверхности.\n        {\n            let _guard = self.temporarily_make_context_current(context);\n            self.lock_surface(&surface);\n        }\n        context.framebuffer",
        count: 1,
    },
    Edit {
        file: "src/wgl/device.rs",
        old: "            Framebuffer::Surface(surface) => {\n                self.unlock_surface(&surface);",
        new: "            Framebuffer::Surface(surface) => {\n                // __obsos__\n                let _guard = self.temporarily_make_context_current(context);\n                self.unlock_surface(&surface);",
        count: 1,
    },
    Edit {
        file: "src/wgl/device.rs",
        old: "                &mut gl_dx_interop_object,\n            );\n            assert_ne!(ok, FALSE);",
        new: "                &mut gl_dx_interop_object,\n            );\n            assert_ne!(ok, FALSE, \"DX interop: {}\", std::io::Error::last_os_error());",
        count: 2,
    },
];

// Звонки (WebRTC): без плагинов SRTP и SCTP из GStreamer соединение не
// устанавливается, а в списке Servo их нет.
const SERVO_EDITS: &[Edit] = &[
    Edit {
        file: "gstreamer_plugin_lists/windows.rs.in",
        old: "\"gstwasapi\"",
        new: "\"gstwasapi\",\n// __obsos__\n\"gstsrtp\",\n\"gstsctp\"",
        count: 1,
    },
];

// Звонки и микрофон (26.11.2026): выключение и остановка дорожек.
const STREAMS_EDITS: &[Edit] = &[
    Edit {
        file: "lib.rs",
        old: "    fn ty(&self) -> MediaStreamType;\n}\n",
        new: "    fn ty(&self) -> MediaStreamType;\n    // __obsos__: `track.enabled` и `track.stop()` (patches/README.md).\n    fn set_enabled(&mut self, _enabled: bool) {}\n    fn stop(&mut self) {}\n}\n",
        count: 1,
    },
];

const MEDIA_GST_EDITS: &[Edit] = &[
    Edit {
        file: "media_stream.rs",
        old: "    fn ty(&self) -> MediaStreamType {\n        self.type_\n    }\n}\n",
        new: "    fn ty(&self) -> MediaStreamType {\n        self.type_\n    }\n\n    // __obsos__: звук глушится (идёт тишина), видео перестаёт идти.\n    fn set_enabled(&mut self, enabled: bool) {\n        for element in &self.elements {\n            match element.factory().map(|f| f.name()).as_deref() {\n                Some(\"volume\") => element.set_property(\"mute\", !enabled),\n                Some(\"valve\") => element.set_property(\"drop\", !enabled),\n                _ => {},\n            }\n        }\n    }\n\n    // __obsos__: источник (микрофон, камера) освобождает устройство.\n    fn stop(&mut self) {\n        self.set_enabled(false);\n        if let Some(source) = self.elements.first() {\n            source.set_locked_state(true);\n            // Не на потоке скрипта: остановка устройства может длиться секунду.\n            source.call_async(|source| {\n                let _ = source.set_state(gstreamer::State::Null);\n            });\n        }\n    }\n}\n",
        count: 1,
    },
    Edit {
        file: "media_stream.rs",
        old: "    pub fn src_element(&self) -> gstreamer::Element {\n",
        new: "    /// __obsos__: поток уже отдан получателю (второго он не выдерживает).\n    pub fn is_attached(&self) -> bool {\n        self.pipeline.is_some()\n    }\n\n    pub fn src_element(&self) -> gstreamer::Element {\n",
        count: 1,
    },
    Edit {
        file: "media_stream.rs",
        old: "        let queue2 = gstreamer::ElementFactory::make(\"queue\").build().unwrap();\n\n        register_stream(",
        new: "        let queue2 = gstreamer::ElementFactory::make(\"queue\").build().unwrap();\n        // __obsos__: для `track.enabled`.\n        let volume = gstreamer::ElementFactory::make(\"volume\").build().unwrap();\n\n        register_stream(",
        count: 1,
    },
    Edit {
        file: "media_stream.rs",
        old: "vec![source, queue, audioconvert, audioresample, queue2],",
        new: "vec![source, queue, audioconvert, volume, audioresample, queue2],",
        count: 1,
    },
    Edit {
        file: "media_stream.rs",
        old: "        let queue = gstreamer::ElementFactory::make(\"queue\").build().unwrap();\n\n        register_stream(Arc::new(Mutex::new(GStreamerMediaStream::new(\n            MediaStreamType::Video,\n            vec![source, videoconvert, queue],",
        new: "        let queue = gstreamer::ElementFactory::make(\"queue\").build().unwrap();\n        // __obsos__: для `track.enabled`.\n        let valve = gstreamer::ElementFactory::make(\"valve\").build().unwrap();\n\n        register_stream(Arc::new(Mutex::new(GStreamerMediaStream::new(\n            MediaStreamType::Video,\n            vec![source, videoconvert, valve, queue],",
        count: 1,
    },
    Edit {
        file: "media_capture.rs",
        old: "pub fn create_audioinput_stream(constraint_set: MediaTrackConstraintSet) -> Option<MediaStreamId> {\n",
        new: "pub fn create_audioinput_stream(constraint_set: MediaTrackConstraintSet) -> Option<MediaStreamId> {\n    // __obsos__: микрофон по умолчанию, а не первый в списке (первым может\n    // стоять виртуальный, и собеседник слышит тишину).\n    #[cfg(windows)]\n    if let Ok(source) = gstreamer::ElementFactory::make(\"wasapisrc\")\n        .property_from_str(\"role\", \"comms\")\n        .build()\n    {\n        return Some(GStreamerMediaStream::create_audio_from(source));\n    }\n",
        count: 1,
    },
    Edit {
        file: "webrtc.rs",
        old: "    fn configure(&mut self, stun_server: &str, policy: BundlePolicy) -> WebRtcResult {\n        self.webrtc\n            .set_property_from_str(\"stun-server\", stun_server);\n",
        new: "    fn configure(&mut self, servers: &str, policy: BundlePolicy) -> WebRtcResult {\n        // __obsos__: строки от RTCPeerConnection: stun://…, turn://…, relay.\n        let mut stun = false;\n        for line in servers.lines() {\n            if line.starts_with(\"stun://\") {\n                if !stun {\n                    self.webrtc.set_property_from_str(\"stun-server\", line);\n                    stun = true;\n                }\n            } else if line.starts_with(\"turn://\") || line.starts_with(\"turns://\") {\n                // Адрес не пишется в журнал: в нём пароль.\n                if !self.webrtc.emit_by_name::<bool>(\"add-turn-server\", &[&line]) {\n                    warn!(\"TURN server was not accepted\");\n                }\n            } else if line == \"relay\" {\n                self.webrtc.set_property_from_str(\"ice-transport-policy\", \"relay\");\n            }\n        }\n",
        count: 1,
    },
    Edit {
        file: "webrtc.rs",
        old: "    ) -> WebRtcResult {\n        let caps = stream.caps();\n",
        new: "    ) -> WebRtcResult {\n        // __obsos__: раньше здесь была паника.\n        if stream.is_attached() {\n            warn!(\"Media stream is already in use\");\n            return Err(WebRtcError::Backend(\"Media stream is already in use\".to_owned()));\n        }\n        let caps = stream.caps();\n",
        count: 1,
    },
    Edit {
        file: "webrtc.rs",
        old: "        if let Some(idx) = idx {\n            if idx >= self.request_pad_counter {\n                for i in self.request_pad_counter..=idx {\n                    // webrtcbin needs you to request pads (or use element.link(webrtcbin))\n                    // however, it also wants them to be connected in the correct order.\n                    //\n                    // Here, we make sure all the numbered sink pads have been created beforehand, up to\n                    // and including the one we need here.\n                    //\n                    // An alternate fix is to sort pending_streams according to the m-line index\n                    // and just do it in order. This also seems brittle.\n                    self.webrtc\n                        .request_pad_simple(&format!(\"sink_{}\", i))\n                        .ok_or(\"Cannot request sink pad\")?;\n                }\n                self.request_pad_counter = idx + 1;\n            }\n",
        new: "        if let Some(idx) = idx {\n            // __obsos__: после чужого предложения GStreamer помечает такую\n            // дорожку «только приём» и отказывает в sink-входе: звонок принимался,\n            // а свой микрофон подключить было нельзя.\n            self.allow_sending(idx as u32);\n            let name = format!(\"sink_{}\", idx);\n            if self.webrtc.static_pad(&name).is_none() {\n                self.webrtc\n                    .request_pad_simple(&name)\n                    .ok_or(\"Cannot request sink pad\")?;\n            }\n            self.request_pad_counter = cmp::max(self.request_pad_counter, idx + 1);\n",
        count: 1,
    },
    Edit {
        file: "webrtc.rs",
        old: "                    self.flush_pending_streams(false)?;\n",
        new: "                    // __obsos__: ошибка не должна оставить промис страницы без ответа.\n                    if let Err(error) = self.flush_pending_streams(false) {\n                        warn!(\"Could not link local media: {:?}\", error);\n                    }\n",
        count: 1,
    },
    Edit {
        file: "webrtc.rs",
        old: "    /// link_stream, but for all pending streams\n",
        new: "    /// __obsos__: дорожке m-line `mline` разрешается и передавать.\n    fn allow_sending(&self, mline: u32) {\n        for index in 0..32i32 {\n            let Some(transceiver) = self\n                .webrtc\n                .emit_by_name::<Option<gstreamer_webrtc::WebRTCRTPTransceiver>>(\n                    \"get-transceiver\",\n                    &[&index],\n                )\n            else {\n                break;\n            };\n            if transceiver.property::<u32>(\"mlineindex\") == mline {\n                transceiver.set_property(\n                    \"direction\",\n                    gstreamer_webrtc::WebRTCRTPTransceiverDirection::Sendrecv,\n                );\n            }\n        }\n    }\n\n    /// link_stream, but for all pending streams\n",
        count: 1,
    },
    Edit {
        file: "webrtc.rs",
        old: "            .set_property_from_str(\"bundle-policy\", policy.as_str());\n",
        new: "            .set_property_from_str(\n                \"bundle-policy\",\n                // __obsos__: «balanced» в webrtcbin не сделан.\n                match policy.as_str() {\n                    \"balanced\" => \"max-bundle\",\n                    other => other,\n                },\n            );\n",
        count: 1,
    },
    Edit {
        file: "webrtc.rs",
        old: "    let caps = pad.query_caps(None);\n    let name = caps\n        .structure(0)\n        .unwrap()\n        .get::<String>(\"media\")\n        .expect(\"Invalid 'media' field\");\n",
        new: "",
        count: 1,
    },
    Edit {
        file: "webrtc.rs",
        old: "            on_incoming_decodebin_stream(pad, &pipe, thread.clone(), &name);\n",
        new: "            // __obsos__: вид берётся из декодированного потока: у выхода webrtcbin\n            // в GStreamer 1.26 поле \"media\" ещё не заполнено (здесь была паника).\n            let caps = pad.current_caps().unwrap_or_else(|| pad.query_caps(None));\n            let video = caps\n                .structure(0)\n                .is_some_and(|s| s.name().starts_with(\"video/\"));\n            let name = if video { \"video\" } else { \"audio\" };\n            on_incoming_decodebin_stream(pad, &pipe, thread.clone(), name);\n",
        count: 1,
    },
];

const BINDINGS_EDITS: &[Edit] = &[
    Edit {
        file: "webidls/MediaStreamTrack.webidl",
        old: "    readonly        attribute DOMString id;\n",
        new: "    readonly        attribute DOMString id;\n    // __obsos__\n    attribute boolean enabled;\n    undefined stop();\n",
        count: 1,
    },
];

const CRATES: &[Krate] = &[
        Krate {
            name: "servo-media-streams",
            version: "0.8.0",
            sha256: "6a36e094d79e768473b699e84513547521476529f5b2253dad9edf3a337dd116",
            edits: STREAMS_EDITS,
            files: &[],
        },
        Krate {
            name: "servo-media-gstreamer",
            version: "0.8.0",
            sha256: "7bb5bc60a53436a224a57f23059a494bf84296119f1c6e8f5bbba8a2af3fa96f",
            edits: MEDIA_GST_EDITS,
            files: &[],
        },
        Krate {
            name: "servo-script-bindings",
            version: "0.8.0",
            sha256: "75f9bbcb6eb03e96610c1cbd9e71a26869874178e4f109a9a1c3f810bce88f81",
            edits: BINDINGS_EDITS,
            files: &[],
        },
        Krate {
            name: "servo",
            version: "0.8.0",
            sha256: "58a502704ce2fee976f20ee6192c575a3a77dfe844431f83cd4946da6e524985",
            edits: SERVO_EDITS,
            files: &[],
        },
        Krate {
            name: "surfman",
            version: "0.14.0",
            sha256: "7fe194717def845ecb64e7aeda3e2108c4b3b1584a01aa6e545e702cedca2a00",
            edits: SURFMAN_EDITS,
            files: &[],
        },
        Krate {
            name: "servo-layout",
            version: "0.8.0",
            sha256: "02047fd51cca1b92cfd38bc4663c9f11f6ebf99cd2d5550bac507a0a2b77addc",
            edits: LAYOUT_EDITS,
            files: &[],
        },
    Krate {
        name: "servo-script",
        version: "0.8.0",
        sha256: "b6c3e52be4165b3f98b992f384156b049d30b769004a96cf52903ea27cd32f9b",
        edits: SCRIPT_EDITS,
        files: &[],
    },
    Krate {
        name: "servo-net",
        version: "0.8.0",
        sha256: "e274e25650db0dacd2a290d13f51effd12735c17d5448403c2c77fea530e034d",
        edits: NET_EDITS,
        files: &[],
    },
    Krate {
        name: "servo-paint",
        version: "0.8.0",
        sha256: "06e4bddf028b69849fe310dfcbdc2707d0eaef1cdc49e48adad60bdffdcc703c",
        edits: PAINT_EDITS,
        files: &[],
    },
    Krate {
        name: "servo-storage",
        version: "0.8.0",
        sha256: "346be02eac3b3be12d17a806b56076f2cb706adbe2aa1b3331964a77ec3a5856",
        edits: STORAGE_EDITS,
        files: &[],
    },
    Krate {
        name: "servo-fonts-traits",
        version: "0.8.0",
        sha256: "6ee68596af5e88e51cf5e9b8deda7243f706f7a0c6b697c5cd0231341dd1ffcf",
        edits: FONTS_TRAITS_EDITS,
        files: &[],
    },
    Krate {
        name: "servo-fonts",
        version: "0.8.0",
        sha256: "908148cafdbb232f4ac59b6e0fb0ac8b31f97c80f1ad98beeddb4ed689cddae0",
        edits: FONTS_EDITS,
        files: &[("patches/fonts/font_list.rs", "platform/freetype/freeos/font_list.rs")],
    },
];

/// gaol — песочница процессов; у servo она под cfg по ОС (не Windows), а
/// нужна только фиче `multiprocess`, которой у OBSOS нет. FossAi gaol не
/// знает (нет `platform::Operation`) — пустой крейт той же версии.
const GAOL_TOML: &str = "[package]\nname = \"gaol\"\nversion = \"0.2.1\"\nedition = \"2018\"\ndescription = \"OBSOS: empty stand-in for gaol (sandboxing is only used by servo's multiprocess feature, which is off)\"\n";
const GAOL_LIB: &str = "//! OBSOS: песочница gaol нужна только Servo с `multiprocess` (выключен) — пусто.\n";

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("xtask in repo").to_path_buf()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Хеш правок крейта (и файлов из patches/) — метка копии.
fn stamp(root: &Path, k: &Krate) -> Result<String, String> {
    let mut h = Sha256::new();
    h.update(k.sha256);
    for e in k.edits {
        h.update(e.file);
        h.update([0]);
        h.update(e.old);
        h.update([0]);
        h.update(e.new);
        h.update(e.count.to_le_bytes());
    }
    for (from, to) in k.files {
        h.update(std::fs::read(root.join(from)).map_err(|e| format!("{from}: {e}"))?);
        h.update(to);
    }
    Ok(hex(&h.finalize()))
}

fn cargo_home() -> PathBuf {
    if let Some(h) = std::env::var_os("CARGO_HOME") {
        return PathBuf::from(h);
    }
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).unwrap_or_default();
    PathBuf::from(home).join(".cargo")
}

/// Архив крейта: кеш cargo, иначе static.crates.io (curl есть и в Windows 10+).
fn fetch(k: &Krate, work: &Path) -> Result<Vec<u8>, String> {
    let file = format!("{}-{}.crate", k.name, k.version);
    let cache = cargo_home().join("registry").join("cache");
    if let Ok(dirs) = std::fs::read_dir(&cache) {
        for d in dirs.flatten() {
            let p = d.path().join(&file);
            if let Ok(bytes) = std::fs::read(&p) {
                if hex(&Sha256::digest(&bytes)) == k.sha256 {
                    return Ok(bytes);
                }
            }
        }
    }
    let url = format!("https://static.crates.io/crates/{}/{file}", k.name);
    let out = work.join(&file);
    println!("xtask: download {url}");
    let ok = Command::new("curl").args(["-sSfL", "-o"]).arg(&out).arg(&url).status().map_err(|e| format!("curl: {e}"))?;
    if !ok.success() {
        return Err(format!("curl {url}: {ok}"));
    }
    let bytes = std::fs::read(&out).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&out);
    let got = hex(&Sha256::digest(&bytes));
    if got != k.sha256 {
        return Err(format!("{file}: SHA-256 {got}, ждали {}", k.sha256));
    }
    Ok(bytes)
}

fn vendor_crate(root: &Path, vendor: &Path, k: &Krate, check: bool) -> Result<(), String> {
    let dir = vendor.join(format!("{}-{}", k.name, k.version));
    let mark = stamp(root, k)?;
    let mark_file = dir.join(".obsos-patch");
    if std::fs::read_to_string(&mark_file).is_ok_and(|m| m.trim() == mark) {
        println!("xtask: {} {} — уже готов", k.name, k.version);
        return Ok(());
    }
    if check {
        return Err(format!("{} {}: копия устарела — cargo xtask vendor", k.name, k.version));
    }
    let bytes = fetch(k, vendor)?;
    let _ = std::fs::remove_dir_all(&dir);
    // В архиве — каталог `<имя>-<версия>/`.
    // Даты из архива не переносятся: у surfman они такие, что Windows их не
    // ставит («Параметр задан неверно»).
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(&bytes[..]));
    archive.set_preserve_mtime(false);
    archive.unpack(vendor).map_err(|e| format!("{}: unpack: {e} ({:?})", k.name, std::error::Error::source(&e)))?;
    for e in k.edits {
        let path = dir.join(e.file);
        let text = std::fs::read_to_string(&path).map_err(|err| format!("{}: {err}", path.display()))?;
        // Архивы crates.io — с концами строк LF; правки — тоже.
        let found = text.matches(e.old).count();
        if found != e.count {
            return Err(format!("{}/{}: правка найдена {found} раз(а), ждали {} — Servo поменялся?\n--- искали:\n{}", k.name, e.file, e.count, e.old));
        }
        std::fs::write(&path, text.replace(e.old, e.new)).map_err(|err| err.to_string())?;
    }
    for (from, to) in k.files {
        let dst = dir.join(to);
        std::fs::create_dir_all(dst.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::copy(root.join(from), &dst).map_err(|e| format!("{from}: {e}"))?;
    }
    std::fs::write(&mark_file, &mark).map_err(|e| e.to_string())?;
    println!("xtask: {} {} — {} правок", k.name, k.version, k.edits.len() + k.files.len());
    Ok(())
}

fn vendor(check: bool) -> Result<(), String> {
    let root = root();
    let vendor = root.join("vendor");
    std::fs::create_dir_all(&vendor).map_err(|e| e.to_string())?;
    let gaol = vendor.join("gaol-stub");
    std::fs::create_dir_all(gaol.join("src")).map_err(|e| e.to_string())?;
    for (file, text) in [("Cargo.toml", GAOL_TOML), ("src/lib.rs", GAOL_LIB)] {
        let p = gaol.join(file);
        if std::fs::read_to_string(&p).ok().as_deref() != Some(text) {
            std::fs::write(&p, text).map_err(|e| e.to_string())?;
        }
    }
    for k in CRATES {
        vendor_crate(&root, &vendor, k, check)?;
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("vendor") => vendor(args.iter().any(|a| a == "--check")),
        _ => {
            eprintln!("cargo xtask vendor [--check]   — крейты Servo с правками OBSOS в vendor/");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("xtask: {e}");
            ExitCode::FAILURE
        }
    }
}
