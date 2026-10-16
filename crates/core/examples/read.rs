//! A read of a view as the overview makes it, printed: `read [--orchestration
//! <name> | --worktree <dir>] [--twice]`, in the repo of the cwd. What it
//! found, how long GitLab took and how many calls it made: the check of
//! core's readers against the gauntlet's world (#305's budget).

use breadboard_core::model::orchestration::{NextBand, ViewState};
use breadboard_core::model::text_of;
use breadboard_core::model::tree::facts_text;
use breadboard_core::reader::{LocalReader, Model, RemoteReader, View, model_of};
use breadboard_core::time::now_ms;
use std::time::Instant;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| {
        args.iter()
            .position(|arg| arg == name)
            .and_then(|at| args.get(at + 1))
            .cloned()
    };
    let view = match (flag("--orchestration"), flag("--worktree")) {
        (Some(name), _) => View::Orchestration(name),
        (None, Some(dir)) => View::Worktree(dir.into()),
        _ => View::Everything,
    };
    let here = std::env::current_dir().unwrap_or_default();
    let Some(mut local_reader) = LocalReader::new(&here, view) else {
        eprintln!("no repo here");
        std::process::exit(1);
    };
    let started = Instant::now();
    let local = local_reader.read();
    let local_ms = started.elapsed().as_millis();
    let mut remote_reader = RemoteReader::new(&local_reader.main);
    let ask = local_reader.ask(&local);
    let started = Instant::now();
    let remote = remote_reader.read(&ask, true).await;
    let remote_ms = started.elapsed().as_millis();
    let first_calls = remote_reader.gitlab.calls();
    println!("local {local_ms} ms · GitLab {remote_ms} ms, {first_calls} calls");
    if args.iter().any(|arg| arg == "--twice") {
        let started = Instant::now();
        let again = remote_reader.read(&ask, false).await;
        println!(
            "second read {} ms, {} calls{}",
            started.elapsed().as_millis(),
            remote_reader.gitlab.calls() - first_calls,
            again
                .as_ref()
                .err()
                .map(|error| format!(" ({error})"))
                .unwrap_or_default()
        );
    }
    let remote = match remote {
        Ok(remote) => Some(remote),
        Err(error) => {
            println!("GitLab didn't answer: {error}");
            None
        }
    };
    let mut state = ViewState::default();
    match model_of(&local, remote.as_ref(), &mut state, now_ms()) {
        Model::Orchestration(model) => {
            println!("{}", model.label);
            match &model.next {
                Ok(next) => match &next.band {
                    NextBand::Action(item) | NextBand::UpNext(item) => {
                        println!("NEXT {} {}  {}", item.kind, item.who, item.text)
                    }
                    NextBand::Merge(merge) => println!("NEXT ready to merge !{} {}", merge.iid, merge.feature),
                    NextBand::Nothing(text) => println!("NEXT {text}"),
                },
                Err(error) => println!("NEXT {error}"),
            }
            match &model.actions {
                Ok(actions) => {
                    println!("ACTIONS {}", actions.actions.len());
                    for item in &actions.actions {
                        println!("  {} {}  {}", item.kind, item.who, item.text);
                    }
                    println!("UP NEXT {}", actions.up_next.len());
                    for item in &actions.up_next {
                        println!("  {} {}  {}", item.kind, item.who, item.text);
                    }
                }
                Err(error) => println!("ACTIONS {error}"),
            }
            match &model.worktrees {
                Ok(worktrees) => {
                    println!("WORKTREES {}", text_of(&worktrees.head));
                    println!("  {}  {}", worktrees.name, worktrees.reuse);
                    let tree = &worktrees.tree;
                    for node in tree.live.iter().chain(&tree.queued).chain(&tree.removed) {
                        println!("  {}. {}  {}", node.place, node.name, node.state.text);
                        println!("     {}", facts_text(&node.facts()));
                        println!("     {}", node.title);
                    }
                }
                Err(error) => println!("WORKTREES {error}"),
            }
            match &model.free_agents {
                Ok(rows) => {
                    for row in rows {
                        println!("AGENT {} {}  {}", row.state.text, row.name, row.text);
                    }
                }
                Err(error) => println!("AGENTS {error}"),
            }
            match &model.merge_requests {
                Ok(mrs) => {
                    println!("MERGE REQUESTS {}", mrs.counts);
                    for row in mrs.open.iter().chain(&mrs.merged) {
                        match &row.mr {
                            Some(mr) => {
                                let reasons: Vec<String> = mr
                                    .reasons
                                    .iter()
                                    .map(|reason| format!("{} → {}", reason.text, reason.who))
                                    .collect();
                                println!(
                                    "  {} !{} {} {}  {}  {:?} {}",
                                    row.graph,
                                    mr.iid,
                                    mr.pipeline.text,
                                    mr.title,
                                    mr.status.as_ref().map(|status| status.text.as_str()).unwrap_or(""),
                                    mr.conflict,
                                    reasons.join(" · ")
                                );
                            }
                            None => println!("  {}", row.graph),
                        }
                    }
                }
                Err(error) => println!("MERGE REQUESTS {error}"),
            }
        }
        Model::Worktree(model) => {
            let tree = model.local.tree.as_ref();
            println!(
                "{} {}",
                tree.map(|tree| tree.name.as_str()).unwrap_or("?"),
                tree.map(|tree| tree.branch.as_str()).unwrap_or("")
            );
            if let Some(remote) = &model.remote {
                println!(
                    "MR {:?}",
                    remote
                        .mr
                        .as_ref()
                        .map(|mr| (mr.iid, mr.state.clone(), mr.title.clone()))
                );
                println!(
                    "ISSUE {:?}",
                    remote.issue.as_ref().map(|issue| (issue.iid, issue.title.clone()))
                );
            }
            if let Ok(Some(pipeline)) = &model.pipeline {
                println!(
                    "PIPELINE {}",
                    pipeline
                        .jobs
                        .iter()
                        .map(|job| format!("{} {}", job.status, job.name))
                        .collect::<Vec<_>>()
                        .join(" · ")
                );
            }
            println!(
                "DIFF {:?}",
                model
                    .local
                    .diff
                    .as_ref()
                    .map(|diff| (diff.files, diff.added, diff.removed))
            );
            println!(
                "WAITING FOR {:?}",
                model.waiting.iter().map(|waiting| waiting.mr.iid).collect::<Vec<_>>()
            );
        }
        Model::Everything(model) => {
            for row in model.trees.iter().flatten() {
                println!("  {} {:?}", row.tree.name, row.mr.as_ref().map(|mr| mr.iid));
            }
            if let Some(remote) = &model.remote {
                println!(
                    "MERGE REQUESTS {} open · ISSUES {} open",
                    remote.mrs.len(),
                    remote.issues.len()
                );
            }
        }
    }
}
