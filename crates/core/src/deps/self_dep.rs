//! A file that needs nothing else (`src/core/deps/self.ts`).

use crate::deps::context::DepContext;
use crate::deps::types::{DependencyReport, DependencyWarning, ReportKind};
use crate::errors::SourceError;

/// A texture, an audio clip or the like: itself and its `.meta`.
pub async fn resolve_self<C: DepContext>(path: &str, ctx: &C) -> Result<DependencyReport, SourceError> {
    let meta = format!("{path}.meta");
    let has_meta = ctx.by_path().has(&meta);
    let mut report = DependencyReport::new(ReportKind::None);
    if has_meta {
        report.files = vec![path.to_string(), meta];
    } else {
        report.warnings = vec![DependencyWarning::NoMeta];
        report.files = vec![path.to_string()];
    }
    Ok(report)
}
