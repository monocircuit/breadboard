//! `pnpm commands [--all]` (commands.mjs): every pnpm command of the repo with
//! what it is for. The scripts come from the package.json files, so nothing
//! goes out of date; a script without a line here shows its command.

use super::proc::{git, outln};
use super::repo;
use super::view::is_color;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Where the lines of the commands are kept, the one place a new script gets
/// its line (AGENTS.md): the `DESCRIPTIONS` of commands.mjs, read as data.
const SOURCE: &str = "packages/breadboard/src/commands.mjs";

/// `<package dir>` → script → what it is for; the root is "".
type Descriptions = Vec<(String, Vec<(String, String)>)>;

/// The `DESCRIPTIONS` object of commands.mjs: a `{ key: value, … }` of
/// package dirs, each one of scripts and strings (`'…'` or `"…"`, joined
/// with `+`). Nothing when the file or the object isn't there.
fn descriptions_of(text: &str) -> Descriptions {
    let Some(start) = text.find("const DESCRIPTIONS = {") else {
        return Vec::new();
    };
    let body = &text[start + "const DESCRIPTIONS = ".len()..];
    let tokens = tokens_of(body);
    let mut at = 0;
    match object_of(&tokens, &mut at) {
        Some(Value::Object(dirs)) => dirs
            .into_iter()
            .map(|(dir, scripts)| {
                let lines = scripts
                    .as_object()
                    .map(|scripts| {
                        scripts
                            .iter()
                            .filter_map(|(script, line)| Some((script.clone(), line.as_str()?.to_string())))
                            .collect()
                    })
                    .unwrap_or_default();
                (dir, lines)
            })
            .collect(),
        _ => Vec::new(),
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Text(String),
    Word(String),
    Mark(char),
}

/// The tokens of a JS object literal up to the `}` that closes it.
fn tokens_of(body: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut depth = 0;
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' | '\'' => {
                let mut text = String::new();
                while let Some(next) = chars.next() {
                    match next {
                        '\\' => text.extend(chars.next()),
                        _ if next == c => break,
                        _ => text.push(next),
                    }
                }
                tokens.push(Token::Text(text));
            }
            '/' if chars.peek() == Some(&'/') => {
                for next in chars.by_ref() {
                    if next == '\n' {
                        break;
                    }
                }
            }
            '{' | '}' | ':' | ',' | '+' => {
                tokens.push(Token::Mark(c));
                depth += match c {
                    '{' => 1,
                    '}' => -1,
                    _ => 0,
                };
                if depth == 0 {
                    break;
                }
            }
            _ if c.is_alphanumeric() || c == '_' || c == '$' => {
                let mut word = c.to_string();
                while let Some(next) = chars
                    .peek()
                    .copied()
                    .filter(|n| n.is_alphanumeric() || *n == '_' || *n == '$')
                {
                    word.push(next);
                    chars.next();
                }
                tokens.push(Token::Word(word));
            }
            _ => {}
        }
    }
    tokens
}

/// `{ key: value, … }` from `at`, a value a string (joined) or an object.
fn object_of(tokens: &[Token], at: &mut usize) -> Option<Value> {
    if tokens.get(*at) != Some(&Token::Mark('{')) {
        return None;
    }
    *at += 1;
    let mut object = serde_json::Map::new();
    loop {
        let key = match tokens.get(*at)? {
            Token::Mark('}') => {
                *at += 1;
                return Some(Value::Object(object));
            }
            Token::Mark(',') => {
                *at += 1;
                continue;
            }
            Token::Text(key) | Token::Word(key) => key.clone(),
            Token::Mark(_) => return None,
        };
        *at += 1;
        if tokens.get(*at) != Some(&Token::Mark(':')) {
            return None;
        }
        *at += 1;
        let value = if tokens.get(*at) == Some(&Token::Mark('{')) {
            object_of(tokens, at)?
        } else {
            let mut text = String::new();
            loop {
                let Some(Token::Text(part)) = tokens.get(*at) else {
                    return None;
                };
                text.push_str(part);
                *at += 1;
                if tokens.get(*at) != Some(&Token::Mark('+')) {
                    break;
                }
                *at += 1;
            }
            Value::String(text)
        };
        object.insert(key, value);
    }
}

/// The lines of the repo at `root`, read from its commands.mjs.
fn descriptions(root: &Path) -> Descriptions {
    descriptions_of(&fs::read_to_string(root.join(SOURCE)).unwrap_or_default())
}

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

fn section(root: &Path, said: &Descriptions, dir: &str, prefix: &str) {
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
    let said = said
        .iter()
        .find(|(known, _)| known == dir)
        .map(|(_, table)| table.as_slice())
        .unwrap_or(&[]);
    let width = entries
        .iter()
        .map(|(script, _)| format!("{prefix}{script}").chars().count())
        .max()
        .unwrap_or(0);
    outln!(
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
            outln!();
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
        outln!("  {}  {what}", paint("36", &call));
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
    let said = descriptions(&root);
    if !current.is_empty() {
        section(&root, &said, &current, "pnpm ");
    }
    section(&root, &said, "", if current.is_empty() { "pnpm " } else { "pnpm -w " });
    if is_all {
        for dir in packages(&root).into_iter().filter(|dir| *dir != current) {
            let name = read_json(&root.join(&dir).join("package.json"))
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            section(&root, &said, &dir, &format!("pnpm --filter {name} "));
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
    outln!(
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
    fn reads_the_lines_of_commands_mjs() {
        let text = "const X = 1;\nconst DESCRIPTIONS = {\n  \"\": {\n    build: \"build it\", // why\n    \"wt:new\":\n      '<feature> it\\'s new' +\n      \" (--fix)\",\n  },\n  \"packages/x\": { test: \"t\" },\n};\nconst Y = { z: 'no' };\n";
        let read = descriptions_of(text);
        assert_eq!(
            read,
            vec![
                (
                    String::new(),
                    vec![
                        ("build".to_string(), "build it".to_string()),
                        ("wt:new".to_string(), "<feature> it's new (--fix)".to_string())
                    ]
                ),
                ("packages/x".to_string(), vec![("test".to_string(), "t".to_string())])
            ]
        );
        assert!(descriptions_of("const DESCRIPTIONS = { broken").is_empty());
    }

    #[test]
    fn describes_the_setup_commands() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../..");
        let read = descriptions(&repo);
        let root = &read.iter().find(|(dir, _)| dir.is_empty()).unwrap().1;
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
            assert!(root.iter().any(|(known, _)| known == script), "{script}");
        }
        assert!(read.iter().any(|(dir, _)| dir == "packages/breadboard"));
    }
}
