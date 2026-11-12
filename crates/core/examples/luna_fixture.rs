//! Dumps what the app's parsers read from a Unity prefab folder as JSON, so `scripts/luna-smoke.mjs` can feed real
//! prefabs through the Luna bridge without the Tauri app.
//!
//! Usage: cargo run -p asset_hub_core --example luna_fixture -- <folder> [prefab name]...
//! Prints `{ "<prefab name>": { scene, emitters, materials } }`. `materials` maps a material GUID to its shader
//! (label and kind as the dependency report names them), its parsed properties and look, and the texture file and
//! import settings of each slot.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use asset_hub_core::unity::particles::parse_particle_systems;
use asset_hub_core::unity::prefab_scene::parse_prefab_scene;
use asset_hub_core::unity::yaml::{builtin_shader_name, known_package_shader, meta_guid, parse_material, parse_material_look, parse_material_props, parse_texture_meta, shader_name};
use serde_json::{json, Value};

/// Files under `dir`, as (path, guid from its `.meta`).
fn guid_index(dir: &Path, out: &mut HashMap<String, PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            guid_index(&path, out);
        } else if path.extension().is_some_and(|e| e == "meta") {
            if let Some(guid) = fs::read_to_string(&path).ok().as_deref().and_then(meta_guid) {
                out.insert(guid, path.with_extension(""));
            }
        }
    }
}

fn shader_of(guid: &str, file_id: &str, index: &HashMap<String, PathBuf>) -> Value {
    if guid.starts_with("0000000000000000f") {
        return json!({ "guid": guid, "kind": "builtin", "label": builtin_shader_name(file_id).unwrap_or("Built-in") });
    }
    if let Some(path) = index.get(guid) {
        let text = fs::read_to_string(path).unwrap_or_default();
        let label = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        return json!({ "guid": guid, "kind": "project", "label": label, "name": shader_name(&text) });
    }
    let label = known_package_shader(guid).map_or_else(|| format!("Package shader ({})", &guid[..8]), str::to_string);
    json!({ "guid": guid, "kind": "package", "label": label })
}

fn material_of(guid: &str, index: &HashMap<String, PathBuf>) -> Option<Value> {
    let path = index.get(guid)?;
    let text = fs::read_to_string(path).ok()?;
    let info = parse_material(&text);
    let mut textures = serde_json::Map::new();
    for slot in &info.textures {
        let Some(file) = index.get(&slot.guid) else { continue };
        let import = fs::read_to_string(format!("{}.meta", file.display())).map(|m| parse_texture_meta(&m)).ok();
        textures.insert(slot.prop.clone(), json!({ "file": file, "import": import }));
    }
    let shader = info.shader.as_ref().map(|s| shader_of(&s.guid, &s.file_id, index));
    Some(json!({
        "name": info.name,
        "shader": shader,
        "props": parse_material_props(&text),
        "look": parse_material_look(&text),
        "textures": textures,
    }))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(folder) = args.first() else {
        eprintln!("Usage: luna_fixture <folder> [prefab name]...");
        std::process::exit(2);
    };
    let wanted = &args[1..];
    let root = Path::new(folder);
    let mut index = HashMap::new();
    guid_index(root, &mut index);

    let mut out = serde_json::Map::new();
    let mut prefabs: Vec<PathBuf> = fs::read_dir(root).expect("the folder is readable").flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "prefab")).collect();
    prefabs.sort();
    for path in prefabs {
        let name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        if !wanted.is_empty() && !wanted.contains(&name) {
            continue;
        }
        let text = fs::read_to_string(&path).expect("the prefab is readable");
        let scene = parse_prefab_scene(&text);
        let emitters = parse_particle_systems(&text);
        let mut materials = serde_json::Map::new();
        for guid in emitters.iter().filter_map(|e| e.material.as_deref()) {
            if let Some(m) = material_of(guid, &index) {
                materials.insert(guid.to_string(), m);
            }
        }
        out.insert(name, json!({ "scene": scene, "emitters": emitters, "materials": materials }));
    }
    println!("{}", serde_json::to_string(&Value::Object(out)).expect("the fixture serialises"));
}
