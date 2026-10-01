//! `pnpm commands [--all]` (commands.mjs): every pnpm command of the repo with
//! what it is for. The scripts come from the package.json files, so nothing
//! goes out of date; a script without a line here shows its command.

use super::proc::git;
use super::repo;
use super::view::is_color;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// `<package dir>` → script → what it is for; the root is "".
const DESCRIPTIONS: &[(&str, &[(&str, &str)])] = &[
    (
        "",
        &[
            ("build", "build every package (turbo)"),
            ("dev", "every dev server (turbo)"),
            ("dev:vita", "the vita dev server, from anywhere"),
            ("build:vita", "a production build of vita"),
            ("storybook", "the monolithium Storybook on :6006"),
            ("lint", "lint every package (turbo)"),
            ("typecheck", "typecheck every package (turbo)"),
            ("test", "unit tests of every package"),
            (
                "changeset",
                "note a change of monolithium/utilities for the next release",
            ),
            ("commands", "this list (--all: every package)"),
            (
                "check:agents",
                "every AGENTS.md, skill and subagent within 150 lines, every link in them resolves",
            ),
            (
                "check:leaf-parity",
                "every *.web.tsx / *.native.tsx leaf has its X.types.ts, and its partner where the package declares native",
            ),
            (
                "herdr",
                "the pinned herdr with the repo's session and plugin (--reload new breadboard code, --stop)",
            ),
            (
                "herdr:install",
                "the repo in your own herdr instead (e.g. Omarchy's): plugin, keys, the space Monocircuit/Platform",
            ),
            (
                "browser",
                "the Chromium extension for a tab group per herdr space: install it, say how to load it",
            ),
            (
                "overview",
                "live: worktrees, stacks, merge requests and issues (q quits, --worktree <dir> for one, --orchestration <name> for its issues and MRs)",
            ),
            (
                "changelog",
                "the live view of what got into develop, by product and package: rewritten from GitLab and printed (--since 30d|2w|<date>, --watch: live, the tab changelog of the main space); not CHANGELOG.md",
            ),
            (
                "changelog:add",
                "[--type <t>] [--area <a>] \"<sentence>\" an entry of this branch's MR for CHANGELOG.md, as .changelog/<issue>-<slug>.md",
            ),
            (
                "changelog:unreleased",
                "what the next release's section of CHANGELOG.md would say, from .changelog/",
            ),
            (
                "changelog:release",
                "<X.Y.Z> on the release branch: the fragments folded into CHANGELOG.md, vita's version set (skill release)",
            ),
            (
                "changelog:notes",
                "<X.Y.Z> a release's section of CHANGELOG.md (the GitLab release's description)",
            ),
            (
                "changelog:check",
                "every .changelog/ fragment valid; --mr: this MR adds one or carries no-changelog (CI check:changelog)",
            ),
            (
                "stack:up",
                "start the vita stack — of the worktree on --port <p> / <feature>, else yours",
            ),
            ("stack:down", "stop that stack, data stays"),
            ("stack:reset", "throw away the data of that stack and set it up again"),
            ("stack:ps", "the containers of that stack"),
            (
                "wt:new",
                "<feature> a worktree on a new branch with its own stack (--shared, --fix, --origin <orchestration>)",
            ),
            (
                "wt:orchestrate",
                "<name> <issue>… a space ⇶ <name> for this session, per issue a worktree with a briefed claude (--dry-run, --queue [--after !<mr>…]); <name> <issue> --to <agent> hands it to an idle agent (--after !<mr>…: once they are merged); <name> --title <text> what it shows instead of its name; <name> --end ends it once its worktrees are gone; <name> --changelog [--watch] its changelog",
            ),
            (
                "wt:plan",
                "<name> [--work <n>…] [--merge <n>…] [--why <t>] the order of an orchestration's issues and merges, for its overview",
            ),
            ("wt:stack", "give the worktree you are in its own stack"),
            (
                "wt:check",
                "lint, typecheck and test of what the branch changes (and what depends on it), cached; stamps the tree it passed: the pre-push hook refuses any other commit",
            ),
            (
                "wt:mr",
                "open the draft MR of this worktree (--update sends the description again)",
            ),
            ("wt:note", "<issue> --message-file <f> comment a file on an issue"),
            (
                "wt:tick",
                "[<issue>] [<box>…] the boxes of the issue, numbered; ticks the given ones",
            ),
            (
                "wt:action",
                "<kind> <text> [--url <u> | --run <cmd>] ask the human for merge, try (opens the spot to check), decide (a question, --option per consequence, --recommend <option>), plan or look: a row in ACTIONS of the orchestration's overview (--list, --done <id>, --orchestration <name> for the orchestrator); change <what>: a change for the MR, draft at once and its merge withdrawn (--mr !N with --orchestration)",
            ),
            (
                "wt:wait",
                "!<mr>… [--then \"<what>\"] run in the background: ends with the line to go on with once the MRs are merged (early when one is closed or stays in conflict, after --hours, default 8); the overview shows the wait (--cancel)",
            ),
            ("wt:issues", "issues of merged MRs nobody commented on since the merge"),
            ("wt:prune", "close worktrees and branches of merged MRs (--dry-run)"),
            (
                "wt:herdr",
                "herdr: root and overview, a space per worktree, tabs of running stacks; after a restart it all runs again",
            ),
            ("wt:list", "every worktree: branch, url, stack"),
            ("wt:remove", "<feature> stack, worktree and merged branch away"),
        ],
    ),
    (
        "products/vita/web",
        &[
            ("commands", "this list: vita first, then the root"),
            (
                "overview",
                "live: worktrees, stacks, merge requests and issues (q quits)",
            ),
            ("dev", "the app on the port of this worktree, logged in"),
            (
                "dev:ready",
                "[<route>…] wait until pnpm dev answers and has built these pages (default /editor)",
            ),
            (
                "dev:setup",
                "stack up, migrate, bucket, Keycloak, seed — a fresh stack in one go",
            ),
            ("dev:webpack", "the app with webpack instead of turbopack"),
            ("dev:cpu", "the app with a CPU profile"),
            (
                "logs",
                "[<path>…] what the browser's devtools logger wrote: entries, --state <name>, --paths, --since 2m",
            ),
            ("build", "production build"),
            ("build:cpu", "production build with a CPU profile"),
            ("build:debug", "production build with memory debugging"),
            ("build:migrate", "bundle the migration runner for the image"),
            ("start", "serve the production build"),
            ("stack:up", "start Postgres, Keycloak and SeaweedFS of this checkout"),
            ("stack:down", "stop the containers of this checkout, data stays"),
            (
                "stack:reset",
                "throw away the data of this checkout and set it up again",
            ),
            (
                "routes:generate",
                "the page.tsx files from core's route table (--check: only compare, CI check:routes)",
            ),
            (
                "pg:generate",
                "generate a migration from src/server/db/schema.ts (skill db-migration)",
            ),
            ("pg:migrate", "apply the migrations"),
            ("pg:seed", "fill the database with the dev data"),
            ("pg:studio", "Drizzle Studio on the database"),
            ("pg:visualizer", "the schema as a diagram, on the port of this worktree"),
            ("storage:setup", "create the bucket in SeaweedFS"),
            (
                "startup-images",
                "the iOS launch screens per device, light and dark, from the logo",
            ),
            (
                "screenshots:manifest",
                "the install dialog's screenshots of the atlas, from pnpm dev (Chromium)",
            ),
            (
                "keycloak:setup",
                "the MCP clients, their scopes and the login theme in Keycloak",
            ),
            ("lint", "ESLint"),
            ("typecheck", "next typegen and tsc"),
            ("test", "unit tests"),
            ("test:coverage", "unit tests with coverage (CI has a floor)"),
            (
                "test:int",
                "the operations against the real Postgres and object storage",
            ),
        ],
    ),
    (
        "packages/monolithium",
        &[
            ("storybook", "Storybook on :6006"),
            ("build-storybook", "static Storybook, like CI"),
            ("check-contrast", "contrast check of the built Storybook"),
            ("tokens", "write tokens.css and theme.css from src/tokens/tokens.ts"),
        ],
    ),
    (
        "packages/breadboard",
        &[
            ("lint", "ESLint, cargo fmt and clippy"),
            (
                "lint:rs",
                "cargo fmt --check and clippy -D warnings of the Rust breadboard",
            ),
            ("test", "node --test and cargo test"),
            ("test:rs", "cargo test of the Rust breadboard"),
            ("build:rs", "the breadboard binary, release (target/release/breadboard)"),
        ],
    ),
    (
        "packages/breadboard-gauntlet",
        &[
            (
                "snap",
                "a screenshot (text, PNG) of any TUI in a pty, in a gauntlet world: for the rewrite's critics",
            ),
            (
                "gauntlet",
                "the black-box suite: breadboard in a pty against a recorded world (BREADBOARD_SUBJECT: a rewrite)",
            ),
        ],
    ),
    (
        "packages/eslint-config",
        &[
            ("build", "build the configs (dev) and types"),
            ("build:prod", "build the configs as the hooks and CI load them"),
            ("test", "the platform rules on fixtures (build:prod first)"),
        ],
    ),
];

fn paint(code: &str, text: &str) -> String {
    if is_color() {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

fn read_json(file: &Path) -> Value {
    fs::read_to_string(file)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(Value::Null)
}

/// The workspace packages, from pnpm-workspace.yaml's `dir/*` globs.
fn packages(root: &Path) -> Vec<String> {
    let text = fs::read_to_string(root.join("pnpm-workspace.yaml")).unwrap_or_default();
    let mut dirs: Vec<String> = text
        .lines()
        .filter_map(|line| {
            let item = line
                .trim_start()
                .strip_prefix('-')?
                .trim()
                .trim_matches(|c| c == '"' || c == '\'');
            let dir = item.strip_suffix("/*")?;
            (!dir.is_empty() && !dir.contains('*')).then(|| dir.to_string())
        })
        .flat_map(|dir| {
            fs::read_dir(root.join(&dir))
                .map(|entries| {
                    entries
                        .flatten()
                        .map(|entry| format!("{dir}/{}", entry.file_name().to_string_lossy()))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        })
        .filter(|dir| root.join(dir).join("package.json").exists())
        .collect();
    dirs.sort();
    dirs
}

/// The package the command was typed in (INIT_CWD), relative to the root; "" for the root.
fn here(root: &Path) -> String {
    let mut dir: PathBuf = repo::here();
    while dir.starts_with(root) && dir != root {
        if dir.join("package.json").exists() {
            return dir
                .strip_prefix(root)
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default();
        }
        if !dir.pop() {
            break;
        }
    }
    String::new()
}

fn section(root: &Path, dir: &str, prefix: &str) {
    let package = read_json(&root.join(dir).join("package.json"));
    let name = package.get("name").and_then(Value::as_str).unwrap_or_default();
    let scripts: Vec<(String, String)> = package
        .get("scripts")
        .and_then(Value::as_object)
        .map(|scripts| {
            scripts
                .iter()
                .map(|(key, value)| (key.clone(), value.as_str().unwrap_or_default().to_string()))
                .collect()
        })
        .unwrap_or_default();
    let group_of = |script: &str| script.split(':').next().unwrap_or_default().to_string();
    let mut order: Vec<String> = Vec::new();
    for (script, _) in &scripts {
        let group = group_of(script);
        if !order.contains(&group) {
            order.push(group);
        }
    }
    let mut entries: Vec<&(String, String)> = scripts.iter().filter(|(script, _)| script != "preinstall").collect();
    entries.sort_by_key(|(script, _)| order.iter().position(|group| *group == group_of(script)));
    if entries.is_empty() {
        return;
    }
    let size_of = |group: &str| entries.iter().filter(|(script, _)| group_of(script) == group).count();
    let said = DESCRIPTIONS
        .iter()
        .find(|(known, _)| *known == dir)
        .map(|(_, table)| *table)
        .unwrap_or(&[]);
    let width = entries
        .iter()
        .map(|(script, _)| format!("{prefix}{script}").chars().count())
        .max()
        .unwrap_or(0);
    println!(
        "\n{} {}",
        paint("1", name),
        paint("2", if dir.is_empty() { "(root)" } else { dir })
    );
    let mut group: Option<String> = None;
    for (script, command) in &entries {
        let next = group_of(script);
        if let Some(previous) = &group
            && *previous != next
            && (size_of(&next) > 1 || size_of(previous) > 1)
        {
            println!();
        }
        group = Some(next);
        let call = format!("{:<width$}", format!("{prefix}{script}"));
        let what = match said.iter().find(|(known, _)| known == script) {
            Some((_, description)) => description.to_string(),
            None if command.chars().count() > 70 => {
                paint("2", &format!("{}…", command.chars().take(69).collect::<String>()))
            }
            None => paint("2", command),
        };
        println!("  {}  {what}", paint("36", &call));
    }
}

pub fn command(is_all: bool) -> ExitCode {
    let top = git(&["rev-parse", "--show-toplevel"], &repo::here()).text().to_string();
    let root = if top.is_empty() {
        repo::main_dir()
    } else {
        PathBuf::from(top)
    };
    let current = here(&root);
    if !current.is_empty() {
        section(&root, &current, "pnpm ");
    }
    section(&root, "", if current.is_empty() { "pnpm " } else { "pnpm -w " });
    if is_all {
        for dir in packages(&root).into_iter().filter(|dir| *dir != current) {
            let name = read_json(&root.join(&dir).join("package.json"))
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            section(&root, &dir, &format!("pnpm --filter {name} "));
        }
    }
    let from_here = if current.is_empty() {
        ""
    } else {
        "pnpm -w <script> runs a root command from here. "
    };
    let all = if is_all {
        ""
    } else {
        "pnpm commands --all: every package. "
    };
    println!(
        "{}",
        paint(
            "2",
            &format!("\n  {from_here}{all}In a package: its own commands first.\n")
        )
    );
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_the_setup_commands() {
        let root = DESCRIPTIONS.iter().find(|(dir, _)| dir.is_empty()).unwrap().1;
        for script in [
            "wt:new",
            "wt:orchestrate",
            "wt:stack",
            "wt:check",
            "stack:up",
            "changelog",
            "changelog:add",
            "commands",
            "browser",
        ] {
            assert!(root.iter().any(|(known, _)| *known == script), "{script}");
        }
    }
}
