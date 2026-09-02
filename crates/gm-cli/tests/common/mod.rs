//! A harness for driving the real `gm` binary the way a person would.
//!
//! Everything else in the test suite calls the library directly, which means it
//! would all still pass with a completely broken command line. These tests type
//! commands into a sandbox and read what comes back: stdout, stderr, and the
//! exit status.
//!
//! `CARGO_BIN_EXE_gm` is set by Cargo for integration tests of a package that
//! builds a binary, so the tests always run the binary from the same build
//! rather than whatever happens to be on PATH.

#![allow(dead_code)] // each test file uses a different part of the harness

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use tempfile::TempDir;

pub const GM: &str = env!("CARGO_BIN_EXE_gm");
pub const AUTHOR: &str = "Test Engineer <test@example.com>";

/// The six-chainage route used by the UAT walkthrough.
pub fn example(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(name)
        .canonicalize()
        .unwrap_or_else(|e| panic!("example {name} should exist: {e}"))
}

/// A sandbox directory that commands run inside, so relative paths and the
/// "find the single *.gm here" behaviour work exactly as they would for a user.
pub struct Gm {
    pub dir: TempDir,
}

impl Gm {
    pub fn empty() -> Self {
        Gm {
            dir: TempDir::new().expect("temp dir"),
        }
    }

    /// A sandbox holding one committed file: the usual starting point.
    pub fn with_route() -> Self {
        let gm = Gm::empty();
        gm.ok(&[
            "init",
            "a13.gm",
            "--name",
            "A13 corridor",
            "--crs",
            "EPSG:27700",
            "--datum",
            "Ordnance Datum Newlyn",
        ]);
        gm.ok(&["import", example("a13-route.gm.json").to_str().unwrap()]);
        gm.ok(&["commit", "-m", "Import interpretation 001"]);
        gm
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn command(&self) -> Command {
        let mut cmd = Command::new(GM);
        cmd.current_dir(self.dir.path())
            .env("GM_AUTHOR", AUTHOR)
            // Inherited GM_FILE would silently redirect every command away from
            // the sandbox, so make sure the tests control it.
            .env_remove("GM_FILE")
            .env_remove("GM_TOKEN")
            .env("NO_COLOR", "1");
        cmd
    }

    /// Run a command and return the result, whatever it was.
    pub fn run(&self, args: &[&str]) -> Run {
        Run::of(self.command().args(args), args)
    }

    /// Run a command with no author configured anywhere.
    pub fn run_anonymous(&self, args: &[&str]) -> Run {
        Run::of(self.command().env_remove("GM_AUTHOR").args(args), args)
    }

    /// Run a command that must succeed.
    pub fn ok(&self, args: &[&str]) -> Run {
        let run = self.run(args);
        if !run.success() {
            panic!("expected success\n{run}");
        }
        run
    }

    /// Run a command that must fail. Refusals are a large part of what this
    /// tool promises, so they need testing as carefully as the happy path.
    pub fn fails(&self, args: &[&str]) -> Run {
        let run = self.run(args);
        if run.success() {
            panic!("expected failure\n{run}");
        }
        run
    }

    /// Start `gm ui` and wait until it answers. The child is killed on drop.
    pub fn serve(&self, file: &str) -> Server {
        self.spawn_server(file, "ui", &[])
    }

    /// Start `gm serve` with extra flags, e.g. `["--allow-push"]`.
    pub fn serve_sync(&self, file: &str, extra: &[&str]) -> Server {
        self.spawn_server(file, "serve", extra)
    }

    fn spawn_server(&self, file: &str, subcommand: &str, extra: &[&str]) -> Server {
        let port = free_port();
        let mut args = vec!["-f", file, subcommand, "--port"];
        let port_text = port.to_string();
        args.push(&port_text);
        args.extend_from_slice(extra);

        let child = self
            .command()
            // A token in the environment would silently authenticate every
            // request and hide the tests that check it is required.
            .env_remove("GM_TOKEN")
            .args(&args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap_or_else(|e| panic!("spawn gm {subcommand}: {e}"));
        let server = Server { child, port };
        server.wait_until_ready();
        server
    }

    /// Edit the file the way a user with any SQLite client would.
    pub fn sqlite(&self, file: &str, sql: &str) {
        rusqlite::Connection::open(self.path(file))
            .expect("open for editing")
            .execute_batch(sql)
            .unwrap_or_else(|e| panic!("SQL failed: {e}\n{sql}"));
    }

    /// The full hash of the current revision of `file`.
    pub fn head(&self, file: &str) -> String {
        self.ok(&["-f", file, "log", "-n", "1"])
            .stdout
            .split_whitespace()
            .next()
            .expect("a commit in the log")
            .to_string()
    }
}

pub struct Run {
    pub command: String,
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Run {
    fn of(cmd: &mut Command, args: &[&str]) -> Run {
        let out = cmd.output().expect("running gm");
        Run {
            command: format!("gm {}", args.join(" ")),
            code: out.status.code(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        }
    }

    pub fn success(&self) -> bool {
        self.code == Some(0)
    }

    /// Everything the user would see, for assertions that do not care which
    /// stream a message arrived on.
    pub fn output(&self) -> String {
        format!("{}{}", self.stdout, self.stderr)
    }

    #[track_caller]
    pub fn says(&self, needle: &str) -> &Self {
        assert!(
            self.output().contains(needle),
            "expected output to mention {needle:?}\n{self}"
        );
        self
    }

    #[track_caller]
    pub fn does_not_say(&self, needle: &str) -> &Self {
        assert!(
            !self.output().contains(needle),
            "expected output NOT to mention {needle:?}\n{self}"
        );
        self
    }

    #[track_caller]
    pub fn exits_with(&self, code: i32) -> &Self {
        assert_eq!(self.code, Some(code), "wrong exit status\n{self}");
        self
    }

    /// The first stdout line containing `needle`, split into whitespace-
    /// separated fields. Lets a test assert on the numbers in a table row
    /// without pinning the exact column widths.
    #[track_caller]
    pub fn row(&self, needle: &str) -> Vec<String> {
        let line = self
            .stdout
            .lines()
            .find(|l| l.contains(needle))
            .unwrap_or_else(|| panic!("no line containing {needle:?}\n{self}"));
        line.split_whitespace().map(str::to_string).collect()
    }

    #[track_caller]
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.stdout)
            .unwrap_or_else(|e| panic!("stdout should be JSON but {e}\n{self}"))
    }
}

impl std::fmt::Display for Run {
    /// Everything needed to understand a failure without rerunning it.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "  command: {}", self.command)?;
        writeln!(f, "  status:  {:?}", self.code)?;
        writeln!(f, "  stdout:\n{}", indent(&self.stdout))?;
        write!(f, "  stderr:\n{}", indent(&self.stderr))
    }
}

fn indent(text: &str) -> String {
    if text.trim().is_empty() {
        return "    (empty)".to_string();
    }
    text.lines()
        .map(|l| format!("    {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

// -- the web UI -------------------------------------------------------------

pub struct Server {
    child: Child,
    pub port: u16,
}

impl Server {
    /// The base URL a client would be given.
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn wait_until_ready(&self) {
        for _ in 0..200 {
            if std::net::TcpStream::connect(("127.0.0.1", self.port)).is_ok() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        panic!("gm ui never started listening on port {}", self.port);
    }

    pub fn get(&self, path: &str) -> Response {
        self.request("GET", path)
    }

    pub fn post(&self, path: &str) -> Response {
        self.request("POST", path)
    }

    /// A deliberately minimal HTTP client. `Connection: close` makes the server
    /// hang up after one response, so reading to EOF is enough and the test
    /// does not have to understand keep-alive or chunked encoding.
    pub fn request(&self, method: &str, path: &str) -> Response {
        use std::io::{Read, Write};

        let mut stream =
            std::net::TcpStream::connect(("127.0.0.1", self.port)).expect("connecting to gm ui");
        write!(
            stream,
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
        )
        .expect("sending request");

        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).expect("reading response");
        let text = String::from_utf8_lossy(&raw).into_owned();

        let (head, body) = text
            .split_once("\r\n\r\n")
            .unwrap_or_else(|| panic!("malformed response to {method} {path}:\n{text}"));
        let status: u16 = head
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|c| c.parse().ok())
            .unwrap_or_else(|| panic!("no status line in:\n{head}"));
        let content_type = head
            .lines()
            .find(|l| l.to_ascii_lowercase().starts_with("content-type:"))
            .map(|l| l[13..].trim().to_string())
            .unwrap_or_default();

        Response {
            path: path.to_string(),
            status,
            content_type,
            body: body.to_string(),
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub struct Response {
    pub path: String,
    pub status: u16,
    pub content_type: String,
    pub body: String,
}

impl Response {
    #[track_caller]
    pub fn ok(&self) -> &Self {
        assert_eq!(self.status, 200, "GET {} was not 200", self.path);
        self
    }

    #[track_caller]
    pub fn status_is(&self, expected: u16) -> &Self {
        assert_eq!(self.status, expected, "wrong status for {}", self.path);
        self
    }

    #[track_caller]
    pub fn says(&self, needle: &str) -> &Self {
        assert!(
            self.body.contains(needle),
            "{} should mention {needle:?}\n{}",
            self.path,
            self.body
        );
        self
    }

    #[track_caller]
    pub fn does_not_say(&self, needle: &str) -> &Self {
        assert!(
            !self.body.contains(needle),
            "{} should NOT mention {needle:?}",
            self.path
        );
        self
    }
}

/// Ask the OS for a free port, then release it. There is a small race between
/// releasing and the server binding, but nothing else on a test machine is
/// racing for ephemeral ports, and the alternative is a fixed port that makes
/// the tests unable to run in parallel.
fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("binding an ephemeral port");
    listener.local_addr().expect("local addr").port()
}
