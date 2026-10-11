//! `breadboard init [--check]` (#342): what breadboard brings into a repo,
//! made from the copies in this binary and the repo's breadboard.toml.
//!
//! - the skill `parallel-work` in `.claude/skills/`, rendered from
//!   `claude/skills/parallel-work/` with the repo's commands, branch, stack;
//! - Claude Code's hooks in `.claude/settings.json`: `hook pre-edit` (no edit
//!   in the main checkout) and `hook stop-answers` (the overview's answers);
//! - in a repo breadboard isn't developed in: the herdr plugin with the
//!   repo's id (`[herdr] plugin`), its keys and base config, the nvim Lua and
//!   the Chromium extension in `.breadboard/`, where herdr links it from;
//! - a `breadboard.toml` when there is none, and the files breadboard writes
//!   kept out of git (`.git/info/exclude`).
//!
//! Run again after an update of breadboard: it rewrites what changed and
//! nothing else. `--check` writes nothing and fails when a file it renders
//! differs (CI, so the repo's copies stay those of its breadboard.toml).

use super::proc::{Args, fail, outln};
use super::repo;
use breadboard_core::config::{self, Loaded};
use regex::Regex;
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::LazyLock;

pub const USAGE: &str = "usage: breadboard init [--check]   breadboard's skill, hooks and herdr plugin into this repo";

/// The skill's templates, by their file name.
const SKILL: [(&str, &str); 4] = [
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
    (
        "cleanup.md",
        include_str!("../../../../../claude/skills/parallel-work/cleanup.md"),
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
        ("branch", loaded.branch.clone()),
        ("repo", loaded.name()),
        ("space", loaded.space()),
        ("plugin", loaded.plugin()),
        ("main", folder),
        ("setup", setup),
        ("check", check),
        ("test_origin_env", config.tries.test_origin_env.clone()),
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
    }
    all
}

/// A template with its blocks kept or dropped and its values filled in:
/// `{{#stack}}…{{/stack}}` stays only with a `[stack]`, a block's marks on
/// lines of their own go with their lines. A value it doesn't know is a
/// mistake of the template: it fails.
pub fn render(template: &str, values: &[(&str, String)], has_stack: bool) -> Result<String, String> {
    static BLOCK: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?s)\{\{#stack\}\}\n?(.*?)\{\{/stack\}\}\n?").unwrap());
    static NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{\{([a-z_.0-9]+)\}\}").unwrap());
    let kept = BLOCK.replace_all(
        template,
        |found: &regex::Captures| {
            if has_stack { found[1].to_string() } else { String::new() }
        },
    );
    let mut missing = Vec::new();
    let filled = NAME.replace_all(&kept, |found: &regex::Captures| {
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
    let has_stack = loaded.config.stack.is_some();
    SKILL
        .iter()
        .map(|(name, template)| {
            let text = render(template, &values, has_stack).unwrap_or_else(|error| fail(&format!("{name}: {error}")));
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
fn hooks_of(is_developed: bool) -> [(&'static str, Option<&'static str>, String); 2] {
    let run = |hook: &str| {
        if is_developed {
            format!("sh \"$CLAUDE_PROJECT_DIR/packages/breadboard/hooks/{hook}\"")
        } else {
            format!("breadboard hook {hook}")
        }
    };
    [
        ("PreToolUse", Some("Edit|Write|MultiEdit|NotebookEdit"), run("pre-edit")),
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
    if let Some(merged) = with_hooks(&settings, is_developed) {
        if is_check {
            stale.push(".claude/settings.json (breadboard's hooks)".into());
        } else {
            let text = format!("{}\n", serde_json::to_string_pretty(&merged).unwrap_or_default());
            let _ = fs::create_dir_all(top.join(".claude"));
            if let Err(error) = fs::write(&settings_file, text) {
                fail(&format!("couldn't write {}: {error}", settings_file.display()));
            }
            wrote.push(".claude/settings.json (breadboard's hooks)".into());
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
            render(template, &values(), true).unwrap(),
            "a pnpm wt:new from develop\nstack line\nend, inline.\n"
        );
        assert_eq!(
            render(template, &values(), false).unwrap(),
            "a pnpm wt:new from develop\nend.\n"
        );
        assert_eq!(
            render("{{nope}} {{run}}", &values(), false),
            Err("the template names nope, which breadboard doesn't know".into())
        );
    }

    #[test]
    fn every_template_renders_with_and_without_a_stack() {
        let with = config::Config::parse(
            "[stack]\nproject = \"p\"\n[stack.app]\nport = 3000\n[[stack.services]]\nname = \"db\"\nlabel = \"DB\"\nport = 5432\nenv = \"DB_PORT\"\n[[stack.tools]]\nname = \"studio\"\nport = 4983\nrun = \"studio -p {port}\"\nis_with_stack = true\n",
        )
        .unwrap();
        for config in [config::Config::default(), with] {
            let loaded = Loaded {
                config,
                main: "/r/app".into(),
                branch: "main".into(),
                error: None,
            };
            for (path, text) in skill_of(&loaded) {
                assert!(!text.contains("{{"), "{}: {text}", path.display());
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
