//! A TextMeshPro font asset (`src/core/deps/tmpFont.ts`).

use crate::deps::context::{unique, DepContext, GuidKind, OrderedSet};
use crate::deps::self_dep::resolve_self;
use crate::deps::types::{DependencyReport, PartKind, PartRef};
use crate::errors::SourceError;
use crate::js::slice_units;
use crate::registry::base_name;
use crate::unity::tmp_font::parse_tmp_font;

/// A TextMeshPro font asset: itself (its atlas and material live inside it), the font assets it falls
/// back to, and, when it is dynamic and adds glyphs at run time, the font file it was made from.
pub async fn resolve_tmp_font_dependencies<C: DepContext>(path: &str, ctx: &C) -> Result<DependencyReport, SourceError> {
    let report = resolve_self(path, ctx).await?;
    let text = ctx.read_texts(&[path.to_string()]).await?.remove(path).unwrap_or_default();
    let Some(info) = parse_tmp_font(&text) else { return Ok(report) };
    let source = info.source_font.clone().filter(|s| info.dynamic && !s.is_empty());
    let guids = unique(source.into_iter().chain(info.fallbacks.iter().cloned()));
    if guids.is_empty() {
        return Ok(report);
    }
    ctx.prepare(&guids, GuidKind::Other, &[]).await?;
    let mut files: OrderedSet<String> = OrderedSet::default();
    files.extend(report.files.iter().cloned());
    let parts = guids
        .into_iter()
        .map(|guid| {
            let p = ctx.path_for_guid(&guid);
            if let Some(p) = &p {
                files.add(p.clone());
                let meta = format!("{p}.meta");
                if ctx.by_path().has(&meta) {
                    files.add(meta);
                }
            }
            PartRef {
                kind: PartKind::Other,
                name: p.as_deref().map_or_else(|| slice_units(&guid, 0, 8).to_string(), |p| base_name(p).to_string()),
                size: p.as_deref().and_then(|p| ctx.by_path().get(p)).and_then(|e| e.size),
                missing: p.is_none(),
                path: p,
                unsure: None,
                uses: 1,
                guid,
            }
        })
        .collect();
    Ok(DependencyReport { files: files.into_vec(), parts: Some(parts), ..report })
}
