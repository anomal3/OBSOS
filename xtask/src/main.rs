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

const CRATES: &[Krate] = &[
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
    tar::Archive::new(flate2::read::GzDecoder::new(&bytes[..])).unpack(vendor).map_err(|e| format!("{}: unpack: {e}", k.name))?;
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
