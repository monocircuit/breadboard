//! What a click on a try opens (try.mjs, #295): `--run` a terminal, the
//! overview's detail pane running the command (#408); `--url` the page on
//! the worktree's app, its stack and `pnpm dev` started first when the app
//! is down; `--on test` the path on the test instance once it is deployed.
//! A newer click takes the place of an older one. And `stack:up` for a try
//! or the view of everything (the worktree view has its own buttons).

use super::spawn::{self, Background, Run, error_of};
use super::{Actor, Effect, browser, herdr};
use crate::tui::overview::Overview;
use breadboard_core::actions::{Action, actions_file, actions_of};
use breadboard_core::git::feature_of;
use breadboard_core::local::is_port_taken;
use breadboard_core::model::try_state::{TryState, app_origin, page_of, short_page};
use breadboard_core::model::{Paint, Span, Tree};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How long a click waits for `pnpm dev` to answer after the stack is up.
const APP_WAIT: Duration = Duration::from_secs(120);
const APP_POLL: Duration = Duration::from_millis(1000);

/// Where a click on its way to a page stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Stack,
    App,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trying {
    pub stage: Stage,
    pub error: Option<String>,
    pub page: String,
    pub id: i64,
    pub replaced: Option<i64>,
}

/// The tries' state, by worktree.
#[derive(Debug, Default)]
pub struct Tries {
    pub trying: HashMap<PathBuf, Trying>,
    /// Worktrees whose try terminal a click couldn't open, and why (no herdr, no pane beside).
    pub unopened: HashMap<PathBuf, String>,
    /// What the last click on a try of a worktree did: (its id, the text).
    pub clicked: HashMap<PathBuf, (i64, String)>,
}

/// Whether `page` is on the worktree's own app on `port`, the one a click starts when it is down.
pub fn is_on_app(page: &str, port: i64) -> bool {
    let origin = app_origin(port);
    page == origin || page.starts_with(&format!("{origin}/"))
}

/// The action of a try, from its file (a worktree's) or the record (the orchestrator's).
fn action_of(app: &Overview, dir: Option<&Path>, id: i64) -> Option<Action> {
    match dir {
        Some(dir) => actions_of(&actions_file(dir))
            .into_iter()
            .find(|action| action.id == id),
        None => {
            let (record, _) = app.record_and_name();
            actions_of(record.as_deref()?)
                .into_iter()
                .find(|action| action.id == id)
        }
    }
}

/// A url into the browser, in the tab group of the space of `dir` (`open`).
pub fn open(actor: &Actor, url: &str, dir: Option<&Path>) {
    let label = herdr::space_label(&actor.main, dir.unwrap_or(&actor.main));
    browser::open_in_group(&actor.main, url, &label);
}

/// A click on a try (`clickTry`): one that isn't ready opens why (a failed
/// deployment's job) or does nothing; else its spot.
pub fn click(app: &mut Overview, dir: Option<PathBuf>, id: i64, state: Option<TryState>) {
    let Some(action) = action_of(app, dir.as_deref(), id) else {
        app.actor.log(Paint::Dim, format!("try #{id} is answered already"));
        return;
    };
    if let Some(state) = state.filter(|state| !state.is_ready) {
        if let Some(url) = &state.url {
            open(&app.actor, url, None);
            app.actor
                .log(Paint::Dim, format!("try #{id}: {}, opened {url}", state.label));
        }
        return;
    }
    let tree = dir.as_deref().and_then(|dir| app.tree_of(dir));
    if action.on.as_deref() == Some("test") {
        let origin = app
            .orchestration_local()
            .and_then(|local| local.test_origin.clone())
            .or_else(|| breadboard_core::local::test_origin(&app.actor.main));
        if let (Some(origin), Some(url)) = (origin, &action.url) {
            let page = format!("{origin}{url}");
            open(&app.actor, &page, dir.as_deref());
            app.actor.log(Paint::Green, format!("try #{id}: opened {page}"));
        }
        return;
    }
    match (&action.run, &action.url, tree) {
        (Some(run), _, Some(tree)) => open_terminal(app, &tree.dir, id, run.clone()),
        (None, Some(_), Some(tree)) => open_page(app, &tree, &action),
        (None, Some(url), None) => {
            let page = page_of(url, 0);
            open(&app.actor, &page, None);
            app.actor.log(Paint::Green, format!("try #{id}: opened {page}"));
        }
        // Nothing to open: its worktree beside (#409: it has no space of its own).
        _ => {
            if let Some(dir) = dir {
                let (main, detail) = (app.actor.main.clone(), app.actor.detail.clone());
                super::super::act::in_detail(app, move || {
                    detail.show(&main, &dir, super::detail::Shows::Editor, None)
                });
            }
        }
    }
}

/// The try's command in the overview's detail pane (`openTerminal`, #408):
/// the same try again runs again there, another replaces what ran there.
fn open_terminal(app: &mut Overview, dir: &Path, id: i64, run: String) {
    let detail = app.actor.detail.clone();
    let dir = dir.to_path_buf();
    app.actor.queue(move || {
        let text = detail.run_try(&dir, id, &run);
        Some(Box::new(move |app: &mut Overview| {
            match text {
                Ok(text) => {
                    app.actor.tries.unopened.remove(&dir);
                    app.actor.log_as(
                        &format!("try:{id}"),
                        Paint::Green,
                        format!("try #{id}: {text}"),
                        Some(run.clone()),
                    );
                    app.actor.tries.clicked.insert(dir.clone(), (id, text));
                }
                Err(why) => {
                    app.actor.log_as(
                        &format!("try:{id}"),
                        Paint::Red,
                        format!("try #{id}: {why}: run it there yourself"),
                        Some(run.clone()),
                    );
                    app.actor.tries.unopened.insert(dir.clone(), why);
                }
            }
            app.redraw();
        }) as Effect)
    });
}

/// The page of a try (`openPage`): on the worktree's own app that is down,
/// its stack starts first, then the page waits for `pnpm dev`; a click while
/// one waits takes its place.
fn open_page(app: &mut Overview, tree: &Tree, action: &Action) {
    let dir = tree.dir.clone();
    let port = tree.app_port + tree.offset;
    let page = page_of(action.url.as_deref().unwrap_or_default(), port);
    let stack_error = app.actor.stacks.get(&dir).and_then(|run| run.error.clone());
    if let Some(state) = app.actor.tries.trying.get_mut(&dir)
        && state.error.is_none()
        && stack_error.is_none()
    {
        if state.id != action.id {
            state.replaced = Some(state.id);
            state.id = action.id;
            state.page = page;
        }
        return;
    }
    let is_own = tree.has_own_port && tree.has_env && is_on_app(&page, port);
    let is_starting = app.actor.stacks.get(&dir).is_some_and(|run| run.is_running);
    if !is_own || is_starting || is_port_taken(port) {
        app.actor.tries.trying.remove(&dir);
        app.actor
            .tries
            .clicked
            .insert(dir.clone(), (action.id, "opened".into()));
        open(&app.actor, &page, Some(&dir));
        app.actor
            .log(Paint::Green, format!("try #{}: opened {page}", action.id));
        return;
    }
    app.actor.tries.trying.insert(
        dir.clone(),
        Trying {
            stage: Stage::Stack,
            error: None,
            page,
            id: action.id,
            replaced: None,
        },
    );
    start_stack(app, &dir, Some(port));
}

/// The stack is up for a waiting try: the page once `pnpm dev` answers.
fn app_up(app: &mut Overview, dir: &Path, port: i64) {
    let Some(state) = app.actor.tries.trying.get_mut(dir) else {
        return;
    };
    state.stage = Stage::App;
    let poster = app.actor.poster();
    let dir = dir.to_path_buf();
    std::thread::spawn(move || {
        let until = Instant::now() + APP_WAIT;
        let is_up = loop {
            if is_port_taken(port) {
                break true;
            }
            if Instant::now() >= until {
                break false;
            }
            std::thread::sleep(APP_POLL);
        };
        poster.post(move |app: &mut Overview| {
            let Some(state) = app.actor.tries.trying.get(&dir).cloned() else {
                return;
            };
            if is_up {
                app.actor.tries.trying.remove(&dir);
                let text = match state.replaced {
                    Some(id) => format!("opened, replaced the try of #{id}"),
                    None => "opened".into(),
                };
                app.actor.tries.clicked.insert(dir.clone(), (state.id, text));
                open(&app.actor, &state.page, Some(&dir));
                app.actor
                    .log(Paint::Green, format!("try #{}: opened {}", state.id, state.page));
            } else {
                let why = format!(
                    "the app on {} didn't answer: see the tab dev",
                    short_page(&app_origin(port))
                );
                app.actor.log(Paint::Red, format!("try #{}: {why}", state.id));
                if let Some(trying) = app.actor.tries.trying.get_mut(&dir) {
                    trying.error = Some(why);
                }
            }
            app.redraw();
        });
    });
}

/// What a try's row says after its text while its click is under way or done
/// (`trySpot`); None: the row as the model has it.
pub fn spot(actor: &Actor, action: &Action, tree: Option<&Tree>) -> Option<Vec<Span>> {
    let tree = tree?;
    let dir = &tree.dir;
    if action.on.as_deref() == Some("test") {
        return None;
    }
    let clicked = actor
        .tries
        .clicked
        .get(dir)
        .filter(|(id, _)| *id == action.id)
        .map(|(_, text)| text.clone());
    if let Some(run) = &action.run {
        if let Some(why) = actor.tries.unopened.get(dir) {
            return Some(vec![Span::new(format!("{why}: run {run} there yourself"), Paint::Red)]);
        }
        return clicked.map(|done| vec![Span::new(done, Paint::Green), Span::dim(format!(" {run}"))]);
    }
    let page = page_of(action.url.as_deref()?, tree.app_port + tree.offset);
    let state = actor.tries.trying.get(dir).filter(|state| state.id == action.id);
    let stack = actor.stacks.get(dir);
    let error = state.and_then(|state| state.error.clone()).or_else(|| {
        state
            .filter(|state| state.stage == Stage::Stack)
            .and_then(|_| stack.and_then(|run| run.error.clone()))
    });
    if let Some(error) = error {
        return Some(vec![Span::new(error, Paint::Red)]);
    }
    let replacing = state
        .and_then(|state| state.replaced)
        .map(|id| format!(", replaced the try of #{id}"))
        .unwrap_or_default();
    match state.map(|state| state.stage) {
        Some(Stage::Stack) => {
            return Some(vec![
                Span::new(format!("◐ starting the stack …{replacing}"), Paint::Yellow),
                Span::dim(format!(" {}", stack.map(Run::last_line).unwrap_or_default())),
            ]);
        }
        Some(Stage::App) => {
            return Some(vec![
                Span::new(format!("◐ waiting for pnpm dev …{replacing}"), Paint::Yellow),
                Span::dim(format!(" {}", short_page(&page))),
            ]);
        }
        None => {}
    }
    clicked.map(|done| {
        vec![
            Span::new(done, Paint::Green),
            Span::dim(format!(" {}", short_page(&page))),
        ]
    })
}

/// `stack:up` of a checkout, detached (`startStack`, `▶ start the stack`, `u`);
/// `port`: a try waits for its page on the app's port once it is up.
pub fn start_stack(app: &mut Overview, dir: &Path, port: Option<i64>) {
    if app.actor.stacks.get(dir).is_some_and(|run| run.is_running) {
        return;
    }
    let base = dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let log = spawn::log_file(&format!("overview-stack-{base}.log"));
    let program = spawn::breadboard();
    let poster = app.actor.poster();
    let done = dir.to_path_buf();
    let started = spawn::start(
        Background {
            program: &program,
            args: vec!["stack:up".into()],
            cwd: dir,
            log: &log,
            env: vec![("INIT_CWD", dir.to_string_lossy().into_owned())],
            is_paneless: false,
        },
        move |code, last| {
            poster.post(move |app: &mut Overview| {
                let error = error_of(code, &last, "stack:up ");
                let who = feature_of(&app.actor.main, &done);
                match &error {
                    None => {
                        // Kept with its end: WORKTREES says `stack …` until docker tells after it (#332).
                        app.actor.stacks.insert(
                            done.clone(),
                            Run {
                                done_at: Some(breadboard_core::time::now_ms()),
                                ..Run::default()
                            },
                        );
                        app.actor.log_as(
                            &format!("stack:{who}"),
                            Paint::Green,
                            format!("the stack of {who} is up"),
                            None,
                        );
                        if let Some(port) = port {
                            app_up(app, &done, port);
                        }
                    }
                    Some(why) => {
                        if let Some(run) = app.actor.stacks.get_mut(&done) {
                            run.is_running = false;
                            run.error = Some(why.clone());
                        }
                        app.actor.log_as(
                            &format!("stack:{who}"),
                            Paint::Red,
                            format!("the stack of {who} didn't start: {why}"),
                            None,
                        );
                    }
                }
                app.redraw();
            });
        },
    );
    let who = feature_of(&app.actor.main, dir);
    match started {
        Ok(()) => {
            app.actor.stacks.insert(dir.to_path_buf(), Run::running(&log));
            app.actor.log_as(
                &format!("stack:{who}"),
                Paint::Yellow,
                format!("starting the stack of {who} …"),
                Some(breadboard_core::config::of(dir).command("stack:up")),
            );
        }
        Err(why) => {
            app.actor.stacks.insert(
                dir.to_path_buf(),
                Run {
                    error: Some(why.clone()),
                    ..Run::default()
                },
            );
            app.actor
                .log(Paint::Red, format!("the stack of {who} didn't start: {why}"));
        }
    }
}

/// The stack's migrations (`[stack.migrations] run`) in a worktree, detached
/// (STACK's `▶ <run>`, from WORKTREES since #332), once at a time; its log in
/// `$TMPDIR/overview-migrate-<basename>.log`. Its end forgets the
/// migrations core holds of it: the next read asks the database again.
pub fn migrate(app: &mut Overview, dir: &Path) {
    if app.actor.migrations.get(dir).is_some_and(|run| run.is_running) {
        return;
    }
    let base = dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let Some(run) = breadboard_core::stack::of(dir)
        .and_then(|stack| stack.migrations)
        .map(|m| m.run)
    else {
        return;
    };
    let log = spawn::log_file(&format!("overview-migrate-{base}.log"));
    let poster = app.actor.poster();
    let done = dir.to_path_buf();
    let said = format!("{run} ");
    let shown = run.clone();
    let started = spawn::start(
        Background {
            program: Path::new("sh"),
            args: vec!["-c".into(), run],
            cwd: &breadboard_core::stack::dir_of(dir),
            log: &log,
            env: Vec::new(),
            is_paneless: false,
        },
        move |code, last| {
            breadboard_core::model::develop::forget_migrations(&done);
            poster.post(move |app: &mut Overview| {
                let error = error_of(code, &last, &said);
                let who = feature_of(&app.actor.main, &done);
                let (paint, text) = match &error {
                    None => (Paint::Green, format!("the migrations of {who} are applied")),
                    Some(why) => (Paint::Red, format!("the migrations of {who} failed: {why}")),
                };
                app.actor.migrations.insert(
                    done.clone(),
                    Run {
                        done_at: error.is_none().then(breadboard_core::time::now_ms),
                        error,
                        ..Run::default()
                    },
                );
                app.actor.log_as(&format!("migrate:{who}"), paint, text, None);
                app.redraw();
            });
        },
    );
    let who = feature_of(&app.actor.main, dir);
    match started {
        Ok(()) => {
            app.actor.migrations.insert(dir.to_path_buf(), Run::running(&log));
            app.actor.log_as(
                &format!("migrate:{who}"),
                Paint::Yellow,
                format!("migrating the database of {who} …"),
                Some(shown),
            );
        }
        Err(why) => {
            app.actor.migrations.insert(
                dir.to_path_buf(),
                Run {
                    error: Some(why.clone()),
                    ..Run::default()
                },
            );
            app.actor
                .log(Paint::Red, format!("the migrations of {who} didn't start: {why}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_the_pages_of_the_worktrees_app() {
        assert!(is_on_app("http://localhost:3002/atlas", 3002));
        assert!(is_on_app("http://localhost:3002", 3002));
        assert!(!is_on_app("http://localhost:30021", 3002));
        assert!(!is_on_app("https://example.org/", 3002));
    }

    #[test]
    fn a_try_row_says_what_its_click_did() {
        let mut actor = Actor::new(
            &breadboard_core::reader::View::Everything,
            Path::new("/r"),
            false,
            false,
        );
        let tree = Tree {
            dir: "/r-7".into(),
            offset: 2,
            app_port: 3000,
            ..Default::default()
        };
        let run = Action {
            id: 1,
            kind: "try".into(),
            run: Some("cargo run".into()),
            ..Default::default()
        };
        assert_eq!(spot(&actor, &run, Some(&tree)), None);
        actor.tries.clicked.insert("/r-7".into(), (1, "runs beside".into()));
        let text = breadboard_core::model::text_of(&spot(&actor, &run, Some(&tree)).unwrap());
        assert_eq!(text, "runs beside cargo run");
        let page = Action {
            id: 2,
            kind: "try".into(),
            url: Some("/atlas".into()),
            ..Default::default()
        };
        actor.tries.trying.insert(
            "/r-7".into(),
            Trying {
                stage: Stage::App,
                error: None,
                page: "http://localhost:3002/atlas".into(),
                id: 2,
                replaced: Some(1),
            },
        );
        let text = breadboard_core::model::text_of(&spot(&actor, &page, Some(&tree)).unwrap());
        assert_eq!(text, "◐ waiting for pnpm dev …, replaced the try of #1 :3002/atlas");
    }
}
