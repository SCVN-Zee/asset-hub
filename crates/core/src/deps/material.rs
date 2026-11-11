//! A `.mat` on its own (`src/core/deps/material.ts`).

use std::collections::HashMap;

use crate::deps::context::{DepContext, GuidOverlay, OrderedSet};
use crate::deps::fbx::{describe_materials, MaterialQuery};
use crate::deps::types::{DependencyReport, DependencyWarning, ReportKind};
use crate::errors::SourceError;
use crate::unity::yaml::meta_guid;

/// A `.mat` on its own: its shader, textures (and a variant's parent), the same way an FBX's materials
/// are described. Its GUID comes from its `.meta`, so the material resolves even before the branch's
/// GUID index has it.
pub async fn resolve_material_dependencies<C: DepContext>(mat_path: &str, ctx: &C) -> Result<DependencyReport, SourceError> {
    let meta_path = format!("{mat_path}.meta");
    let guid = ctx.read_texts(std::slice::from_ref(&meta_path)).await?.get(&meta_path).and_then(|t| meta_guid(t));
    // Without a .meta there is no GUID: any placeholder resolves to the file itself.
    let own = guid.clone().unwrap_or_else(|| "self".to_string());
    let sub = GuidOverlay { inner: ctx, guids: HashMap::from([(own.clone(), mat_path.to_string())]) };
    let described = describe_materials(&[MaterialQuery { name: None, guid: own }], &sub).await?;
    let mut files: OrderedSet<String> = OrderedSet::default();
    files.add(mat_path.to_string());
    files.extend(described.files);
    let mut materials = described.materials;
    let partial = materials.first().is_some_and(|m| m.unsure == Some(true) || m.textures.iter().any(|t| t.unsure == Some(true)));
    let mut warnings = described.warnings;
    if guid.is_none() {
        if let Some(m) = materials.first_mut() {
            m.guid = None;
        }
        warnings.insert(0, DependencyWarning::NoMeta);
    }
    let mut report = DependencyReport::new(ReportKind::A);
    report.materials = materials;
    report.partial = partial.then_some(true);
    report.warnings = warnings;
    report.files = files.into_vec();
    Ok(report)
}
