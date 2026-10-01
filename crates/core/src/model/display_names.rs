//! Every agent of an orchestration has a person's name (#292,
//! worktree/display-names.mjs of its branch), the one the user knows it by
//! while it moves from issue to issue: `Mira Albrecht`. It is only shown;
//! `claude --name` and SendMessage keep the feature. The record keeps it as
//! the worker's `displayName` from its first start on; an agent without one
//! gets the name its conversation id picks (the id is random, so is the
//! name), the first not taken in the orchestration. Reading never writes it.

use super::Entry;
use super::handovers::Known;
use super::history::history_of;
use std::collections::{BTreeSet, HashMap, HashSet};

/// First names, varied and international.
pub const FIRST: [&str; 60] = [
    "Mira",
    "Jonas",
    "Amara",
    "Kenji",
    "Lucía",
    "Tomasz",
    "Ingrid",
    "Rafael",
    "Noor",
    "Elias",
    "Saanvi",
    "Mateo",
    "Freya",
    "Kwame",
    "Yuki",
    "Henrik",
    "Leila",
    "Dario",
    "Anouk",
    "Tariq",
    "Sofia",
    "Malik",
    "Hana",
    "Oskar",
    "Zainab",
    "Luca",
    "Aiko",
    "Emil",
    "Priya",
    "Bastian",
    "Ines",
    "Kofi",
    "Marta",
    "Arjun",
    "Elif",
    "Niklas",
    "Chiara",
    "Omar",
    "Signe",
    "Diego",
    "Nadia",
    "Felix",
    "Ayumi",
    "Ruben",
    "Thandi",
    "Mikael",
    "Lina",
    "Joaquín",
    "Ama",
    "Viktor",
    "Mei",
    "Soren",
    "Fatima",
    "Pavel",
    "Clara",
    "Idris",
    "Astrid",
    "Rohan",
    "Valentina",
    "Jakob",
];

/// Last names, as varied.
pub const LAST: [&str; 60] = [
    "Albrecht",
    "Okafor",
    "Tanaka",
    "García",
    "Nowak",
    "Lindqvist",
    "Haddad",
    "Moreau",
    "Rossi",
    "Kowalczyk",
    "Mensah",
    "Sato",
    "Fischer",
    "Novák",
    "Petrov",
    "Ibrahim",
    "Jensen",
    "Costa",
    "Yilmaz",
    "Kim",
    "Bauer",
    "Silva",
    "Nakamura",
    "Dubois",
    "Andersen",
    "Mehta",
    "Herrera",
    "Schulz",
    "Osei",
    "Virtanen",
    "Rahman",
    "Bianchi",
    "Kovač",
    "Lehmann",
    "Adeyemi",
    "Ito",
    "Navarro",
    "Holm",
    "Farouk",
    "Wójcik",
    "Brandt",
    "Chen",
    "Duarte",
    "Eriksen",
    "Kaya",
    "Mbeki",
    "Lambert",
    "Horvat",
    "Keller",
    "Reyes",
    "Sørensen",
    "Abe",
    "Vogel",
    "Nkemelu",
    "Marino",
    "Šimek",
    "Weber",
    "Zhou",
    "Ortega",
    "Laine",
];

/// A number from `id`, the same every time (FNV-1a over its code points, as Node's).
fn hash_of(id: &str) -> u32 {
    id.chars()
        .fold(0x811c_9dc5u32, |hash, ch| (hash ^ ch as u32).wrapping_mul(0x0100_0193))
}

/// The name in the place `n` of all FIRST × LAST, the first names varying first.
fn name_at(n: usize) -> String {
    format!("{} {}", FIRST[n % FIRST.len()], LAST[(n / FIRST.len()) % LAST.len()])
}

/// The name the conversation `id` picks, none of `taken`: from the place its
/// id points to on, the first whose first name no one has yet, else the
/// first not taken whole.
pub fn pick_name<'a>(id: &str, taken: impl IntoIterator<Item = &'a str>) -> String {
    let names: HashSet<&str> = taken.into_iter().collect();
    let firsts: HashSet<&str> = names.iter().filter_map(|name| name.split(' ').next()).collect();
    let all = FIRST.len() * LAST.len();
    let start = hash_of(id) as usize % all;
    let mut free = None;
    for step in 0..all {
        let name = name_at(start + step);
        if names.contains(name.as_str()) {
            continue;
        }
        if !name.split(' ').next().is_some_and(|first| firsts.contains(first)) {
            return name;
        }
        free.get_or_insert(name);
    }
    free.unwrap_or_else(|| name_at(start))
}

/// The name of every agent, by its conversation id: the record's workers
/// (`known`) that have one keep it, the others (and each of `ids` the record
/// doesn't know) pick theirs in the order of their ids.
pub fn display_names_of(known: &Known, ids: &[&str]) -> HashMap<String, String> {
    let mut names: HashMap<String, String> = known
        .iter()
        .filter_map(|worker| Some((worker.id.clone(), worker.display_name.clone()?)))
        .collect();
    let others: BTreeSet<&str> = known
        .iter()
        .map(|worker| worker.id.as_str())
        .chain(ids.iter().copied())
        .filter(|id| !id.is_empty() && !names.contains_key(*id))
        .collect();
    for id in others {
        let name = pick_name(id, names.values().map(String::as_str));
        names.insert(id.to_string(), name);
    }
    names
}

/// The names of an orchestration's agents (handovers.mjs `agentNamesOf`):
/// by conversation id, and by every name an agent goes by (its record's,
/// its worktree's feature, its name before its last handover).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Names {
    pub by_id: HashMap<String, String>,
    by_name: HashMap<String, String>,
}

impl Names {
    /// The display name of the agent the name `who` goes by, else `who`.
    pub fn shown(&self, who: &str) -> String {
        self.by_name.get(who).cloned().unwrap_or_else(|| who.to_string())
    }
}

/// The names of the record's workers and of the agents in `issues`' worktrees.
pub fn names_of(issues: &[Entry], known: &Known) -> Names {
    let trees: Vec<(&str, &str)> = issues
        .iter()
        .filter_map(|issue| {
            let tree = issue.tree.as_ref()?;
            Some((tree.claude.as_ref()?.id.as_str(), tree.name.as_str()))
        })
        .collect();
    let ids: Vec<&str> = trees.iter().map(|(id, _)| *id).collect();
    let by_id = display_names_of(known, &ids);
    let mut by_name = HashMap::new();
    for worker in known {
        for name in [&worker.was, &worker.name].into_iter().flatten() {
            if let Some(shown) = by_id.get(&worker.id) {
                by_name.insert(name.clone(), shown.clone());
            }
        }
    }
    for (id, feature) in trees {
        if let Some(shown) = by_id.get(id) {
            by_name.insert(feature.to_string(), shown.clone());
            if let Some(name) = known
                .iter()
                .find(|worker| worker.id == id)
                .and_then(|worker| worker.name.clone())
            {
                by_name.insert(name, shown.clone());
            }
        }
    }
    Names { by_id, by_name }
}

/// What an agent's name line counts (history.mjs `countsOf`): the issues it
/// worked on, `number` included, and how many of their MRs are merged.
pub fn counts_of(known: &Known, id: &str, number: Option<i64>, is_merged: impl Fn(i64) -> bool) -> (usize, usize) {
    let mut numbers: Vec<i64> = Vec::new();
    let worked = known
        .iter()
        .find(|worker| worker.id == id)
        .map(history_of)
        .unwrap_or_default();
    for number in worked.iter().map(|start| start.number).chain(number) {
        if number != 0 && !numbers.contains(&number) {
            numbers.push(number);
        }
    }
    let merged = numbers.iter().filter(|number| is_merged(**number)).count();
    (numbers.len(), merged)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::Worker;

    fn worker(id: &str, display: Option<&str>) -> Worker {
        Worker {
            id: id.into(),
            display_name: display.map(String::from),
            ..Default::default()
        }
    }

    #[test]
    fn picks_the_name_node_picks_for_the_same_id() {
        // Node's display-names.mjs on these ids (#292's branch).
        assert_eq!(pick_name("0b1e2c3d-6c1e-4b7a-9f00-0d2c1a0", []), "Nadia Moreau");
        assert_eq!(pick_name("f1", []), "Priya Virtanen");
        assert_eq!(pick_name("a8f3c2d1-7777-4b7a-9f00-000000000000", []), "Lina Okafor");
        let known = [worker("other", Some("Mira Albrecht"))];
        let names = display_names_of(&known, &["0b1e2c3d-6c1e-4b7a-9f00-0d2c1a0", "f1", "x"]);
        assert_eq!(names["other"], "Mira Albrecht");
        assert_eq!(names["f1"], "Priya Virtanen");
        assert_eq!(names["x"], "Henrik Farouk");
    }

    #[test]
    fn no_two_agents_share_a_name_or_while_they_can_a_first_name() {
        let ids: Vec<String> = (0..40).map(|i| format!("{i:08x}-6c1e-4b7a-9f00-0d2c1a{i}")).collect();
        let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
        let names = display_names_of(&[], &ids);
        let whole: HashSet<&String> = names.values().collect();
        let firsts: HashSet<&str> = names.values().filter_map(|name| name.split(' ').next()).collect();
        assert_eq!((whole.len(), firsts.len()), (40, 40));
    }

    #[test]
    fn keeps_the_recorded_name_and_picks_around_it() {
        let own = pick_name("me", []);
        let known = [worker("other", Some(&own)), worker("me", None)];
        let names = display_names_of(&known, &[]);
        assert_eq!(names["other"], own);
        assert_ne!(names["me"], own);
    }

    #[test]
    fn counts_the_issues_worked_on_and_their_merged_mrs() {
        let known = [Worker {
            id: "a".into(),
            issues: vec![235, 240],
            ..Default::default()
        }];
        assert_eq!(counts_of(&known, "a", Some(252), |number| number == 235), (3, 1));
        assert_eq!(counts_of(&known, "a", Some(240), |_| false), (2, 0));
        assert_eq!(counts_of(&[], "b", Some(7), |_| true), (1, 1));
    }
}
