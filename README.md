# OBSOS — Open Browser for System Open Source

Браузер на [Servo](https://servo.org) 0.8: свой интерфейс (вкладки сверху или
сбоку, омнибокс, история, закладки, зашифрованные пароли, синхронизация через
свой репозиторий GitHub/GitLab/Bitbucket, Яндекс Диск, Google Диск, WebDAV или
папку, загрузки, блокировка рекламы, режим чтения, инструменты разработчика,
пользовательские скрипты и стили). Родился внутри ОС
[FossAi](https://github.com/anomal3/FreeOpenSourceSystemAI) и вынесен оттуда 25.11.2026, чтобы его можно было
разрабатывать и отлаживать прямо на Windows (и Linux), а в FossAi подключать
готовым.

## Как устроено

```
obsos-core/      оболочка без привязки к ОС: Servo, swgl, egui, вкладки, профиль,
                 пароли (vault), синхронизация (syncer), загрузки, страницы obsos://,
                 скрипт страниц (shell::PAGE_SCRIPT), главный цикл (app::run);
                 договор с платформой — obsos-core/src/window.rs
obsos-desktop/   Windows и Linux: окно winit, кадр softbuffer, буфер обмена arboard,
                 окна выбора файла rfd → программа obsos(.exe)
obsos-fossai/    FossAi: окно стола FossAi (freeos_window_*), syscalls, /bin/filedlg;
                 программы browser (в пакете — /opt/obsos/bin/obsos) и servoprobe
crates/browser-sync/   ядро синхронизации (журналы операций, HLC, площадки)
third_party/egui-software-backend/   копия с правкой (холст обновляется при исчезновении фигуры)
patches/         правки Servo (patches/README.md) и список шрифтов FossAi
xtask/           cargo xtask vendor — готовит vendor/ с правленными крейтами Servo
assets/icons/    значок OBSOS
docs/NOTICE.md   лицензии (страница obsos://licenses)
```

Рисует OBSOS везде одинаково и **программно**: страницу — swgl (программный
OpenGL из Firefox, «Software WebRender»), оболочку — egui прямо в пиксели. От
платформы нужны только поверхность окна и события (в кодировке окна FossAi,
`WinEvent`) — поэтому то, что видно на Windows, то же самое, что на FossAi.

История файлов перенесена из репозитория FossAi (`git filter-repo`): `git log
--follow obsos-core/src/shell.rs` показывает и прежние коммиты.

## Сборка и запуск на Windows

Нужно (на ПК Романа всё уже есть):

* Rust через rustup — ночная сборка из `rust-toolchain.toml` ставится сама;
* Visual Studio Build Tools (MSVC, Windows SDK) — линковщик;
* LLVM (clang-cl) — swgl и C++ Servo собираются им, `cl.exe` swgl не берёт;
* Python 3 — генерация кода Servo (`PYTHON3`).

SpiderMonkey (`mozjs_sys`) на Windows не собирается из исходников — его
build.rs скачивает готовую библиотеку с github.com/servo/mozjs.

```bat
cargo xtask vendor
set CC=clang-cl
set CXX=clang-cl
set PYTHON3=C:\Python314\python.exe
cargo build --release
target\release\obsos.exe https://web.pynop.online
```

Первая сборка — около 40 минут и ~15 ГБ в `target\` (release; отладочная
сборка Servo заняла бы ещё столько же). Дальше пересобирается только OBSOS.
Консоль остаётся открытой — в ней журнал `browser: …` (страницы, сеть, ошибки
скриптов страниц).

Ключи: `obsos [--profile <имя>] [--private] [--sync-folder <папка>] [адрес]`.

Данные: `%USERPROFILE%\.config\obsos\<профиль>` (настройки, закладки, история,
пароли), `%USERPROFILE%\.config\servo` (куки, localStorage, IndexedDB),
загрузки — `%USERPROFILE%\Downloads`.

На Linux то же самое (`cargo build --release`, нужен clang); окна выбора
файла — через портал xdg.

## Как FossAi получает OBSOS

FossAi собирает программы со `std` под `<арх>-unknown-freeos` в контейнере
`fossai-builder` (Docker, `ports/servo/builder` в репозитории FossAi): там его
sysroot, обёртка компилятора, правки std, SpiderMonkey под FreeOS
(`ports/servo/mozjs`) и `aws-lc-sys` с `getentropy`. Этот репозиторий
монтируется в контейнер (`/obsos`), версия закреплена в FossAi файлом
`ports/servo/OBSOS_REV`:

```sh
docker run --rm -v <FossAi>:/work -v <OBSOS>:/obsos -v fossai-servo:/cache \
    -v fossai-cargo-registry:/opt/cargo/registry \
    fossai-builder sh /work/ports/servo/builder/build-servoprobe.sh x86_64
```

Скрипт делает `cargo xtask vendor` здесь, собирает `obsos-fossai` и кладёт
`browser` и `servoprobe` в `build/linux/servo-browser-<арх>` и
`servoprobe-<арх>` FossAi — оттуда их берёт `cargo xtask` FossAi в пакет
`obsos-0.1.fpk` (и в том для телефона).

## Что ещё не так, как на FossAi

* Масштаб экрана (HiDPI) не учитывается: оболочка и страница — в точках
  экрана, на 150 % мелко. Нужен масштаб от окна в `obsos-core`.
* Рисование программное (как на FossAi): на больших окнах медленнее
  настоящих браузеров. GPU-путь Servo (surfman/ANGLE) — отдельная работа.
* «Открыть в Chromium» на Windows/Linux открывает браузер системы по умолчанию.

## Лицензия

Свой код OBSOS — © 2026 Roman <anomal3@yandex.ru>, на выбор под одной из
двух лицензий:

* MIT — [LICENSE-MIT](LICENSE-MIT);
* Apache License 2.0 — [LICENSE-APACHE](LICENSE-APACHE).

Пользоваться, менять и встраивать OBSOS в свои программы (в том числе
закрытые) можно свободно, сохраняя уведомление об авторстве. Если не
сказано иначе, присланный в проект код тоже лицензируется как
`MIT OR Apache-2.0`, без дополнительных условий.

Сторонний код остаётся под своими лицензиями (полный список — в
[docs/NOTICE.md](docs/NOTICE.md), страница `obsos://licenses`):

* движок **Servo** 0.8 (зависимость с crates.io), SpiderMonkey, swgl/WebRender
  — MPL-2.0. Правки крейтов Servo, которые вносит `cargo xtask vendor`, и
  `patches/fonts/font_list.rs` — тоже MPL-2.0 (MPL действует пофайлово:
  исправленные файлы Servo остаются под MPL, остальной код OBSOS — нет);
* `third_party/egui-software-backend` — копия крейта
  [egui_software_backend](https://github.com/DGriffin91/egui_software_backend)
  (© DGriffin91) с правкой, MIT или Apache-2.0, файлы лицензий лежат рядом;
* дизайн оболочки — по мотивам [Zervo](https://github.com/Goddv/Zervo)
  (© Goddv, MPL-2.0); код оболочки свой.
