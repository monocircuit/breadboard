//! `breadboard init [--check]` (#342): what breadboard brings into a repo,
//! made from the copies in this binary and the repo's breadboard.toml.
//!
//! - the skill `parallel-work` in `.claude/skills/`, rendered from
//!   `claude/skills/parallel-work/` with the repo's commands, branch, stack;
//! - Claude Code's hooks in `.claude/settings.json`: `hook pre-edit` (no edit
//!   in the main checkout), `hook pre-bash` (no merge, `--no-verify` or force
//!   push by an agent, #427) and `hook stop-answers` (the overview's answers),
//!   and the deny rules of `[agents.deny]` (#375);
//! - in a repo breadboard isn't developed in: the herdr plugin with the
//!   repo's id (`[herdr] plugin`), its keys and base config, the nvim Lua and
//!   the Chromium extension in `.breadboard/`, where herdr links it from;
//! - a `breadboard.toml` when there is none, and the files breadboard writes
//!   kept out of git (`.git/info/exclude`).
//!
//! Run again after an update of breadboard: it rewrites what changed and
//! nothing else. `--check` writes nothing and fails when a file it renders
//! differs (CI, so the repo's copies stay those of its breadboard.toml).

use super::proc::{Args, Shape, fail, outln};
use super::repo;
use breadboard_core::config::{self, Loaded};
use regex::Regex;
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::LazyLock;

/// What `init` takes.
pub const SHAPE: Shape = Shape::new(&["--check"], &[], Some(0), USAGE);

pub const USAGE: &str = "usage: breadboard init [--check]   breadboard's skill, hooks and herdr plugin into this repo";

/// The skill's templates, by their file name.
const SKILL: [(&str, &str); 6] = [
    (
        "SKILL.md",
        include_str!("../../../../../claude/skills/parallel-work/SKILL.md"),
    ),
    (
        "herdr.md",
        include_str!("../../../../../claude/skills/parallel-work/herdr.md"),
    ),
    (
        "orchestrate.md",
        include_str!("../../../../../claude/skills/parallel-work/orchestrate.md"),
    ),
    // What an orchestrated agent's brief leaves out (#429).
    (
        "agent.md",
        include_str!("../../../../../claude/skills/parallel-work/agent.md"),
    ),
    (
        "cleanup.md",
        include_str!("../../../../../claude/skills/parallel-work/cleanup.md"),
    ),
    // Only where products work through batches (`[products.*]`, `[settings] batching`).
    (
        "batches.md",
        include_str!("../../../../../claude/skills/parallel-work/batches.md"),
    ),
];

/// What a repo breadboard isn't developed in gets in `.breadboard/`.
const MANIFEST: &str = include_str!("../../../../../herdr/herdr-plugin.toml");
const KEYS: &str = include_str!("../../../../../herdr/keys.toml");
const HERDR_CONFIG: &str = include_str!("../../../../../herdr/config.toml");
const SHIPPED: [(&str, &str); 7] = [
    ("nvim/changes.lua", include_str!("../../../../../nvim/changes.lua")),
    ("nvim/colours.lua", include_str!("../../../../../nvim/colours.lua")),
    ("nvim/ide.lua", include_str!("../../../../../nvim/ide.lua")),
    (
        "nvim/orchestration.lua",
        include_str!("../../../../../nvim/orchestration.lua"),
    ),
    (
        "chromium/manifest.json",
        include_str!("../../../../../chromium/manifest.json"),
    ),
    ("chromium/open.html", include_str!("../../../../../chromium/open.html")),
    ("chromium/open.js", include_str!("../../../../../chromium/open.js")),
];

/// Where `.breadboard/` keeps what it got (git ignores it, see `exclude`).
pub const FILES: &str = ".breadboard";
const MARK: &str = "# >>> breadboard";
const END: &str = "# <<< breadboard";

/// Whether breadboard is developed in the checkout `top`: its package is
/// there, and herdr links the plugin from it as it is.
pub fn is_developed_in(top: &Path) -> bool {
    top.join("packages/breadboard/herdr/herdr-plugin.toml").is_file()
}

/// The folder breadboard's files (herdr plugin, nvim, Chromium) are in for
/// the repo of the main checkout `main`: its package where breadboard is
/// developed, else `.breadboard/` (breadboard init).
pub fn files_dir(main: &Path) -> PathBuf {
    if is_developed_in(main) {
        main.join("packages/breadboard")
    } else {
        main.join(FILES)
    }
}

/// The values a template names, `{{name}}`.
fn values(loaded: &Loaded) -> Vec<(&'static str, String)> {
    let config = &loaded.config;
    let folder = loaded
        .main
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let run = config.repo.run.clone().unwrap_or_else(|| "breadboard".into());
    let setup: Vec<String> = config
        .worktrees
        .setup
        .iter()
        .filter_map(|step| match &step.says {
            Some(says) => Some(says.clone()),
            None => (!step.is_quiet).then(|| format!("`{}`", step.run)),
        })
        .collect();
    let setup = if setup.is_empty() {
        "runs no setup (breadboard.toml has no `[[worktrees.setup]]`)".to_string()
    } else {
        format!("runs its setup ({})", setup.join("; "))
    };
    let check = if config.check.turbo {
        format!(
            "lint, typecheck and test of every package the branch changes against `{}` and of those depending on them (turbo, cached: a second run on the same tree is quick)",
            loaded.branch
        )
    } else if let Some(run) = config.check.plain() {
        format!("`{run}` over the branch (breadboard.toml's `[check] run`)")
    } else {
        "nothing to check (breadboard.toml names no `[check]`): it stamps the tree at once".into()
    };
    let mut all = vec![
        ("run", run),
        ("bb", loaded.bb().into()),
        ("branch", loaded.branch.clone()),
        ("repo", loaded.name()),
        ("space", loaded.space()),
        ("plugin", loaded.plugin()),
        ("main", folder),
        ("setup", setup),
        ("check", check),
        ("test_origin_env", config.tries.test_origin_env.clone()),
        (
            "test_instance",
            config
                .tries
                .test_instance
                .clone()
                .unwrap_or_else(|| "the test instance".into()),
        ),
    ];
    if let Some(stack) = &config.stack {
        let named: Vec<&config::Service> = stack
            .services
            .iter()
            .filter(|service| service.label.is_some())
            .collect();
        let first = named.first().copied().or(stack.services.first());
        let ports: Vec<String> = std::iter::once(format!("app `{}+n`", stack.app.port))
            .chain(
                named
                    .iter()
                    .map(|service| format!("{} `{}+n`", service.label(), service.port)),
            )
            .chain(
                stack
                    .tools
                    .iter()
                    .map(|tool| format!("{} `{}+n`", tool.name, tool.port)),
            )
            .collect();
        let with_stack: Vec<String> = stack
            .tools
            .iter()
            .filter(|tool| tool.is_with_stack)
            .map(|tool| {
                format!(
                    " and `{}` running `{}` on `{} + offset`",
                    tool.name,
                    tool.run.replace("{port}", "<port>"),
                    tool.port
                )
            })
            .collect();
        all.extend([
            ("stack.env", stack.env_file.clone()),
            (
                "stack.setup",
                stack
                    .setup
                    .as_ref()
                    .map(|setup| setup.run.clone())
                    .unwrap_or_else(|| "docker compose up -d".into()),
            ),
            ("stack.project", stack.project.clone()),
            ("stack.ports", ports.join(", ")),
            (
                "stack.first",
                first.map(|service| service.label().to_string()).unwrap_or_default(),
            ),
            // The database the epic's tasks clone theirs on, and the services a task stack runs.
            (
                "stack.database",
                first
                    .map(|service| service.label().to_string())
                    .unwrap_or_else(|| "database".into()),
            ),
            (
                "stack.others",
                named
                    .iter()
                    .skip(1)
                    .map(|service| service.label().to_string())
                    .collect::<Vec<_>>()
                    .join(" + "),
            ),
            (
                "stack.first_port",
                first.map(|service| service.port.to_string()).unwrap_or_default(),
            ),
            ("stack.with_stack", with_stack.concat()),
            (
                "stack.cd",
                if stack.dir == "." {
                    String::new()
                } else {
                    format!("cd {} && ", stack.dir)
                },
            ),
            ("app.dev", stack.app.dev.clone()),
            ("app.port", stack.app.port.to_string()),
            ("app.port_1", (u32::from(stack.app.port) + 1).to_string()),
        ]);
        // The epic stack's names (`[stack.batch]`, #365): its template, the cloned database, the others.
        if let Some(epic) = &stack.epic {
            let others: Vec<String> = epic
                .databases
                .iter()
                .filter(|database| !database.is_clone)
                .map(|database| format!("`{}_<feature>`", database.name))
                .collect();
            all.extend([
                ("epic.template", epic.template.clone()),
                ("epic.database", epic.clone_database().name.clone()),
                (
                    "epic.others",
                    if others.is_empty() {
                        "no other database".into()
                    } else {
                        others.join(", ")
                    },
                ),
                (
                    "epic.service",
                    stack
                        .service(&epic.service)
                        .map(|service| service.label().to_string())
                        .unwrap_or_else(|| epic.service.clone()),
                ),
            ]);
        }
    }
    all
}

/// A template with its blocks kept or dropped and its values filled in:
/// `{{#stack}}…{{/stack}}` stays only with a `[stack]`, a block's marks on
/// lines of their own go with their lines. A value it doesn't know is a
/// mistake of the template: it fails.
pub fn render(template: &str, values: &[(&str, String)], blocks: &[(&str, bool)]) -> Result<String, String> {
    static CLOSE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{\{/([a-z_]+)\}\}").unwrap());
    static NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{\{([a-z_.0-9]+)\}\}").unwrap());
    let mut missing = Vec::new();
    // The innermost block first: the first close and the last open of its name before it.
    let mut text = template.to_string();
    while let Some(close) = CLOSE.captures(&text) {
        let name = close[1].to_string();
        let whole = close
            .get(0)
            .map(|found| (found.start(), found.end()))
            .unwrap_or_default();
        let open_mark = format!("{{{{#{name}}}}}");
        let Some(open) = text[..whole.0].rfind(&open_mark) else {
            return Err(format!("{{{{/{name}}}}} closes no block"));
        };
        let is_kept = match blocks.iter().find(|(known, _)| *known == name) {
            Some((_, is_kept)) => *is_kept,
            None => {
                missing.push(format!("{{{{#{name}}}}}"));
                false
            }
        };
        // A mark on a line of its own goes with its line; one inside a line keeps the line's end.
        let is_line = |at: usize, end: usize| {
            (at == 0 || text[..at].ends_with('\n')) && (end == text.len() || text[end..].starts_with('\n'))
        };
        let body_start = open + open_mark.len();
        let body_start = if is_line(open, body_start) {
            body_start + 1
        } else {
            body_start
        };
        let end = if is_line(whole.0, whole.1) && whole.1 < text.len() {
            whole.1 + 1
        } else {
            whole.1
        };
        let body = if is_kept {
            text[body_start..whole.0].to_string()
        } else {
            String::new()
        };
        text = format!("{}{body}{}", &text[..open], &text[end..]);
    }
    let filled = NAME.replace_all(&text, |found: &regex::Captures| {
        match values.iter().find(|(name, _)| *name == &found[1]) {
            Some((_, value)) => value.clone(),
            None => {
                missing.push(found[1].to_string());
                String::new()
            }
        }
    });
    if missing.is_empty() {
        Ok(filled.into_owned())
    } else {
        Err(format!(
            "the template names {}, which breadboard doesn't know",
            missing.join(", ")
        ))
    }
}

/// The skill, rendered: `(path in the checkout, text)`.
fn skill_of(loaded: &Loaded) -> Vec<(PathBuf, String)> {
    let values = values(loaded);
    let has_batches = loaded.config.settings.batching.unwrap_or(true) && !loaded.config.products.is_empty();
    let has_epic_stack = loaded.config.stack.as_ref().is_some_and(|stack| stack.epic.is_some());
    let blocks = [
        ("stack", loaded.config.stack.is_some()),
        ("batches", has_batches),
        ("epic_stack", has_epic_stack),
    ];
    SKILL
        .iter()
        .filter(|(name, _)| *name != "batches.md" || has_batches)
        .map(|(name, template)| {
            let text = render(template, &values, &blocks).unwrap_or_else(|error| fail(&format!("{name}: {error}")));
            (Path::new(".claude/skills/parallel-work").join(name), text)
        })
        .collect()
}

/// The herdr plugin of a repo breadboard isn't developed in: the shipped
/// manifest with the repo's id, its commands run by the `breadboard` on PATH.
pub fn manifest_of(manifest: &str, plugin: &str, space: &str) -> String {
    static ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?m)^id = "[^"]*"$"#).unwrap());
    static NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?m)^name = "[^"]*"$"#).unwrap());
    static ABOUT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?m)^description = "[^"]*"$"#).unwrap());
    let body = manifest
        .split_once("\nid = ")
        .map(|(_, rest)| format!("id = {rest}"))
        .unwrap_or_else(|| manifest.to_string());
    let body = ID.replace(&body, format!("id = \"{plugin}\"").as_str());
    let body = NAME.replace(&body, format!("name = \"breadboard: {space}\"").as_str());
    let body = ABOUT.replace(
        &body,
        format!("description = \"Worktrees, dev stacks and the live overview of {space}\"").as_str(),
    );
    let body = body
        .replace(r#"["sh", "breadboard.sh", "--no-wait", "#, r#"["breadboard", "#)
        .replace(r#"["sh", "breadboard.sh", "#, r#"["breadboard", "#);
    format!(
        "# The herdr plugin of {space}, written by breadboard init (#342) from\n# breadboard's own: run it again after an update of breadboard.\n{body}"
    )
}

/// The plugin's keys with the repo's id: `<shipped id>.overview` → `<plugin>.overview`.
pub fn keys_of(keys: &str, manifest: &str, plugin: &str, folder: &str) -> String {
    static ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?m)^id = "([^"]*)"$"#).unwrap());
    static SAYS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?m)^description = "[^":]+: "#).unwrap());
    let shipped = ID
        .captures(manifest)
        .map(|found| found[1].to_string())
        .unwrap_or_default();
    let keys = keys.replace(&format!("\"{shipped}."), &format!("\"{plugin}."));
    let keys = keys.replace(&format!("plugin {shipped}"), &format!("plugin {plugin}"));
    SAYS.replace_all(&keys, format!("description = \"{folder}: ").as_str())
        .into_owned()
}

/// What `.breadboard/` holds in a repo breadboard isn't developed in.
fn files_of(loaded: &Loaded) -> Vec<(PathBuf, String)> {
    let folder = loaded
        .main
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let plugin = loaded.plugin();
    let at = |name: &str| Path::new(FILES).join(name);
    let mut all = vec![
        (
            at("herdr/herdr-plugin.toml"),
            manifest_of(MANIFEST, &plugin, &loaded.space()),
        ),
        (at("herdr/keys.toml"), keys_of(KEYS, MANIFEST, &plugin, &folder)),
        (at("herdr/config.toml"), HERDR_CONFIG.to_string()),
    ];
    all.extend(SHIPPED.iter().map(|(name, text)| (at(name), text.to_string())));
    all
}

/// The hooks `.claude/settings.json` needs, with what each runs: through the
/// package's shims where breadboard is developed, else the `breadboard` on PATH.
fn hooks_of(is_developed: bool) -> [(&'static str, Option<&'static str>, String); 3] {
    let run = |hook: &str| {
        if is_developed {
            format!("sh \"$CLAUDE_PROJECT_DIR/packages/breadboard/hooks/{hook}\"")
        } else {
            format!("breadboard hook {hook}")
        }
    };
    [
        ("PreToolUse", Some("Edit|Write|MultiEdit|NotebookEdit"), run("pre-edit")),
        ("PreToolUse", Some("Bash"), run("pre-bash")),
        ("Stop", None, run("stop-answers")),
    ]
}

/// `settings` with breadboard's hooks where they are missing, None when all are there.
pub fn with_hooks(settings: &Value, is_developed: bool) -> Option<Value> {
    let mut settings = settings.clone();
    if !settings.is_object() {
        settings = json!({});
    }
    let mut changed = false;
    for (event, matcher, command) in hooks_of(is_developed) {
        // `hook pre-edit` or `hooks/pre-edit`: breadboard's, however it is run.
        let name = command
            .rsplit(['/', ' '])
            .next()
            .unwrap_or_default()
            .trim_end_matches('"')
            .to_string();
        let groups = &mut settings["hooks"][event];
        let has = groups.as_array().is_some_and(|groups| {
            groups.iter().any(|group| {
                group["hooks"].as_array().is_some_and(|hooks| {
                    hooks
                        .iter()
                        .filter_map(|hook| hook["command"].as_str())
                        .any(|run| run.contains(&format!("hook {name}")) || run.contains(&format!("hooks/{name}")))
                })
            })
        });
        if has {
            continue;
        }
        let mut group = json!({ "hooks": [{ "type": "command", "command": command, "timeout": 10 }] });
        if let Some(matcher) = matcher {
            group["matcher"] = json!(matcher);
        }
        if !groups.is_array() {
            *groups = json!([]);
        }
        if let Some(list) = groups.as_array_mut() {
            list.push(group);
        }
        changed = true;
    }
    changed.then_some(settings)
}

/// The deny rules of `[agents.deny]` (#375): `Edit(<path>)` (Claude Code
/// applies it to every tool that edits) and `Bash(<command>)`.
pub fn deny_rules_of(deny: &config::Deny) -> Vec<String> {
    let paths = deny.paths.iter().map(|path| format!("Edit({path})"));
    paths
        .chain(deny.commands.iter().map(|command| format!("Bash({command})")))
        .collect()
}

/// `settings` with the deny rules of `[agents.deny]` added to
/// `permissions.deny` where they are missing, None when all are there.
pub fn with_denies(settings: &Value, deny: &config::Deny) -> Option<Value> {
    let mut settings = settings.clone();
    if !settings.is_object() {
        settings = json!({});
    }
    let list = &mut settings["permissions"]["deny"];
    if !list.is_array() {
        *list = json!([]);
    }
    let rules = list.as_array_mut()?;
    let mut changed = false;
    for rule in deny_rules_of(deny) {
        if !rules.iter().any(|had| had.as_str() == Some(rule.as_str())) {
            rules.push(json!(rule));
            changed = true;
        }
    }
    changed.then_some(settings)
}

/// The lines breadboard keeps out of git: what it writes into `.claude/`
/// (Claude Code's own tracked files stay), `.breadboard/`, the pinned herdr.
const EXCLUDED: &str = ".claude/*
!.claude/settings.json
!.claude/skills/
!.claude/agents/
!.claude/hooks/
!.claude/commands/
/.breadboard/
/.herdr/";

/// `text` of `.git/info/exclude` with breadboard's block, replaced where it was.
pub fn with_exclude(text: &str) -> String {
    let block = format!("{MARK}\n{EXCLUDED}\n{END}\n");
    match (text.find(MARK), text.find(END)) {
        (Some(from), Some(to)) if to > from => {
            let after = text[to + END.len()..].trim_start_matches('\n');
            format!("{}{block}{after}", &text[..from])
        }
        _ if text.is_empty() || text.ends_with('\n') => format!("{text}{block}"),
        _ => format!("{text}\n{block}"),
    }
}

/// A first breadboard.toml: the defaults said, the repo named.
fn first_config(loaded: &Loaded) -> String {
    format!(
        "# What breadboard knows about this repo: packages/breadboard/docs/config.md\n# of breadboard, every key and its default.\n\n[repo]\nname = \"{}\"\ndefault_branch = \"{}\"\n",
        loaded.name(),
        loaded.branch
    )
}

pub fn command(args: &Args) -> ExitCode {
    let is_check = args.has("--check");
    let top = PathBuf::from(super::proc::git(&["rev-parse", "--show-toplevel"], &repo::cwd()).text());
    if top.as_os_str().is_empty() {
        fail("breadboard init runs in a checkout of a git repo");
    }
    let loaded = config::of(&top);
    if let Some(error) = &loaded.error {
        fail(error);
    }
    let is_developed = is_developed_in(&top);
    let mut files = skill_of(&loaded);
    if !is_developed {
        files.extend(files_of(&loaded));
    }
    let mut stale = Vec::new();
    let mut wrote = Vec::new();
    for (path, text) in &files {
        let file = top.join(path);
        if fs::read_to_string(&file).ok().as_deref() == Some(text.as_str()) {
            continue;
        }
        if is_check {
            stale.push(path.display().to_string());
            continue;
        }
        if let Some(parent) = file.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Err(error) = fs::write(&file, text) {
            fail(&format!("couldn't write {}: {error}", file.display()));
        }
        wrote.push(path.display().to_string());
    }
    let settings_file = top.join(".claude/settings.json");
    let settings: Value = fs::read_to_string(&settings_file)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| json!({}));
    let hooked = with_hooks(&settings, is_developed);
    let denied = with_denies(hooked.as_ref().unwrap_or(&settings), &loaded.config.agents.deny);
    let parts: Vec<&str> = [
        (hooked.is_some(), "breadboard's hooks"),
        (denied.is_some(), "[agents.deny]"),
    ]
    .into_iter()
    .filter_map(|(is_changed, part)| is_changed.then_some(part))
    .collect();
    if let Some(merged) = denied.or(hooked) {
        let what = format!(".claude/settings.json ({})", parts.join(", "));
        if is_check {
            stale.push(what);
        } else {
            let text = format!("{}\n", serde_json::to_string_pretty(&merged).unwrap_or_default());
            let _ = fs::create_dir_all(top.join(".claude"));
            if let Err(error) = fs::write(&settings_file, text) {
                fail(&format!("couldn't write {}: {error}", settings_file.display()));
            }
            wrote.push(what);
        }
    }
    if is_check {
        if stale.is_empty() {
            outln!("✓ breadboard's files are those of breadboard.toml");
            return ExitCode::SUCCESS;
        }
        fail(&format!(
            "not as breadboard init writes them: {}\n  Run breadboard init and commit what it changed.",
            stale.join(", ")
        ));
    }
    if !top.join(config::FILE).exists() {
        let file = top.join(config::FILE);
        if let Err(error) = fs::write(&file, first_config(&loaded)) {
            fail(&format!("couldn't write {}: {error}", file.display()));
        }
        wrote.push(config::FILE.into());
    }
    let common =
        PathBuf::from(super::proc::git(&["rev-parse", "--path-format=absolute", "--git-common-dir"], &top).text());
    let exclude = common.join("info").join("exclude");
    let before = fs::read_to_string(&exclude).unwrap_or_default();
    let after = with_exclude(&before);
    if after != before {
        let _ = fs::create_dir_all(common.join("info"));
        if let Err(error) = fs::write(&exclude, after) {
            fail(&format!("couldn't write {}: {error}", exclude.display()));
        }
        wrote.push(".git/info/exclude (what breadboard writes stays out of git)".into());
    }
    if wrote.is_empty() {
        outln!("✓ breadboard is set up here, nothing changed");
    } else {
        outln!("✓ breadboard init wrote:");
        for path in &wrote {
            outln!("  {path}");
        }
        outln!("  Commit breadboard.toml, .claude/settings.json and .claude/skills/; .breadboard/ stays out of git.");
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values() -> Vec<(&'static str, String)> {
        vec![("run", "pnpm".into()), ("branch", "develop".into())]
    }

    #[test]
    fn renders_a_template_with_its_stack_blocks() {
        let template =
            "a {{run}} wt:new from {{branch}}\n{{#stack}}\nstack line\n{{/stack}}\nend{{#stack}}, inline{{/stack}}.\n";
        assert_eq!(
            render(template, &values(), &[("stack", true)]).unwrap(),
            "a pnpm wt:new from develop\nstack line\nend, inline.\n"
        );
        assert_eq!(
            render(template, &values(), &[("stack", false)]).unwrap(),
            "a pnpm wt:new from develop\nend.\n"
        );
        assert_eq!(
            render("{{nope}} {{run}}", &values(), &[]),
            Err("the template names nope, which breadboard doesn't know".into())
        );
    }

    #[test]
    fn renders_a_block_inside_another() {
        let template = "{{#stack}}a{{#epics}} b{{/epics}} c{{/stack}}\n";
        let blocks = |stack, epics| [("stack", stack), ("epics", epics)];
        assert_eq!(render(template, &values(), &blocks(true, true)).unwrap(), "a b c\n");
        assert_eq!(render(template, &values(), &blocks(true, false)).unwrap(), "a c\n");
        assert_eq!(render(template, &values(), &blocks(false, true)).unwrap(), "\n");
        assert!(render("{{/x}}", &values(), &[]).is_err());
    }

    #[test]
    fn every_template_renders_with_and_without_a_stack() {
        let with = config::Config::parse(
            "[stack]\nproject = \"p\"\n[stack.app]\nport = 3000\n[[stack.services]]\nname = \"db\"\nlabel = \"DB\"\nport = 5432\nenv = \"DB_PORT\"\n[[stack.tools]]\nname = \"studio\"\nport = 4983\nrun = \"studio -p {port}\"\nis_with_stack = true\n",
        )
        .unwrap();
        let stack = "[stack]\nproject = \"p\"\n[stack.app]\nport = 3000\n\
                     [[stack.services]]\nname = \"db\"\nlabel = \"DB\"\nport = 5432\nenv = \"DB_PORT\"\n\
                     [[stack.services]]\nname = \"auth\"\nlabel = \"Auth\"\nport = 8080\nenv = \"AUTH_PORT\"\n\
                     [stack.migrations]\njournal = \"j\"\nrun = \"migrate\"\n[products.app]\npaths = [\"app\"]\n";
        let epic = "[stack.batch]\nservice = \"db\"\ntemplate = \"app_tpl\"\nstamp = \"db\"\n\
                    [[stack.batch.databases]]\nname = \"app\"\nis_clone = true\nowner = \"app\"\n\
                    [[stack.batch.databases]]\nname = \"auth\"\nowner = \"auth\"\n";
        let epics = config::Config::parse(stack).unwrap();
        let epic_stack = config::Config::parse(&format!("{stack}{epic}")).unwrap();
        let rendered = |config: config::Config| {
            let loaded = Loaded {
                config,
                main: "/r/app".into(),
                branch: "main".into(),
                error: None,
            };
            let skill = skill_of(&loaded);
            for (path, text) in &skill {
                assert!(!text.contains("{{"), "{}: {text}", path.display());
            }
            skill
        };
        for config in [config::Config::default(), with] {
            rendered(config);
        }
        let epics_md = |skill: &[(PathBuf, String)]| {
            skill
                .iter()
                .find(|(path, _)| path.ends_with("batches.md"))
                .map(|(_, text)| text.clone())
                .unwrap()
        };
        // Without [stack.batch] an orchestration's worktrees get own stacks: no batch stack to describe (#365).
        assert!(!epics_md(&rendered(epics)).contains("The batch stack and task stacks"));
        let text = epics_md(&rendered(epic_stack));
        assert!(text.contains("template\n  database `app_tpl`"), "{text}");
        assert!(text.contains("A task gets `app_<feature>`"), "{text}");
        assert!(text.contains("and `auth_<feature>` next to"), "{text}");
    }

    /// #428: the description is all Claude Code sees before it loads the
    /// skill, so it names every task the skill's details are for; every side
    /// file a rendered page links is rendered too.
    #[test]
    fn the_description_names_its_tasks_and_every_link_resolves() {
        for config in [
            config::Config::default(),
            config::Config::parse("[products.app]\npaths = [\"app\"]\n").unwrap(),
        ] {
            let loaded = Loaded {
                config,
                main: "/r/app".into(),
                branch: "main".into(),
                error: None,
            };
            let skill = skill_of(&loaded);
            let names: Vec<String> = skill
                .iter()
                .map(|(path, _)| path.file_name().unwrap().to_string_lossy().into_owned())
                .collect();
            let main = &skill.iter().find(|(path, _)| path.ends_with("SKILL.md")).unwrap().1;
            let description = main
                .lines()
                .find_map(|line| line.strip_prefix("description: "))
                .unwrap();
            assert!(description.len() <= 1024, "{}", description.len());
            for task in [
                "worktree",
                "wt:new",
                "wt:remove",
                "wt:orchestrate",
                "wt:action",
                "wt:wait",
                "wt:check",
                "stack:*",
                "overview",
                "herdr",
                "batches",
                "agents",
            ] {
                assert!(
                    description.contains(task),
                    "the description lacks {task}: {description}"
                );
            }
            static LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\]\(([A-Za-z]+\.md)\)").unwrap());
            for (path, text) in &skill {
                for link in LINK.captures_iter(text) {
                    assert!(
                        names.contains(&link[1].to_string()),
                        "{} links {}",
                        path.display(),
                        &link[1]
                    );
                }
            }
        }
    }

    #[test]
    fn makes_the_plugin_of_another_repo() {
        let manifest = "# ours\n# more\nid = \"x.y\"\nname = \"X\"\ndescription = \"x\"\n[[actions]]\ncommand = [\"sh\", \"breadboard.sh\", \"--no-wait\", \"herdr\", \"open\", \"overview\"]\n[[panes]]\ncommand = [\"sh\", \"breadboard.sh\", \"herdr\", \"pane\", \"overview\"]\n";
        let made = manifest_of(manifest, "breadboard.shop", "me/shop");
        assert!(made.starts_with("# The herdr plugin of me/shop, written by breadboard init"));
        assert!(made.contains("\nid = \"breadboard.shop\"\nname = \"breadboard: me/shop\"\n"));
        assert!(made.contains("command = [\"breadboard\", \"herdr\", \"open\", \"overview\"]"));
        assert!(made.contains("command = [\"breadboard\", \"herdr\", \"pane\", \"overview\"]"));
        assert!(!made.contains("# ours"));
        let keys = "# The keys of the plugin x.y.\n[[keys.command]]\ncommand = \"x.y.overview\"\ndescription = \"platform: live overview\"\n";
        assert_eq!(
            keys_of(keys, manifest, "breadboard.shop", "shop"),
            "# The keys of the plugin breadboard.shop.\n[[keys.command]]\ncommand = \"breadboard.shop.overview\"\ndescription = \"shop: live overview\"\n"
        );
    }

    #[test]
    fn adds_its_hooks_once_beside_the_repos_own() {
        let own =
            json!({ "hooks": { "Stop": [{ "hooks": [{ "type": "command", "command": "node lint.mjs" }] }] }, "x": 1 });
        let merged = with_hooks(&own, false).unwrap();
        assert_eq!(merged["x"], 1);
        assert_eq!(merged["hooks"]["Stop"].as_array().unwrap().len(), 2);
        assert_eq!(
            merged["hooks"]["Stop"][1]["hooks"][0]["command"],
            "breadboard hook stop-answers"
        );
        assert_eq!(
            merged["hooks"]["PreToolUse"][0]["matcher"],
            "Edit|Write|MultiEdit|NotebookEdit"
        );
        assert_eq!(merged["hooks"]["PreToolUse"][1]["matcher"], "Bash");
        assert_eq!(
            merged["hooks"]["PreToolUse"][1]["hooks"][0]["command"],
            "breadboard hook pre-bash"
        );
        assert_eq!(with_hooks(&merged, false), None, "once");
        // The package's shims count as breadboard's too.
        let developed = with_hooks(&json!({}), true).unwrap();
        assert_eq!(
            developed["hooks"]["PreToolUse"][0]["hooks"][0]["command"],
            "sh \"$CLAUDE_PROJECT_DIR/packages/breadboard/hooks/pre-edit\""
        );
        assert_eq!(with_hooks(&developed, false), None);
    }

    #[test]
    fn adds_the_deny_rules_once_beside_the_repos_own() {
        let deny = config::Deny {
            paths: vec!["**/migrations/*.sql".into()],
            commands: vec!["git tag:*".into()],
        };
        let own = json!({ "permissions": { "deny": ["Bash(git tag:*)"], "allow": ["Bash(ls)"] } });
        let merged = with_denies(&own, &deny).unwrap();
        assert_eq!(
            merged["permissions"]["deny"],
            json!(["Bash(git tag:*)", "Edit(**/migrations/*.sql)"])
        );
        assert_eq!(merged["permissions"]["allow"], json!(["Bash(ls)"]));
        assert_eq!(with_denies(&merged, &deny), None, "once");
        assert_eq!(with_denies(&json!({}), &config::Deny::default()), None);
    }

    #[test]
    fn keeps_its_block_in_the_exclude_file() {
        let once = with_exclude("# git's\n*.log\n");
        assert!(once.starts_with("# git's\n*.log\n# >>> breadboard\n.claude/*\n"));
        assert!(once.ends_with("/.herdr/\n# <<< breadboard\n"));
        assert_eq!(with_exclude(&once), once);
        let old = once.replace("/.herdr/\n", "");
        assert_eq!(with_exclude(&format!("{old}tail\n")), format!("{once}tail\n"));
        assert_eq!(with_exclude("x"), format!("x\n{MARK}\n{EXCLUDED}\n{END}\n"));
    }
}
