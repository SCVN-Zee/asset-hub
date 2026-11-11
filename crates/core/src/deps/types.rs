//! Shapes of a dependency report (`src/core/deps/types.ts`).

use serde::{Deserialize, Serialize};

use crate::unity::particles::ParticleEmitter;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShaderKind {
    Builtin,
    Package,
    Project,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShaderRef {
    pub guid: String,
    pub label: String,
    pub kind: ShaderKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MatchedBy {
    Guid,
    Name,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextureRef {
    /// Material slot, e.g. `_BaseMap`; absent for names found inside the FBX.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prop: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guid: Option<String>,
    /// Repo path when found.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub name: String,
    /// How it was found: by GUID (exact) or by file name (heuristic, needs confirmation).
    pub matched_by: MatchedBy,
    pub missing: bool,
    /// Not found yet, but the project was not fully searched: may still turn up.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unsure: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaterialRef {
    pub name: String,
    /// Material name inside the FBX that this `.mat` replaces (externalObjects), used to place textures in the preview.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub missing: bool,
    /// Not found yet, but the project was not fully searched: may still turn up.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unsure: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shader: Option<ShaderRef>,
    pub textures: Vec<TextureRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DependencyWarning {
    MissingMaterial,
    MissingTexture,
    TextureByName,
    CustomShader,
    NoMeta,
    NoMaterials,
    MissingModel,
    CustomScript,
    Particles,
    UiPrefab,
}

/// `known` only names prefabs read before that use the model (nothing loaded); `cached` also loads
/// their materials; `near` reads related folders; `all` reads every prefab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PrefabScope {
    Known,
    Cached,
    Near,
    All,
}

/// A prefab that uses a model, and the materials its renderers put on that model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrefabUsage {
    pub path: String,
    pub materials: Vec<MaterialRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrefabMaterialsResult {
    pub scope: PrefabScope,
    pub prefabs: Vec<PrefabUsage>,
    /// Prefabs checked for this answer (scanned now or known from earlier scans).
    pub scanned: usize,
    /// Prefabs not checked yet, and their size: what "scan all" would add.
    pub remaining: usize,
    pub remaining_bytes: u64,
    /// Materials, textures and shaders (with `.meta`) to add to an export.
    pub files: Vec<String>,
    pub warnings: Vec<DependencyWarning>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PartKind {
    Model,
    Prefab,
    Script,
    Other,
}

/// A model, nested prefab, script or other asset a prefab uses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PartRef {
    pub kind: PartKind,
    pub guid: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub missing: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unsure: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// How many times the prefab (and the prefabs inside it) uses it.
    pub uses: u32,
}

/// `A`: materials extracted to `.mat`; `B`: embedded in the FBX; `none`: material import disabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReportKind {
    A,
    B,
    #[serde(rename = "none")]
    None,
}

/// `.mat` files next to an FBX that has no materials of its own: a guess, since nothing references them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Suggestion {
    pub material: MaterialRef,
    pub files: Vec<String>,
}

/// A particle system of a prefab; `prefab` names the nested prefab it comes from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrefabParticle {
    #[serde(flatten)]
    pub emitter: ParticleEmitter,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefab: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyReport {
    pub kind: ReportKind,
    pub materials: Vec<MaterialRef>,
    /// Textures referenced by name inside the FBX itself.
    pub embedded_textures: Vec<TextureRef>,
    /// Answered from the files nearest the model only; a full answer follows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partial: Option<bool>,
    pub warnings: Vec<DependencyWarning>,
    pub suggested: Vec<Suggestion>,
    /// Every repo path that an import would copy (assets and their `.meta` files).
    pub files: Vec<String>,
    /// Prefabs: models, nested prefabs, scripts and other assets, in order of use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parts: Option<Vec<PartRef>>,
    /// Animation clips: models that may carry the rig it animates, most likely first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rigs: Option<Vec<String>>,
    /// Prefabs: GUID to repo path of everything found, so a preview can put the prefab together.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guid_paths: Option<std::collections::BTreeMap<String, String>>,
    /// Prefabs: GUIDs of scripts not in the repo (from packages): the target project needs them too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_scripts: Option<Vec<String>>,
    /// Prefabs: their particle systems, nested prefabs' included.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub particles: Option<Vec<PrefabParticle>>,
}

impl DependencyReport {
    /// A report with nothing in it yet.
    pub fn new(kind: ReportKind) -> Self {
        Self {
            kind,
            materials: Vec::new(),
            embedded_textures: Vec::new(),
            partial: None,
            warnings: Vec::new(),
            suggested: Vec::new(),
            files: Vec::new(),
            parts: None,
            rigs: None,
            guid_paths: None,
            package_scripts: None,
            particles: None,
        }
    }
}
