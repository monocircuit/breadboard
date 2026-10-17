//! The product of an orchestration (#405): `--product <name>`, else the one
//! its issues' milestones name (`vita · Epic 12: …`), kept in its record.
//! Every orchestration of a product works in the product's herdr space.

use super::issue::{Item, object};
use crate::cli::setup::repo;
use breadboard_core::config::Config;
use breadboard_core::record;
use serde_json::json;

/// The product the orchestration `name` belongs to, written into its record
/// (not on a dry run): `given`, else the record's, else the one its issues'
/// milestones share; None for a repo without `[products.*]` or issues of
/// none. Err why it can't be told.
pub fn ensure_product(name: &str, plan: &[Item], given: Option<&str>, is_dry: bool) -> Result<Option<String>, String> {
    let main = repo::main_dir();
    let config = &breadboard_core::config::of(&main).config;
    let known = record::record_of(&main, name).and_then(|found| found.product);
    let milestones: Vec<Option<String>> = plan.iter().map(|item| item.milestone.clone()).collect();
    let Some(product) = product_for(config, given, known.as_deref(), &milestones)? else {
        return Ok(None);
    };
    if !is_dry && known.as_deref() != Some(product.as_str()) {
        record::update_record(&main, name, object(json!({ "product": product })))
            .map_err(|error| format!("the record of {name} not written: {error}"))?;
    }
    Ok(Some(product))
}

/// What [`ensure_product`] decides: `given` when breadboard.toml names it,
/// else `known` (the record's), else the product of the milestones that
/// name one, when they agree.
fn product_for(
    config: &Config,
    given: Option<&str>,
    known: Option<&str>,
    milestones: &[Option<String>],
) -> Result<Option<String>, String> {
    let names = || config.products.keys().cloned().collect::<Vec<_>>().join(", ");
    if let Some(given) = given {
        if !config.products.contains_key(given) {
            return Err(if config.products.is_empty() {
                format!("--product {given}: breadboard.toml names no products ([products.<name>])")
            } else {
                format!("--product {given} is no product of breadboard.toml: {}", names())
            });
        }
        return Ok(Some(given.to_string()));
    }
    if let Some(known) = known.filter(|known| config.products.contains_key(*known)) {
        return Ok(Some(known.to_string()));
    }
    let mut found: Vec<String> = milestones
        .iter()
        .flatten()
        .filter_map(|milestone| config.product_of_milestone(milestone))
        .collect();
    found.sort();
    found.dedup();
    match found.as_slice() {
        [] => Ok(None),
        [one] => Ok(Some(one.clone())),
        _ => Err(format!(
            "the issues' milestones name the products {}: name the orchestration's with --product <name>",
            found.join(" and ")
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        Config::parse(
            "[products.vita]\npaths = [\"products/vita\"]\n[products.breadboard]\npaths = [\"packages/breadboard\"]\n",
        )
        .unwrap()
    }

    fn milestones(titles: &[&str]) -> Vec<Option<String>> {
        titles.iter().map(|title| Some(title.to_string())).collect()
    }

    #[test]
    fn the_product_comes_from_the_flag_the_record_or_the_milestones() {
        let config = config();
        let vita = milestones(&["vita · Epic 13: PWA", "vita · Epic 13: PWA"]);
        assert_eq!(
            product_for(&config, None, None, &vita).unwrap().as_deref(),
            Some("vita")
        );
        assert_eq!(
            product_for(&config, Some("breadboard"), None, &vita)
                .unwrap()
                .as_deref(),
            Some("breadboard")
        );
        assert_eq!(
            product_for(&config, None, Some("breadboard"), &vita)
                .unwrap()
                .as_deref(),
            Some("breadboard")
        );
        // An issue without a milestone, or with one of no product, says nothing.
        let some = vec![None, Some("vita · MVP".to_string()), Some("Backlog".to_string())];
        assert_eq!(
            product_for(&config, None, None, &some).unwrap().as_deref(),
            Some("vita")
        );
        assert_eq!(product_for(&config, None, None, &[None]).unwrap(), None);
        let mixed = milestones(&["vita · MVP", "breadboard · Epic 15: The dev workflow"]);
        let error = product_for(&config, None, None, &mixed).unwrap_err();
        assert!(
            error.starts_with("the issues' milestones name the products breadboard and vita"),
            "{error}"
        );
        let error = product_for(&config, Some("app"), None, &vita).unwrap_err();
        assert_eq!(
            error,
            "--product app is no product of breadboard.toml: breadboard, vita"
        );
        let error = product_for(&Config::default(), Some("app"), None, &vita).unwrap_err();
        assert!(error.contains("names no products"), "{error}");
        assert_eq!(product_for(&Config::default(), None, None, &vita).unwrap(), None);
    }
}
