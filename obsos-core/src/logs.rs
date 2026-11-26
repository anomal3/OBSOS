//! Что попадает в журнал (на Windows это файл `obsos.log`): адреса http(s) —
//! без логина, параметров и якоря (там бывают токены), а из окна инкогнито —
//! ничего о посещаемом.

use std::sync::atomic::{AtomicBool, Ordering};

static PRIVATE: AtomicBool = AtomicBool::new(false);

/// Это окно инкогнито — до первой записи о страницах.
pub fn set_private(on: bool) {
    PRIVATE.store(on, Ordering::Relaxed);
}

pub fn private() -> bool {
    PRIVATE.load(Ordering::Relaxed)
}

fn clean_url(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(mut u) if matches!(u.scheme(), "http" | "https" | "ws" | "wss") => {
            u.set_query(None);
            u.set_fragment(None);
            let _ = u.set_username("");
            let _ = u.set_password(None);
            u.to_string()
        }
        _ => url.to_string(),
    }
}

/// Адрес для журнала.
pub fn url(url: &str) -> String {
    if private() {
        return String::from("(инкогнито)");
    }
    if url.starts_with("data:") {
        return String::from("data:…");
    }
    clean_url(url)
}

/// Имя файла, значение и прочее о том, что делал пользователь.
pub fn text(text: &str) -> String {
    if private() { String::from("(инкогнито)") } else { text.to_string() }
}

/// Произвольный текст (сообщения страницы и движка): адреса внутри — без
/// параметров.
pub fn scrub(message: &str) -> String {
    if private() {
        return String::from("(инкогнито)");
    }
    let mut out = String::with_capacity(message.len());
    let mut rest = message;
    loop {
        let start = ["http://", "https://", "ws://", "wss://"].iter().filter_map(|p| rest.find(p)).min();
        let Some(start) = start else { break };
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        let end = tail.find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ')' | ']' | '>' | ',' | ';')).unwrap_or(tail.len());
        let address = &tail[..end];
        match address.find(['?', '#']) {
            Some(cut) => out.push_str(&address[..cut]),
            None => out.push_str(address),
        }
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_lose_secrets() {
        assert_eq!(url("https://u:p@x.org/a?token=1#f"), "https://x.org/a");
        assert_eq!(url("obsos://view?path=%2Fa.md"), "obsos://view?path=%2Fa.md");
        assert_eq!(url("data:text/html,secret"), "data:…");
        assert_eq!(scrub("GET https://x.org/a?k=1 failed (wss://y.org/s?id=2): 404"), "GET https://x.org/a failed (wss://y.org/s): 404");
    }
}
