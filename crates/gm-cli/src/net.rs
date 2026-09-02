//! A minimal HTTP/1.1 client, just enough for sync.
//!
//! Deliberately hand-rolled rather than pulling in an HTTP stack: the whole
//! premise is one self-contained binary with nothing to install, and the four
//! sync endpoints need one request shape between them.
//!
//! **Plain HTTP only.** Adding TLS would mean a large dependency tree for
//! something this small, so `gm serve` is for a network you already trust — a
//! LAN, a VPN, or a tunnel you made yourself. `https://` is refused rather than
//! silently downgraded, and `--token` guards access but does not encrypt
//! anything.

use anyhow::{Context, Result, anyhow, bail};
use gm_core::wire::{self, Object, RemoteInfo};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(30);

pub struct HttpRemote {
    pub url: String,
    host: String,
    port: u16,
    /// Path the server is mounted under, without a trailing slash.
    prefix: String,
    token: Option<String>,
}

impl std::fmt::Debug for HttpRemote {
    /// Written by hand rather than derived: a derived `Debug` would print the
    /// token, and secrets have a habit of ending up in panic messages, logs and
    /// bug reports.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpRemote")
            .field("url", &self.url)
            .field("token", &self.token.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

impl HttpRemote {
    /// True when this string names an HTTP remote rather than a file path.
    pub fn looks_like_url(text: &str) -> bool {
        text.starts_with("http://") || text.starts_with("https://")
    }

    pub fn parse(url: &str, token: Option<String>) -> Result<Self> {
        if let Some(rest) = url.strip_prefix("https://") {
            let _ = rest;
            bail!(
                "https is not supported: gm speaks plain HTTP and is meant for a \
                 network you already trust. Use http://, or put a tunnel in front."
            );
        }
        let rest = url
            .strip_prefix("http://")
            .ok_or_else(|| anyhow!("not an http:// URL: {url}"))?;

        let (authority, path) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, ""),
        };
        let (host, port) = match authority.rsplit_once(':') {
            Some((h, p)) => (
                h.to_string(),
                p.parse().with_context(|| format!("bad port in {url}"))?,
            ),
            None => (authority.to_string(), 80u16),
        };
        if host.is_empty() {
            bail!("no host in {url}");
        }

        let prefix = path.trim_end_matches('/').to_string();
        Ok(HttpRemote {
            url: format!("http://{host}:{port}{prefix}"),
            host,
            port,
            prefix,
            token,
        })
    }

    // -- the four endpoints -------------------------------------------------

    pub fn info(&self) -> Result<RemoteInfo> {
        let response = self.get("/sync/info")?;
        let body = response.expect_ok("/sync/info")?;
        serde_json::from_slice(&body).with_context(|| {
            format!(
                "{} did not answer with a ground-model remote description; \
                 is it running `gm serve`?",
                self.url
            )
        })
    }

    pub fn commits(&self) -> Result<Vec<String>> {
        let body = self.get("/sync/commits")?.expect_ok("/sync/commits")?;
        Ok(String::from_utf8_lossy(&body)
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect())
    }

    /// Ask for everything reachable from the remote's head that we lack.
    pub fn bundle(&self, we_have: &[String]) -> Result<Vec<Object>> {
        let body = we_have.join("\n");
        let response = self.post("/sync/bundle", body.as_bytes(), "text/plain", None)?;
        Ok(wire::decode_bundle(&response.expect_ok("/sync/bundle")?)?)
    }

    pub fn push(&self, objects: &[Object], head: &str) -> Result<PushAccepted> {
        let body = wire::encode_bundle(objects);
        let response = self.post("/sync/push", &body, "application/vnd.gm.bundle", Some(head))?;
        let body = response.expect_ok("/sync/push")?;
        serde_json::from_slice(&body).context("the remote's answer to a push was not JSON")
    }

    // -- plumbing -----------------------------------------------------------

    fn get(&self, path: &str) -> Result<HttpResponse> {
        self.request("GET", path, &[], None, None)
    }

    fn post(
        &self,
        path: &str,
        body: &[u8],
        content_type: &str,
        head: Option<&str>,
    ) -> Result<HttpResponse> {
        self.request("POST", path, body, Some(content_type), head)
    }

    fn request(
        &self,
        method: &str,
        path: &str,
        body: &[u8],
        content_type: Option<&str>,
        head: Option<&str>,
    ) -> Result<HttpResponse> {
        let mut stream = TcpStream::connect((self.host.as_str(), self.port))
            .with_context(|| format!("connecting to {}", self.url))?;
        // Without these a server that accepts the connection and then stops
        // talking would hang the command indefinitely.
        stream.set_read_timeout(Some(TIMEOUT))?;
        stream.set_write_timeout(Some(TIMEOUT))?;

        let mut request = format!(
            "{method} {}{path} HTTP/1.1\r\nHost: {}:{}\r\nConnection: close\r\n",
            self.prefix, self.host, self.port
        );
        if let Some(token) = &self.token {
            request.push_str(&format!("Authorization: Bearer {token}\r\n"));
        }
        if let Some(head) = head {
            request.push_str(&format!("{}: {head}\r\n", crate::web::sync::HEAD_HEADER));
        }
        if let Some(content_type) = content_type {
            request.push_str(&format!(
                "Content-Type: {content_type}\r\nContent-Length: {}\r\n",
                body.len()
            ));
        }
        request.push_str("\r\n");

        stream.write_all(request.as_bytes())?;
        if !body.is_empty() {
            stream.write_all(body)?;
        }
        stream.flush()?;

        // `Connection: close` means reading to EOF is the whole response, so
        // there is no keep-alive or chunked framing to understand.
        let mut raw = Vec::new();
        stream
            .read_to_end(&mut raw)
            .with_context(|| format!("reading the reply from {}", self.url))?;
        HttpResponse::parse(&raw, &self.url)
    }
}

struct HttpResponse {
    status: u16,
    body: Vec<u8>,
    url: String,
}

impl HttpResponse {
    fn parse(raw: &[u8], url: &str) -> Result<Self> {
        let split = raw
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .ok_or_else(|| anyhow!("{url} sent a malformed reply"))?;
        let head = String::from_utf8_lossy(&raw[..split]);
        let status: u16 = head
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|c| c.parse().ok())
            .ok_or_else(|| anyhow!("{url} sent a reply with no status line"))?;
        Ok(HttpResponse {
            status,
            body: raw[split + 4..].to_vec(),
            url: url.to_string(),
        })
    }

    /// The body, or an error carrying whatever the server said went wrong. The
    /// remote's own message is almost always more useful than a status code.
    fn expect_ok(self, endpoint: &str) -> Result<Vec<u8>> {
        if self.status == 200 {
            return Ok(self.body);
        }
        let detail = String::from_utf8_lossy(&self.body);
        let detail = detail.trim();
        let hint = match self.status {
            401 => "  (this remote needs --token)",
            403 => "  (start it with --allow-push to accept pushes)",
            404 => "  (is it running `gm serve` rather than `gm ui`?)",
            _ => "",
        };
        if detail.is_empty() {
            bail!("{}{endpoint} returned {}{hint}", self.url, self.status);
        }
        bail!("{}: {detail}{hint}", self.url)
    }
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushAccepted {
    pub outcome: String,
    pub head: String,
    #[serde(default)]
    pub received: usize,
    #[serde(default)]
    pub commits: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_are_parsed_into_their_parts() {
        let remote = HttpRemote::parse("http://office.local:8766", None).expect("parse");
        assert_eq!(remote.host, "office.local");
        assert_eq!(remote.port, 8766);
        assert_eq!(remote.prefix, "");
        assert_eq!(remote.url, "http://office.local:8766");
    }

    #[test]
    fn the_default_port_is_80_and_a_path_prefix_is_kept() {
        let remote = HttpRemote::parse("http://example.com/jobs/a13/", None).expect("parse");
        assert_eq!(remote.port, 80);
        assert_eq!(remote.prefix, "/jobs/a13");
    }

    #[test]
    fn https_is_refused_rather_than_quietly_downgraded() {
        let err = HttpRemote::parse("https://example.com", None).expect_err("should refuse");
        assert!(
            err.to_string().contains("https is not supported"),
            "got: {err}"
        );
    }

    #[test]
    fn a_file_path_is_not_mistaken_for_a_url() {
        assert!(!HttpRemote::looks_like_url("../office/a13.gm"));
        assert!(!HttpRemote::looks_like_url("/srv/jobs/a13.gm"));
        assert!(HttpRemote::looks_like_url("http://office.local:8766"));
    }

    #[test]
    fn a_response_is_split_into_status_and_body() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\nhello";
        let response = HttpResponse::parse(raw, "http://x").expect("parse");
        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"hello");
    }

    #[test]
    fn an_error_reply_carries_the_remotes_own_message() {
        let raw = b"HTTP/1.1 403 Forbidden\r\n\r\nthis remote is read-only";
        let err = HttpResponse::parse(raw, "http://x")
            .expect("parse")
            .expect_ok("/sync/push")
            .expect_err("should be an error");
        assert!(
            err.to_string().contains("this remote is read-only"),
            "got: {err}"
        );
        assert!(err.to_string().contains("--allow-push"), "got: {err}");
    }
}
