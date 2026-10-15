//! `pnpm wt:orchestrate` (commands/orchestrate.mjs, handover.mjs): one
//! session orchestrates several issues, each in its own worktree with a
//! briefed claude; `--to` hands one to an idle agent (`--after`: once MRs are
//! merged), `--title`, `--end`, `--changelog [--watch]`, `--changes`.

mod changes;
mod end;
mod epic;
mod issue;
pub mod setups;
mod space;
mod workers;

use crate::cli::setup::herdr;
use crate::cli::setup::proc::{self, Args, fail, name_refusal, outln, quote, step};
use crate::cli::setup::repo;
use breadboard_core::record::{self, ORCHESTRATOR, is_name};
use breadboard_core::time::{now_ms, to_iso};
use issue::{
    BRIEF_PROMPT, Brief, Item, brief_of, known_entry, make_worktree, number_of, object, read_issue, record_entry,
};
use serde_json::{Value, json};
use std::fs;
use std::process::{ExitCode, Stdio};
use workers::Start;

pub const USAGE: &str = "usage: pnpm wt:orchestrate <name> <issue|n-feature>… [--title <text>] [--epic <n>] [--shared] [--peer <session>] [--brief <file>] [--queue [--after !<mr>…]] [--dry-run]
       pnpm wt:orchestrate <name> --title <text>
       pnpm wt:orchestrate <name> <issue> --to <agent> [--after !<mr>…] [--limit <percent>] [--dry-run]
       pnpm wt:orchestrate <name> --end
       pnpm wt:orchestrate <name> --changelog [--watch]
       pnpm wt:orchestrate <name> --changes";

/// `--after`'s values and where they stand, the MRs in them: the handover's
/// reading, one for `--queue --after` and `--to … --after`.
use crate::cli::lifecycle::handover::{after_args, parse_after};

fn is_issue_arg(arg: &str) -> bool {
    (!arg.is_empty() && arg.chars().all(|c| c.is_ascii_digit()))
        || (is_name(arg) && !number_of(arg).is_empty() && arg[number_of(arg).len()..].starts_with('-'))
}

fn read_or_fail(arg: &str) -> Item {
    read_issue(arg).unwrap_or_else(|why| fail(&why))
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

/// The brief of `--brief` kept per issue (#247): a queued issue started later
/// gets it, no other. `brief` is written as today too: the Node overview's
/// queue starts read it while both run side by side (rule 5).
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
    let _ = record::update_record(&main, name, object(json!({ "brief": text, "briefs": briefs })));
}

/// The flags a form that only ends, reads or watches takes besides its own:
/// `--end --dry-run` would end it for real, so any other is refused.
const FORMS: [(&str, &[&str]); 3] = [("--end", &[]), ("--changelog", &["--watch"]), ("--changes", &[])];

/// The first flag `args` gives a form of [`FORMS`] that the form doesn't take.
fn stray_flag(args: &Args) -> Option<String> {
    let (form, takes) = FORMS.iter().find(|(form, _)| args.has(form))?;
    let flag = args
        .all
        .iter()
        .take_while(|arg| *arg != "--")
        .filter(|arg| arg.starts_with("--"))
        .map(|arg| arg.split_once('=').map_or(arg.as_str(), |(flag, _)| flag))
        .find(|flag| flag != form && !takes.contains(flag))?;
    Some(format!("{form} takes no {flag}"))
}

pub fn command(args: &Args) -> ExitCode {
    let argv = &args.all;
    let after = after_args(argv);
    let valued = ["--peer", "--brief", "--to", "--limit", "--title", "--epic"];
    // The words, wherever the flags stand (`--dry-run breadboard 301`): the
    // orchestration, then its issues; the MRs after `--after` are none.
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
    let name = positional.first().map(|name| name.to_string());
    let is_valid = name.as_deref().is_some_and(is_name);
    if let Some(stray) = stray_flag(args) {
        fail(&format!("{stray}\n{USAGE}"));
    }
    if args.has("--watch") && !args.has("--changelog") {
        fail(&format!(
            "--watch goes with --changelog: {run} wt:orchestrate {} --changelog --watch\n{USAGE}",
            name.as_deref().unwrap_or("<name>"),
            run = breadboard_core::config::run_word()
        ));
    }
    if args.has("--cap") {
        fail(
            "--cap is gone (#229): every issue starts at once, the machine's memory is the only limit; --queue queues them",
        );
    }
    let refuse_name = || -> ! { fail(&name_refusal("the orchestration", name.as_deref(), USAGE)) };
    if args.has("--end") {
        if !is_valid {
            refuse_name();
        }
        crate::cli::setup::heal_plugin();
        return end::command(name.as_deref().unwrap_or_default());
    }
    if args.has("--changelog") {
        if !is_valid {
            refuse_name();
        }
        return crate::cli::setup::changelog::orchestration_command(
            name.as_deref().unwrap_or_default(),
            args.has("--watch"),
        );
    }
    if args.has("--changes") {
        if !is_valid {
            refuse_name();
        }
        let name = name.unwrap_or_default();
        let set = match changes::read_change_set(&name) {
            Some(Ok(set)) => set,
            Some(Err((branch, why))) => fail(&format!(
                "GitLab did not answer for {branch} ({why}): the change set would miss its merge requests"
            )),
            None => fail(&format!("no orchestration {name}")),
        };
        outln!("{}", serde_json::to_string(&set).unwrap_or_default());
        return ExitCode::SUCCESS;
    }
    let wanted: Vec<String> = positional.iter().skip(1).map(|arg| arg.to_string()).collect();
    let title = args.value("--title").map(|title| title.trim().to_string());
    if args.has("--title") && title.as_deref().is_none_or(str::is_empty) {
        fail(&format!("--title needs a text\n{USAGE}"));
    }
    let is_dry = args.has("--dry-run");
    if let Some(title) = &title
        && wanted.is_empty()
        && is_valid
    {
        // A dry run writes nothing, to herdr neither (the plugin's link).
        if !is_dry {
            crate::cli::setup::heal_plugin();
        }
        return retitle(name.as_deref().unwrap_or_default(), title, is_dry);
    }
    if !is_valid {
        refuse_name();
    }
    let Some(name) = name.filter(|_| !wanted.is_empty()) else {
        fail(&format!("which issues? name them after the orchestration\n{USAGE}"))
    };
    if let Some(arg) = wanted.iter().find(|arg| !is_issue_arg(arg)) {
        fail(&format!(
            "{arg} is no issue: its number (12) or <n>-<feature> (12-my-feature)\n{USAGE}"
        ));
    }
    let brief_file = args.value("--brief");
    let given = brief_file
        .as_ref()
        .map(|file| fs::read_to_string(file).unwrap_or_else(|_| fail(&format!("{file} does not exist"))));
    let main = repo::main_dir();
    let found = record::record_of(&main, &name);
    let peer = args
        .value("--peer")
        .or_else(|| found.as_ref().and_then(|found| found.peer.clone()));
    let parsed_after = after
        .as_ref()
        .map(|(values, _)| parse_after(values).unwrap_or_else(|error| fail(&error.0)));
    if after.is_some() && !args.has("--to") && !args.has("--queue") {
        fail("--after needs --queue (the issues wait in the queue for the MRs) or --to (a handover announced)");
    }
    if !is_dry {
        crate::cli::setup::heal_plugin();
    }

    let mut plan: Vec<Item> = wanted.iter().map(|arg| read_or_fail(arg)).collect();
    let slot = herdr::slot_of(&name);
    let label = match &title {
        Some(title) => format!("{ORCHESTRATOR} {title}"),
        None => record::orchestrator_label(&main, &name),
    };
    let (colour, _) = herdr::SLOT_COLOURS[slot as usize % herdr::SLOT_COLOURS.len()];
    outln!(
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
        outln!("  #{:<4} {:<width$}  {what}", item.number, item.branch);
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
        .map(|at| (plan[*at].number.to_string(), plan[*at].description.clone()))
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
            outln!("  #{:<4} {line}", item.number);
        }
    }
    // The epic before anything is written: its tasks branch off it (#350).
    match epic::ensure_epic(&name, &plan, args.value("--epic").as_deref(), is_dry) {
        Ok(Some(branch)) => outln!("  epic   {branch}: its tasks branch off it and merge into it by themselves"),
        Ok(None) => {}
        Err(why) => fail(&format!("no epic branch: {why}")),
    }
    if is_dry {
        let first = plan.iter().find(|item| item.why.is_none()).unwrap_or(&plan[0]);
        outln!(
            "\n▸ nothing was done (--dry-run). The brief of {} (.claude/brief.md), started with\n  claude --session-id <new id> --name {} {}\n",
            first.feature,
            first.feature,
            quote(BRIEF_PROMPT)
        );
        let extra = extra_for(&name, first.number, given.as_deref());
        outln!(
            "{}",
            brief_of(&Brief {
                item: first,
                name: &name,
                peer: peer.as_deref(),
                extra: &extra,
                before: None,
                handoff: issue::handoff_for(&name, first).map(|(_, text)| text),
            })
        );
        // Which brief each issue gets (#247): only this run's --brief.
        for at in &to_start {
            let item = &plan[*at];
            let source = if given.is_some() {
                format!("the brief of --brief {}", brief_file.as_deref().unwrap_or_default())
            } else if !extra_for(&name, item.number, None).is_empty() {
                "the brief an earlier run gave it".into()
            } else {
                "no brief of the orchestrator".into()
            };
            outln!("  #{} gets {source}", item.number);
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
        let numbers: Vec<i64> = to_start.iter().map(|at| plan[*at].number).collect();
        keep_briefs(&name, &numbers, given);
    }
    let queued_at = to_iso(now_ms());
    let known: Vec<(i64, Value)> = plan
        .iter()
        .filter_map(|item| known_entry(&name, item.number).map(|entry| (item.number, entry)))
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
                issue::start_entry(item, known_of(item.number).as_ref(), None)
            }
        })
        .collect();
    let _ = record::write_record(&main, &name, &entries);
    if is_queued {
        let numbers: Vec<String> = to_start.iter().map(|at| format!("#{}", plan[*at].number)).collect();
        let mrs: Vec<String> = waits_for.iter().map(|iid| format!("!{iid}")).collect();
        step(&format!(
            "queued {} (--queue{}). The overview of {label} hands each to a free agent{}, in the work order of wt:plan, else starts a fresh one (UP NEXT); by hand: {run} wt:orchestrate {name} <issue> --to <agent>",
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
            },
            run = breadboard_core::config::run_word()
        ));
    }

    let mut results: Vec<String> = Vec::new();
    let mut started: Vec<String> = Vec::new();
    if !is_queued {
        for (at, start) in to_start.iter().zip(&starts) {
            let item = &mut plan[*at];
            let number = item.number;
            match start {
                Start::Worker { worker, shared } => {
                    let mut child = vec![
                        "wt:orchestrate".to_string(),
                        name.clone(),
                        item.number.to_string(),
                        "--to".into(),
                        worker.name.clone(),
                    ];
                    if args.has("--shared") {
                        child.push("--shared".into());
                    }
                    step(&format!("pnpm {}", child.join(" ")));
                    let handed = proc::child().args(&child).stdin(Stdio::null()).status();
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
                        // Renamed after its new issue by the handover (#251): its name now.
                        started.push(item.feature.clone());
                    } else {
                        results.push(format!(
                            "✗ #{} {}: the handover to {} failed (see above); a fresh agent: {run} wt:orchestrate {name} {} --now",
                            item.number, item.feature, worker.name, item.number, run = breadboard_core::config::run_word()));
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
                                setups::holder_text(holder.as_ref())
                            ));
                            continue;
                        }
                    };
                    let before = known_of(number);
                    step(&format!(
                        "{run} wt:new {} --origin {name}{}",
                        item.feature,
                        if *is_shared { " --shared" } else { "" },
                        run = breadboard_core::config::run_word()
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
                    // A retired agent's note on its files (#381), in its worktree before the brief names it.
                    let handoff = issue::handoff_for(&name, item).and_then(|(kept, text)| {
                        let is_given = issue::give_handoff(&kept, &item.dir);
                        if is_given {
                            step("a retired agent's handoff on its files: .claude/handoff.md");
                        }
                        is_given.then_some(text)
                    });
                    let brief = brief_of(&Brief {
                        item,
                        name: &name,
                        peer: peer.as_deref(),
                        extra: &extra,
                        before: None,
                        handoff,
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
                            "✓ #{} {}: claude queued headless (breadboard runs), named {}{note}",
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
    outln!("\n{label}");
    for line in &results {
        outln!("  {line}");
    }
    if !started.is_empty() {
        let hello: Vec<String> = started
            .iter()
            .map(|agent| format!("    SendMessage(to: \"{agent}\", message: \"…\")"))
            .collect();
        let peer_note = peer.as_ref().map(|peer| format!(" ({peer})")).unwrap_or_default();
        outln!(
            "
  A folder claude has not seen asks to be trusted first: answer it in the agent's tab.
  The overview's WORK shows where each agent is: look there, don't ask and don't relay it.
  An agent messages you{peer_note} only for a decision, a changed plan or another agent's work.
  Say hello first, so it has your address:
{}

  The overview of {label} answers the user's \"what now?\": keep it true.
    {run} wt:plan {name} --work <issue>… --merge <issue>… --why \"<one line>\"
                                  the order now, again whenever it changes (a dependency, a failed pipeline)
    {run} wt:action decide \"<what is decided>? <why the recommendation>\" --option \"<label>\" --why \"<what happens>\"… --recommend <label> --orchestration {name}
                                  a step that is the user's and no agent's (--done <id> --orchestration {name} after)
  The agents leave their own actions (merge, try, decide, look, plan) whenever they wait for the user.
",
            hello.join("\n"), run = breadboard_core::config::run_word());
    }
    // A start that failed fails the command: a script and the overview's
    // `--now` (it reads the last line only on an exit code) see it.
    if results.iter().any(|line| line.starts_with('✗')) {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// `wt:orchestrate <name> --title "<text>"`: the title into the record, its space and marks renamed.
fn retitle(name: &str, title: &str, is_dry: bool) -> ExitCode {
    let main = repo::main_dir();
    let before = record::orchestrator_label(&main, name);
    if is_dry {
        outln!("\n▸ nothing was done (--dry-run): {before} → {ORCHESTRATOR} {title}\n");
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
    outln!(
        "\n✓ {before} → {}{}; commands keep the name: {run} wt:orchestrate {name} …\n",
        record::orchestrator_label(&main, name),
        if has_space {
            ", its space and sidebar marks too"
        } else {
            ""
        },
        run = breadboard_core::config::run_word()
    );
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_a_number_or_a_numbered_feature() {
        assert!(is_issue_arg("12"));
        assert!(is_issue_arg("12-x"));
        assert!(!is_issue_arg("abc"));
        assert!(!is_issue_arg("x-12"));
    }
}
