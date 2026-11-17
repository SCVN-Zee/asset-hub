//! Fact kinds the desktop shares per repo. Must equal `SHARED_FACT_KINDS` in
//! `crates/core/src/github/source.rs`.

pub const FACT_KINDS: [&str; 5] = ["metaguids", "prefabmodels", "prefabfx3", "spritemeta", "tmpfont"];

/// Whether `name` is one of the names `/cache/:name` accepts: `catalog`, `stats` and `facts-<kind>`.
pub fn is_cache_object_name(name: &str) -> bool {
    name == "catalog" || name == "stats" || name.strip_prefix("facts-").is_some_and(|kind| FACT_KINDS.contains(&kind))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_catalog_stats_and_every_fact_kind() {
        assert!(is_cache_object_name("catalog"));
        assert!(is_cache_object_name("stats"));
        for kind in FACT_KINDS {
            assert!(is_cache_object_name(&format!("facts-{kind}")));
        }
    }

    #[test]
    fn rejects_everything_else() {
        for name in ["facts-other", "facts", "facts-", "secrets", "catalog.json", "Catalog", ""] {
            assert!(!is_cache_object_name(name), "{name:?}");
        }
    }
}
