# OBSOS — авторство и лицензии сторонних компонентов

Этот файл ставится вместе с браузером (`NOTICE.md` рядом с `obsos.exe`, на
FossAi — `/opt/obsos/share/NOTICE.md`) и показывается на странице
`obsos://licenses`, в «О программе» и в FossAi в «Параметры → О системе →
Сторонние компоненты».

Свой код OBSOS — © 2026 Roman <anomal3@yandex.ru>, на выбор под лицензией
MIT или Apache-2.0 (`MIT OR Apache-2.0`, файлы `LICENSE-MIT` и
`LICENSE-APACHE`). Компоненты ниже остаются под своими лицензиями; файлы под
MPL-2.0 сохраняют свой заголовок с лицензией (MPL действует пофайлово):

* `patches/fonts/font_list.rs` — список шрифтов FossAi, вставляется в крейт
  Servo `servo-fonts`, MPL-2.0;
* правки крейтов Servo, которые `cargo xtask vendor` вносит в `vendor/`
  (`xtask/src/main.rs`, `patches/README.md`), — на тех же условиях MPL-2.0,
  что и исправленные файлы Servo;
* `third_party/egui-software-backend` — копия крейта DGriffin91 с правкой,
  MIT или Apache-2.0, со своими файлами лицензий.

Исходники OBSOS вместе со всеми правками сторонних компонентов:
https://github.com/anomal3/OBSOS. Правки C/C++ и сборки под FossAi
(`ports/servo/mozjs/freeos.patch` и т. п.):
https://github.com/anomal3/FreeOpenSourceSystemAI

| Компонент | Авторы | Лицензия |
|---|---|---|
| Движок **Servo** | © The Servo Project Developers, https://servo.org | MPL-2.0 |
| Дизайн оболочки — **по мотивам Zervo** | © Goddv, https://github.com/Goddv/Zervo | MPL-2.0 |
| **SpiderMonkey** (через `mozjs`/`mozjs_sys`) | © Mozilla Foundation и участники | MPL-2.0 |
| **swgl / WebRender** | © Mozilla Foundation и участники | MPL-2.0 |
| **ICU** (Intl в SpiderMonkey) | © Unicode, Inc. и участники | Unicode License v3 (`Unicode-3.0`) |
| **ICU4X** (обёртки Intl в `mozjs_sys`) | © Unicode, Inc. и участники | Unicode-3.0 |
| **egui** | © Emil Ernerfeldt и участники | MIT или Apache-2.0 |
| **egui_software_backend** (с правкой OBSOS, `third_party/egui-software-backend`) | © DGriffin91 (github.com/DGriffin91) и участники | MIT или Apache-2.0 |
| **RustCrypto**: `argon2`, `chacha20poly1305`, `sha2` (хранилище паролей) | © RustCrypto Developers | MIT или Apache-2.0 |
| **zlib** (`libz-sys`) | © Jean-loup Gailly, Mark Adler | Zlib |
| **HarfBuzz** (`harfbuzz-sys`) | © HarfBuzz contributors | MIT («Old MIT») |
| **FreeType** (`freetype-sys`) | © The FreeType Project | FTL (BSD-подобная, с упоминанием) или GPL-2.0 — берём FTL |
| **libpng** (в FreeType) | © Contributing Authors and Group 42, Inc. | libpng License |
| **OTS** — OpenType Sanitizer (`fontsan`) | © The Chromium Authors | BSD-3-Clause |
| **LZ4** (в `fontsan`) | © Yann Collet | BSD-2-Clause |
| **AWS-LC** (`aws-lc-sys`, `aws-lc-rs`) | © Amazon.com, Inc.; © Google (BoringSSL); © OpenSSL Project | Apache-2.0 / ISC / OpenSSL |
| **rustls** | © rustls contributors | Apache-2.0 / ISC / MIT |
| **SQLite** (`libsqlite3-sys`) | авторы SQLite | общественное достояние |
| **Zstandard** (`zstd-sys`) | © Meta Platforms, Inc. | BSD-3-Clause |
| **winit**, **softbuffer**, **arboard**, **rfd** (окно, кадр, буфер обмена и окна выбора файла в obsos-desktop) | © их авторы и участники | Apache-2.0 (winit); MIT или Apache-2.0 (softbuffer, arboard); MIT (rfd) |
| **surfman** (OpenGL для рисования видеокартой), **gleam**, **png** | © их авторы и участники | MIT или Apache-2.0 (surfman, gleam); MIT или Apache-2.0 (png) |
| **GStreamer** и его плагины — видео и звук (только сборка с фичей `video`; DLL рядом с программой) | © GStreamer и участники | LGPL-2.1-or-later (отдельные библиотеки динамически; плагины — по своим лицензиям) |

Правки FossAi в SpiderMonkey (ОС `freeos` в сборке, `getentropy()` для
случайности, видимость символов) лежат открыто в репозитории FossAi,
`ports/servo/mozjs/freeos.patch`, и распространяются на тех же условиях
MPL-2.0, что и исправленные файлы.

Текст MPL-2.0: https://mozilla.org/MPL/2.0/
Текст Unicode License v3: https://www.unicode.org/license.txt

Правка FossAi в AWS-LC (случайность через `getentropy()`) делается при
сборке под FossAi — `ports/servo/builder/patch-crates.sh` (репозиторий FossAi).

Список пополняется вместе с зависимостями движка: каждая новая библиотека
в пакете `servo` — строка здесь.
