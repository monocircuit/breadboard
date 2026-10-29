//! The node's pushes to lab web (#501, core's `lab`), and `breadboard lab`.
//!
//! While the user is logged in (`lab login`), the daemon stays and pushes
//! what this node runs, outbound only: the full snapshot first, then every
//! EVERY a batch of events: each repo whose state changed (`Event::Repo`)
//! and one sample of what its processes take (`Event::Resources`), which
//! doubles as the heartbeat. A push that fails is tried again after
//! `lab::backoff`, as a snapshot; a token lab web refuses waits for the
//! next login. The repos pushed: those of its overviews and those `lab
//! login` was run in. One daemon of the user pushes (`lab.pid` beside the
//! login): a build of other sources has its own.

use super::serve::State;
use breadboard_core::lab::{self, Client, Config, Failed, Register, Sequenced};
use breadboard_core::node::{Event, Node, Repo, RepoFacts, host, snapshot_of};
use breadboard_core::resources::{Owners, Sampler, summary_of};
use breadboard_core::time::now_ms;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

/// How often a batch of events goes.
const EVERY: Duration = Duration::from_secs(10);
/// How often a node logged out looks for a login.
const LOOK: Duration = Duration::from_secs(5);

/// EVERY and LOOK, both `BREADBOARD_LAB_EVERY_MS` in a gauntlet world.
fn every() -> (Duration, Duration) {
    std::env::var("BREADBOARD_LAB_EVERY_MS")
        .ok()
        .and_then(|ms| ms.parse().ok())
        .map(|ms| (Duration::from_millis(ms), Duration::from_millis(ms)))
        .unwrap_or((EVERY, LOOK))
}

/// Whether the user is logged in to lab: the daemon stays while they are.
pub fn is_on() -> bool {
    lab::read().is_some_and(|config| config.is_logged_in())
}

/// The pushes, as long as the daemon runs.
pub(super) async fn push(state: Arc<State>) {
    let mut pusher = Pusher::default();
    loop {
        let wait = pusher.round(&state).await;
        tokio::time::sleep(wait).await;
    }
}

/// What the pushes keep between rounds.
#[derive(Default)]
struct Pusher {
    /// The token and server of the last round: a new login starts over.
    login: Option<(String, String)>,
    /// What lab web has of each repo, by main; empty: the next push is a snapshot.
    sent: HashMap<PathBuf, Repo>,
    seq: i64,
    failures: u32,
    /// A token lab web refused: no push until another login.
    refused: Option<String>,
    sampler: Option<Sampler>,
}

impl Pusher {
    /// One round: pushed, or why not; how long until the next one.
    async fn round(&mut self, state: &State) -> Duration {
        let (every, look) = every();
        let Some(config) = tokio::task::spawn_blocking(lab::read).await.ok().flatten() else {
            *self = Pusher::default();
            return look;
        };
        let Some(token) = config.token.clone() else {
            *self = Pusher::default();
            return look;
        };
        let login = Some((config.server.clone(), token.clone()));
        if self.login != login {
            *self = Pusher {
                login,
                ..Pusher::default()
            };
        }
        if self.refused.as_deref() == Some(token.as_str()) || !holds() {
            return look;
        }
        let mains = mains_of(state, &config);
        let mut sampler = self.sampler.take().unwrap_or_default();
        let (repos, resources, sampler) = tokio::task::spawn_blocking(move || {
            let now = now_ms();
            let repos: Vec<RepoFacts> = mains.iter().filter_map(|main| RepoFacts::read(main, now)).collect();
            let resources = sampler.sample().map(|usage| summary_of(&usage, &Owners::read(&mains)));
            (repos, resources, sampler)
        })
        .await
        .unwrap_or_default();
        self.sampler = Some(sampler);
        let now = now_ms();
        let snapshot = snapshot_of(node(), &repos, resources, now);
        let client = Client::new(&config.server);
        let pushed = if self.sent.is_empty() {
            let pushed = client.snapshot(&token, snapshot.clone()).await;
            pushed.map(|pushed| {
                self.seq = pushed.seq;
                self.sent = snapshot.repos.into_iter().map(|repo| (repo.main.clone(), repo)).collect();
            })
        } else {
            let events = self.events_of(snapshot, now);
            let seq = self.seq;
            client.events(&token, events.clone()).await.map(|pushed| {
                self.seq = pushed.seq.max(seq);
                for sequenced in events {
                    if let Event::Repo { repo, .. } = sequenced.event {
                        self.sent.insert(repo.main.clone(), repo);
                    }
                }
            })
        };
        match pushed {
            Ok(()) => {
                self.failures = 0;
                every
            }
            Err(Failed::Unauthorized | Failed::Protocol(_)) => {
                self.refused = Some(token);
                look
            }
            Err(Failed::Other(_)) => {
                self.failures += 1;
                self.sent.clear();
                lab::backoff(self.failures)
            }
        }
    }

    /// The events since the last push: each repo that changed, the resources.
    fn events_of(&self, snapshot: breadboard_core::node::Snapshot, now: i64) -> Vec<Sequenced> {
        let mut events: Vec<Event> = snapshot
            .repos
            .into_iter()
            .filter(|repo| self.sent.get(&repo.main) != Some(repo))
            .map(|repo| Event::Repo { at: now, repo })
            .collect();
        if let Some(resources) = snapshot.resources {
            events.push(lab::slim_event(Event::Resources { at: now, resources }));
        }
        events
            .into_iter()
            .zip(self.seq + 1..)
            .map(|(event, seq)| Sequenced { seq, event })
            .collect()
    }
}

/// The main checkouts pushed: those of its overviews, those of the login.
fn mains_of(state: &State, config: &Config) -> Vec<PathBuf> {
    let mut mains: Vec<PathBuf> = state.queues.lock().unwrap_or_else(|e| e.into_inner()).iter().cloned().collect();
    for main in &config.repos {
        if !mains.contains(main) && main.is_dir() {
            mains.push(main.clone());
        }
    }
    mains.sort();
    mains
}

/// This machine and who runs it, as the snapshot names them.
fn node() -> Node {
    Node {
        host: host(),
        user: std::env::var("USER").unwrap_or_default(),
        version: format!("{} {}", env!("CARGO_PKG_VERSION"), crate::source_key())
            .trim_end()
            .to_string(),
    }
}

/// Whether this daemon is the one of the user that pushes: the first one alive holds `lab.pid`.
pub(super) fn holds() -> bool {
    let file = lab::config_file().with_file_name("lab.pid");
    let own = i64::from(std::process::id());
    match breadboard_core::lock::pid_in(&file) {
        pid if pid == own => true,
        pid if pid != 0 && breadboard_core::lock::is_alive(pid) => false,
        _ => {
            let beside = file.with_extension(format!("pid.{own}"));
            std::fs::write(&beside, format!("{own}\n")).is_ok() && std::fs::rename(&beside, &file).is_ok()
        }
    }
}

const USAGE: &str = "usage: breadboard lab login [--server <url>] | logout | status";

/// `breadboard lab login|logout|status`.
pub fn cli(args: &[String]) -> ExitCode {
    match args.first().map(String::as_str) {
        Some("login") => login(&args[1..]),
        Some("logout") => logout(),
        Some("status") => status(),
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

/// The device flow, the node registered, the login kept, the daemon started.
fn login(args: &[String]) -> ExitCode {
    let kept = lab::read();
    let server = match args {
        [] => kept
            .as_ref()
            .map(|config| config.server.clone())
            .unwrap_or_else(|| lab::DEFAULT_SERVER.to_string()),
        [flag, url] if flag == "--server" => url.trim_end_matches('/').to_string(),
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    let machine = kept
        .as_ref()
        .map(|config| config.machine.clone())
        .filter(|machine| !machine.is_empty())
        .unwrap_or_else(lab::new_machine);
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => return fail(&error.to_string()),
    };
    let registered = runtime.block_on(async {
        let client = Client::new(&server);
        let login = client.login().await?;
        let (endpoints, device) = client.device(&login).await?;
        let link = device.verification_uri_complete.as_deref().unwrap_or(&device.verification_uri);
        println!("Open {link}\nand confirm the code {}", device.user_code);
        let mut interval = Duration::from_secs(device.interval.max(1));
        let until = tokio::time::Instant::now() + Duration::from_secs(device.expires_in);
        let access = loop {
            tokio::time::sleep(interval).await;
            if tokio::time::Instant::now() > until {
                return Err(Failed::Other("the code expired: run lab login again".into()));
            }
            if let Some(access) = client.poll(&endpoints, &login, &device, &mut interval).await? {
                break access;
            }
        };
        let register = Register {
            protocol: lab::PROTOCOL,
            machine: machine.clone(),
            host: host(),
        };
        client.register(&access, &register).await
    });
    let registered = match registered {
        Ok(registered) => registered,
        Err(error) => return fail(&error.to_string()),
    };
    let mut repos = kept.map(|config| config.repos).unwrap_or_default();
    let here = std::env::current_dir().ok().and_then(|cwd| breadboard_core::git::main_dir(&cwd));
    if let Some(main) = here.filter(|main| !repos.contains(main)) {
        repos.push(main);
    }
    let config = Config {
        server,
        machine,
        node: Some(registered.node),
        token: Some(registered.token),
        user: Some(registered.user.clone()),
        repos,
    };
    if let Err(error) = lab::write(&config) {
        return fail(&format!("the login wasn't kept: {error}"));
    }
    let is_pushing = super::client::wake_lab();
    println!(
        "✓ logged in to {} as {}{}",
        config.server,
        registered.user,
        if is_pushing { ", this node pushes" } else { ": the daemon didn't start, an overview starts it" }
    );
    ExitCode::SUCCESS
}

/// The token forgotten; the machine's id stays for the next login.
fn logout() -> ExitCode {
    let Some(mut config) = lab::read() else {
        println!("not logged in to lab");
        return ExitCode::SUCCESS;
    };
    config.node = None;
    config.token = None;
    config.user = None;
    if let Err(error) = lab::write(&config) {
        return fail(&error.to_string());
    }
    println!("✓ logged out of {}", config.server);
    ExitCode::SUCCESS
}

fn status() -> ExitCode {
    match lab::read().filter(Config::is_logged_in) {
        Some(config) => println!(
            "logged in to {} as {} (node {}), pushes {}",
            config.server,
            config.user.as_deref().unwrap_or("?"),
            config.node.as_deref().unwrap_or("?"),
            config
                .repos
                .iter()
                .map(|main| main.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        None => println!("not logged in to lab: breadboard lab login"),
    }
    ExitCode::SUCCESS
}

fn fail(why: &str) -> ExitCode {
    eprintln!("\n✗ lab: {why}");
    ExitCode::FAILURE
}
