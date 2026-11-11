//! The asset type handlers (`src/core/handlers/*.ts`) and the default registry
//! (`createDefaultRegistry`). A handler is data plus two declarative rules: what else its tab lists
//! (`also_shows`) and what is left out of lists (`unlisted`). The renderer reads the same rules from
//! `golden/expected/registry/rules.json`.

use serde::Serialize;

use crate::registry::HandlerRegistry;
use crate::stats::StatsSummary;
use crate::types::{Asset, AssetFacts, FactKind, PreviewKind, TreeEntry};
use crate::js::Re;
use crate::re;

/// How a user takes an asset: a `.unitypackage` with its dependencies, or the file itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Delivery {
    Unitypackage,
    File,
}

/// The dependency resolver of a handler; the resolvers themselves are in [`crate::deps`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolver {
    Fbx,
    Itself,
    Material,
    Prefab,
    Animation,
    TmpFont,
}

impl Resolver {
    /// The name the TypeScript resolver has.
    pub fn name(self) -> &'static str {
        match self {
            Self::Fbx => "resolveFbxDependencies",
            Self::Itself => "resolveSelf",
            Self::Material => "resolveMaterialDependencies",
            Self::Prefab => "resolvePrefabDependencies",
            Self::Animation => "resolveAnimationDependencies",
            Self::TmpFont => "resolveTmpFontDependencies",
        }
    }
}

/// Assets of other types a tab also lists, judged by what is known about them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlsoShows {
    Nothing,
    /// Assets of `from_type` whose SHA the scan of `fact` found.
    Fact { from_type: &'static str, fact: FactKind },
    /// Models holding animation clips (measured), or named like animation files (`Model@Run.fbx`).
    AnimatedModel,
}

/// Left out of lists once measured to be of no use to look at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unlisted {
    Never,
    /// Its card found no model to pose it on.
    NotRigged,
}

type Matcher = fn(&TreeEntry) -> bool;

#[derive(Debug, Clone)]
pub struct AssetHandler {
    pub id: &'static str,
    pub label_key: &'static str,
    pub badge: &'static str,
    pub preview_kind: PreviewKind,
    pub delivery: Delivery,
    pub facts: Option<FactKind>,
    /// Models whose materials may be set on prefabs (FBX).
    pub materials_from_prefabs: bool,
    /// False when the resolver never looks a GUID up.
    pub uses_guids: bool,
    /// No tab and not under All: only listed by other tabs.
    pub hidden: bool,
    pub resolver: Option<Resolver>,
    pub also_shows: AlsoShows,
    pub unlisted: Unlisted,
    matcher: Matcher,
}

/// What a handler declares, as the renderer reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HandlerDescriptor {
    pub id: &'static str,
    pub label_key: &'static str,
    pub badge: &'static str,
    pub preview_kind: PreviewKind,
    pub hidden: bool,
    pub delivery: Delivery,
    pub facts: Option<FactKind>,
    pub materials_from_prefabs: bool,
    pub uses_guids: bool,
    pub resolver: Option<&'static str>,
}

impl AssetHandler {
    pub fn matches(&self, entry: &TreeEntry) -> bool {
        (self.matcher)(entry)
    }

    pub fn descriptor(&self) -> HandlerDescriptor {
        HandlerDescriptor {
            id: self.id,
            label_key: self.label_key,
            badge: self.badge,
            preview_kind: self.preview_kind,
            hidden: self.hidden,
            delivery: self.delivery,
            facts: self.facts,
            materials_from_prefabs: self.materials_from_prefabs,
            uses_guids: self.uses_guids,
            resolver: self.resolver.map(Resolver::name),
        }
    }

    /// Whether this handler's tab also lists the asset.
    pub fn shows_also(&self, asset: &Asset, stats: Option<&StatsSummary>, facts: Option<&AssetFacts>) -> bool {
        match self.also_shows {
            AlsoShows::Nothing => false,
            AlsoShows::Fact { from_type, fact } => asset.kind == from_type && facts.and_then(|f| f.of(fact)).is_some_and(|shas| shas.contains(&asset.sha)),
            AlsoShows::AnimatedModel => asset.kind == "fbx" && (stats.is_some_and(|s| s.clips > 0.0) || asset.name.contains('@')),
        }
    }

    /// Whether lists leave the asset out, given what is measured about it.
    pub fn is_unlisted(&self, stats: Option<&StatsSummary>) -> bool {
        match self.unlisted {
            Unlisted::Never => false,
            Unlisted::NotRigged => stats.is_some_and(|s| s.rigged == Some(false)),
        }
    }
}

/// `.asset` files that may be TextMeshPro fonts: TextMeshPro names them "<Font> SDF" and keeps them
/// under Fonts folders. Other `.asset` files cannot be previewed: they are not listed at all.
pub fn tmp_candidate() -> &'static Re {
    re!(r"(sdf|font)[^/]*\.asset$|\/fonts?[^/]*\/[^/]*\.asset$", "i")
}

fn base(id: &'static str, label_key: &'static str, badge: &'static str, preview_kind: PreviewKind, matcher: Matcher) -> AssetHandler {
    AssetHandler {
        id,
        label_key,
        badge,
        preview_kind,
        delivery: Delivery::Unitypackage,
        facts: None,
        materials_from_prefabs: false,
        uses_guids: true,
        hidden: false,
        resolver: None,
        also_shows: AlsoShows::Nothing,
        unlisted: Unlisted::Never,
        matcher,
    }
}

/// The handlers in registration order.
pub fn create_default_registry() -> HandlerRegistry {
    let handlers = [
        AssetHandler { resolver: Some(Resolver::Fbx), materials_from_prefabs: true, ..base("fbx", "assetTypes.fbx", "FBX", PreviewKind::Three, |e| re!(r"\.fbx$", "i").test(&e.path)) },
        AssetHandler {
            resolver: Some(Resolver::Itself),
            delivery: Delivery::File,
            uses_guids: false,
            ..base("texture", "assetTypes.texture", "TEX", PreviewKind::Image, |e| re!(r"\.(png|jpe?g|tga|psd|gif|bmp|tiff?)$", "i").test(&e.path))
        },
        AssetHandler {
            facts: Some(FactKind::Sprites),
            also_shows: AlsoShows::Fact { from_type: "texture", fact: FactKind::Sprites },
            ..base("sprite", "assetTypes.sprite", "SPR", PreviewKind::Image, |_| false)
        },
        AssetHandler { resolver: Some(Resolver::Material), ..base("material", "assetTypes.material", "MAT", PreviewKind::Material, |e| re!(r"\.mat$", "i").test(&e.path)) },
        AssetHandler {
            resolver: Some(Resolver::Itself),
            delivery: Delivery::File,
            uses_guids: false,
            ..base("audio", "assetTypes.audio", "AUD", PreviewKind::Audio, |e| re!(r"\.(wav|mp3|ogg)$", "i").test(&e.path))
        },
        AssetHandler { resolver: Some(Resolver::Prefab), ..base("prefab", "assetTypes.prefab", "PFB", PreviewKind::Prefab, |e| re!(r"\.prefab$", "i").test(&e.path)) },
        AssetHandler {
            facts: Some(FactKind::Effects),
            also_shows: AlsoShows::Fact { from_type: "prefab", fact: FactKind::Effects },
            ..base("vfx", "assetTypes.vfx", "VFX", PreviewKind::Prefab, |_| false)
        },
        AssetHandler {
            resolver: Some(Resolver::Animation),
            also_shows: AlsoShows::AnimatedModel,
            unlisted: Unlisted::NotRigged,
            ..base("animation", "assetTypes.animation", "ANIM", PreviewKind::Animation, |e| re!(r"\.anim$", "i").test(&e.path))
        },
        AssetHandler {
            resolver: Some(Resolver::Itself),
            uses_guids: false,
            facts: Some(FactKind::TmpFonts),
            also_shows: AlsoShows::Fact { from_type: "tmpFont", fact: FactKind::TmpFonts },
            ..base("font", "assetTypes.font", "FONT", PreviewKind::Font, |e| re!(r"\.(ttf|otf|ttc)$", "i").test(&e.path))
        },
        AssetHandler { resolver: Some(Resolver::TmpFont), hidden: true, ..base("tmpFont", "assetTypes.tmpFont", "TMP", PreviewKind::Tmpfont, |e| tmp_candidate().test(&e.path)) },
    ];
    let mut registry = HandlerRegistry::new();
    for handler in handlers {
        registry.register(handler).expect("the default handlers have distinct ids");
    }
    registry
}
