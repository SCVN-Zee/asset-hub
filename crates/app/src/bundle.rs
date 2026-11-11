//! A model with its textures and a Blender script that rebuilds its materials (`modelBundle` in
//! `src/main/github.ts`). Which files go in is decided here; where they go (a .zip, or a temporary
//! folder Blender opens) is up to the command.

use asset_hub_core::errors::SourceError;
use asset_hub_core::export::model_bundle::{blender_material, blender_script, bundle_file_names, bundle_texture_paths, BundleMaterial};
use asset_hub_core::js::decode_utf8;
use futures_util::stream::{self, StreamExt};

use crate::exporter::ExportSource;

/// Files read at the same time.
const READERS: usize = 8;

/// The files of a bundle, by their names in it.
#[derive(Debug)]
pub struct Bundle {
    pub files: Vec<(String, Vec<u8>)>,
    /// Textures that came with the model.
    pub textures: usize,
}

async fn optional<S: ExportSource>(source: &S, repo: &str, branch: &str, path: &str) -> Option<Vec<u8>> {
    source.read(repo, branch, path, false).await.ok()
}

async fn mat_text<S: ExportSource>(source: &S, repo: &str, branch: &str, m: &BundleMaterial) -> Option<String> {
    let path = m.mat.as_deref()?;
    optional(source, repo, branch, path).await.map(|d| decode_utf8(&d))
}

/// The model must come; a texture or .mat that fails to download is left out. Files are read from
/// the cache when there, never added to it: they are only passed on. `glb` replaces the model's own
/// bytes (a prefab comes as the GLB the page exported). `cleanup` makes the script delete its folder
/// once everything is in Blender.
pub async fn build_bundle<S: ExportSource>(source: &S, repo: &str, branch: &str, model: &str, materials: &[BundleMaterial], glb: Option<Vec<u8>>, cleanup: bool) -> Result<Bundle, SourceError> {
    let stem = model.rsplit('/').next().filter(|n| !n.is_empty()).unwrap_or("model");
    let stem = stem.rsplit_once('.').map_or(stem, |(s, _)| s);
    let model_name = if glb.is_some() { format!("{stem}.glb") } else { model.rsplit('/').next().filter(|n| !n.is_empty()).unwrap_or("model.fbx").to_owned() };
    let textures = bundle_texture_paths(materials);
    let model_key = format!("/{model_name}");
    let mut keys = vec![model_key.clone()];
    keys.extend(textures.iter().cloned());
    let names = bundle_file_names(&keys);

    let model_bytes = match glb {
        Some(bytes) => bytes,
        None => source.read(repo, branch, model, false).await?,
    };
    let tex_bytes: Vec<Option<Vec<u8>>> = stream::iter(textures.clone()).map(|p| async move { optional(source, repo, branch, &p).await }).buffered(READERS).collect().await;
    let mat_texts: Vec<Option<String>> = stream::iter(materials.to_vec()).map(|m| async move { mat_text(source, repo, branch, &m).await }).buffered(READERS).collect().await;

    let got: Vec<(&String, Vec<u8>)> = textures.iter().zip(tex_bytes).filter_map(|(p, d)| Some((p, d?))).collect();
    let file_of = |p: &str| if got.iter().any(|(g, _)| g.as_str() == p) { names.get(p).cloned() } else { None };
    let specs: Vec<_> = materials.iter().zip(&mat_texts).map(|(m, text)| blender_material(m, text.as_deref(), &file_of)).collect();
    let model_file = names.get(&model_key).cloned().unwrap_or(model_name);
    let mut files = vec![(model_file.clone(), model_bytes)];
    files.extend(got.iter().map(|(p, d)| (names.get(p).cloned().unwrap_or_else(|| (*p).clone()), d.clone())));
    // Opened from the temporary folder, the script deletes it once everything is in Blender.
    files.push(("open_in_blender.py".to_owned(), blender_script(&model_file, &specs, cleanup).into_bytes()));
    Ok(Bundle { files, textures: got.len() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exporter::BoxFuture;
    use asset_hub_core::deps::types::DependencyReport;
    use asset_hub_core::errors::SourceErrorCode;
    use asset_hub_core::export::model_bundle::BundleTexture;
    use std::collections::HashMap;

    struct Fake(HashMap<String, Vec<u8>>);

    impl ExportSource for Fake {
        fn resolve_full<'a>(&'a self, _: &'a str, _: &'a str, _: &'a str, _: &'a str) -> BoxFuture<'a, Result<DependencyReport, SourceError>> {
            Box::pin(async { Err(SourceError::new(SourceErrorCode::NotFound, "not used")) })
        }

        fn read<'a>(&'a self, _: &'a str, _: &'a str, path: &'a str, keep: bool) -> BoxFuture<'a, Result<Vec<u8>, SourceError>> {
            Box::pin(async move {
                assert!(!keep, "bundle files are not added to the cache");
                self.0.get(path).cloned().ok_or_else(|| SourceError::new(SourceErrorCode::NotFound, format!("{path} is missing")))
            })
        }
    }

    fn material() -> BundleMaterial {
        BundleMaterial { target: "Body".into(), name: Some("Body".into()), mat: Some("Assets/body.mat".into()), shader: None, textures: vec![BundleTexture { prop: "_BaseMap".into(), path: "Assets/a/body.png".into() }, BundleTexture { prop: "_BumpMap".into(), path: "Assets/a/gone.png".into() }] }
    }

    #[tokio::test]
    async fn a_missing_texture_is_left_out_but_a_missing_model_fails() {
        let fake = Fake(HashMap::from([("Assets/m/hero.fbx".to_string(), b"fbx".to_vec()), ("Assets/a/body.png".to_string(), b"png".to_vec())]));
        let bundle = build_bundle(&fake, "o/r", "main", "Assets/m/hero.fbx", &[material()], None, false).await.unwrap();
        let names: Vec<&str> = bundle.files.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["hero.fbx", "body.png", "open_in_blender.py"]);
        assert_eq!(bundle.textures, 1);
        let script = String::from_utf8(bundle.files[2].1.clone()).unwrap();
        assert!(script.contains("hero.fbx") && script.contains("body.png") && !script.contains("gone.png"));

        let err = build_bundle(&Fake(HashMap::new()), "o/r", "main", "Assets/m/hero.fbx", &[], None, false).await.unwrap_err();
        assert_eq!(err.code, SourceErrorCode::NotFound);
    }

    #[tokio::test]
    async fn a_prefab_comes_as_the_glb_the_page_made() {
        let fake = Fake(HashMap::new());
        let bundle = build_bundle(&fake, "o/r", "main", "Assets/p/Hero.prefab", &[], Some(b"glb".to_vec()), true).await.unwrap();
        assert_eq!(bundle.files[0], ("Hero.glb".to_string(), b"glb".to_vec()));
        assert!(String::from_utf8(bundle.files[1].1.clone()).unwrap().contains("Hero.glb"));
    }
}
