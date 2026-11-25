# Правки OBSOS в Servo 0.8

Servo берётся с crates.io (`servo = "=0.8.0"`, та же версия и тот же
`Cargo.lock`, что были у FossAi). Свои правки — не форк репозитория Servo, а
набор замен в нескольких его крейтах:

* `cargo xtask vendor` (программа `xtask/`) берёт архив крейта из кеша cargo
  или с static.crates.io, сверяет SHA-256, распаковывает в `vendor/<крейт>`
  (каталог не в git) и делает замены. Каждая замена обязана найтись ровно
  столько раз, сколько записано: при переходе на новую версию Servo правки
  надо пересмотреть, а не наложить вслепую.
* Корневой `Cargo.toml` подменяет эти крейты (`[patch.crates-io]`).
* Копия пересоздаётся, только если правки поменялись (метка `.obsos-patch`),
  иначе cargo пересобирал бы `servo-script` и всё, что выше.

Почему не форк: репозиторий Servo — гигабайты истории, а правок — десяток
строк в шести крейтах; замены с проверкой числа совпадений видны целиком в
одном файле (`xtask/src/main.rs`) и переживают обновление Servo с понятной
ошибкой. Если правок станет много или они начнут уходить в Servo — тогда
ветка форка `anomal3/servo` и `[patch]` на git.

## Список

| Крейт | Правка | Где действует |
|---|---|---|
| servo-script | без потоковой компиляции WebAssembly (`compileStreaming` терял начало тела — Blazor на web.pynop.online не запускался) | везде |
| servo-script | `navigator.platform` = «Linux» | FossAi |
| servo-net | только HTTP/1.1 (потоки HTTP/2 к CDN повисали) | FossAi |
| servo-paint | очистка кэша WebRender без `DepthFunc(ALWAYS)`, без дизеринга (swgl их не умеет; OBSOS рисует swgl везде) | везде |
| servo-storage | SQLite без WAL (нет общей памяти `-shm`): localStorage/IndexedDB | FossAi |
| servo-fonts, servo-fonts-traits | FreeType и свой список шрифтов (`fonts/font_list.rs`) | FossAi |
| gaol | пустой крейт (песочница нужна только `multiprocess`) | везде (не используется) |
| egui_software_backend | холст обновляется, когда фигура исчезла (`third_party/`) | везде |

«FossAi» — закрыто `cfg(target_os = "freeos")`, на Windows и Linux Servo
работает как без правки.

Правки уровня C/C++ и набора сборки FossAi (SpiderMonkey `mozjs_sys` под
FreeOS, `aws-lc-sys` с `getentropy`) остались в репозитории FossAi
(`ports/servo/mozjs`, `ports/servo/builder/patch-crates.sh`): они нужны любой
программе на Servo под FossAi, а не только браузеру.
