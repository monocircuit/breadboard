//! The web UI (#435): the overviews of the daemon's repos as a page, served
//! from the binary on 127.0.0.1 only, its port the first repo's `[web] port`
//! (`BREADBOARD_WEB_PORT` wins; 0: none). The page is the overview as the
//! terminal draws it (`tui::overview::drawn`, rewrite rule 4): the view of
//! everything or an orchestration, read on each look of the page, GitLab
//! from the daemon's feed of that view, so a page costs GitLab nothing more
//! than an overview does.
//!
//! Safe by default: it binds the loopback address alone, answers only a
//! Host of its own or one `[web] hosts` or `BREADBOARD_WEB_HOSTS` names (a
//! proxy's, `tailscale serve`; a page of another site can't reach it under a name of its own),
//! sends nothing anywhere (its assets are in the binary, its CSP
//! allows only itself), and only reads. What changes something, today
//! `read GitLab again` for every overview of the view, needs `[web] writes
//! = true`, and comes only from the page itself (its header, its origin).

mod html;

use super::Feed as Said;
use super::serve::{State, feed_key, feed_of, leave};
use breadboard_core::reader::{LocalReader, View};
use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// How long a view's feed is followed after its page last looked.
const FOLLOWED_FOR: Duration = Duration::from_secs(30);
/// The longest request head read.
const HEAD_MAX: usize = 16 * 1024;
/// How long a request may take to arrive.
const READ_FOR: Duration = Duration::from_secs(10);
/// The width of a page that doesn't say its own.
const WIDTH: u16 = 120;

/// The header a write must carry: a form of another page can't set it.
const WRITE_HEADER: &str = "x-breadboard";

const INDEX: &str = include_str!("index.html");
const SCRIPT: &str = include_str!("app.js");
const STYLE: &str = include_str!("app.css");

/// The page loads itself and nothing else; its colours are attributes of the drawn cells.
const POLICY: &str = "default-src 'none'; script-src 'self'; style-src 'self'; style-src-attr 'unsafe-inline'; \
connect-src 'self'; img-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

/// The web UI's part of the daemon's state.
#[derive(Default)]
pub struct Pages {
    is_started: AtomicBool,
    /// The feeds the pages follow, by feed key: when a page last looked.
    followed: Mutex<HashMap<String, (Instant, Arc<super::feed::Feed>)>>,
}

/// What the listener was told by the first repo.
struct Served {
    port: u16,
    is_writable: bool,
    /// The names it answers to besides its own (`[web] hosts`).
    hosts: Vec<String>,
}

impl Served {
    /// 127.0.0.1 or localhost on its port, or a name `[web] hosts` gives:
    /// what a page of another site, through a name of its own that points
    /// here (DNS rebinding), doesn't send.
    fn is_own_host(&self, host: Option<&str>) -> bool {
        let Some(host) = host else {
            return false;
        };
        let port = self.port;
        host == format!("127.0.0.1:{port}")
            || host == format!("localhost:{port}")
            || self
                .hosts
                .iter()
                .any(|name| host == name || host.split_once(':').is_some_and(|(bare, _)| bare == name))
    }

    /// An `Origin` of the page itself: one of its hosts, over http or https.
    fn is_own_origin(&self, origin: &str) -> bool {
        let host = origin
            .strip_prefix("http://")
            .or_else(|| origin.strip_prefix("https://"));
        self.is_own_host(host)
    }
}

/// The web UI started once per daemon, on the port of `main`'s `[web]`; a
/// port in use leaves the daemon without one (said on its stderr).
pub fn start(state: &Arc<State>, main: &Path) {
    if state.web.is_started.swap(true, Ordering::SeqCst) {
        return;
    }
    let web = breadboard_core::config::of_main(main).config.web.clone();
    let port = std::env::var("BREADBOARD_WEB_PORT")
        .ok()
        .and_then(|port| port.parse().ok())
        .unwrap_or(web.port);
    if port == 0 {
        return;
    }
    let served = Arc::new(Served {
        port,
        is_writable: web.writes,
        hosts: hosts_of(web.hosts, std::env::var("BREADBOARD_WEB_HOSTS").ok().as_deref()),
    });
    let state = state.clone();
    tokio::spawn(async move {
        let listener = match TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port))).await {
            Ok(listener) => listener,
            Err(error) => {
                eprintln!("serve: no web UI on 127.0.0.1:{port}: {error}");
                return;
            }
        };
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                continue;
            };
            let (state, served) = (state.clone(), served.clone());
            tokio::spawn(async move {
                let _ = answer(stream, &state, &served).await;
            });
        }
    });
}

/// `[web] hosts` and the names of `BREADBOARD_WEB_HOSTS` (comma-separated):
/// a machine's name kept out of the committed file.
fn hosts_of(mut hosts: Vec<String>, env: Option<&str>) -> Vec<String> {
    hosts.extend(
        env.unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(String::from),
    );
    hosts
}

/// The feeds no page looked at for FOLLOWED_FOR: left (the daemon's look).
pub fn forget_closed(state: &State) {
    let gone: Vec<(String, Arc<super::feed::Feed>)> = {
        let mut followed = state.web.followed.lock().unwrap_or_else(|e| e.into_inner());
        let keys: Vec<String> = followed
            .iter()
            .filter(|(_, (at, _))| at.elapsed() >= FOLLOWED_FOR)
            .map(|(key, _)| key.clone())
            .collect();
        keys.into_iter()
            .filter_map(|key| followed.remove(&key).map(|(_, feed)| (key, feed)))
            .collect()
    };
    for (key, feed) in gone {
        leave(state, key, &feed);
    }
}

/// A request as far as the web UI reads it.
#[derive(Debug, PartialEq)]
struct Asked {
    method: String,
    path: String,
    query: HashMap<String, String>,
    host: Option<String>,
    origin: Option<String>,
    is_from_page: bool,
}

/// What goes back.
struct Answer {
    status: &'static str,
    kind: &'static str,
    body: String,
}

impl Answer {
    fn of(status: &'static str, kind: &'static str, body: impl Into<String>) -> Answer {
        Answer {
            status,
            kind,
            body: body.into(),
        }
    }

    fn text(status: &'static str, body: impl Into<String>) -> Answer {
        Answer::of(status, "text/plain; charset=utf-8", body)
    }
}

async fn answer(mut stream: TcpStream, state: &Arc<State>, served: &Served) -> std::io::Result<()> {
    let Ok(Ok(head)) = tokio::time::timeout(READ_FOR, read_head(&mut stream)).await else {
        return Ok(());
    };
    let answer = match parse(&head) {
        Some(asked) => route(asked, state, served).await,
        None => Answer::text("400 Bad Request", "bad request"),
    };
    let head = format!(
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nContent-Security-Policy: {POLICY}\r\n\
Referrer-Policy: no-referrer\r\nX-Content-Type-Options: nosniff\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        answer.status,
        answer.kind,
        answer.body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(answer.body.as_bytes()).await?;
    stream.shutdown().await
}

/// The request's head, up to its empty line; a body is never read.
async fn read_head(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let mut head = Vec::new();
    let mut chunk = [0u8; 2048];
    while !head.windows(4).any(|end| end == b"\r\n\r\n") && head.len() < HEAD_MAX {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        head.extend_from_slice(&chunk[..read]);
    }
    Ok(head)
}

fn parse(head: &[u8]) -> Option<Asked> {
    let mut headers = [httparse::EMPTY_HEADER; 64];
    let mut request = httparse::Request::new(&mut headers);
    if !request.parse(head).ok()?.is_complete() {
        return None;
    }
    let header = |name: &str| {
        request
            .headers
            .iter()
            .find(|header| header.name.eq_ignore_ascii_case(name))
            .and_then(|header| std::str::from_utf8(header.value).ok())
            .map(str::to_string)
    };
    let (path, query) = request.path?.split_once('?').unwrap_or((request.path?, ""));
    Some(Asked {
        method: request.method?.to_string(),
        path: path.to_string(),
        query: query_of(query),
        host: header("host"),
        origin: header("origin"),
        is_from_page: header(WRITE_HEADER).is_some(),
    })
}

/// `a=1&b=x%20y` by key, decoded.
fn query_of(query: &str) -> HashMap<String, String> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (decoded(key), decoded(value))
        })
        .collect()
}

/// Percent-decoded, `+` a space.
fn decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'+' => out.push(b' '),
            b'%' => match bytes
                .get(at + 1..at + 3)
                .and_then(|hex| std::str::from_utf8(hex).ok())
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
            {
                Some(byte) => {
                    out.push(byte);
                    at += 2;
                }
                None => out.push(b'%'),
            },
            byte => out.push(byte),
        }
        at += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

async fn route(asked: Asked, state: &Arc<State>, served: &Served) -> Answer {
    if !served.is_own_host(asked.host.as_deref()) {
        return Answer::text(
            "403 Forbidden",
            "only as 127.0.0.1, localhost or a name of `[web] hosts`",
        );
    }
    // A page that looks keeps the daemon, as an overview does.
    *state.idle_since.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
    let is_read = matches!(asked.method.as_str(), "GET" | "HEAD");
    if !is_read && !served.is_writable {
        return Answer::text(
            "405 Method Not Allowed",
            "read-only: `[web] writes = true` in breadboard.toml lets the page change something",
        );
    }
    match (asked.method.as_str(), asked.path.as_str()) {
        ("GET" | "HEAD", "/") => Answer::of("200 OK", "text/html; charset=utf-8", INDEX),
        ("GET" | "HEAD", "/app.js") => Answer::of("200 OK", "text/javascript; charset=utf-8", SCRIPT),
        ("GET" | "HEAD", "/app.css") => Answer::of("200 OK", "text/css; charset=utf-8", STYLE),
        ("GET" | "HEAD", "/page") => page(&asked, state, served).await,
        ("POST", "/refresh") => refresh(&asked, state, served),
        _ => Answer::text("404 Not Found", "not found"),
    }
}

/// The repos the daemon's overviews show, by path.
fn repos(state: &State) -> Vec<PathBuf> {
    let mut repos: Vec<PathBuf> = state
        .queues
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .cloned()
        .collect();
    repos.sort();
    repos
}

/// The repo and view a request names: `repo` its index, `orchestration` one of the repo's.
fn chosen(asked: &Asked, repos: &[PathBuf]) -> Option<(usize, View)> {
    let index = asked.query.get("repo").and_then(|repo| repo.parse().ok()).unwrap_or(0);
    let main = repos.get(index)?;
    let view = match asked.query.get("orchestration") {
        Some(name) if breadboard_core::record::recorded(main).contains(name) => View::Orchestration(name.clone()),
        Some(_) => return None,
        None => View::Everything,
    };
    Some((index, view))
}

/// The page's frame: the views to pick, then the view drawn.
async fn page(asked: &Asked, state: &Arc<State>, served: &Served) -> Answer {
    let repos = repos(state);
    let Some((index, view)) = chosen(asked, &repos) else {
        return Answer::text("404 Not Found", "no such repo or orchestration");
    };
    let width = asked
        .query
        .get("width")
        .and_then(|width| width.parse::<u16>().ok())
        .map_or(WIDTH, |width| width.clamp(40, 400));
    let main = repos[index].clone();
    let nav = html::nav(
        &repos,
        index,
        &view,
        breadboard_core::record::recorded,
        served.is_writable,
    );
    let read = {
        let (main, view) = (main.clone(), view.clone());
        tokio::task::spawn_blocking(move || {
            let mut reader = LocalReader::new(&main, view)?;
            let local = reader.read();
            let ask = reader.ask(&local);
            Some((local, ask))
        })
    };
    let Ok(Some((local, ask))) = read.await else {
        return Answer::text("500 Internal Server Error", "the local read failed");
    };
    let said = followed(state, &main, &view, ask);
    let drawn = tokio::task::spawn_blocking(move || {
        let buffer = crate::tui::overview::drawn(&main, view, local, said, width);
        html::frame(&buffer)
    })
    .await;
    match drawn {
        Ok(frame) => Answer::of("200 OK", "text/html; charset=utf-8", nav + &frame),
        Err(_) => Answer::text("500 Internal Server Error", "the view failed to draw"),
    }
}

/// What the feed of `view` said last (the last answer with data, a failure
/// after it), the feed followed for FOLLOWED_FOR from now.
fn followed(state: &State, main: &Path, view: &View, ask: breadboard_core::reader::RemoteAsk) -> Vec<Said> {
    let key = feed_key(main, &crate::tui::overview::view_key(view));
    let known = state
        .web
        .followed
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_mut(&key)
        .map(|(at, feed)| {
            *at = Instant::now();
            feed.clone()
        });
    let feed = match known {
        Some(feed) => {
            feed.ask(ask);
            feed
        }
        None => {
            let (feed, _, _) = feed_of(
                state,
                &key,
                main.to_path_buf(),
                crate::tui::overview::view_key(view),
                ask,
            );
            let mut followed = state.web.followed.lock().unwrap_or_else(|e| e.into_inner());
            match followed.get(&key) {
                // Another look of the page started it meanwhile: this one's follow is given back.
                Some((_, held)) => {
                    let held = held.clone();
                    drop(followed);
                    leave(state, key, &feed);
                    held
                }
                None => {
                    followed.insert(key, (Instant::now(), feed.clone()));
                    feed
                }
            }
        }
    };
    feed.last()
        .map(|last| {
            last.lines()
                .filter_map(|line| serde_json::from_str::<Said>(line).ok())
                .collect()
        })
        .unwrap_or_default()
}

/// `read GitLab again` for every overview of the view (a write: `[web] writes`), from the page alone.
fn refresh(asked: &Asked, state: &State, served: &Served) -> Answer {
    let is_own_origin = asked
        .origin
        .as_deref()
        .is_none_or(|origin| served.is_own_origin(origin));
    if !asked.is_from_page || !is_own_origin {
        return Answer::text("403 Forbidden", "only from the page itself");
    }
    let repos = repos(state);
    let Some((index, view)) = chosen(asked, &repos) else {
        return Answer::text("404 Not Found", "no such repo or orchestration");
    };
    let key = feed_key(&repos[index], &crate::tui::overview::view_key(&view));
    let feed = state
        .web
        .followed
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&key)
        .map(|(_, feed)| feed.clone());
    match feed {
        Some(feed) => {
            feed.refresh();
            Answer::text("200 OK", "reading again")
        }
        None => Answer::text("409 Conflict", "no page follows that view"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asked(head: &str) -> Asked {
        parse(head.replace('\n', "\r\n").as_bytes()).expect("a request")
    }

    #[test]
    fn reads_the_request_line_its_query_and_headers() {
        let request = asked("GET /page?repo=1&orchestration=a%20b+c&width=90 HTTP/1.1\nHost: 127.0.0.1:7435\n\n");
        assert_eq!(request.method, "GET");
        assert_eq!(request.path, "/page");
        assert_eq!(request.query["orchestration"], "a b c");
        assert_eq!(request.query["repo"], "1");
        assert_eq!(request.host.as_deref(), Some("127.0.0.1:7435"));
        assert!(!request.is_from_page);
        assert!(parse(b"GET / HTTP/1.1\r\nHost: x").is_none(), "a head without its end");
    }

    #[test]
    fn decodes_what_a_browser_encodes_and_keeps_what_is_broken() {
        assert_eq!(decoded("%E2%87%B6%20x"), "⇶ x");
        assert_eq!(decoded("100%"), "100%");
        assert_eq!(decoded("%zz"), "%zz");
        assert_eq!(decoded("a%2"), "a%2");
    }

    #[test]
    fn answers_only_its_own_names_and_those_it_was_given() {
        let served = Served {
            port: 7435,
            is_writable: false,
            hosts: vec!["box.tail1.ts.net".into()],
        };
        assert!(served.is_own_host(Some("127.0.0.1:7435")));
        assert!(served.is_own_host(Some("localhost:7435")));
        // tailscale serve's name, as it forwards it, with or without a port.
        assert!(served.is_own_host(Some("box.tail1.ts.net")));
        assert!(served.is_own_host(Some("box.tail1.ts.net:443")));
        // A name of another site that points at 127.0.0.1 (DNS rebinding), another port, none.
        assert!(!served.is_own_host(Some("evil.example:7435")));
        assert!(!served.is_own_host(Some("other.tail1.ts.net")));
        assert!(!served.is_own_host(Some("127.0.0.1:80")));
        assert!(!served.is_own_host(None));
        assert!(served.is_own_origin("https://box.tail1.ts.net"));
        assert!(served.is_own_origin("http://127.0.0.1:7435"));
        assert!(!served.is_own_origin("https://evil.example"));
    }

    #[test]
    fn takes_the_names_of_the_file_and_the_environment() {
        let hosts = hosts_of(vec!["a.ts.net".into()], Some(" b.ts.net, ,c.local"));
        assert_eq!(hosts, ["a.ts.net", "b.ts.net", "c.local"]);
        assert_eq!(hosts_of(Vec::new(), None), Vec::<String>::new());
    }

    #[test]
    fn its_page_loads_nothing_from_elsewhere() {
        for asset in [INDEX, SCRIPT, STYLE] {
            assert!(!asset.contains("http://") && !asset.contains("https://") && !asset.contains("//cdn"));
        }
        assert!(POLICY.starts_with("default-src 'none'"));
        assert!(!POLICY.contains("http"));
    }
}
