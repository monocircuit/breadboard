//! The settings of an orchestration's overview (#415): what the overview
//! and the run engine do by themselves, switched in its settings panel.
//! The team's defaults are breadboard.toml's `[settings]`; a change in the
//! panel is kept per product (`.claude/products/<name>.json`, local, never
//! committed), for an orchestration of no product in its record, and wins
//! over the default until it is reset. Read on every tick: a change holds
//! from the next one, without a restart. The desktop notifications (#449)
//! are the repo's, not an orchestration's: their changes are kept in
//! `.claude/notifications.json`, their defaults are `[notifications]`.

use crate::config::Config;
use crate::json;
use crate::record::{self, product_file, record_file};
use serde_json::{Map, Value, json};
use std::io;
use std::path::{Path, PathBuf};

/// What a setting holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// On or off: a click switches it.
    Switch,
    /// A whole number in `min..=max`, `unit` after it: `−` and `+` step it.
    Count { unit: &'static str, min: i64, max: i64 },
}

/// A setting: its key (in `[settings]` and the overrides), its label in the panel, what it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Def {
    pub key: &'static str,
    pub label: &'static str,
    /// One line: what it does when on (or what its number means).
    pub about: &'static str,
    pub kind: Kind,
    pub scope: Scope,
}

/// Whose a setting is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// An orchestration's, kept per product.
    Product,
    /// The repo's, the same in every orchestration (the notifications, #449).
    Repo,
}

pub const BATCHING: &str = "batching";
pub const AUTO_MERGE: &str = "auto_merge_into_batch";
pub const SHIP_TASKS: &str = "ship_after_tasks";
pub const SHIP_DAYS: &str = "ship_after_days";
pub const TRY_CHECKER: &str = "try_checker";
pub const AUTO_FRESH: &str = "auto_fresh";
// In thousands of tokens: a panel's change of the percent before (#381) is no value of these.
pub const AGENT_FRESH: &str = "agent_fresh_above_k";
pub const AGENT_IDLE_FRESH: &str = "agent_idle_fresh_above_k";
pub const AGENT_IDLE_MINUTES: &str = "agent_idle_minutes";

/// Any desktop notification (#449); each event's own is `notify_<event>`.
pub const NOTIFICATIONS: &str = "notifications";

/// Every setting, in the panel's order.
pub const DEFS: [Def; 17] = [
    Def {
        key: BATCHING,
        label: "batching",
        about: "tasks branch off a batch branch and merge into it; off: their MRs go straight into develop",
        kind: Kind::Switch,
        scope: Scope::Product,
    },
    Def {
        key: AUTO_MERGE,
        label: "auto-merge into batch",
        about: "a task MR merges into the batch by itself once its bar is full and its checks are green; off: it waits for your click",
        kind: Kind::Switch,
        scope: Scope::Product,
    },
    Def {
        key: SHIP_TASKS,
        label: "ship suggestion: tasks",
        about: "the overview suggests shipping the batch once this many tasks are in it",
        kind: Kind::Count {
            unit: "tasks",
            min: 1,
            max: 99,
        },
        scope: Scope::Product,
    },
    Def {
        key: SHIP_DAYS,
        label: "ship suggestion: age",
        about: "or once its oldest task has waited in it this long",
        kind: Kind::Count {
            unit: "days",
            min: 1,
            max: 30,
        },
        scope: Scope::Product,
    },
    Def {
        key: TRY_CHECKER,
        label: "try checker",
        about: "a small model checks and answers the tries (auto mode, #384); off: they wait for you",
        kind: Kind::Switch,
        scope: Scope::Product,
    },
    Def {
        key: AUTO_FRESH,
        label: "orchestrator fresh start",
        about: "the overview starts the orchestrator's chat fresh at a quiet point by itself; off: it offers a click",
        kind: Kind::Switch,
        scope: Scope::Product,
    },
    Def {
        key: AGENT_FRESH,
        label: "agent fresh start",
        about: "an agent past this much context retires at its merge, its next issue goes to a fresh one with its handoff (#381, #445)",
        kind: Kind::Count {
            unit: "k tokens",
            min: 10,
            max: 1000,
        },
        scope: Scope::Product,
    },
    Def {
        key: AGENT_IDLE_FRESH,
        label: "agent fresh start when idle",
        about: "past this much context it retires too once idle past the time below: its cache is gone, a resume writes it all again (#445)",
        kind: Kind::Count {
            unit: "k tokens",
            min: 10,
            max: 1000,
        },
        scope: Scope::Product,
    },
    Def {
        key: AGENT_IDLE_MINUTES,
        label: "agent idle time",
        about: "how long idle that is: a little under the prompt cache's hour (#445)",
        kind: Kind::Count {
            unit: "min",
            min: 1,
            max: 600,
        },
        scope: Scope::Product,
    },
    Def {
        key: NOTIFICATIONS,
        label: "desktop notifications",
        about: "a desktop notification (notify-send) for each event switched on below, from one overview of the repo; off: none",
        kind: Kind::Switch,
        scope: Scope::Repo,
    },
    notify_def("notify_issue_created", "· issue created", "an issue is created, by anyone"),
    notify_def("notify_mr_opened", "· MR opened", "an MR is opened"),
    notify_def("notify_mr_merged", "· MR merged", "an MR is merged: into develop, an epic or a batch"),
    notify_def("notify_mr_ready", "· MR ready", "an MR is marked ready"),
    notify_def("notify_pipeline_failed", "· pipeline failed", "an MR's pipeline failed"),
    notify_def("notify_mr_conflict", "· MR conflict", "an MR got a conflict with its target"),
    notify_def("notify_decide", "· asks you", "an agent or an orchestrator asks you (a decide is posted)"),
];

/// The switch of one notification event: `notify_<event>`.
const fn notify_def(key: &'static str, label: &'static str, about: &'static str) -> Def {
    Def {
        key,
        label,
        about,
        kind: Kind::Switch,
        scope: Scope::Repo,
    }
}

/// The setting of the event `event` (`notify::EVENTS`).
pub fn notify_key(event: &str) -> String {
    format!("notify_{event}")
}

/// The setting of `key`.
pub fn def_of(key: &str) -> Option<&'static Def> {
    DEFS.iter().find(|def| def.key == key)
}

/// The team's defaults as breadboard.toml says them; what it leaves out
/// holds as built in (batching on, #411; fresh starts `[orchestrator] auto_fresh`).
pub fn defaults_of(config: &Config) -> Map<String, Value> {
    let given = &config.settings;
    let mut out = Map::new();
    out.insert(BATCHING.into(), json!(given.batching.unwrap_or(true)));
    out.insert(AUTO_MERGE.into(), json!(given.auto_merge_into_batch.unwrap_or(true)));
    out.insert(SHIP_TASKS.into(), json!(given.ship_after_tasks.unwrap_or(5)));
    out.insert(SHIP_DAYS.into(), json!(given.ship_after_days.unwrap_or(2)));
    out.insert(TRY_CHECKER.into(), json!(given.try_checker.unwrap_or(false)));
    out.insert(
        AUTO_FRESH.into(),
        json!(given.auto_fresh.unwrap_or(config.orchestrator.auto_fresh)),
    );
    out.insert(AGENT_FRESH.into(), json!(config.agents.fresh_above / 1000));
    out.insert(AGENT_IDLE_FRESH.into(), json!(config.agents.idle_fresh_above / 1000));
    out.insert(AGENT_IDLE_MINUTES.into(), json!(config.agents.idle_minutes));
    let notifications = &config.notifications;
    out.insert(NOTIFICATIONS.into(), json!(notifications.desktop));
    for event in crate::notify::EVENTS {
        let is_on = notifications
            .events
            .as_ref()
            .is_none_or(|events| events.iter().any(|key| key == event.key));
        out.insert(notify_key(event.key), json!(is_on));
    }
    out
}

/// Where the overrides of the orchestration `name` are kept: its product's
/// file, else its record.
fn file_of(main: &Path, name: &str) -> PathBuf {
    match record::product_of(main, name) {
        Some(product) => product_file(main, &product),
        None => record_file(main, name),
    }
}

/// Where the overrides of the repo's settings are kept (`Scope::Repo`, #449).
pub fn repo_file(main: &Path) -> PathBuf {
    main.join(".claude").join("notifications.json")
}

/// What holds for a new orchestration of `product` (none yet recorded):
/// the product's overrides over the team's defaults. With no product: the
/// repo's settings as they hold (the daemon's notifications).
pub fn of_product(main: &Path, product: Option<&str>) -> Settings {
    let mut overrides = product
        .map(|product| of_scope(overrides_in(&product_file(main, product)), Scope::Product))
        .unwrap_or_default();
    overrides.extend(of_scope(overrides_in(&repo_file(main)), Scope::Repo));
    settings_of(&rows_from(main, &overrides))
}

/// Only the overrides of settings of `scope`.
fn of_scope(overrides: Map<String, Value>, scope: Scope) -> Map<String, Value> {
    overrides
        .into_iter()
        .filter(|(key, _)| def_of(key).is_some_and(|def| def.scope == scope))
        .collect()
}

/// The overrides the panel wrote, only those of a known setting with a value of its kind.
pub fn overrides_of(main: &Path, name: &str) -> Map<String, Value> {
    let mut out = of_scope(overrides_in(&file_of(main, name)), Scope::Product);
    out.extend(of_scope(overrides_in(&repo_file(main)), Scope::Repo));
    // The auto mode of before (#384) was the record's `auto`: it holds until the panel says.
    if !out.contains_key(TRY_CHECKER)
        && let Some(auto) = json::read(&record_file(main, name))
            .and_then(|raw| raw.get("auto").and_then(Value::as_bool))
            .filter(|auto| *auto)
    {
        out.insert(TRY_CHECKER.into(), json!(auto));
    }
    out
}

/// The overrides kept in `file`.
fn overrides_in(file: &Path) -> Map<String, Value> {
    let mut out = Map::new();
    let raw = json::read(file);
    let given = raw
        .as_ref()
        .and_then(|raw| raw.get("settings"))
        .and_then(Value::as_object);
    for (key, value) in given.into_iter().flatten() {
        if def_of(key).is_some_and(|def| is_of_kind(def, value)) {
            out.insert(key.clone(), value.clone());
        }
    }
    out
}

fn is_of_kind(def: &Def, value: &Value) -> bool {
    match def.kind {
        Kind::Switch => value.is_boolean(),
        Kind::Count { min, max, .. } => value.as_i64().is_some_and(|n| (min..=max).contains(&n)),
    }
}

/// A setting as the panel shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub def: &'static Def,
    pub value: Value,
    pub default: Value,
    /// The panel set it: it differs from the team's default until reset.
    pub is_overridden: bool,
}

impl Row {
    /// `on`, `off`, `5 tasks`.
    pub fn text(&self) -> String {
        text_of(self.def, &self.value)
    }

    pub fn default_text(&self) -> String {
        text_of(self.def, &self.default)
    }
}

/// `on`, `off`, `5 tasks`.
pub fn text_of(def: &Def, value: &Value) -> String {
    match def.kind {
        Kind::Switch => if value.as_bool() == Some(true) { "on" } else { "off" }.into(),
        Kind::Count { unit, .. } => format!("{} {unit}", value.as_i64().unwrap_or(0)),
    }
}

/// Every setting of the orchestration `name`, as it holds now.
pub fn rows_of(main: &Path, name: &str) -> Vec<Row> {
    rows_from(main, &overrides_of(main, name))
}

fn rows_from(main: &Path, overrides: &Map<String, Value>) -> Vec<Row> {
    let defaults = defaults_of(&crate::config::of(main).config);
    DEFS.iter()
        .map(|def| {
            let default = defaults.get(def.key).cloned().unwrap_or(Value::Null);
            let given = overrides.get(def.key).cloned();
            Row {
                def,
                is_overridden: given.is_some(),
                value: given.unwrap_or_else(|| default.clone()),
                default,
            }
        })
        .collect()
}

/// What holds for the orchestration `name`, read where it acts.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub batching: bool,
    pub auto_merge_into_batch: bool,
    pub ship_after_tasks: i64,
    pub ship_after_days: i64,
    pub try_checker: bool,
    pub auto_fresh: bool,
    /// When an agent starts fresh (#445): `agent fresh start`, `… when idle`
    /// and `agent idle time`, by default `[agents]`'s.
    pub agent_fresh: crate::fresh::Fresh,
    /// The events that send a desktop notification (#449, `notify::EVENTS`'s keys): none while they are off.
    pub notify: Vec<&'static str>,
}

pub fn of(main: &Path, name: &str) -> Settings {
    settings_of(&rows_of(main, name))
}

/// The settings `rows` hold (an orchestration's panel rows, `OrchestrationLocal::settings`).
pub fn settings_of(rows: &[Row]) -> Settings {
    let value = |key: &str| rows.iter().find(|row| row.def.key == key).map(|row| row.value.clone());
    let switch = |key: &str| value(key).and_then(|value| value.as_bool()).unwrap_or(false);
    let count = |key: &str| value(key).and_then(|value| value.as_i64()).unwrap_or(0);
    Settings {
        batching: switch(BATCHING),
        auto_merge_into_batch: switch(AUTO_MERGE),
        ship_after_tasks: count(SHIP_TASKS),
        ship_after_days: count(SHIP_DAYS),
        try_checker: switch(TRY_CHECKER),
        auto_fresh: switch(AUTO_FRESH),
        agent_fresh: crate::fresh::Fresh {
            above: count(AGENT_FRESH).max(0) as u64 * 1000,
            idle_above: count(AGENT_IDLE_FRESH).max(0) as u64 * 1000,
            idle_ms: count(AGENT_IDLE_MINUTES) * 60_000,
        },
        notify: crate::notify::EVENTS
            .iter()
            .filter(|event| switch(NOTIFICATIONS) && switch(&notify_key(event.key)))
            .map(|event| event.key)
            .collect(),
    }
}

/// The setting `key` of the orchestration `name` set to `value` by the
/// panel, None: back to the team's default. Kept where its overrides are,
/// the line written to the orchestration's journal: `auto-merge into batch:
/// off (by you)`. Err for an unknown key or a value not of its kind.
pub fn set(main: &Path, name: &str, key: &str, value: Option<Value>, at: i64) -> io::Result<String> {
    let invalid = |why: String| io::Error::new(io::ErrorKind::InvalidInput, why);
    let def = def_of(key).ok_or_else(|| invalid(format!("no setting {key}")))?;
    if let Some(value) = value.as_ref().filter(|value| !is_of_kind(def, value)) {
        return Err(invalid(format!("{} takes no {value}", def.label)));
    }
    let file = match def.scope {
        Scope::Product => file_of(main, name),
        Scope::Repo => repo_file(main),
    };
    let mut raw = match json::read(&file) {
        Some(Value::Object(raw)) => raw,
        _ if file == record_file(main, name) => {
            return Err(invalid(format!("{name} has no record")));
        }
        _ => Map::new(),
    };
    let mut settings = raw
        .get("settings")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let line = match &value {
        Some(value) => {
            settings.insert(key.into(), value.clone());
            format!("{}: {} (by you)", def.label, text_of(def, value))
        }
        None => {
            settings.remove(key);
            let default = defaults_of(&crate::config::of(main).config)
                .get(key)
                .cloned()
                .unwrap_or(Value::Null);
            format!("{}: {} (the default again)", def.label, text_of(def, &default))
        }
    };
    raw.insert("settings".into(), Value::Object(settings));
    // The auto mode of before goes with the first change of the try checker.
    if key == TRY_CHECKER && file == record_file(main, name) {
        raw.remove("auto");
    }
    json::write_pretty(&file, &Value::Object(raw))?;
    if key == TRY_CHECKER && file != record_file(main, name) {
        let record = record_file(main, name);
        if let Some(Value::Object(mut raw)) = json::read(&record)
            && raw.remove("auto").is_some()
        {
            json::write_pretty(&record, &Value::Object(raw))?;
        }
    }
    let _ = crate::journal::append(main, name, &format!("settings: {line}"), at);
    Ok(line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lock::scratch::Scratch;
    use std::fs;

    fn world(toml: &str) -> (Scratch, PathBuf) {
        let scratch = Scratch::new();
        let main = scratch.join("platform");
        fs::create_dir_all(main.join(".claude/orchestrations")).unwrap();
        fs::write(main.join(crate::config::FILE), toml).unwrap();
        (scratch, main)
    }

    fn record(main: &Path, name: &str, raw: Value) {
        fs::write(record_file(main, name), raw.to_string()).unwrap();
    }

    #[test]
    fn the_defaults_come_from_the_file_and_what_it_leaves_out() {
        let config = Config::parse("").unwrap();
        let defaults = defaults_of(&config);
        assert_eq!(defaults[BATCHING], json!(true), "the product's batch (#411)");
        assert_eq!(defaults[AUTO_MERGE], json!(true));
        assert_eq!(
            (defaults[SHIP_TASKS].clone(), defaults[SHIP_DAYS].clone()),
            (json!(5), json!(2))
        );
        assert_eq!(defaults[TRY_CHECKER], json!(false));
        assert_eq!(
            (
                defaults[AGENT_FRESH].clone(),
                defaults[AGENT_IDLE_FRESH].clone(),
                defaults[AGENT_IDLE_MINUTES].clone()
            ),
            (json!(150), json!(100), json!(55)),
            "[agents]"
        );
        let config =
            Config::parse("[settings]\nbatching = false\nauto_merge_into_batch = false\nship_after_tasks = 8\n")
                .unwrap();
        let defaults = defaults_of(&config);
        assert_eq!(defaults[BATCHING], json!(false));
        assert_eq!(defaults[AUTO_MERGE], json!(false));
        assert_eq!(defaults[SHIP_TASKS], json!(8));
        let config = Config::parse("[settings]\nbatching = true\n").unwrap();
        assert_eq!(defaults_of(&config)[BATCHING], json!(true));
    }

    #[test]
    fn a_change_holds_per_product_until_reset_and_goes_to_the_journal() {
        let (_scratch, main) =
            world("[products.breadboard]\npaths = [\"packages/breadboard\"]\n[settings]\nbatching = true\n");
        record(&main, "a", json!({ "issues": [], "product": "breadboard" }));
        record(&main, "b", json!({ "issues": [], "product": "breadboard" }));
        record(&main, "alone", json!({ "issues": [] }));
        assert!(of(&main, "a").auto_merge_into_batch);
        let line = set(&main, "a", AUTO_MERGE, Some(json!(false)), 0).unwrap();
        assert_eq!(line, "auto-merge into batch: off (by you)");
        // The product's: its other orchestration too, one of no product not.
        assert!(!of(&main, "b").auto_merge_into_batch);
        assert!(of(&main, "alone").auto_merge_into_batch);
        let row = rows_of(&main, "b")
            .into_iter()
            .find(|row| row.def.key == AUTO_MERGE)
            .unwrap();
        assert!(row.is_overridden);
        assert_eq!((row.text(), row.default_text()), ("off".into(), "on".into()));
        assert!(
            crate::journal::entries(&main, "a")
                .last()
                .unwrap()
                .ends_with("settings: auto-merge into batch: off (by you)")
        );
        let line = set(&main, "b", AUTO_MERGE, None, 0).unwrap();
        assert_eq!(line, "auto-merge into batch: on (the default again)");
        assert!(of(&main, "a").auto_merge_into_batch);
        assert!(!rows_of(&main, "a").iter().any(|row| row.is_overridden));
        assert_eq!(of(&main, "a").agent_fresh, crate::fresh::Fresh::default());
        let line = set(&main, "a", AGENT_FRESH, Some(json!(200)), 0).unwrap();
        assert_eq!(line, "agent fresh start: 200 k tokens (by you)");
        set(&main, "a", AGENT_IDLE_MINUTES, Some(json!(30)), 0).unwrap();
        let fresh = of(&main, "b").agent_fresh;
        assert_eq!(
            (fresh.above, fresh.idle_above, fresh.idle_ms),
            (200_000, 100_000, 30 * 60_000)
        );
        // One of no product keeps its own, in its record.
        set(&main, "alone", SHIP_TASKS, Some(json!(3)), 0).unwrap();
        assert_eq!(of(&main, "alone").ship_after_tasks, 3);
        assert_eq!(of(&main, "a").ship_after_tasks, 5);
        // Wrong values are refused.
        assert!(set(&main, "a", SHIP_TASKS, Some(json!(0)), 0).is_err());
        assert!(set(&main, "a", BATCHING, Some(json!(3)), 0).is_err());
        assert!(set(&main, "a", "nothing", Some(json!(true)), 0).is_err());
    }

    #[test]
    fn the_notifications_are_the_repos_from_the_file_until_the_panel_says() {
        let (_scratch, main) = world("[products.breadboard]\npaths = [\"packages/breadboard\"]\n");
        record(&main, "a", json!({ "issues": [], "product": "breadboard" }));
        record(&main, "alone", json!({ "issues": [] }));
        assert_eq!(of(&main, "a").notify.len(), crate::notify::EVENTS.len(), "every one by default");
        for event in crate::notify::EVENTS {
            assert_eq!(def_of(&notify_key(event.key)).map(|def| def.scope), Some(Scope::Repo));
        }
        set(&main, "a", &notify_key(crate::notify::MR_OPENED), Some(json!(false)), 0).unwrap();
        // The repo's: every orchestration's, and the daemon's (no product).
        for settings in [of(&main, "alone"), of_product(&main, None)] {
            assert!(!settings.notify.contains(&crate::notify::MR_OPENED));
            assert!(settings.notify.contains(&crate::notify::MR_MERGED));
        }
        assert!(json::read(&repo_file(&main)).is_some());
        set(&main, "alone", NOTIFICATIONS, Some(json!(false)), 0).unwrap();
        assert!(of(&main, "a").notify.is_empty(), "off: none");
        // The team's default: off, or only some events.
        let (_scratch, main) = world("[notifications]\ndesktop = false\n");
        assert!(of_product(&main, None).notify.is_empty());
        let (_scratch, main) = world("[notifications]\nevents = [\"decide\", \"mr_merged\"]\n");
        assert_eq!(
            of_product(&main, None).notify,
            vec![crate::notify::MR_MERGED, crate::notify::DECIDE]
        );
        assert!(Config::parse("[notifications]\nevents = [\"nothing\"]\n").is_err());
    }

    #[test]
    fn the_auto_mode_of_before_is_the_try_checker_until_the_panel_says() {
        let (_scratch, main) = world("");
        record(&main, "old", json!({ "issues": [], "auto": true }));
        assert!(of(&main, "old").try_checker);
        set(&main, "old", TRY_CHECKER, Some(json!(false)), 0).unwrap();
        assert!(!of(&main, "old").try_checker);
        let raw = json::read(&record_file(&main, "old")).unwrap();
        assert!(raw.get("auto").is_none() && raw["issues"].is_array(), "{raw}");
    }
}
