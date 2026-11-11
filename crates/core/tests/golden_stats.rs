//! Golden cases of the persisted statistics.

mod common;

use asset_hub_core::stats::{budget_for, image_size, is_stats_summary, keep_card_versions, merge_stats, newer_card, summarize_stats, summary_issues, ModelStats, PartialBudget, PlayableBudget, StatsSource, StatsSummary};
use common::*;
use serde_json::{json, Value};

fn summary(v: &Value) -> Option<StatsSummary> {
    (!v.is_null()).then(|| from(v))
}

#[test]
fn stats_match() {
    let dir = "core/checks/playable";
    run(dir, "isStatsSummary", |a| json!(is_stats_summary(&a[0])));
    run(dir, "mergeStats", |a| serde_json::to_value(merge_stats(summary(&a[0]).as_ref(), &summary(&a[1]).expect("summary"))).expect("serialize"));
    run(dir, "keepCardVersions", |a| serde_json::to_value(keep_card_versions(summary(&a[0]).as_ref(), &summary(&a[1]).expect("summary"))).expect("serialize"));
    run(dir, "newerCard", |a| json!(newer_card(summary(&a[0]).as_ref(), summary(&a[1]).as_ref())));
}

#[test]
fn budgets_and_summaries_match() {
    let dir = "core/checks/playable";
    run(dir, "budgetFor", |a| {
        let custom: Option<PartialBudget> = a.get(1).filter(|v| !v.is_null()).map(from);
        serde_json::to_value(budget_for(a[0].as_str(), custom.as_ref())).expect("serialize")
    });
    run(dir, "summarizeStats", |a| {
        let source = if s(&a[1]) == "thumb" { StatsSource::Thumb } else { StatsSource::Viewer };
        serde_json::to_value(summarize_stats(&from::<ModelStats>(&a[0]), source)).expect("serialize")
    });
    run(dir, "summaryIssues", |a| {
        let budget: PlayableBudget = a.get(1).filter(|v| !v.is_null()).map(from).unwrap_or(asset_hub_core::stats::PLAYABLE_BUDGET);
        serde_json::to_value(summary_issues(&from::<StatsSummary>(&a[0]), &budget)).expect("serialize")
    });
    run(dir, "imageSize", |a| {
        let path = a[0]["$file"].as_str().expect("file");
        json!(image_size(path, &file_bytes(path)))
    });
}
