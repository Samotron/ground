//! HTML shell and stylesheet.
//!
//! Everything is inlined and served from the binary: no CDN, no build step, no
//! network access. `gm ui` has to work on a site laptop with no internet.

use std::fmt::Write;

/// Escape text for use in HTML element content or a quoted attribute.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// Tokens and the section drawing, shared verbatim with the browser editor so
/// the two surfaces cannot drift apart. See `assets/gm.css`.
const SHARED: &str = include_str!("../../../../assets/gm.css");

/// Chrome for the served pages only: header, nav, tables and panels. The
/// editor has its own, because an editor and a viewer legitimately differ in
/// everything except how they draw ground.
const STYLE: &str = r#"
* { box-sizing: border-box; }
body {
  margin: 0;
  background: var(--bg);
  color: var(--ink);
  font: 15px/1.55 var(--font-sans);
}
header.top {
  border-bottom: 1px solid var(--line);
  background: var(--panel);
  padding: 14px 24px;
  display: flex;
  align-items: baseline;
  gap: 20px;
  flex-wrap: wrap;
  position: sticky;
  top: 0;
  z-index: 2;
}
header.top h1 { font-size: 16px; margin: 0; font-weight: 650; letter-spacing: .01em; }
header.top h1 a { color: var(--ink); text-decoration: none; }
nav a { color: var(--muted); text-decoration: none; margin-right: 16px; font-size: 14px; }
nav a:hover, nav a.on { color: var(--accent); }
main { max-width: 1000px; margin: 0 auto; padding: 24px; }
h2 { font-size: 15px; text-transform: uppercase; letter-spacing: .06em;
     color: var(--muted); font-weight: 600; margin: 32px 0 10px; }
h2:first-child { margin-top: 0; }
a { color: var(--accent); }
table { border-collapse: collapse; width: 100%; font-size: 14px; }
th {
  text-align: left; font-weight: 600; color: var(--muted); padding: 6px 12px 6px 0;
  border-bottom: 1px solid var(--line); font-size: 12px;
  text-transform: uppercase; letter-spacing: .05em; white-space: nowrap;
}
td { padding: 7px 12px 7px 0; border-bottom: 1px solid var(--line); vertical-align: top; }
td.num, th.num { text-align: right; font-variant-numeric: tabular-nums;
                 font-family: var(--font-mono); }
.panel { background: var(--panel); border: 1px solid var(--line);
         border-radius: 8px; padding: 18px 20px; }
.cols { display: flex; gap: 28px; align-items: flex-start; flex-wrap: wrap; }
.cols > .grow { flex: 1 1 340px; min-width: 0; }
dl.facts { display: grid; grid-template-columns: max-content 1fr; gap: 4px 18px; margin: 0; font-size: 14px; }
dl.facts dt { color: var(--muted); }
dl.facts dd { margin: 0; }
code, .mono { font-family: var(--font-mono); font-size: 13px; }
.hash { color: var(--muted); font-family: var(--font-mono); font-size: 13px; }
.note { color: var(--muted); font-size: 14px; }
.tag { display: inline-block; padding: 1px 7px; border-radius: 999px;
       border: 1px solid var(--line); font-size: 12px; color: var(--muted); }
.sev-error { color: var(--error); font-weight: 600; }
.sev-warning { color: var(--warning); font-weight: 600; }
.empty { color: var(--muted); font-style: italic; }

/* The editor fits the drawing into a pane; here it flows with the page. */
svg.section { display: block; max-width: 100%; height: auto; }
"#;

/// Wrap body content in the full page shell.
pub fn render(file_name: &str, title: &str, active: &str, body: &str) -> String {
    let mut out = String::with_capacity(body.len() + STYLE.len() + 1024);
    let _ = write!(
        out,
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>{} — {}</title><style>{SHARED}{STYLE}</style></head><body>",
        escape(title),
        escape(file_name)
    );

    let _ = write!(
        out,
        "<header class=\"top\"><h1><a href=\"/\">{}</a></h1><nav>",
        escape(file_name)
    );
    for (href, label, key) in [
        ("/", "Models", "models"),
        ("/materials", "Materials", "materials"),
        ("/history", "History", "history"),
        ("/validate", "Validation", "validate"),
    ] {
        let class = if key == active { " class=\"on\"" } else { "" };
        let _ = write!(out, "<a href=\"{href}\"{class}>{label}</a>");
    }
    out.push_str("</nav></header><main>");
    out.push_str(body);
    out.push_str("</main></body></html>");
    out
}

/// A number formatted for a table cell, or an em dash when absent.
pub fn num(value: Option<f64>, places: usize) -> String {
    match value {
        Some(v) => format!("{v:.places$}"),
        None => "&mdash;".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escaping_neutralises_markup() {
        assert_eq!(
            escape(r#"<script>alert("x")</script>"#),
            "&lt;script&gt;alert(&quot;x&quot;)&lt;/script&gt;"
        );
        assert_eq!(escape("A & B"), "A &amp; B");
        assert_eq!(escape("it's"), "it&#39;s");
    }

    #[test]
    fn escaping_leaves_ordinary_keys_alone() {
        assert_eq!(escape("CH-100"), "CH-100");
        assert_eq!(escape("LONDON_CLAY"), "LONDON_CLAY");
    }

    #[test]
    fn missing_numbers_render_as_a_dash() {
        assert_eq!(num(Some(82.5), 2), "82.50");
        assert_eq!(num(None, 2), "&mdash;");
    }
}
