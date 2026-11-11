//! The data types of the contract: what commands take and return, and what events carry.
//!
//! They mirror the interfaces of `src/shared/ipc.ts`, `src/shared/settings.ts` and `src/core`
//! field for field, with the same camelCase names and string values. A field that is optional there
//! is an `Option` here, left out of the JSON when it is `None`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use specta::Type;

use crate::ipc::IpcError;

/// Derives and the camelCase naming that every type of the contract shares.
macro_rules! dto {
    ($($item:item)*) => {
        $(
            #[skip_serializing_none]
            #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
            #[serde(rename_all = "camelCase")]
            $item
        )*
    };
}

// ---------------------------------------------------------------------------------------------
// Settings (`src/shared/settings.ts`, `src/shared/collections.ts`)
// ---------------------------------------------------------------------------------------------

dto! {
    /// Languages of the UI.
    #[derive(Copy, Eq)]
    pub enum Language {
        En,
        Ko,
        Vi,
    }

    /// Where exported packages put files in Unity.
    #[derive(Copy, Eq)]
    pub enum PackageLayout {
        Source,
        Grouped,
        ByType,
    }

    /// The playable budget assets are judged against.
    #[derive(Copy, Eq)]
    pub enum BudgetPreset {
        Off,
        Standard,
        Tight,
        Custom,
    }

    /// Limits of the `custom` preset; unset ones come from the default.
    #[derive(Default)]
    pub struct CustomBudget {
        pub triangles: Option<f64>,
        pub texture_size: Option<f64>,
        pub texture_bytes: Option<f64>,
        pub file_bytes: Option<f64>,
        pub bones: Option<f64>,
        pub audio_bytes: Option<f64>,
        pub audio_seconds: Option<f64>,
        pub font_bytes: Option<f64>,
        pub particles: Option<f64>,
    }

    /// One asset in a collection or a recent list.
    pub struct CollectionItem {
        pub repo: String,
        pub branch: String,
        /// Repo path (project root included), as links carry it.
        pub path: String,
        /// Content when added: the thumbnail and measurements cached for it.
        pub sha: String,
        pub name: String,
        /// Handler id (fbx, prefab...).
        #[serde(rename = "type")]
        pub kind: String,
        pub size: Option<f64>,
    }

    /// Favourites and named collections of assets.
    pub struct Collection {
        pub id: String,
        pub name: String,
        pub items: Vec<CollectionItem>,
    }

    /// Assets viewed and exported lately, newest first.
    pub struct RecentLists {
        pub viewed: Vec<CollectionItem>,
        pub exported: Vec<CollectionItem>,
    }

    /// A branch the user follows: new assets since `seen` (a commit) are pointed out.
    pub struct WatchedBranch {
        pub repo: String,
        pub branch: String,
        /// The commit everything up to which has been seen.
        pub seen: String,
    }

    /// The user's settings.
    pub struct Settings {
        pub language: Language,
        /// Where clones, downloaded files and thumbnails are cached.
        pub cache_dir: String,
        /// Most the downloaded files may take, in GB (absent: the default; 0: no limit).
        #[serde(rename = "cacheLimitGB")]
        pub cache_limit_gb: Option<u32>,
        /// Folder names hidden from browsing at any depth.
        pub excluded_folders: Vec<String>,
        /// Repo and branch opened last, restored on the next launch.
        pub last_repo: Option<String>,
        pub last_branch: Option<String>,
        /// Playable budget assets are judged against.
        pub budget_preset: Option<BudgetPreset>,
        pub custom_budget: Option<CustomBudget>,
        /// Where exported packages put files in Unity.
        pub export_layout: Option<PackageLayout>,
        /// Unity project packages are meant for (read only).
        pub target_project: Option<String>,
        /// Folder a team shares, holding an index everyone takes in.
        pub shared_index_dir: Option<String>,
        pub collections: Option<Vec<Collection>>,
        pub recent: Option<RecentLists>,
        pub watched: Option<Vec<WatchedBranch>>,
        /// Usage notes sent to the team's feedback repo (absent: on).
        pub usage_stats: Option<bool>,
    }
}

// ---------------------------------------------------------------------------------------------
// Source and assets (`src/core/types.ts`, `src/core/deps/types.ts`)
// ---------------------------------------------------------------------------------------------

dto! {
    pub struct RepoRef {
        /// "owner/name"
        pub full_name: String,
        pub default_branch: String,
    }

    #[derive(Copy, Eq)]
    pub enum TreeEntryKind {
        Blob,
        Tree,
    }

    /// One file or folder from a branch, shaped like GitHub's Git Trees API entries.
    pub struct TreeEntry {
        pub path: String,
        #[serde(rename = "type")]
        pub kind: TreeEntryKind,
        /// Blob SHA from git.
        pub sha: String,
        /// Size in bytes as stored in git.
        pub size: Option<f64>,
        /// True when the blob is a Git LFS pointer.
        pub lfs: Option<bool>,
        /// Real object size from the LFS pointer, when known.
        pub lfs_size: Option<f64>,
        /// sha256 of the real object from the LFS pointer.
        pub lfs_oid: Option<String>,
    }

    /// What `watchCheck` found: the tip commit and the asset files added since the last one seen.
    pub struct WatchCheck {
        pub head: String,
        pub added: Vec<String>,
        pub more: bool,
    }

    #[derive(Copy, Eq)]
    pub enum ShaderKind {
        Builtin,
        Package,
        Project,
    }

    pub struct ShaderRef {
        pub guid: String,
        pub label: String,
        pub kind: ShaderKind,
        pub path: Option<String>,
    }

    /// How a texture was found: by GUID (exact) or by file name (heuristic).
    #[derive(Copy, Eq)]
    pub enum MatchedBy {
        Guid,
        Name,
    }

    pub struct TextureRef {
        /// Material slot, e.g. _BaseMap; absent for names found inside the FBX.
        pub prop: Option<String>,
        pub guid: Option<String>,
        /// Repo path when found.
        pub path: Option<String>,
        pub name: String,
        pub matched_by: MatchedBy,
        pub missing: bool,
        /// Not found yet, but the project was not fully searched.
        pub unsure: Option<bool>,
        pub size: Option<f64>,
    }

    pub struct MaterialRef {
        pub name: String,
        /// Material name inside the FBX that this .mat replaces.
        pub source_name: Option<String>,
        pub guid: Option<String>,
        pub path: Option<String>,
        pub missing: bool,
        pub unsure: Option<bool>,
        pub shader: Option<ShaderRef>,
        pub textures: Vec<TextureRef>,
    }

    #[derive(Copy, Eq)]
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

    /// `known` only names prefabs read before that use the model; `cached` also loads their
    /// materials; `near` reads related folders; `all` reads every prefab.
    #[derive(Copy, Eq)]
    pub enum PrefabScope {
        Known,
        Cached,
        Near,
        All,
    }

    /// A prefab that uses a model, and the materials its renderers put on that model.
    pub struct PrefabUsage {
        pub path: String,
        pub materials: Vec<MaterialRef>,
    }

    pub struct PrefabMaterialsResult {
        pub scope: PrefabScope,
        pub prefabs: Vec<PrefabUsage>,
        /// Prefabs checked for this answer.
        pub scanned: u32,
        /// Prefabs not checked yet, and their size.
        pub remaining: u32,
        pub remaining_bytes: f64,
        /// Materials, textures and shaders (with .meta) to add to an export.
        pub files: Vec<String>,
        pub warnings: Vec<DependencyWarning>,
    }

    #[derive(Copy, Eq)]
    pub enum PartKind {
        Model,
        Prefab,
        Script,
        Other,
    }

    /// A model, nested prefab, script or other asset a prefab uses.
    pub struct PartRef {
        pub kind: PartKind,
        pub guid: String,
        pub name: String,
        pub path: Option<String>,
        pub missing: bool,
        pub unsure: Option<bool>,
        pub size: Option<f64>,
        /// How many times the prefab (and the prefabs inside it) uses it.
        pub uses: u32,
    }

    /// A: materials extracted to .mat; B: embedded in the FBX; none: material import disabled.
    #[derive(Copy, Eq)]
    pub enum DependencyKind {
        #[serde(rename = "A")]
        A,
        #[serde(rename = "B")]
        B,
        #[serde(rename = "none")]
        None,
    }

    /// A .mat next to an FBX that has no materials of its own, with the files it would add.
    pub struct SuggestedMaterial {
        pub material: MaterialRef,
        pub files: Vec<String>,
    }

    pub struct DependencyReport {
        pub kind: DependencyKind,
        pub materials: Vec<MaterialRef>,
        /// Textures referenced by name inside the FBX itself.
        pub embedded_textures: Vec<TextureRef>,
        /// Answered from the files nearest the model only; a full answer follows.
        pub partial: Option<bool>,
        pub warnings: Vec<DependencyWarning>,
        pub suggested: Vec<SuggestedMaterial>,
        /// Every repo path that an import would copy (assets and their .meta files).
        pub files: Vec<String>,
        /// Prefabs: models, nested prefabs, scripts and other assets, in order of use.
        pub parts: Option<Vec<PartRef>>,
        /// Animation clips: models that may carry the rig it animates, most likely first.
        pub rigs: Option<Vec<String>>,
        /// Prefabs: GUID to repo path of everything found.
        pub guid_paths: Option<HashMap<String, String>>,
        /// Prefabs: GUIDs of scripts not in the repo.
        pub package_scripts: Option<Vec<String>>,
        /// Prefabs: their particle systems, nested prefabs' included.
        pub particles: Option<Vec<PrefabParticle>>,
    }

    #[derive(Copy, Eq)]
    pub enum ResolveMode {
        Instant,
        Quick,
        Full,
    }

    /// Kinds of assets known only by reading them.
    #[derive(Copy, Eq)]
    pub enum FactKind {
        Effects,
        Sprites,
        TmpFonts,
    }

    /// A material of a model bundle: what Blender needs to rebuild it.
    pub struct BundleTexture {
        pub prop: String,
        pub path: String,
    }

    pub struct BundleMaterial {
        pub target: String,
        /// Name to give it in Blender (the .mat's).
        pub name: Option<String>,
        /// Repo path of the .mat.
        pub mat: Option<String>,
        /// Shader name, to tell unlit shaders.
        pub shader: Option<String>,
        pub textures: Vec<BundleTexture>,
    }

    /// `zip` asks where to save one .zip; `blender` writes a temporary folder and opens Blender.
    #[derive(Copy, Eq)]
    pub enum BundleMode {
        Zip,
        Blender,
    }

    pub struct SaveFileResult {
        pub canceled: bool,
        pub path: Option<String>,
        pub bytes: Option<f64>,
    }

    pub struct ModelBundleResult {
        pub canceled: bool,
        pub path: Option<String>,
        pub bytes: Option<f64>,
        pub textures: Option<u32>,
    }
}

// ---------------------------------------------------------------------------------------------
// Particle systems (`src/core/unity/particles.ts`, `particleSim.ts`, `animClip.ts`)
// ---------------------------------------------------------------------------------------------

dto! {
    /// One key of an animation curve; `v`, `inS` and `outS` have one number per component.
    pub struct CurveKey {
        pub t: f64,
        pub v: Vec<f64>,
        /// A step tangent is an infinite slope, which JSON writes as `null`.
        pub in_s: Vec<Option<f64>>,
        pub out_s: Vec<Option<f64>>,
    }

    /// Unity's MinMaxCurve: a constant, a curve (x scalar), random between two curves or two constants.
    pub struct MinMax {
        pub mode: f64,
        pub scalar: f64,
        pub min: f64,
        pub curve: Option<Vec<CurveKey>>,
        pub min_curve: Option<Vec<CurveKey>>,
    }

    pub struct GradientColor {
        pub t: f64,
        pub r: f64,
        pub g: f64,
        pub b: f64,
    }

    pub struct GradientAlpha {
        pub t: f64,
        pub a: f64,
    }

    pub struct Gradient {
        pub colors: Vec<GradientColor>,
        pub alphas: Vec<GradientAlpha>,
    }

    /// Unity's MinMaxGradient.
    pub struct MinMaxColor {
        pub mode: f64,
        /// r, g, b, a in 0-1.
        pub max: [f64; 4],
        pub min: [f64; 4],
        pub gradient: Option<Gradient>,
        pub min_gradient: Option<Gradient>,
    }

    pub struct ParticleShape {
        /// Unity's ParticleSystemShapeType.
        #[serde(rename = "type")]
        pub kind: f64,
        pub radius: f64,
        /// 0 emits from the edge only, 1 from the whole volume.
        pub radius_thickness: f64,
        /// Cone half angle, in degrees.
        pub angle: f64,
        /// Degrees of the circle used (360 = all).
        pub arc: f64,
        /// Cone volume length.
        pub length: f64,
        pub position: [f64; 3],
        /// Euler degrees.
        pub rotation: [f64; 3],
        pub scale: [f64; 3],
        pub random_direction: f64,
    }

    pub struct ParticleBurst {
        pub time: f64,
        pub count: MinMax,
        pub cycles: f64,
        pub interval: f64,
        pub probability: f64,
    }

    pub struct VelocityLimit {
        pub limit: MinMax,
        pub dampen: f64,
    }

    pub struct SheetAnimation {
        pub tiles_x: f64,
        pub tiles_y: f64,
        pub single_row: bool,
        pub row: f64,
        pub frame: MinMax,
        pub start_frame: MinMax,
        pub cycles: f64,
    }

    pub struct ParticleNoise {
        pub strength: MinMax,
        pub frequency: f64,
        pub scroll_speed: f64,
    }

    #[derive(Copy, Eq)]
    pub enum SubEmitterType {
        Birth,
        Collision,
        Death,
        Trigger,
        Manual,
    }

    pub struct SubEmitter {
        pub id: String,
        #[serde(rename = "type")]
        pub kind: SubEmitterType,
        /// Unity's inherit flags: 1 colour, 2 size, 4 rotation, 8 lifetime.
        pub inherit: f64,
        pub probability: f64,
    }

    /// Trails behind particles (Trail module, Particles mode).
    pub struct ParticleTrail {
        /// Share of particles that get a trail.
        pub ratio: f64,
        /// How long a trail point lasts, as a share of the particle's lifetime.
        pub lifetime: MinMax,
        pub min_vertex_distance: f64,
        pub size_affects_width: bool,
        pub inherit_particle_color: bool,
        pub width: MinMax,
        pub color_over_lifetime: MinMaxColor,
        pub color_over_trail: MinMaxColor,
    }

    /// Which way billboards face.
    #[derive(Copy, Eq)]
    pub enum ParticleAlignment {
        View,
        World,
        Local,
        Facing,
        Velocity,
    }

    /// What it takes to play a particle system.
    pub struct ParticleSim {
        pub duration: f64,
        pub looping: bool,
        pub prewarm: bool,
        /// Particles keep moving in the world (true) or with the emitter (false).
        pub world_space: bool,
        pub simulation_speed: f64,
        pub start_delay: MinMax,
        pub start_lifetime: MinMax,
        pub start_speed: MinMax,
        pub start_size: MinMax,
        /// Radians.
        pub start_rotation: MinMax,
        /// Start height and depth, when the system sizes particles per axis.
        #[serde(rename = "startSize3D")]
        pub start_size_3d: Option<[MinMax; 2]>,
        /// Radians around X and Y too, when the system rotates particles in 3D.
        #[serde(rename = "startRotation3D")]
        pub start_rotation_3d: Option<[MinMax; 2]>,
        pub start_color: MinMaxColor,
        pub gravity: MinMax,
        pub max_particles: f64,
        pub rate: MinMax,
        pub bursts: Vec<ParticleBurst>,
        pub shape: Option<ParticleShape>,
        /// Added to each particle's velocity (local axes unless `velocityWorld`).
        pub velocity: Option<[MinMax; 3]>,
        pub velocity_world: Option<bool>,
        pub force: Option<[MinMax; 3]>,
        /// Limit velocity: speed above `limit` is reduced by `dampen` each frame.
        pub limit: Option<VelocityLimit>,
        pub color_over_lifetime: Option<MinMaxColor>,
        pub size_over_lifetime: Option<MinMax>,
        #[serde(rename = "sizeOverLifetime3D")]
        pub size_over_lifetime_3d: Option<[MinMax; 2]>,
        /// Radians per second.
        pub rotation_over_lifetime: Option<MinMax>,
        #[serde(rename = "rotationOverLifetime3D")]
        pub rotation_over_lifetime_3d: Option<[MinMax; 2]>,
        pub sheet: Option<SheetAnimation>,
        pub noise: Option<ParticleNoise>,
        /// Systems this one starts at its particles; they emit only then.
        pub sub_emitters: Option<Vec<SubEmitter>>,
        pub trail: Option<ParticleTrail>,
        /// Renderer settings for stretched billboards.
        pub length_scale: f64,
        pub velocity_scale: f64,
        pub pivot: [f64; 3],
        /// Largest size on screen, as a fraction of the view height.
        pub max_screen_size: f64,
        pub sorting_order: f64,
        pub sorting_fudge: f64,
        pub alignment: ParticleAlignment,
        /// Custom Data module, vector mode.
        pub custom1: Option<Vec<MinMax>>,
        pub custom2: Option<Vec<MinMax>>,
        /// The renderer's vertex streams, in order; absent: Unity's default set.
        pub streams: Option<Vec<u32>>,
    }

    #[derive(Copy, Eq)]
    pub enum ParticleRenderMode {
        Billboard,
        Stretched,
        Horizontal,
        Vertical,
        Mesh,
        None,
    }

    /// Optional modules worth knowing about.
    #[derive(Copy, Eq)]
    pub enum ParticleModule {
        Noise,
        Collision,
        Trails,
        Lights,
        SubEmitters,
        TextureSheet,
        Velocity,
        Force,
        ExternalForces,
    }

    /// Unity's built-in meshes.
    #[derive(Copy, Eq)]
    pub enum BuiltinMesh {
        Cube,
        Cylinder,
        Sphere,
        Capsule,
        Plane,
        Quad,
    }

    pub struct ParticleEmitter {
        /// fileID of the ParticleSystem.
        pub id: String,
        pub name: String,
        /// Seconds one cycle lasts.
        pub duration: f64,
        pub looping: bool,
        pub prewarm: bool,
        /// Upper ends of the start values.
        pub lifetime: f64,
        pub speed: f64,
        pub size: f64,
        pub max_particles: f64,
        /// Particles per second.
        pub rate: f64,
        /// Particles from bursts in one cycle.
        pub bursts: f64,
        /// Most particles alive at once, as far as the settings tell.
        pub peak: f64,
        pub shape: Option<String>,
        pub render_mode: ParticleRenderMode,
        /// Material GUIDs: the particles', and the trails' when trails are on.
        pub material: Option<String>,
        pub trail_material: Option<String>,
        /// Mesh GUID when particles are meshes.
        pub mesh: Option<String>,
        /// Which mesh of that model (its fileID).
        #[serde(rename = "meshFileID")]
        pub mesh_file_id: Option<String>,
        /// One of Unity's own meshes, which no repo holds.
        pub builtin_mesh: Option<BuiltinMesh>,
        pub modules: Vec<ParticleModule>,
        /// Texture sheet animation grid (columns x rows).
        pub sheet: Option<[f64; 2]>,
        pub sub_emitters: f64,
        /// Transform (fileID) of its GameObject, to place it in the prefab.
        pub node: Option<String>,
        /// What a preview needs to play it.
        pub sim: ParticleSim,
    }

    /// A particle system of a prefab; `prefab` names the nested prefab it came from.
    pub struct PrefabParticle {
        #[serde(flatten)]
        pub emitter: ParticleEmitter,
        pub prefab: Option<String>,
    }
}

// ---------------------------------------------------------------------------------------------
// Auth, tools, progress
// ---------------------------------------------------------------------------------------------

dto! {
    #[derive(Copy, Eq)]
    pub enum AuthMethod {
        Gh,
        Pat,
    }

    pub struct AuthStatus {
        pub signed_in: bool,
        /// The active account.
        pub login: Option<String>,
        /// Every signed-in account, the active one first (one unless gh holds several).
        pub accounts: Vec<String>,
        pub method: Option<AuthMethod>,
        /// False when the OS keychain was unavailable and the token lives only in memory.
        pub persisted: Option<bool>,
    }

    /// Options of `signOut`: `gh` also logs the account out of GitHub CLI on this Mac.
    pub struct SignOutOptions {
        pub gh: Option<bool>,
    }

    pub struct GhInstalled {
        pub version: String,
    }

    /// A download the app runs by itself: GitHub CLI, an update.
    #[serde(tag = "stage")]
    pub enum InstallProgress {
        Download { received: f64, total: f64 },
        Verify,
        Extract,
        /// An update the app started by itself failed; the user can try again from the button.
        Failed { message: String },
    }

    /// Bytes or items done out of a total.
    pub struct DoneTotal {
        pub done: f64,
        pub total: f64,
    }

    /// Anything the app answers with `{ ok: true }` or `{ ok: false, message }`.
    pub struct OkMessage {
        pub ok: bool,
        pub message: Option<String>,
    }
}

// ---------------------------------------------------------------------------------------------
// Catalog, caches, shared index, stats
// ---------------------------------------------------------------------------------------------

dto! {
    #[derive(Copy, Eq)]
    pub enum CatalogState {
        Idle,
        Running,
        Paused,
        Error,
    }

    /// Background indexing of every branch.
    pub struct CatalogStatus {
        pub state: CatalogState,
        /// Branches listed in this run, out of `total`.
        pub done: f64,
        pub total: f64,
        /// repo@branch being listed.
        pub current: Option<String>,
        pub error: Option<IpcError>,
    }

    /// The repo and branch open now, indexed first.
    pub struct CatalogCurrent {
        pub repo: String,
        pub branch: Option<String>,
    }

    pub struct CatalogFilter {
        pub repo: Option<String>,
        pub branch: Option<String>,
    }

    /// One place an asset lives: a path in a repo, on one or more branches.
    pub struct CatalogLocation {
        pub repo: String,
        pub path: String,
        /// Default branch first, then most recently committed.
        pub branches: Vec<String>,
        /// In a listing, only the first branches are sent: how many there are in all.
        pub branch_total: Option<f64>,
    }

    /// A file found anywhere in the catalog; identical files (same blob SHA) are one entry.
    pub struct CatalogAsset {
        /// Stable id within one repo + branch: the file path.
        pub id: String,
        /// Handler id, e.g. 'fbx'.
        #[serde(rename = "type")]
        pub kind: String,
        pub path: String,
        pub name: String,
        pub sha: String,
        pub size: Option<f64>,
        pub lfs: bool,
        pub locations: Vec<CatalogLocation>,
        /// In a listing, only the first locations are sent: how many there are in all.
        pub location_total: Option<f64>,
        pub branch_count: f64,
        pub repo_count: f64,
    }

    /// Counts per repo and per branch, most first.
    pub struct CatalogFacets {
        pub repos: Vec<(String, f64)>,
        pub branches: Vec<(String, f64)>,
    }

    pub struct CatalogListing {
        pub assets: Vec<CatalogAsset>,
        pub facets: CatalogFacets,
    }

    /// Bytes the cache folder holds, by kind.
    pub struct CacheUsage {
        /// Downloaded files, listings and indexes (count toward the size limit).
        pub downloads: f64,
        pub thumbs: f64,
        /// Git clones, measurements and the rest.
        pub other: f64,
    }

    pub struct SharedIndexRepo {
        pub repo: String,
        pub branches: f64,
        pub commits: f64,
    }

    /// What a shared index folder holds (`manifest.json`).
    pub struct SharedIndexManifest {
        pub format: u8,
        /// ISO time it was written; a newer one is taken in.
        pub exported_at: String,
        /// GitHub login of who wrote it.
        pub exported_by: Option<String>,
        /// Asset types indexed (handler ids).
        pub types: String,
        pub repos: Vec<SharedIndexRepo>,
        pub thumbs: f64,
        pub stats: f64,
    }

    /// Counts of what an import took in, or an export wrote.
    pub struct SharedIndexResult {
        pub commits: f64,
        pub facts: f64,
        pub stats: f64,
        pub thumbs: f64,
    }

    #[derive(Copy, Eq)]
    pub enum SharedIndexState {
        Idle,
        Importing,
        Exporting,
        Error,
    }

    /// Shared through the GitHub repo (no folder picked) or a folder.
    #[derive(Copy, Eq)]
    pub enum SharedIndexVia {
        Github,
        Folder,
    }

    #[derive(Copy, Eq)]
    pub enum SharedIndexAction {
        Imported,
        Exported,
    }

    pub struct SharedIndexStatus {
        pub state: SharedIndexState,
        pub error: Option<String>,
        pub via: Option<SharedIndexVia>,
        /// The GitHub repo, `via: github`.
        pub repo: Option<String>,
        /// No signed-in account can write to the GitHub repo (each can read it).
        pub read_only: Option<bool>,
        /// The signed-in account that pushes to the repo, once known (only checked when there is a
        /// choice of accounts and a push or the page's status check needs it).
        pub writer: Option<String>,
        /// In the shared folder now.
        pub manifest: Option<SharedIndexManifest>,
        /// `exportedAt` of the index taken in last.
        pub imported_at: Option<String>,
        pub last: Option<SharedIndexResult>,
        pub last_action: Option<SharedIndexAction>,
    }

    #[derive(Copy, Eq)]
    pub enum StatsSource {
        Thumb,
        Viewer,
    }

    /// What is kept per model (by content SHA) so results can be ranked without opening them.
    pub struct StatsSummary {
        pub triangles: f64,
        pub vertices: f64,
        pub bones: f64,
        pub clips: f64,
        pub materials: f64,
        pub file_bytes: f64,
        pub texture_bytes: f64,
        /// Largest texture side in pixels.
        pub max_texture_size: f64,
        /// For a texture asset: its size in pixels.
        pub width: Option<f64>,
        pub height: Option<f64>,
        /// For an audio clip: its length in seconds.
        pub duration: Option<f64>,
        /// Names of the animation clips inside a model, so they can be searched.
        pub clip_names: Option<Vec<String>>,
        /// For an animation clip: its length in seconds.
        pub clip_length: Option<f64>,
        /// For a material: how many textures it uses.
        pub texture_count: Option<f64>,
        /// For a font: how many glyphs it has.
        pub glyphs: Option<f64>,
        /// For an effect: most particles alive at once, and how many emitters.
        pub particles: Option<f64>,
        pub emitters: Option<f64>,
        /// Versions of the way each kind of card was drawn; older ones are drawn again.
        pub fx_thumb: Option<f64>,
        pub anim_thumb: Option<f64>,
        pub model_thumb: Option<f64>,
        /// For an animation clip: whether its card found a model to pose it on.
        pub rigged: Option<bool>,
        /// `viewer` counts every texture the model uses; `thumb` the ones named inside the FBX.
        pub source: StatsSource,
        /// A model thumbnail that also counted the textures of its .mat files.
        pub mat_textures: Option<bool>,
    }
}

// ---------------------------------------------------------------------------------------------
// Export and Unity
// ---------------------------------------------------------------------------------------------

dto! {
    /// What to export. `previews` and `glb` hold ids of bytes sent with `stash_bytes`.
    pub struct ExportRequest {
        pub repo: String,
        pub branch: String,
        pub root: String,
        /// Repo paths of the models to export; their dependencies are added automatically.
        pub paths: Vec<String>,
        pub layout: PackageLayout,
        /// Leave out textures that were only matched by file name.
        pub skip_by_name: bool,
        /// More repo paths to include, e.g. materials found on prefabs (with .meta).
        pub extra_files: Vec<String>,
        /// Ids of PNG previews sent with `stash_bytes`, keyed by repo path.
        pub previews: HashMap<String, String>,
        /// Default file name in the save dialog.
        pub suggested_name: String,
        /// Unity project the package is meant for (read only).
        pub target_project: Option<String>,
        /// Leave out files the project has under another GUID.
        pub reuse_existing: Option<bool>,
        /// Remove the project's own scripts from prefabs and leave those files out.
        pub strip_scripts: Option<bool>,
    }

    /// A path of the export and the path in the project it meets ("existing" may hold a note).
    pub struct PathPair {
        pub path: String,
        pub existing: String,
    }

    pub struct UrpInfo {
        pub source: String,
        pub target: Option<String>,
        /// Materials that still need URP after conversion.
        pub needed: Option<f64>,
    }

    /// What the package does to the target project.
    pub struct TargetReport {
        pub project: String,
        /// Already there with the same GUID and content: left out.
        pub same: f64,
        pub reused: Vec<PathPair>,
        pub overwrite: Vec<PathPair>,
        pub renamed: Vec<PathPair>,
        pub shader_by_name: Vec<PathPair>,
        pub shader_kept: Vec<PathPair>,
        pub shader_clash: Vec<PathPair>,
        pub shader_mapped: Vec<PathPair>,
        pub urp_converted: Vec<PathPair>,
        pub missing_scripts: Vec<PathPair>,
        pub missing_shaders: Vec<PathPair>,
        /// Problems with the mapping file (rules skipped).
        pub mapping_errors: Vec<String>,
        pub urp: Option<UrpInfo>,
    }

    #[derive(Copy, Eq)]
    pub enum SkipReason {
        NoMeta,
        NoGuid,
        DuplicateGuid,
    }

    pub struct SkippedFile {
        pub path: String,
        pub reason: SkipReason,
    }

    pub struct ExportResult {
        pub canceled: bool,
        pub path: Option<String>,
        pub assets: Option<f64>,
        pub bytes: Option<f64>,
        pub skipped: Option<Vec<SkippedFile>>,
        pub target: Option<TargetReport>,
        /// Opened in this Unity project (Import into Unity) instead of saved.
        pub unity: Option<String>,
        /// Script components removed from prefabs.
        pub scripts_removed: Option<f64>,
    }

    /// A Unity editor open on this Mac, with its project.
    pub struct RunningUnity {
        pub pid: u32,
        pub project_path: String,
        /// The project folder's name.
        pub project: String,
        /// Unity's productName, when it differs from the folder.
        pub product: Option<String>,
        pub version: Option<String>,
    }

    #[derive(Copy, Eq)]
    pub enum ExportStage {
        Resolve,
        Download,
        Write,
    }
}

// ---------------------------------------------------------------------------------------------
// Update, usage, feedback
// ---------------------------------------------------------------------------------------------

dto! {
    /// This version and, when the release repo has a later one, that one.
    pub struct UpdateInfo {
        pub current: String,
        pub latest: Option<String>,
        /// The app can replace itself.
        pub can_install: bool,
        /// The release page, for installing by hand.
        pub page: String,
    }

    /// What the page reports itself; the rest is seen in the backend.
    #[serde(tag = "kind")]
    pub enum UsageEvent {
        View {
            #[serde(rename = "type")]
            asset_type: String,
        },
        Search,
        Feedback,
    }

    #[derive(Copy, Eq)]
    pub enum FeedbackKind {
        Bug,
        Idea,
        Other,
    }

    /// Where the user was, when they chose to include it.
    pub struct FeedbackContext {
        pub repo: Option<String>,
        pub branch: Option<String>,
        pub asset: Option<String>,
    }

    pub struct FeedbackReport {
        pub kind: FeedbackKind,
        pub title: String,
        pub body: String,
        pub context: Option<FeedbackContext>,
    }

    /// Sent: the issue's link. Not sent (offline, no access): kept and retried on a later launch.
    pub struct FeedbackResult {
        pub sent: bool,
        pub url: Option<String>,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github_net::convert;
    use asset_hub_core::unity::particle_sim as core;

    /// The core writes every number as a float (`0.0`), so a whole-number field of the mirror must be a float too.
    #[test]
    fn particle_modules_read_back_the_floats_the_core_writes() {
        let curve = core::MinMax { mode: 0.0, scalar: 1.0, min: 0.0, curve: None, min_curve: None };
        assert_eq!(convert::<_, MinMax>(&curve).unwrap().mode, 0.0);
        let color = core::MinMaxColor { mode: 1.0, max: [1.0; 4], min: [0.0; 4], gradient: None, min_gradient: None };
        assert_eq!(convert::<_, MinMaxColor>(&color).unwrap().mode, 1.0);
        let shape = core::ParticleShape { kind: 4.0, radius: 1.0, radius_thickness: 1.0, angle: 25.0, arc: 360.0, length: 5.0, position: [0.0; 3], rotation: [0.0; 3], scale: [1.0; 3], random_direction: 0.0 };
        assert_eq!(convert::<_, ParticleShape>(&shape).unwrap().kind, 4.0);
        let sub = core::SubEmitter { id: "a".into(), kind: core::SubEmitterType::Birth, inherit: 3.0, probability: 1.0 };
        assert_eq!(convert::<_, SubEmitter>(&sub).unwrap().inherit, 3.0);
    }
}
