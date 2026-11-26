//! OBSOS — Open Browser for System Open Source: браузер на Servo, независимый
//! от ОС. Здесь всё, кроме окна и служб системы: вкладки, адресная строка,
//! история, закладки, пароли (`vault`), синхронизация (`syncer`,
//! `browser-sync`), загрузки, настройки, свои страницы `obsos://`, скрипт
//! страниц (`shell::PAGE_SCRIPT`), инструменты разработчика.
//!
//! Платформа (FossAi — `obsos-fossai`, Windows/Linux — `obsos-desktop`)
//! ставит службы `window::install` и зовёт `app::run` с окном.

pub mod app;
mod automation;
mod chrome;
mod perf;
pub mod blocker;
pub mod devtools;
pub mod downloads;
#[cfg(windows)]
mod gpu_ctx;
mod logs;
pub mod net;
pub mod pages;
pub mod profile;
pub mod shell;
pub mod swgl_ctx;
pub mod syncer;
pub mod tools;
pub mod ui;
pub mod vault;
mod webui;
mod webui_host;
pub mod window;
mod windows;

pub use app::run;
