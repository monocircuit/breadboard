//! `pnpm wt:action` (commands/action.mjs): someone asks the human for
//! something concrete. The row lands in ACTIONS of the orchestration's
//! overview, where a click does it and ✓ ticks it off; an agent's go into
//! its worktree's `.claude/actions.json`, the orchestrator's into the record.

use super::super::typo::nearest;
use super::change;
use super::common::{Argv, Here, Outcome, Takes, help, say};
use breadboard_core::actions::{
    ACTION_KINDS, Action, NewAction, actions_file, actions_of, add_action, answer_line, answered_of, answers_of,
    decide_problem, done_action, split_option, take_answers,
};
use breadboard_core::git::push_state;
use breadboard_core::lock::lock_of;
use breadboard_core::record::{is_name, origin_of, record_file, record_of};
use breadboard_core::said::{note_file, set_note};
use breadboard_core::time::now_ms;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

pub const USAGE: &str = r#"usage: pnpm wt:action <kind> <text> [--url <url>]   kind: merge, try, decide, look, plan
       pnpm wt:action decide "<what is decided>?" --option "<label>" --why "<what happens>" --option …… --recommend <label>
                (or --option "<label>: <what happens>", split at the first colon)
       pnpm wt:action try <what to check> --run <command> | --url <path or url>
       pnpm wt:action try <what to check> --on test --url <path>   on the test instance, ready once the MR is merged and deployed
       pnpm wt:action merge <what is in it> --orchestration <name> --mr !N   the orchestrator's merge of an MR
       pnpm wt:action note <one line>   what you are on, in plain words, in your block in WORK
       pnpm wt:action change <what>     a change for this MR: draft at once, merge withdrawn (--orchestration <name> --mr !N: the orchestrator's)
       pnpm wt:action --list   the open actions, then the answered ones (a decision answered is taken)
       pnpm wt:action --done <id>
       pnpm wt:action --answers   the answers not yet delivered, handed over (they go)
       … --orchestration <name>   the orchestrator's own, from any checkout"#;

/// The flags that take a value.
const VALUED: [&str; 9] = [
    "--url",
    "--run",
    "--done",
    "--orchestration",
    "--option",
    "--why",
    "--recommend",
    "--mr",
    "--on",
];

const ABOUT: &str = "pnpm wt:action asks the human for something concrete: a row in ACTIONS of the
orchestration's overview, where a click does it and ✓ ticks it off.";
const TAKES: Takes = Takes {
    valued: &VALUED,
    switches: &["--list", "--answers"],
    words: None,
    usage: USAGE,
    command: "wt:action",
};

/// How old a lock may grow before the next writer takes it over (actions.mjs `LOCK_STALE`).
const LOCK_STALE: std::time::Duration = std::time::Duration::from_secs(5);

/// Says on stderr what a write waits for when another writer holds the
/// file's lock: silence looked like a hang. A lock older than 5 s is taken
/// over at once, its writer ended without removing it.
pub fn announce_lock(file: &Path) {
    if let Some(line) = lock_line(file) {
        eprintln!("{line}");
    }
}

fn lock_line(file: &Path) -> Option<String> {
    let lock = lock_of(file);
    let modified = std::fs::metadata(&lock).and_then(|meta| meta.modified()).ok()?;
    let age = std::time::SystemTime::now()
        .duration_since(modified)
        .unwrap_or_default();
    Some(if age > LOCK_STALE {
        format!("  takes over {}, left behind by a writer that ended", lock.display())
    } else {
        format!(
            "  waits for {}, held by another writer (taken over after 5 s)",
            lock.display()
        )
    })
}

/// Refuses a file that is there but no JSON object (cut off mid-write): it
/// is kept, as it is. Read as empty, the next write replaced it and every
/// open action in it was gone without a word (Node, #306).
pub fn intact(file: &Path, here: &Here) -> Result<(), String> {
    let Ok(bytes) = std::fs::read(file) else {
        return Ok(());
    };
    let text = String::from_utf8_lossy(&bytes);
    let is_object = serde_json::from_str::<serde_json::Value>(&text).is_ok_and(|value| value.is_object());
    if text.trim().is_empty() || is_object {
        return Ok(());
    }
    let path = file.display();
    Err(format!(
        "{} is no JSON object (cut off mid-write?): its actions can't be read, so it is kept as it is.\n  Repair it, or move it away to start empty, then run this again:\n  mv {path} {path}.broken",
        here.shown(&file.to_string_lossy())
    ))
}

/// The words after `<kind>` that are no flag and no flag's value.
pub fn text_of(argv: &Argv) -> String {
    let args = argv.all().get(1..).unwrap_or_default();
    let mut words = Vec::new();
    let mut at = 0;
    while at < args.len() {
        if VALUED.contains(&args[at].as_str()) {
            at += 1;
        } else if !args[at].starts_with("--") {
            words.push(args[at].as_str());
        }
        at += 1;
    }
    words.join(" ").trim().to_string()
}

/// The options of a decide, `(label, description)` in their order: each
/// `--option`, with the `--why` right after it as what happens, or split at
/// its first `: ` without one. A `--why` before any option is dropped.
pub fn options_of(argv: &Argv) -> Vec<(String, String)> {
    let args = argv.all();
    let value_at = |at: usize, name: &str| -> Option<&str> {
        let arg = args[at].as_str();
        if arg == name {
            return args.get(at + 1).map(String::as_str);
        }
        arg.strip_prefix(name).and_then(|rest| rest.strip_prefix('='))
    };
    let mut options: Vec<(&str, Option<&str>)> = Vec::new();
    for at in 0..args.len() {
        if let Some(option) = value_at(at, "--option").filter(|o| !o.is_empty() && !o.starts_with("--")) {
            options.push((option, None));
        }
        if let Some(why) = value_at(at, "--why").filter(|w| !w.is_empty() && !w.starts_with("--"))
            && let Some(last) = options.last_mut()
        {
            last.1 = Some(why);
        }
    }
    options
        .into_iter()
        .map(|(given, why)| split_option(given, why.unwrap_or("")))
        .collect()
}

/// A number as JavaScript's `Number` reads it, None for NaN.
fn js_number(text: &str) -> Option<f64> {
    let text = text.trim();
    if text.is_empty() {
        return Some(0.0);
    }
    let is_plain = text
        .bytes()
        .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-'));
    text.parse().ok().filter(|_| is_plain)
}

/// A whole number as JavaScript's `Number` reads it.
fn js_integer(text: &str) -> Option<i64> {
    js_number(text)
        .filter(|n| n.fract() == 0.0 && n.abs() < 9e15)
        .map(|n| n as i64)
}

/// The label `--recommend` names, by its label or its number (from 1); the text as given otherwise.
fn recommend_of(argv: &Argv, options: &[(String, String)]) -> Option<String> {
    let given = argv.given("--recommend")?;
    if options.iter().any(|(label, _)| label == given) {
        return Some(given.to_string());
    }
    let by_number = js_integer(given)
        .and_then(|n| usize::try_from(n - 1).ok())
        .and_then(|at| options.get(at));
    Some(
        by_number
            .map(|(label, _)| label.clone())
            .unwrap_or_else(|| given.to_string()),
    )
}

/// Where the actions go: `(file, name)`, the orchestrator's into the record
/// of its orchestration, an agent's into its worktree (so they go with it).
fn where_of(argv: &Argv, here: &Here) -> Result<(PathBuf, Option<String>), String> {
    if let Some(name) = argv.given("--orchestration") {
        // A name outside NAME would reach outside the records (`../x`); said as wt:plan says it.
        if !is_name(name) {
            return Err(format!(
                "{name} is no orchestration's name: names are kebab-case ({run} wt:orchestrate <name> <issue>…)",
                run = breadboard_core::config::run_word()
            ));
        }
        let Some(main) = here.main.as_deref().filter(|main| record_of(main, name).is_some()) else {
            return Err(format!("no orchestration {name} is recorded"));
        };
        return Ok((record_file(main, name), Some(name.to_string())));
    }
    let Some(dir) = here.worktree() else {
        return Err("run it inside a worktree, or name the orchestration: --orchestration <name>".into());
    };
    Ok((actions_file(dir), origin_of(dir).map(|origin| origin.name)))
}

/// A row of `--list`, its id right-aligned to `width` digits: rows of ids
/// 3 and 155 start their kinds in one column (Node's shifted, spec §11).
fn row(action: &Action, width: usize) -> String {
    let mut row = format!("  {:>width$}  {:<13} {}", action.id, action.kind, action.text);
    let set = |value: &Option<String>| value.as_deref().filter(|v| !v.is_empty()).map(str::to_string);
    if let Some(url) = set(&action.url) {
        row += &format!("  {url}");
    }
    if let Some(on) = set(&action.on) {
        row += &format!("  on {on}");
    }
    if let Some(run) = set(&action.run) {
        row += &format!("  $ {run}");
    }
    if !action.options.is_empty() {
        let options: Vec<String> = action
            .options
            .iter()
            .map(|option| {
                if Some(option) == action.recommend.as_ref() {
                    format!("{option} (Recommended)")
                } else {
                    option.clone()
                }
            })
            .collect();
        row += &format!("  [{}]", options.join(" | "));
    }
    if let Some(answer) = set(&action.answer) {
        row += &format!("  → {answer}");
    }
    row
}

fn written(result: std::io::Result<impl Sized>, file: &std::path::Path) -> Result<(), String> {
    result
        .map(|_| ())
        .map_err(|error| format!("{} could not be written: {error}", file.display()))
}

/// The text of `wt:action change`; without one a line that says only what
/// is missing, not the whole usage (the critic, by lazygit's bar).
fn change_text(argv: &Argv) -> Result<String, String> {
    let text = text_of(argv);
    if text.trim().is_empty() {
        return Err("what is the change? pnpm wt:action change <what>".into());
    }
    Ok(text)
}

/// `wt:action note <one line>` (#341): what the agent is on, in plain words,
/// in its block in WORK; no action, nothing for the human to click.
fn note(argv: &Argv, here: &Here) -> Outcome {
    let Some(dir) = here.worktree() else {
        return Err("a note is for a worktree's agent: run it inside its worktree".into());
    };
    let text = text_of(argv);
    let Some(line) = text.lines().map(str::trim).find(|line| !line.is_empty()) else {
        return Err("what are you on? pnpm wt:action note \"<one line>\"".into());
    };
    set_note(dir, line, now_ms())
        .map_err(|error| format!("{} could not be written: {error}", note_file(dir).display()))?;
    let shown = match origin_of(dir) {
        Some(origin) => format!("in its block in WORK of ⇶ {}", origin.name),
        None => "noted (no orchestration shows it)".into(),
    };
    say(&format!("\n✓ {shown}: {line}\n"));
    Ok(ExitCode::SUCCESS)
}

pub fn run(argv: &Argv) -> Outcome {
    // Refused before anything is written: Node dropped a flag it didn't know and posted.
    if argv.check(&TAKES)? {
        return help(ABOUT, USAGE);
    }
    let given = argv.first();
    let kind = match given {
        Some("browser-check") => Some("try"),
        other => other,
    };
    let here = Here::find();
    if kind == Some("change") {
        return change::run(argv, &here, &change_text(argv)?);
    }
    if kind == Some("note") {
        return note(argv, &here);
    }
    let (file, name) = where_of(argv, &here)?;
    intact(&file, &here)?;
    let orchestration = argv.given("--orchestration");

    if argv.has("--list") {
        let open = actions_of(&file);
        let answered = answered_of(&file);
        // The newest first: what was just decided is what gets relayed late.
        let recent: Vec<&Action> = answered.iter().rev().take(10).collect();
        let width = open
            .iter()
            .chain(recent.iter().copied())
            .map(|action| action.id.to_string().len())
            .max()
            .unwrap_or(1);
        let mut text = if open.is_empty() {
            "  no open actions".to_string()
        } else {
            open.iter()
                .map(|action| row(action, width))
                .collect::<Vec<_>>()
                .join("\n")
        };
        if !recent.is_empty() {
            let rows: Vec<String> = recent
                .iter()
                .map(|action| match &action.answered_at {
                    Some(at) => format!("{}  ({at})", row(action, width)),
                    None => row(action, width),
                })
                .collect();
            text += &format!("\n\n  answered, delivered:\n{}", rows.join("\n"));
        }
        say(&text);
        return Ok(ExitCode::SUCCESS);
    }
    if argv.has("--answers") {
        announce_lock(&file);
        let answers =
            take_answers(&file, None).map_err(|error| format!("{} could not be read: {error}", file.display()))?;
        say(&if answers.is_empty() {
            "  no answers waiting".to_string()
        } else {
            answers.iter().map(answer_line).collect::<Vec<_>>().join("\n")
        });
        return Ok(ExitCode::SUCCESS);
    }
    // `--done=3` too: Node took only `--done 3` here, the `=` form fell through to the usage.
    if argv.has("--done") || argv.all().iter().any(|arg| arg.starts_with("--done=")) {
        let Some(id) = argv.value("--done").and_then(js_integer) else {
            return Err(USAGE.into());
        };
        announce_lock(&file);
        let result = done_action(&file, id);
        let is_done = *result.as_ref().unwrap_or(&false);
        written(result, &file)?;
        if !is_done {
            return Err(format!(
                "no open action {id} ({run} wt:action --list)",
                run = breadboard_core::config::run_word()
            ));
        }
        say(&format!("\n✓ action {id} done\n"));
        return Ok(ExitCode::SUCCESS);
    }

    let Some(kind) = kind.filter(|kind| ACTION_KINDS.contains(kind)) else {
        return Err(match kind {
            // A word that is no kind says which it likely is, the usage's 11 lines would bury it.
            Some(word) => {
                let kinds: Vec<&str> = ACTION_KINDS.iter().copied().chain(["change", "note"]).collect();
                let meant = nearest(word, &kinds)
                    .map(|meant| format!(": did you mean {meant}?"))
                    .unwrap_or_default();
                format!(
                    "{word} is no kind of action{meant} (merge, try, decide, look, plan, change, note)\n{}",
                    TAKES.after_error()
                )
            }
            None => USAGE.into(),
        });
    };
    let text = text_of(argv);
    if text.is_empty() {
        return Err(USAGE.into());
    }
    let url = argv.given("--url");
    let run = argv.given("--run");
    if given == Some("browser-check") && url.is_none() {
        return Err("a browser-check needs the page: --url <url>".into());
    }
    if kind == "try" && url.is_some() == run.is_some() {
        return Err("a try opens one spot: --run <command> (a terminal in the worktree) or --url <path or url> (the worktree's app)".into());
    }
    if run.is_some() && (kind != "try" || orchestration.is_some()) {
        return Err("only an agent's try runs a command: --run <command>".into());
    }
    let on = argv.given("--on");
    if on.is_some_and(|on| on != "test") {
        return Err("a try is on the worktree's app (the default) or on the test instance: --on test".into());
    }
    if on.is_some() && (kind != "try" || orchestration.is_some()) {
        return Err("only an agent's try is on the test instance: its MR says when it is deployed".into());
    }
    if on.is_some() && !url.is_some_and(|url| url.starts_with('/')) {
        return Err("a try on the test instance opens a path there: --url /atlas/…, never its hostname".into());
    }
    if kind == "merge" && orchestration.is_none() {
        let dir = here.dir();
        let branch = here.branch();
        let state = push_state(dir, Some(branch.as_str()).filter(|b| !b.is_empty()));
        if state.unpushed > 0 {
            return Err(format!(
                "{} not pushed{}: push first, the merge takes only what GitLab has",
                super::common::plural(state.unpushed as usize, "commit"),
                if state.has_remote_branch {
                    ""
                } else {
                    " (no remote branch)"
                }
            ));
        }
    }
    // The orchestrator's merge names its MR: the overview merges that one.
    let mr = argv
        .value("--mr")
        .and_then(|mr| js_integer(mr.strip_prefix('!').unwrap_or(mr)))
        .filter(|&mr| mr != 0);
    if kind == "merge" && orchestration.is_some() && mr.is_none() {
        return Err("which MR does it merge? --mr !N".into());
    }
    if mr.is_some() && kind != "merge" {
        return Err("only a merge or a change names its MR: --mr !N".into());
    }
    let options = options_of(argv);
    let recommend = recommend_of(argv, &options);
    if kind == "decide" {
        if let Some(problem) = decide_problem(&text, &options, recommend.as_deref()) {
            return Err(problem);
        }
    } else if !options.is_empty() || recommend.is_some() || argv.given("--why").is_some() {
        return Err("only a decide has options: --option <label> --why <what happens> --recommend <label>".into());
    }
    let new = NewAction {
        kind: kind.to_string(),
        text: text.clone(),
        url: url.map(String::from),
        run: run.map(String::from),
        on: on.map(String::from),
        options: options.iter().map(|(label, _)| label.clone()).collect(),
        descriptions: options.iter().map(|(_, description)| description.clone()).collect(),
        recommend,
        pane: std::env::var("HERDR_PANE_ID").ok(),
        mr,
        // Only the daemon posts a `permission` action, from a run's denial (#360).
        rule: None,
    };
    announce_lock(&file);
    let result = add_action(&file, new);
    let action = result.as_ref().ok().cloned().unwrap_or_default();
    written(result, &file)?;
    say(&posted(&action, name.as_deref(), orchestration.is_some()));
    Ok(ExitCode::SUCCESS)
}

/// What a posted action prints: where it is, what a click does, how its answer comes.
fn posted(action: &Action, name: Option<&str>, is_orchestrator: bool) -> String {
    let url = action.url.as_deref().unwrap_or("");
    let spot = if let Some(run) = &action.run {
        format!("  the click opens a terminal in this worktree: {run}\n")
    } else if action.on.is_some() {
        format!(
            "  the click opens {url} on the test instance (VITA_TEST_ORIGIN); the row waits for the merge and the deployment\n"
        )
    } else if action.kind == "try" {
        let local = if url.starts_with('/') {
            " on this worktree's app (its stack started first when it is down)"
        } else {
            ""
        };
        format!("  the click opens {url}{local}\n")
    } else {
        String::new()
    };
    let place = match name {
        Some(name) => format!(
            "in ACTIONS of ⇶ {name}{}",
            if action.kind == "decide" && !is_orchestrator {
                "; send the orchestrator the options too (SendMessage)"
            } else {
                ""
            }
        ),
        None => "this worktree belongs to no orchestration: no overview shows it, tell the user".into(),
    };
    let answer = if answers_of(action).is_empty() {
        String::new()
    } else {
        format!(
            "  the answer comes typed into this pane: [overview] #{} {} \"…\" → <answer>\n",
            action.id, action.kind
        )
    };
    let by_hand = match (is_orchestrator, name) {
        (true, Some(name)) => format!(" --orchestration {name}"),
        _ => String::new(),
    };
    format!(
        "\n✓ action {} ({}): {}\n{spot}  {place}\n{answer}  done by hand: {run} wt:action --done {}{by_hand}\n",
        action.id,
        action.kind,
        action.text,
        action.id,
        run = breadboard_core::config::run_word()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| arg.to_string()).collect()
    }

    #[test]
    fn a_change_without_text_says_only_that() {
        let empty = change_text(&Argv(&args(&["change", " ", "--mr", "!3"])));
        assert_eq!(
            empty,
            Err("what is the change? pnpm wt:action change <what>".to_string())
        );
        let given = change_text(&Argv(&args(&["change", "rename", "it"])));
        assert_eq!(given.as_deref(), Ok("rename it"));
    }

    #[test]
    fn takes_the_text_after_the_kind_without_flags_and_their_values() {
        let list = args(&["try", "the", "--url", "/atlas", "chip", "--fast", "works", "--option=x"]);
        assert_eq!(text_of(&Argv(&list)), "the chip works");
        assert_eq!(text_of(&Argv(&args(&["look"]))), "");
    }

    #[test]
    fn reads_the_options_with_their_why_or_split_at_the_colon() {
        let list = args(&[
            "decide",
            "A or B?",
            "--why",
            "dropped",
            "--option",
            "Go",
            "--why",
            "phases 2-4 build the painter",
            "--option=No-go: pixi stays",
            "--option",
            "--recommend",
            "1",
        ]);
        let options = options_of(&Argv(&list));
        assert_eq!(
            options,
            vec![
                ("Go".to_string(), "phases 2-4 build the painter".to_string()),
                ("No-go".to_string(), "pixi stays".to_string())
            ]
        );
        assert_eq!(recommend_of(&Argv(&list), &options).as_deref(), Some("Go"));
        let by_label = args(&["--recommend", "No-go"]);
        assert_eq!(recommend_of(&Argv(&by_label), &options).as_deref(), Some("No-go"));
        let unknown = args(&["--recommend", "7"]);
        assert_eq!(recommend_of(&Argv(&unknown), &options).as_deref(), Some("7"));
    }

    #[test]
    fn reads_numbers_as_javascript_does() {
        assert_eq!(js_integer("3"), Some(3));
        assert_eq!(js_integer(""), Some(0));
        assert_eq!(js_integer("x"), None);
        assert_eq!(js_integer("1.5"), None);
        assert_eq!(js_integer("inf"), None);
    }

    #[test]
    fn lists_a_row_with_everything_it_has() {
        let action = Action {
            id: 10,
            kind: "decide".into(),
            text: "A or B?".into(),
            options: vec!["A".into(), "B".into()],
            recommend: Some("B".into()),
            answer: Some("B".into()),
            ..Default::default()
        };
        assert_eq!(
            row(&action, 2),
            "  10  decide        A or B?  [A | B (Recommended)]  → B"
        );
        let try_ = Action {
            id: 2,
            kind: "try".into(),
            text: "t".into(),
            run: Some("cargo run".into()),
            ..Default::default()
        };
        assert_eq!(row(&try_, 1), "  2  try           t  $ cargo run");
        // Next to a two-digit id, its kind starts in the same column.
        assert_eq!(row(&try_, 2), "   2  try           t  $ cargo run");
    }

    #[test]
    fn says_where_the_action_went() {
        let look = Action {
            id: 4,
            kind: "look".into(),
            text: "read this".into(),
            ..Default::default()
        };
        assert_eq!(
            posted(&look, Some("breadboard"), false),
            "\n✓ action 4 (look): read this\n  in ACTIONS of ⇶ breadboard\n  the answer comes typed into this pane: [overview] #4 look \"…\" → <answer>\n  done by hand: pnpm wt:action --done 4\n"
        );
        let merge = Action {
            id: 1,
            kind: "merge".into(),
            text: "it".into(),
            ..Default::default()
        };
        assert_eq!(
            posted(&merge, Some("breadboard"), true),
            "\n✓ action 1 (merge): it\n  in ACTIONS of ⇶ breadboard\n  done by hand: pnpm wt:action --done 1 --orchestration breadboard\n"
        );
        let try_ = Action {
            id: 2,
            kind: "try".into(),
            text: "t".into(),
            url: Some("/atlas".into()),
            ..Default::default()
        };
        assert!(posted(&try_, None, false).contains(
            "  the click opens /atlas on this worktree's app (its stack started first when it is down)\n  this worktree belongs to no orchestration"
        ));
    }

    #[test]
    fn keeps_a_broken_actions_file_and_says_what_to_do() {
        let scratch = crate::cli::write::common::scratch::Scratch::new();
        let file = scratch.path().join("actions.json");
        let here = Here {
            cwd: scratch.path().to_path_buf(),
            top: Some(scratch.path().to_path_buf()),
            main: None,
        };
        let intact = |file: &Path| intact(file, &here);
        assert_eq!(intact(&file), Ok(()));
        std::fs::write(&file, "").unwrap();
        assert_eq!(intact(&file), Ok(()));
        std::fs::write(&file, "{\"actions\":[]}").unwrap();
        assert_eq!(intact(&file), Ok(()));
        std::fs::write(&file, "{\"actions\":[{\"id\":1,").unwrap();
        let refused = intact(&file).unwrap_err();
        assert!(
            refused.starts_with("actions.json is no JSON object (cut off mid-write?)"),
            "{refused}"
        );
        assert!(refused.ends_with("actions.json.broken"), "{refused}");
        std::fs::write(&file, "[]").unwrap();
        assert!(intact(&file).is_err());
    }

    #[test]
    fn tells_a_lock_held_from_one_left_behind() {
        let scratch = crate::cli::write::common::scratch::Scratch::new();
        let file = scratch.path().join("actions.json");
        assert_eq!(lock_line(&file), None);
        let lock = lock_of(&file);
        std::fs::create_dir(&lock).unwrap();
        assert!(lock_line(&file).unwrap().contains("held by another writer"));
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(60);
        std::fs::File::open(&lock).unwrap().set_modified(old).unwrap();
        assert!(
            lock_line(&file)
                .unwrap()
                .ends_with("left behind by a writer that ended")
        );
    }
}
