//! What a mistyped name was likely meant to be: `did you mean …?` for a
//! command (cli/mod.rs) or a flag (the write group's command lines).

/// The name of `names` that `given` likely means: the only one it begins
/// (`--message` → `--message-file`), else the nearest at most two edits
/// away (the first of equally near ones).
pub fn nearest<'a>(given: &str, names: &[&'a str]) -> Option<&'a str> {
    let mut begun = names.iter().filter(|name| given.len() > 2 && name.starts_with(given));
    let only = begun.next().filter(|_| begun.next().is_none()).copied();
    only.or_else(|| {
        names
            .iter()
            .map(|name| (distance(given, name), *name))
            .filter(|(far, _)| *far <= 2)
            .min_by_key(|(far, _)| *far)
            .map(|(_, name)| name)
    })
}

/// The edits from `a` to `b` (Levenshtein).
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, left) in a.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, right) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = (above + 1).min(row[j] + 1).min(diagonal + usize::from(left != *right));
            diagonal = above;
        }
    }
    row[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_the_nearest_within_two_edits() {
        assert_eq!(nearest("wt:actoin", &["wt:list", "wt:action"]), Some("wt:action"));
        assert_eq!(nearest("--dryrun", &["--update", "--dry-run"]), Some("--dry-run"));
        assert_eq!(nearest("--hour", &["--then", "--hours"]), Some("--hours"));
        assert_eq!(nearest("--message", &["--message-file"]), Some("--message-file"));
        assert_eq!(nearest("wt:a", &["wt:action", "wt:actions"]), None);
        assert_eq!(nearest("--bogus", &["--url", "--run"]), None);
        assert_eq!(distance("", "ab"), 2);
    }
}
