//! An orchestration's epic branch (#350): `epic/<n>-<name>` off develop,
//! where its tasks branch off and merge into by themselves; only the epic
//! goes to develop by the user's two clicks. `<n>` is the epic of the
//! issues' milestone (`breadboard · Epic 15: …` → 15): epics are milestones.

use crate::json::str_of;
use serde_json::Value;

/// What every epic branch starts with.
pub const PREFIX: &str = "epic/";

/// Whether `branch` is an epic branch, the target of an automatic merge.
pub fn is_epic(branch: &str) -> bool {
    branch.starts_with(PREFIX) && branch.len() > PREFIX.len()
}

/// The epic of a milestone's title: `breadboard · Epic 15: The dev workflow` → 15.
pub fn number_of_milestone(title: &str) -> Option<i64> {
    let (_, after) = title.split_once("Epic ")?;
    let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// The epic branch of the orchestration `name` in epic `number`.
pub fn branch_of(number: i64, name: &str) -> String {
    format!("{PREFIX}{number}-{name}")
}

/// The epic of the issues, by their milestones' titles: the one they share,
/// else why there is none (`--epic <n>` names it then).
pub fn number_of_issues(milestones: &[Option<String>]) -> Result<i64, String> {
    let mut found: Vec<i64> = Vec::new();
    for title in milestones {
        match title.as_deref().and_then(number_of_milestone) {
            Some(number) if !found.contains(&number) => found.push(number),
            Some(_) => {}
            None => return Err("an issue has no epic milestone (`<project> · Epic N: …`)".into()),
        }
    }
    match found[..] {
        [number] => Ok(number),
        [] => Err("no issue names its epic".into()),
        _ => {
            let names: Vec<String> = found.iter().map(|number| format!("Epic {number}")).collect();
            Err(format!("the issues belong to {}", names.join(", ")))
        }
    }
}

/// The epic branch the orchestration's record keeps, None for one that
/// works on develop (made before #350).
pub fn epic_of(record: &Value) -> Option<String> {
    record
        .get("epic")
        .and_then(|epic| str_of(epic, "branch"))
        .filter(|branch| is_epic(branch))
        .map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn names_the_branch_after_the_milestones_epic() {
        assert_eq!(number_of_milestone("breadboard · Epic 15: The dev workflow"), Some(15));
        assert_eq!(number_of_milestone("vita · MVP"), None);
        assert_eq!(branch_of(15, "breadboard"), "epic/15-breadboard");
        assert!(is_epic("epic/15-breadboard"));
        assert!(!is_epic("epic/") && !is_epic("develop") && !is_epic("feature/epic/1"));
    }

    #[test]
    fn the_issues_share_one_epic_or_say_why_not() {
        let fifteen = Some("breadboard · Epic 15: The dev workflow".to_string());
        assert_eq!(number_of_issues(&[fifteen.clone(), fifteen.clone()]), Ok(15));
        assert!(number_of_issues(&[fifteen.clone(), None]).is_err());
        let fourteen = Some("vita · Epic 14: One codebase".to_string());
        assert_eq!(
            number_of_issues(&[fifteen, fourteen]),
            Err("the issues belong to Epic 15, Epic 14".into())
        );
    }

    #[test]
    fn reads_the_epic_of_a_record() {
        assert_eq!(
            epic_of(&json!({ "epic": { "branch": "epic/15-breadboard", "number": 15 } })),
            Some("epic/15-breadboard".into())
        );
        assert_eq!(epic_of(&json!({ "issues": [] })), None);
    }
}
