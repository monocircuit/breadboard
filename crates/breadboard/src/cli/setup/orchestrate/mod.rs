//! `pnpm wt:orchestrate` (commands/orchestrate.mjs, handover.mjs): one
//! session orchestrates several issues, each in its own worktree with a
//! briefed claude; `--to` hands one to an idle agent (`--after`: once MRs are
//! merged), `--title`, `--end`, `--changelog [--watch]`, `--changes`.

mod changes;
mod end;
mod issue;
pub mod setups;
mod space;
mod workers;

use crate::cli::setup::herdr;
use crate::cli::setup::proc::{self, Args, fail, quote, step};
use crate::cli::setup::repo;
use breadboard_core::json::str_of;
use breadboard_core::record::{self, ORCHESTRATOR, is_name};
use breadboard_core::time::{now_ms, to_iso};
use issue::{
    BRIEF_PROMPT, Brief, Item, brief_of, known_entry, make_worktree, number_of, object, read_issue, record_entry,
};
use serde_json::{Value, json};
use std::fs;
use std::process::{Command, ExitCode, Stdio};
use workers::{Start, WorkerInfo};

const USAGE: &str = "usage: pnpm wt:orchestrate <name> <issue|n-feature>… [--title <text>] [--shared] [--peer <session>] [--brief <file>] [--queue [--after !<mr>…]] [--dry-run]
       pnpm wt:orchestrate <name> --title <text>
       pnpm wt:orchestrate <name> <issue> --to <agent> [--after !<mr>…] [--limit <percent>] [--dry-run]
       pnpm wt:orchestrate <name> --end
       pnpm wt:orchestrate <name> --changelog [--watch]
       pnpm wt:orchestrate <name> --changes";

/// The values of `--after` and where they stand (handovers.mjs `afterArgs`).
pub fn after_args(argv: &[String]) -> Option<(Vec<String>, Vec<usize>)> {
    let at = argv
        .iter()
        .position(|arg| arg == "--after" || arg.starts_with("--after="))?;
    if let Some(inline) = argv[at].strip_prefix("--after=") {
        return Some((vec![inline.to_string()], vec![at]));
    }
    let indexes: Vec<usize> = (at + 1..argv.len()).take_while(|i| argv[*i].starts_with('!')).collect();
    let mut all = vec![at];
    all.extend(&indexes);
    Some((indexes.iter().map(|i| argv[*i].clone()).collect(), all))
}

/// The MRs after `--after`, each once, in order; or what is wrong.
pub fn parse_after(values: &[String]) -> Result<Vec<i64>, String> {
    let mut after = Vec::new();
    for value in values.iter().flat_map(|arg| arg.split(',')) {
        let trimmed = value.trim();
        let digits = trimmed.strip_prefix('!').unwrap_or(trimmed);
        if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
            return Err(format!("{value} is no MR (!232)"));
        }
        let iid: i64 = digits.parse().map_err(|_| format!("{value} is no MR (!232)"))?;
        if !after.contains(&iid) {
            after.push(iid);
        }
    }
    if after.is_empty() {
        Err("--after names the MRs to wait for: --after !232".into())
    } else {
        Ok(after)
    }
}

fn is_issue_arg(arg: &str) -> bool {
    (!arg.is_empty() && arg.chars().all(|c| c.is_ascii_digit()))
        || (is_name(arg) && !number_of(arg).is_empty() && arg[number_of(arg).len()..].starts_with('-'))
}

fn read_or_fail(arg: &str) -> Item {
    read_issue(arg).unwrap_or_else(|| fail(&format!("issue #{} could not be read", number_of(arg))))
}

/// The brief text an issue of this run gets: only a `--brief` of this very
/// run (#247), else the one an earlier run gave that issue.
fn extra_for(name: &str, number: i64, given: Option<&str>) -> String {
    if let Some(given) = given {
        return given.to_string();
    }
    breadboard_core::json::read(&record::record_file(&repo::main_dir(), name))
        .and_then(|raw| raw.get("briefs")?.get(number.to_string())?.as_str().map(String::from))
        .unwrap_or_default()
}

/// The brief of `--brief` kept per issue (#247): a queued issue started later gets it, no other.
fn keep_briefs(name: &str, numbers: &[i64], text: &str) {
    let main = repo::main_dir();
    let raw = breadboard_core::json::read(&record::record_file(&main, name)).unwrap_or(json!({}));
    let mut briefs = raw
        .get("briefs")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for number in numbers {
        briefs.insert(number.to_string(), json!(text));
    }
    let _ = record::update_record(&main, name, object(json!({ "briefs": briefs })));
}

pub fn command(args: &Args) -> ExitCode {
    let name = args.first().map(String::from);
    let is_valid = name.as_deref().is_some_and(is_name);
    if args.has("--end") {
        if !is_valid {
            fail(USAGE);
        }
        return end::command(name.as_deref().unwrap_or_default());
    }
    if args.has("--changelog") {
        if !is_valid {
            fail(USAGE);
        }
        return crate::cli::setup::changelog::orchestration_command(
            name.as_deref().unwrap_or_default(),
            args.has("--watch"),
        );
    }
    if args.has("--changes") {
        if !is_valid {
            fail(USAGE);
        }
        let name = name.unwrap_or_default();
        let Some(set) = changes::read_change_set(&name) else {
            fail(&format!("no orchestration {name}"))
        };
        println!("{}", serde_json::to_string(&set).unwrap_or_default());
        return ExitCode::SUCCESS;
    }
    let argv = &args.all;
    let after = after_args(argv);
    let valued = ["--peer", "--brief", "--to", "--limit", "--title"];
    let positional: Vec<&String> = argv
        .iter()
        .enumerate()
        .filter(|(i, arg)| {
            !arg.starts_with("--")
                && !after.as_ref().is_some_and(|(_, at)| at.contains(i))
                && !(*i > 0 && valued.contains(&argv[i - 1].as_str()))
        })
        .map(|(_, arg)| arg)
        .collect();
    let wanted: Vec<String> = positional.iter().skip(1).map(|arg| arg.to_string()).collect();
    let title = args.value("--title").map(|title| title.trim().to_string());
    if args.has("--title") && title.as_deref().is_none_or(str::is_empty) {
        fail(USAGE);
    }
    let is_dry = args.has("--dry-run");
    if let Some(title) = &title
        && wanted.is_empty()
        && is_valid
    {
        return retitle(name.as_deref().unwrap_or_default(), title, is_dry);
    }
    let Some(name) = name.filter(|_| is_valid && !wanted.is_empty()) else {
        fail(USAGE)
    };
    if wanted.iter().any(|arg| !is_issue_arg(arg)) {
        fail(USAGE);
    }
    let brief_file = args.value("--brief");
    let given = brief_file
        .as_ref()
        .map(|file| fs::read_to_string(file).unwrap_or_else(|_| fail(&format!("{file} does not exist"))));
    if args.has("--cap") {
        fail(
            "--cap is gone (#229): every issue starts at once, the machine's memory is the only limit; --queue queues them",
        );
    }
    let main = repo::main_dir();
    let found = record::record_of(&main, &name);
    let peer = args
        .value("--peer")
        .or_else(|| found.as_ref().and_then(|found| found.peer.clone()));
    let parsed_after = after
        .as_ref()
        .map(|(values, _)| parse_after(values).unwrap_or_else(|error| fail(&error)));
    if after.is_some() && !args.has("--to") && !args.has("--queue") {
        fail("--after needs --queue (the issues wait in the queue for the MRs) or --to (a handover announced)");
    }
    if args.has("--to") {
        if wanted.len() != 1 {
            fail("--to hands over one issue at a time");
        }
        if let Some(after) = parsed_after {
            return announce(&name, &wanted[0], args, &after, is_dry);
        }
        return hand_over(&name, &wanted[0], args, peer.as_deref(), given.as_deref(), is_dry);
    }

    let mut plan: Vec<Item> = wanted.iter().map(|arg| read_or_fail(arg)).collect();
    let slot = herdr::slot_of(&name);
    let label = match &title {
        Some(title) => format!("{ORCHESTRATOR} {title}"),
        None => record::orchestrator_label(&main, &name),
    };
    let (colour, _) = herdr::SLOT_COLOURS[slot as usize % herdr::SLOT_COLOURS.len()];
    println!(
        "\n{label}  colour slot {} ({colour}, sidebar token ${})",
        slot + 1,
        herdr::origin_token(slot)
    );
    let width = plan.iter().map(|item| item.branch.chars().count()).max().unwrap_or(0);
    for item in &plan {
        let what = item
            .why
            .as_ref()
            .map(|why| format!("skipped: {why}"))
            .unwrap_or_else(|| item.title.clone());
        println!("  #{:<4} {:<width$}  {what}", item.number, item.branch);
    }
    if plan.iter().all(|item| item.why.is_some()) {
        fail("nothing to start: every issue is closed, claimed or has its worktree already");
    }
    let is_queued = args.has("--queue");
    let is_now = args.has("--now");
    let to_start: Vec<usize> = (0..plan.len()).filter(|at| plan[*at].why.is_none()).collect();
    let idle = if is_queued || is_now || !herdr::is_in_herdr() {
        Vec::new()
    } else {
        workers::idle_workers_now(&name, found.as_ref())
    };
    let items: Vec<(String, String)> = to_start
        .iter()
        .map(|at| (plan[*at].number.clone(), plan[*at].description.clone()))
        .collect();
    let starts = workers::starts_of(&items, &idle, args.has("--shared"), workers::free_memory());
    if !is_queued {
        for (at, start) in to_start.iter().zip(&starts) {
            let item = &plan[*at];
            let line = match start {
                Start::Worker { worker, shared } => {
                    let shares = if shared.is_empty() {
                        String::new()
                    } else {
                        format!(", shares {}", shared.join(", "))
                    };
                    format!(
                        "→ {} (idle{shares}, {} %)",
                        worker.name,
                        (worker.share * 100.0).round() as i64
                    )
                }
                Start::Fresh { why: Some(why), .. } => format!("→ a fresh agent: {why}"),
                Start::Fresh { is_shared: true, .. } => "→ a fresh agent on the shared stack".into(),
                Start::Fresh { .. } => "→ a fresh agent".into(),
            };
            println!("  #{:<4} {line}", item.number);
        }
    }
    if is_dry {
        let first = plan.iter().find(|item| item.why.is_none()).unwrap_or(&plan[0]);
        println!(
            "\n▸ nothing was done (--dry-run). The brief of {} (.claude/brief.md), started with\n  claude --session-id <new id> --name {} {}\n",
            first.feature,
            first.feature,
            quote(BRIEF_PROMPT)
        );
        let extra = extra_for(&name, first.number(), given.as_deref());
        println!(
            "{}",
            brief_of(&Brief {
                item: first,
                name: &name,
                peer: peer.as_deref(),
                extra: &extra,
                before: None
            })
        );
        // Which brief each issue gets (#247): only this run's --brief.
        for at in &to_start {
            let item = &plan[*at];
            let source = if given.is_some() {
                format!("the brief of --brief {}", brief_file.as_deref().unwrap_or_default())
            } else if !extra_for(&name, item.number(), None).is_empty() {
                "the brief an earlier run gave it".into()
            } else {
                "no brief of the orchestrator".into()
            };
            println!("  #{} gets {source}", item.number);
        }
        return ExitCode::SUCCESS;
    }

    if let Some(title) = &title {
        let _ = record::update_record(&main, &name, object(json!({ "title": title })));
    }
    if let Some(opened) = space::open_orchestrator(&name) {
        step(&format!("herdr: {opened}"));
    }
    if title.is_some() && herdr::is_in_herdr() {
        herdr::mark_origins();
    }
    if let Some(peer) = args.value("--peer") {
        let _ = record::update_record(&main, &name, object(json!({ "peer": peer })));
    }
    if let Some(given) = &given {
        let numbers: Vec<i64> = to_start.iter().map(|at| plan[*at].number()).collect();
        keep_briefs(&name, &numbers, given);
    }
    let queued_at = to_iso(now_ms());
    let known: Vec<(i64, Value)> = plan
        .iter()
        .filter_map(|item| known_entry(&name, item.number()).map(|entry| (item.number(), entry)))
        .collect();
    let known_of = |number: i64| {
        known
            .iter()
            .find(|(known, _)| *known == number)
            .map(|(_, entry)| entry.clone())
    };
    let waits_for = parsed_after.clone().unwrap_or_default();
    let entries: Vec<Value> = plan
        .iter()
        .map(|item| {
            if is_queued && item.why.is_none() {
                let mut entry = record_entry(item, None);
                let fields = entry.as_object_mut().expect("an entry is an object");
                fields.insert("queued".into(), json!(true));
                fields.insert("queuedAt".into(), json!(queued_at));
                if !waits_for.is_empty() {
                    fields.insert("after".into(), json!(waits_for));
                }
                entry
            } else if item.why.is_some() {
                record_entry(item, item.why.as_deref())
            } else {
                issue::start_entry(item, known_of(item.number()).as_ref(), None)
            }
        })
        .collect();
    let _ = record::write_record(&main, &name, &entries);
    if is_queued {
        let numbers: Vec<String> = to_start.iter().map(|at| format!("#{}", plan[*at].number)).collect();
        let mrs: Vec<String> = waits_for.iter().map(|iid| format!("!{iid}")).collect();
        step(&format!(
            "queued {} (--queue{}). The overview of {label} hands each to a free agent{}, in the work order of wt:plan, else starts a fresh one (UP NEXT); by hand: pnpm wt:orchestrate {name} <issue> --to <agent>",
            numbers.join(", "),
            if mrs.is_empty() {
                String::new()
            } else {
                format!(", after {}", mrs.join(", "))
            },
            if mrs.is_empty() {
                ""
            } else {
                " once those MRs are merged"
            }
        ));
    }

    let mut results: Vec<String> = Vec::new();
    let mut started: Vec<String> = Vec::new();
    if !is_queued {
        for (at, start) in to_start.iter().zip(&starts) {
            let item = &mut plan[*at];
            let number = item.number();
            match start {
                Start::Worker { worker, shared } => {
                    let mut child = vec![
                        "wt:orchestrate".to_string(),
                        name.clone(),
                        item.number.clone(),
                        "--to".into(),
                        worker.name.clone(),
                    ];
                    if args.has("--shared") {
                        child.push("--shared".into());
                    }
                    step(&format!("pnpm {}", child.join(" ")));
                    let handed = Command::new(proc::entry()).args(&child).stdin(Stdio::null()).status();
                    if handed.is_ok_and(|status| status.success()) {
                        let how = if shared.is_empty() {
                            ", with the most room"
                        } else {
                            " and knowing its files"
                        };
                        results.push(format!(
                            "✓ #{} {}: went to {}, idle{how}",
                            item.number, item.feature, worker.name
                        ));
                        started.push(worker.name.clone());
                    } else {
                        results.push(format!(
                            "✗ #{} {}: the handover to {} failed (see above); a fresh agent: pnpm wt:orchestrate {name} {} --now",
                            item.number, item.feature, worker.name, item.number
                        ));
                    }
                }
                Start::Fresh { is_shared, why } => {
                    if let Some(why) = why {
                        step(&format!("memory: {why}"));
                    }
                    let dir = item.dir.to_string_lossy().into_owned();
                    let claim = match setups::claim_setup(&name, number, &dir, None, None) {
                        Ok(claim) => claim,
                        Err(holder) => {
                            results.push(format!(
                                "✗ #{} {}: set up already, {}",
                                item.number,
                                item.feature,
                                setups::setup_text(&holder)
                            ));
                            continue;
                        }
                    };
                    let before = known_of(number);
                    step(&format!(
                        "pnpm wt:new {} --origin {name}{}",
                        item.feature,
                        if *is_shared { " --shared" } else { "" }
                    ));
                    let made = make_worktree(item, &name, *is_shared);
                    if !made.is_there || made.failed.is_some() {
                        let failed = made.failed.clone().unwrap_or_else(|| "no worktree".into());
                        results.push(format!("✗ #{} {}: {failed}", item.number, item.feature));
                        if !made.is_there || is_now {
                            issue::undo_start(
                                &name,
                                item,
                                before.as_ref(),
                                &format!("its worktree failed: {failed}"),
                                Some(&claim),
                            );
                        }
                        claim.release();
                        continue;
                    }
                    let extra = extra_for(&name, number, given.as_deref());
                    let brief = brief_of(&Brief {
                        item,
                        name: &name,
                        peer: peer.as_deref(),
                        extra: &extra,
                        before: None,
                    });
                    let agent = issue::start_agent(&item.dir, &brief);
                    if !agent.is_started && is_now {
                        issue::undo_start(
                            &name,
                            item,
                            before.as_ref(),
                            "its claude did not start (no idle pane in its space)",
                            Some(&claim),
                        );
                        results.push(format!(
                            "✗ #{} {}: its claude did not start, the worktree is removed again",
                            item.number, item.feature
                        ));
                        continue;
                    }
                    let _ = record::write_record(&main, &name, &[record_entry(item, None)]);
                    claim.release();
                    if agent.is_started {
                        workers::record_start(
                            &name,
                            &agent.id,
                            number,
                            "fresh",
                            Some(0.0),
                            object(json!({ "name": item.feature })),
                        );
                    }
                    let note = if made.status == Some(0) {
                        ""
                    } else {
                        " (wt:new failed after the setup, see above)"
                    };
                    results.push(if agent.is_started {
                        format!(
                            "✓ #{} {}: claude started in its tab, named {}{note}",
                            item.number, item.feature, item.feature
                        )
                    } else {
                        format!(
                            "✓ #{} {}: worktree ready{note}, start its agent:\n      {}",
                            item.number, item.feature, agent.command
                        )
                    });
                    started.push(item.feature.clone());
                }
            }
        }
    }
    println!("\n{label}");
    for line in &results {
        println!("  {line}");
    }
    if !started.is_empty() {
        let hello: Vec<String> = started
            .iter()
            .map(|agent| format!("    SendMessage(to: \"{agent}\", message: \"…\")"))
            .collect();
        let peer_note = peer.as_ref().map(|peer| format!(" ({peer})")).unwrap_or_default();
        println!(
            "
  A folder claude has not seen asks to be trusted first: answer it in the agent's tab.
  The overview's WORKTREES shows where each agent is: look there, don't ask and don't relay it.
  An agent messages you{peer_note} only for a decision, a changed plan or another agent's work.
  Say hello first, so it has your address:
{}

  The overview of {label} answers the user's \"what now?\": keep it true.
    pnpm wt:plan {name} --work <issue>… --merge <issue>… --why \"<one line>\"
                                  the order now, again whenever it changes (a dependency, a failed pipeline)
    pnpm wt:action decide \"<what is decided>? <why the recommendation>\" --option \"<label>\" --why \"<what happens>\"… --recommend <label> --orchestration {name}
                                  a step that is the user's and no agent's (--done <id> --orchestration {name} after)
  The agents leave their own actions (merge, try, decide, look, plan) whenever they wait for the user.
",
            hello.join("\n")
        );
    }
    ExitCode::SUCCESS
}

/// `wt:orchestrate <name> --title "<text>"`: the title into the record, its space and marks renamed.
fn retitle(name: &str, title: &str, is_dry: bool) -> ExitCode {
    let main = repo::main_dir();
    let before = record::orchestrator_label(&main, name);
    if is_dry {
        println!("\n▸ nothing was done (--dry-run): {before} → {ORCHESTRATOR} {title}\n");
        return ExitCode::SUCCESS;
    }
    if let Err(error) = record::update_record(&main, name, object(json!({ "title": title }))) {
        fail(&format!(
            "couldn't write {}: {error}",
            record::record_file(&main, name).display()
        ));
    }
    let has_space = herdr::is_in_herdr()
        && herdr::spaces()
            .iter()
            .any(|space| herdr::orchestration_of(herdr::field(space, "label")).as_deref() == Some(name));
    if has_space {
        space::orchestrator_space(name);
        herdr::mark_origins();
    }
    println!(
        "\n✓ {before} → {}{}; commands keep the name: pnpm wt:orchestrate {name} …\n",
        record::orchestrator_label(&main, name),
        if has_space {
            ", its space and sidebar marks too"
        } else {
            ""
        }
    );
    ExitCode::SUCCESS
}

fn no_agent(name: &str, who: &str, workers: &[WorkerInfo]) -> ! {
    let label = record::orchestrator_label(&repo::main_dir(), name);
    let names: Vec<&str> = workers.iter().map(|worker| worker.name.as_str()).collect();
    fail(&format!(
        "{label} has no agent {who}; its agents: {}",
        if names.is_empty() {
            "none".into()
        } else {
            names.join(", ")
        }
    ))
}

/// `--to <agent> [--limit <percent>]`: the issue to an idle agent of it (handover.mjs).
fn hand_over(name: &str, arg: &str, args: &Args, peer: Option<&str>, given: Option<&str>, is_dry: bool) -> ExitCode {
    if !herdr::is_in_herdr() {
        fail("--to works inside herdr only: the agents are found through it");
    }
    let who = args.value("--to");
    let limit = match args.value("--limit") {
        None => Some(workers::CONTEXT_LIMIT),
        Some(percent) => percent.trim().parse::<f64>().ok().map(|percent| percent / 100.0),
    };
    let (Some(who), Some(limit)) = (who, limit.filter(|limit| *limit > 0.0 && *limit <= 1.0)) else {
        fail("usage: pnpm wt:orchestrate <name> <issue> --to <agent> [--limit <percent>]")
    };
    let all = workers::workers_of(name);
    let Some(worker) = workers::find_worker(&all, &who).cloned() else {
        no_agent(name, &who, &all)
    };
    let share = crate::cli::setup::conversations::context_share_of(&worker.id);
    let refusal = workers::refusal_of(workers::pane_of_conversation(&worker.id).as_ref(), share, limit);
    let item = read_or_fail(arg);
    let number = item.number();
    let failed = |reason: &str| -> ! {
        if !is_dry {
            workers::record_failure(name, &worker.id, number, reason);
        }
        fail(&format!("{} gets no next issue: {reason}", worker.name))
    };
    if let Some(refusal) = refusal {
        failed(&refusal);
    }
    let dir = item.dir.to_string_lossy().into_owned();
    let from = worker.dir.to_string_lossy().into_owned();
    let claim = if is_dry {
        None
    } else {
        match setups::claim_setup(name, number, &dir, Some(&worker.name), Some(&from)) {
            Ok(claim) => Some(claim),
            Err(holder) => fail(&format!("#{number} is set up already: {}", setups::setup_text(&holder))),
        }
    };
    let known = known_entry(name, number);
    let is_made = repo::worktrees().iter().any(|tree| tree.dir == item.dir)
        && record::origin_of(&item.dir).is_some_and(|origin| origin.name == name)
        && crate::cli::setup::conversations::bound_session(&item.dir).is_none();
    if let Some(why) = item.why.as_ref().filter(|_| !is_made) {
        fail(&format!("#{}: {why}", item.number));
    }
    let extra = extra_for(name, number, given);
    let before = workers::before_line(&workers::issues_before(name, &worker));
    let brief = brief_of(&Brief {
        item: &item,
        name,
        peer,
        extra: &extra,
        before,
    });
    if is_dry {
        println!(
            "\n▸ nothing was done (--dry-run). {} would get\n  {}\n  with the brief\n",
            worker.name,
            workers::next_issue_line(&item.number, &item.dir)
        );
        println!("{brief}");
        return ExitCode::SUCCESS;
    }
    let main = repo::main_dir();
    if !is_made {
        let _ = record::write_record(&main, name, &[issue::start_entry(&item, known.as_ref(), None)]);
        step(&format!("pnpm wt:new {} --origin {name}", item.feature));
        let made = make_worktree(&item, name, args.has("--shared"));
        if !made.is_there {
            let failed = made.failed.unwrap_or_else(|| "no worktree".into());
            let why = format!("its worktree failed: {failed}");
            issue::undo_start(name, &item, known.as_ref(), &why, claim.as_ref());
            workers::record_failure(name, &worker.id, number, &why);
            fail(&format!("#{} {}: {failed}", item.number, item.feature));
        }
    }
    let pane = workers::pane_of_conversation(&worker.id);
    if let Some(now) = workers::refusal_of(pane.as_ref(), None, limit) {
        let why = format!(
            "after the setup, {} {}",
            worker.name,
            now.strip_prefix("it ").unwrap_or(&now)
        );
        issue::undo_start(name, &item, known.as_ref(), &why, claim.as_ref());
        workers::record_failure(name, &worker.id, number, &format!("after the setup, {now}"));
        let queued = if known
            .as_ref()
            .is_some_and(|known| breadboard_core::json::is_truthy(known.get("queued")))
        {
            ", the issue is back in the queue"
        } else {
            ""
        };
        fail(&format!(
            "{why}: the worktree of #{} is removed again{queued}; run the same command again once it is idle",
            item.number
        ));
    }
    let pane = pane.expect("a pane: refusal_of refuses none");
    let _ = record::write_brief(&item.dir, &brief);
    let moved = workers::move_worker(&worker, &pane, &item.number, &item.dir);
    if !moved.is_renamed {
        // #313: a rename that didn't take is no handover; its line waits until it did.
        workers::record_failure(name, &worker.id, number, "its /rename did not take");
        if let Some(claim) = &claim {
            claim.release();
        }
        step(&if moved.is_moved {
            format!(
                "herdr: the tab of {} moved into the space {}",
                worker.name, item.feature
            )
        } else {
            format!(
                "herdr: the tab of {} stayed where it is (no space {})",
                worker.name, item.feature
            )
        });
        fail(&format!(
            "#{} {}: /rename {} did not take in the tab of {}, so its next issue was not typed. Type these into its tab once it is idle:\n    {}\n    {}",
            item.number,
            item.feature,
            item.feature,
            worker.name,
            workers::rename_line(&item.dir),
            workers::next_issue_line(&item.number, &item.dir)
        ));
    }
    workers::record_handover(name, &worker, number, &item.dir);
    let _ = record::write_record(&main, name, &[record_entry(&item, None)]);
    if let Some(claim) = &claim {
        claim.release();
    }
    let is_announced =
        record::record_of(&main, name).is_some_and(|found| found.handovers.iter().any(|h| h.number == number));
    workers::record_start(
        name,
        &worker.id,
        number,
        if is_announced { "announced" } else { "handover" },
        share,
        Default::default(),
    );
    let _ = record::drop_handover(&main, name, number);
    step(&if moved.is_moved {
        format!(
            "herdr: the tab of {} moved into the space {}",
            worker.name, item.feature
        )
    } else {
        format!(
            "herdr: the tab of {} stayed where it is (no space {})",
            worker.name, item.feature
        )
    });
    let renamed = if worker.name == item.feature {
        String::new()
    } else {
        format!(", renamed from {}", worker.name)
    };
    if moved.is_told {
        println!(
            "\n✓ #{} went to the agent {}{renamed}, told in its prompt; SendMessage(to: \"{}\")",
            item.number, item.feature, item.feature
        );
    } else {
        println!(
            "\n✓ #{} {} is ready for {}, but herdr took no line: type these into its tab\n    {}\n    {}",
            item.number,
            item.feature,
            worker.name,
            workers::rename_line(&item.dir),
            workers::next_issue_line(&item.number, &item.dir)
        );
    }
    ExitCode::SUCCESS
}

/// `--to <agent> --after !<mr>…`: the handover announced, done by the overview once the MRs are merged.
fn announce(name: &str, arg: &str, args: &Args, after: &[i64], is_dry: bool) -> ExitCode {
    let main = repo::main_dir();
    let label = record::orchestrator_label(&main, name);
    let who = args.value("--to").unwrap_or_default();
    let all = workers::workers_of(name);
    let Some(worker) = workers::find_worker(&all, &who).cloned() else {
        no_agent(name, &who, &all)
    };
    let item = read_or_fail(arg);
    let number = item.number();
    let known = known_entry(name, number);
    let is_queued = known
        .as_ref()
        .is_some_and(|known| breadboard_core::json::is_truthy(known.get("queued")));
    if let Some(why) = item.why.as_ref().filter(|_| !is_queued) {
        fail(&format!("#{number}: {why}"));
    }
    let mrs: Vec<String> = after.iter().map(|iid| format!("!{iid}")).collect();
    let verb = if after.len() > 1 { "are" } else { "is" };
    if is_dry {
        println!(
            "\n▸ nothing was done (--dry-run). {} would take #{number} once {} {verb} merged.",
            worker.name,
            mrs.join(", ")
        );
        return ExitCode::SUCCESS;
    }
    let mut entry = record_entry(&item, item.why.as_deref());
    let queued_at = known
        .as_ref()
        .and_then(|known| str_of(known, "queuedAt"))
        .map(String::from)
        .unwrap_or_else(|| to_iso(now_ms()));
    if let Some(fields) = entry.as_object_mut() {
        fields.insert("queued".into(), json!(true));
        fields.insert("queuedAt".into(), json!(queued_at));
    }
    let _ = record::write_record(&main, name, &[entry]);
    let _ = record::announce_handover(&main, name, number, &worker.name, after, &to_iso(now_ms()));
    println!(
        "\n✓ #{number} {} goes to {} once {} {verb} merged: the overview of {label} hands it over then (its worktree is kept until), shown on {}'s node as → next #{number}",
        item.feature,
        worker.name,
        mrs.join(", "),
        worker.name
    );
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| arg.to_string()).collect()
    }

    #[test]
    fn reads_the_mrs_to_wait_for() {
        assert_eq!(
            after_args(&argv(&["b", "12", "--after", "!232", "!233", "13"])),
            Some((argv(&["!232", "!233"]), vec![2, 3, 4]))
        );
        assert_eq!(
            after_args(&argv(&["--after=!232,!233"])),
            Some((argv(&["!232,!233"]), vec![0]))
        );
        assert_eq!(parse_after(&argv(&["!232,233", "!232"])), Ok(vec![232, 233]));
        assert_eq!(parse_after(&argv(&["x"])), Err("x is no MR (!232)".into()));
        assert_eq!(
            parse_after(&[]),
            Err("--after names the MRs to wait for: --after !232".into())
        );
    }

    #[test]
    fn takes_a_number_or_a_numbered_feature() {
        assert!(is_issue_arg("12"));
        assert!(is_issue_arg("12-x"));
        assert!(!is_issue_arg("abc"));
        assert!(!is_issue_arg("x-12"));
    }
}
