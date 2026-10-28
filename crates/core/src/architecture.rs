//! The LikeC4 model of the repo and which of its elements a change touches
//! (#439): `arch:deltas` of an MR, a worktree or an epic, as JSON for the
//! architecture view (#440). An element owns the paths of its
//! `metadata { paths '…' }`; a changed file belongs to the element with the
//! longest of them that holds it.
//!
//! Only what the deltas need is read of LikeC4's language: the element kinds
//! of `specification`, the elements of `model` (`id = kind 'title' { … }`,
//! `kind id 'title'`, nested, `extend a.b { … }`), their `title` and
//! `metadata`. Relations, styles, views and the rest are skipped.

use crate::git::git;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// The extensions of a LikeC4 source.
const SOURCES: &[&str] = &[".c4", ".likec4"];

/// The metadata key that maps an element to its paths.
const PATHS: &str = "paths";

/// An element of the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    /// Its kind of `specification`: `product`, `library`.
    pub kind: String,
    pub title: String,
    /// The paths it owns, from the repo's root, without a trailing `/`.
    pub paths: Vec<String>,
}

/// The elements of a model by their full id (`vita.web`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Model {
    pub elements: BTreeMap<String, Element>,
}

impl Model {
    /// The model of LikeC4 `sources`, every file of one project.
    pub fn parse(sources: &[String]) -> Model {
        let files: Vec<Vec<Token>> = sources.iter().map(|source| tokens_of(source)).collect();
        let mut kinds = BTreeSet::new();
        for tokens in &files {
            Reader::new(tokens, &mut kinds, &mut BTreeMap::new()).top(Pass::Specification);
        }
        let mut elements = BTreeMap::new();
        for tokens in &files {
            Reader::new(tokens, &mut kinds, &mut elements).top(Pass::Model);
        }
        Model { elements }
    }

    /// The element `file` belongs to: the one with the longest path that holds it.
    pub fn owner_of(&self, file: &str) -> Option<&str> {
        self.elements
            .iter()
            .flat_map(|(id, element)| element.paths.iter().map(move |path| (id, path)))
            .filter(|(_, path)| is_under(file, path))
            .max_by_key(|(_, path)| path.len())
            .map(|(id, _)| id.as_str())
    }
}

/// Whether `file` is `path` or inside it.
fn is_under(file: &str, path: &str) -> bool {
    path.is_empty() || file == path || file.strip_prefix(path).is_some_and(|rest| rest.starts_with('/'))
}

/// What a change did to an element.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// New in the model.
    Added,
    /// Its files changed, or its kind, title or paths.
    Changed,
    /// Gone from the model.
    Removed,
}

/// One element a change touches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Delta {
    pub id: String,
    pub status: Status,
    /// Its changed files, sorted.
    pub files: Vec<String>,
}

/// The elements a change touches, by id, and the changed files no element owns.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Deltas {
    pub elements: Vec<Delta>,
    pub unmapped: Vec<String>,
}

/// What a change from `before` to `after` touches, `files` the files it
/// changed: an element only `after` has is added, one only `before` has is
/// removed (a deleted file it owned is its), one in both is changed when it
/// owns a changed file or its kind, title or paths differ. A file belongs to
/// its owner in `after`; one `after` doesn't place belongs to a removed
/// element that owned it, else to none.
pub fn deltas(before: &Model, after: &Model, files: &BTreeSet<String>) -> Deltas {
    let mut touched: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut unmapped = Vec::new();
    for file in files {
        let owner = after
            .owner_of(file)
            .or_else(|| before.owner_of(file).filter(|id| !after.elements.contains_key(*id)));
        match owner {
            Some(id) => {
                touched.entry(id.to_string()).or_default().insert(file.clone());
            }
            None => unmapped.push(file.clone()),
        }
    }
    let ids: BTreeSet<&String> = before.elements.keys().chain(after.elements.keys()).collect();
    let elements = ids
        .into_iter()
        .filter_map(|id| {
            let status = match (before.elements.get(id), after.elements.get(id)) {
                (None, Some(_)) => Status::Added,
                (Some(_), None) => Status::Removed,
                (Some(was), Some(is)) if was != is || touched.contains_key(id) => Status::Changed,
                _ => return None,
            };
            let files = touched.remove(id).unwrap_or_default().into_iter().collect();
            Some(Delta {
                id: id.clone(),
                status,
                files,
            })
        })
        .collect();
    Deltas { elements, unmapped }
}

/// The model in `dir` (a checkout's root) at `rev`: every source under
/// `model` in that commit; an empty one where it has none.
pub fn model_at(dir: &Path, rev: &str, model: &str) -> Model {
    let listed = git(dir, &["ls-tree", "-r", "--name-only", "--full-tree", rev, "--", model]);
    let sources: Vec<String> = listed
        .lines()
        .filter(|path| is_source(path))
        .map(|path| git(dir, &["show", &format!("{rev}:{path}")]).stdout)
        .collect();
    Model::parse(&sources)
}

/// The model in `dir`'s files as they are, uncommitted changes included.
pub fn model_in(dir: &Path, model: &str) -> Model {
    let mut sources = Vec::new();
    let mut pending = vec![dir.join(model)];
    while let Some(at) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&at) else {
            continue;
        };
        let mut paths: Vec<_> = entries.flatten().map(|entry| entry.path()).collect();
        paths.sort();
        for path in paths {
            if path.is_dir() {
                pending.push(path);
            } else if is_source(&path.to_string_lossy())
                && let Ok(source) = std::fs::read_to_string(&path)
            {
                sources.push(source);
            }
        }
    }
    Model::parse(&sources)
}

fn is_source(path: &str) -> bool {
    SOURCES.iter().any(|extension| path.ends_with(extension))
}

/// The files changed from `base` to `head` (commits), a rename as its old
/// and its new path.
pub fn files_between(dir: &Path, base: &str, head: &str) -> BTreeSet<String> {
    git(dir, &["diff", "--name-only", "--no-renames", base, head])
        .lines()
        .map(String::from)
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Ident(String),
    Text(String),
    Open,
    Close,
    Equals,
    LeftBracket,
    RightBracket,
    Other,
}

/// The tokens of a LikeC4 source, comments left out. An id holds `-` (not
/// of `->` or `-[`) and `.` between its parts (`vita.web`).
fn tokens_of(source: &str) -> Vec<Token> {
    let chars: Vec<char> = source.chars().collect();
    let mut tokens = Vec::new();
    let mut at = 0;
    while at < chars.len() {
        let char = chars[at];
        let next = chars.get(at + 1).copied();
        if char.is_whitespace() {
            at += 1;
        } else if char == '/' && next == Some('/') {
            while at < chars.len() && chars[at] != '\n' {
                at += 1;
            }
        } else if char == '/' && next == Some('*') {
            at += 2;
            while at < chars.len() && !(chars[at] == '*' && chars.get(at + 1) == Some(&'/')) {
                at += 1;
            }
            at += 2;
        } else if char == '\'' || char == '"' {
            let (text, end) = text_of(&chars, at);
            tokens.push(Token::Text(text));
            at = end;
        } else if char.is_alphabetic() || char == '_' {
            let start = at;
            while at < chars.len() {
                let char = chars[at];
                let next = chars.get(at + 1).copied();
                let is_part = char.is_alphanumeric()
                    || char == '_'
                    || (char == '-' && !matches!(next, Some('>' | '[')))
                    || (char == '.' && next.is_some_and(|next| next.is_alphabetic() || next == '_'));
                if !is_part {
                    break;
                }
                at += 1;
            }
            tokens.push(Token::Ident(chars[start..at].iter().collect()));
        } else {
            tokens.push(match char {
                '{' => Token::Open,
                '}' => Token::Close,
                '=' => Token::Equals,
                '[' => Token::LeftBracket,
                ']' => Token::RightBracket,
                _ => Token::Other,
            });
            // A URL of `link https://…` is no comment.
            at += if char == ':' && next == Some('/') {
                1 + count_of(&chars[at + 1..], '/')
            } else {
                1
            };
        }
    }
    tokens
}

fn count_of(chars: &[char], wanted: char) -> usize {
    chars.iter().take_while(|char| **char == wanted).count()
}

/// The text of the quote at `at` (`'…'`, `"…"`, or tripled for more lines)
/// and where it ends.
fn text_of(chars: &[char], at: usize) -> (String, usize) {
    let quote = chars[at];
    let is_tripled = chars.get(at + 1) == Some(&quote) && chars.get(at + 2) == Some(&quote);
    let width = if is_tripled { 3 } else { 1 };
    let mut text = String::new();
    let mut end = at + width;
    while end < chars.len() {
        if chars[end] == '\\' && end + 1 < chars.len() {
            text.push(chars[end + 1]);
            end += 2;
            continue;
        }
        if chars[end..].iter().take(width).all(|char| *char == quote) && end + width <= chars.len() {
            return (text, end + width);
        }
        text.push(chars[end]);
        end += 1;
    }
    (text, end)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pass {
    Specification,
    Model,
}

struct Reader<'a> {
    tokens: &'a [Token],
    at: usize,
    kinds: &'a mut BTreeSet<String>,
    elements: &'a mut BTreeMap<String, Element>,
}

impl<'a> Reader<'a> {
    fn new(
        tokens: &'a [Token],
        kinds: &'a mut BTreeSet<String>,
        elements: &'a mut BTreeMap<String, Element>,
    ) -> Reader<'a> {
        Reader {
            tokens,
            at: 0,
            kinds,
            elements,
        }
    }

    fn peek(&self, ahead: usize) -> Option<&'a Token> {
        self.tokens.get(self.at + ahead)
    }

    fn ident(&self, ahead: usize) -> Option<&'a str> {
        match self.peek(ahead) {
            Some(Token::Ident(name)) => Some(name),
            _ => None,
        }
    }

    /// The top level: `specification { … }` in the first pass, `model { … }` in the second.
    fn top(&mut self, pass: Pass) {
        while let Some(token) = self.peek(0) {
            let block = self.ident(0).filter(|_| self.peek(1) == Some(&Token::Open));
            match (block, pass) {
                (Some("specification"), Pass::Specification) => {
                    self.at += 2;
                    self.specification();
                }
                (Some("model"), Pass::Model) => {
                    self.at += 2;
                    self.body("", None);
                }
                _ if *token == Token::Open => self.skip_block(),
                _ => self.at += 1,
            }
        }
    }

    /// `element <kind>` up to the `}` of `specification`.
    fn specification(&mut self) {
        while let Some(token) = self.peek(0) {
            match token {
                Token::Close => {
                    self.at += 1;
                    return;
                }
                Token::Open => self.skip_block(),
                Token::Ident(word) if word == "element" => {
                    if let Some(kind) = self.ident(1) {
                        self.kinds.insert(kind.to_string());
                        self.at += 2;
                    } else {
                        self.at += 1;
                    }
                }
                _ => self.at += 1,
            }
        }
    }

    /// The statements of a block up to its `}`: elements under `prefix`, the
    /// title and metadata of `owner`.
    fn body(&mut self, prefix: &str, owner: Option<&str>) {
        while let Some(token) = self.peek(0) {
            if *token == Token::Close {
                self.at += 1;
                return;
            }
            if *token == Token::Open {
                self.skip_block();
                continue;
            }
            let Some(word) = self.ident(0) else {
                self.at += 1;
                continue;
            };
            if let Some(kind) = self.ident(2).filter(|_| self.peek(1) == Some(&Token::Equals)) {
                self.at += 3;
                self.element(prefix, word, kind);
            } else if let Some(name) = self.ident(1).filter(|_| self.kinds.contains(word)) {
                self.at += 2;
                self.element(prefix, name, word);
            } else if word == "extend"
                && let Some(target) = self.ident(1)
                && self.peek(2) == Some(&Token::Open)
            {
                self.at += 3;
                self.body(target, None);
            } else if word == "metadata" && self.peek(1) == Some(&Token::Open) {
                self.at += 2;
                let paths = self.metadata();
                if let Some(element) = owner.and_then(|id| self.elements.get_mut(id))
                    && let Some(paths) = paths
                {
                    element.paths = paths;
                }
            } else if word == "title"
                && let Some(Token::Text(title)) = self.peek(1)
            {
                self.at += 2;
                if let Some(element) = owner.and_then(|id| self.elements.get_mut(id)) {
                    element.title = title.clone();
                }
            } else {
                self.at += 1;
            }
        }
    }

    /// An element's title and body, its id and kind read.
    fn element(&mut self, prefix: &str, name: &str, kind: &str) {
        let id = if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}.{name}")
        };
        let title = match self.peek(0) {
            Some(Token::Text(title)) => title.clone(),
            _ => name.to_string(),
        };
        while matches!(self.peek(0), Some(Token::Text(_))) {
            self.at += 1;
        }
        self.elements.insert(
            id.clone(),
            Element {
                kind: kind.to_string(),
                title,
                paths: Vec::new(),
            },
        );
        if self.peek(0) == Some(&Token::Open) {
            self.at += 1;
            self.body(&id, Some(&id));
        }
    }

    /// A `metadata` block up to its `}`: its `paths`, comma separated, in one
    /// text or a list of them; None without the key.
    fn metadata(&mut self) -> Option<Vec<String>> {
        let mut paths = None;
        while let Some(token) = self.peek(0) {
            match token {
                Token::Close => {
                    self.at += 1;
                    break;
                }
                Token::Open => self.skip_block(),
                Token::Ident(key) => {
                    self.at += 1;
                    let mut values = Vec::new();
                    if let Some(Token::Text(value)) = self.peek(0) {
                        values.push(value.clone());
                        self.at += 1;
                    } else if self.peek(0) == Some(&Token::LeftBracket) {
                        self.at += 1;
                        while let Some(token) = self.peek(0) {
                            self.at += 1;
                            match token {
                                Token::RightBracket => break,
                                Token::Text(value) => values.push(value.clone()),
                                _ => {}
                            }
                        }
                    }
                    if key == PATHS {
                        paths = Some(
                            values
                                .iter()
                                .flat_map(|value| value.split(','))
                                .map(|path| path.trim().trim_end_matches('/').to_string())
                                .filter(|path| !path.is_empty())
                                .collect(),
                        );
                    }
                }
                _ => self.at += 1,
            }
        }
        paths
    }

    /// Past the block that opens here, nested ones included.
    fn skip_block(&mut self) {
        let mut depth = 0;
        while let Some(token) = self.peek(0) {
            self.at += 1;
            match token {
                Token::Open => depth += 1,
                Token::Close => {
                    depth -= 1;
                    if depth == 0 {
                        return;
                    }
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEC: &str = "
        specification {
          element product { style { color blue } }
          element app
          element library
        }
    ";

    const MODEL: &str = r#"
        // The small model of the tests.
        model {
          user = actor 'User'
          vita = product 'vita' {
            description '''
              The product, { braces } in a text.
            '''
            metadata {
              paths 'products/vita'
            }
            web = app "vita web" {
              link https://example.com/docs 'Docs'
              metadata { paths 'products/vita/web/, products/vita/infra' }
            }
            app core 'core' {
              metadata { paths ['products/vita/core'] }
            }
          }
          monolithium = library 'monolithium' {
            #ui
            metadata { paths 'packages/monolithium'; owner 'team' }
          }
          /* relations are skipped */
          vita.web -> monolithium 'renders with' {
            title 'not an element'
          }
          user -[uses]-> vita.web
        }
        views { view index { include * } }
    "#;

    fn model(sources: &[&str]) -> Model {
        Model::parse(&sources.iter().map(|source| source.to_string()).collect::<Vec<_>>())
    }

    fn files(files: &[&str]) -> BTreeSet<String> {
        files.iter().map(|file| file.to_string()).collect()
    }

    #[test]
    fn reads_the_elements_with_their_kind_title_and_paths() {
        let model = model(&[SPEC, MODEL]);
        let ids: Vec<&str> = model.elements.keys().map(String::as_str).collect();
        assert_eq!(ids, ["monolithium", "user", "vita", "vita.core", "vita.web"]);
        let web = &model.elements["vita.web"];
        assert_eq!(web.kind, "app");
        assert_eq!(web.title, "vita web");
        assert_eq!(web.paths, ["products/vita/web", "products/vita/infra"]);
        assert_eq!(model.elements["vita.core"].paths, ["products/vita/core"]);
        assert_eq!(model.elements["monolithium"].paths, ["packages/monolithium"]);
        // `actor` isn't a kind of this specification, but `id = kind` is an element all the same.
        assert_eq!(model.elements["user"].kind, "actor");
        assert!(model.elements["user"].paths.is_empty());
    }

    #[test]
    fn the_kinds_may_come_from_another_file_and_extend_adds_children() {
        let extended =
            "model { extend vita.web { server = app 'server' { metadata { paths 'products/vita/web/src/server' } } } }";
        let model = model(&[MODEL, extended, SPEC]);
        assert!(model.elements.contains_key("vita.core"));
        assert_eq!(
            model.owner_of("products/vita/web/src/server/db/schema.ts"),
            Some("vita.web.server")
        );
    }

    #[test]
    fn a_file_belongs_to_the_element_with_the_longest_path_that_holds_it() {
        let model = model(&[SPEC, MODEL]);
        assert_eq!(model.owner_of("products/vita/web/src/app/page.tsx"), Some("vita.web"));
        assert_eq!(model.owner_of("products/vita/infra/compose.yml"), Some("vita.web"));
        assert_eq!(model.owner_of("products/vita/AGENTS.md"), Some("vita"));
        assert_eq!(model.owner_of("products/vita"), Some("vita"));
        assert_eq!(model.owner_of("products/vitamin/x"), None);
        assert_eq!(model.owner_of("README.md"), None);
    }

    #[test]
    fn changed_files_change_their_elements_and_the_rest_is_unmapped() {
        let model = model(&[SPEC, MODEL]);
        let found = deltas(
            &model,
            &model,
            &files(&[
                "products/vita/web/a.ts",
                "products/vita/web/b.ts",
                "products/vita/x.md",
                "README.md",
            ]),
        );
        assert_eq!(
            found.elements,
            vec![
                Delta {
                    id: "vita".into(),
                    status: Status::Changed,
                    files: vec!["products/vita/x.md".into()],
                },
                Delta {
                    id: "vita.web".into(),
                    status: Status::Changed,
                    files: vec!["products/vita/web/a.ts".into(), "products/vita/web/b.ts".into()],
                },
            ]
        );
        assert_eq!(found.unmapped, ["README.md"]);
    }

    #[test]
    fn elements_new_or_gone_in_the_model_are_added_or_removed() {
        let before = model(&[
            SPEC,
            "model { vita = product 'vita' { metadata { paths 'products/vita' } }
                     old = library 'old' { metadata { paths 'packages/old' } } }",
        ]);
        let after = model(&[
            SPEC,
            "model { vita = product 'vita' { metadata { paths 'products/vita' } }
                     tanstack = library 'tanstack' { metadata { paths 'packages/tanstack' } } }",
        ]);
        let found = deltas(
            &before,
            &after,
            &files(&[
                "packages/old/index.ts",
                "packages/tanstack/index.ts",
                "architecture/model.c4",
            ]),
        );
        let statuses: Vec<(&str, Status, Vec<String>)> = found
            .elements
            .iter()
            .map(|delta| (delta.id.as_str(), delta.status, delta.files.clone()))
            .collect();
        assert_eq!(
            statuses,
            vec![
                ("old", Status::Removed, vec!["packages/old/index.ts".to_string()]),
                (
                    "tanstack",
                    Status::Added,
                    vec!["packages/tanstack/index.ts".to_string()]
                ),
            ]
        );
        assert_eq!(found.unmapped, ["architecture/model.c4"]);
    }

    #[test]
    fn an_element_whose_title_or_paths_changed_is_changed_without_files() {
        let before = model(&[
            SPEC,
            "model { vita = product 'vita' { metadata { paths 'products/vita' } } }",
        ]);
        let after = model(&[
            SPEC,
            "model { vita = product 'Vita' { metadata { paths 'products/vita' } } }",
        ]);
        let found = deltas(&before, &after, &BTreeSet::new());
        assert_eq!(
            found.elements,
            vec![Delta {
                id: "vita".into(),
                status: Status::Changed,
                files: vec![],
            }]
        );
        assert!(deltas(&after, &after, &BTreeSet::new()).elements.is_empty());
    }

    #[test]
    fn the_json_names_each_status_in_lowercase() {
        let model = model(&[SPEC, MODEL]);
        let found = deltas(&Model::default(), &model, &files(&["packages/monolithium/a.tsx"]));
        let json = serde_json::to_value(&found).unwrap();
        assert_eq!(json["elements"][0]["id"], "monolithium");
        assert_eq!(json["elements"][0]["status"], "added");
        assert_eq!(json["elements"][0]["files"][0], "packages/monolithium/a.tsx");
        assert_eq!(json["unmapped"], serde_json::json!([]));
    }
}
