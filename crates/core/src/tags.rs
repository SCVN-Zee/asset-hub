//! The tags an asset can carry, shown as chips on cards and in the inspector, filtered above the grid
//! and found by search. `ui/src/tags.ts` lists the same tags; a test there compares the two lists.
//!
//! A tag follows from what is known about an asset: its type and name, what was measured (animation
//! clips), and what the scans of the branch found (prefabs with particle systems or UI objects).

use crate::stats::StatsSummary;

/// The tags, in the order the chips show them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AssetTag {
    /// A model that holds animation clips.
    Animation,
    /// A prefab with a RectTransform or Canvas object.
    Ui,
    /// A prefab with a particle system.
    ParticleSystem,
}

impl AssetTag {
    pub const ALL: [AssetTag; 3] = [Self::Animation, Self::Ui, Self::ParticleSystem];

    /// The id the page uses (`tags.<id>` is its label there).
    pub fn id(self) -> &'static str {
        match self {
            Self::Animation => "animation",
            Self::Ui => "ui",
            Self::ParticleSystem => "particleSystem",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.id() == id)
    }

    /// The words that find assets with this tag: "ui", "animation", "particle" (and "system").
    pub fn search_words(self) -> &'static [&'static str] {
        match self {
            Self::Animation => &["animation"],
            Self::Ui => &["ui"],
            Self::ParticleSystem => &["particle", "system"],
        }
    }
}

/// A model that holds animation clips (measured), or is named like an animation file (`Model@Run.fbx`).
/// The one rule behind the Animation tag and the animation tab's list of models.
pub fn is_animated_model(kind: &str, name: &str, stats: Option<&StatsSummary>) -> bool {
    kind == "fbx" && (stats.is_some_and(|s| s.clips > 0.0) || name.contains('@'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clips(n: f64) -> StatsSummary {
        serde_json::from_value(serde_json::json!({ "triangles": 0, "vertices": 0, "bones": 0, "clips": n, "materials": 0, "fileBytes": 0, "textureBytes": 0, "maxTextureSize": 0, "source": "viewer" })).expect("a summary")
    }

    #[test]
    fn ids_round_trip() {
        for tag in AssetTag::ALL {
            assert_eq!(AssetTag::from_id(tag.id()), Some(tag));
        }
        assert_eq!(AssetTag::from_id("nope"), None);
    }

    #[test]
    fn a_model_is_animated_by_its_clips_or_its_name() {
        assert!(is_animated_model("fbx", "Run.fbx", Some(&clips(2.0))));
        assert!(is_animated_model("fbx", "Model@Run.fbx", None));
        assert!(!is_animated_model("fbx", "Model.fbx", Some(&clips(0.0))));
        assert!(!is_animated_model("fbx", "Model.fbx", None));
        assert!(!is_animated_model("prefab", "Model@Run.prefab", Some(&clips(2.0))));
    }
}
