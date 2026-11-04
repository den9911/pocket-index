use anyhow::{ensure, Result};
use serde::Deserialize;
#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    All,
    Phrase,
    Prefix,
}
pub fn expression(q: &str, mode: Mode) -> Result<String> {
    ensure!(
        q.chars().count() <= 256,
        "Search is limited to 256 characters"
    );
    let tokens: Vec<_> = q
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();
    ensure!(tokens.len() <= 12, "Search is limited to 12 terms");
    if tokens.is_empty() {
        return Ok(String::new());
    }
    Ok(match mode {
        Mode::All => tokens
            .iter()
            .map(|s| format!("\"{s}\""))
            .collect::<Vec<_>>()
            .join(" AND "),
        Mode::Phrase => format!("\"{}\"", tokens.join(" ")),
        Mode::Prefix => tokens
            .iter()
            .map(|s| format!("\"{s}\"*"))
            .collect::<Vec<_>>()
            .join(" AND "),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_literal_queries() {
        assert_eq!(
            expression("Rust systems", Mode::Phrase).expect("query"),
            "\"Rust systems\""
        );
        assert_eq!(
            expression("прив", Mode::Prefix).expect("query"),
            "\"прив\"*"
        );
        assert!(expression(&"a ".repeat(13), Mode::All).is_err());
        assert_eq!(expression("\"():*", Mode::All).expect("query"), "");
    }
}
