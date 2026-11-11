//! What a prefab is made of, read from its YAML: the transform tree, which meshes sit where with
//! which materials, nested prefab and model instances, and the components a preview cannot show
//! (scripts, particle systems, UI). Port of `src/core/unity/prefabScene.ts`.

use serde::{Deserialize, Serialize};

use crate::js::{self, to_number, trim};
use crate::re;

const GUID: &str = "[0-9a-f]{32}";

pub type Vec3 = [f64; 3];
pub type Quat = [f64; 4];

/// A Transform (or RectTransform) and its GameObject. Ids are the Transform's fileID.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneNode {
    pub id: String,
    pub name: String,
    /// Parent Transform id; absent for the root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    pub position: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
    pub active: bool,
    /// Placeholder for a Transform inside a nested instance ("stripped"): stands for that instance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileRef {
    #[serde(rename = "fileID")]
    pub file_id: String,
    pub guid: String,
}

/// A MeshFilter and MeshRenderer pair, or a SkinnedMeshRenderer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneMesh {
    pub node: String,
    pub mesh: FileRef,
    /// Material GUIDs by submesh.
    pub materials: Vec<String>,
    pub skinned: bool,
}

/// A nested prefab or a model dropped in as it is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneInstance {
    pub id: String,
    pub source: String,
    /// Transform it hangs under in this prefab.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Overrides on the instance's root transform.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<Vec3>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<Quat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<Vec3>,
    /// Material overrides (applied to every renderer of the instance).
    pub materials: Vec<String>,
    /// GameObject name override of the root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrefabScene {
    /// Transforms by id, in file order (serialized as `[id, node]` pairs, like a JS `Map`).
    pub nodes: Vec<(String, SceneNode)>,
    pub meshes: Vec<SceneMesh>,
    pub instances: Vec<SceneInstance>,
    /// MonoBehaviour script GUIDs (custom or from packages).
    pub scripts: Vec<String>,
    pub particle_systems: usize,
    /// Materials of particle renderers (particles, then trails), and meshes they emit.
    pub particle_materials: Vec<String>,
    pub particle_meshes: Vec<String>,
    /// RectTransforms: a UI prefab.
    pub ui_elements: usize,
    /// Every GUID the prefab references, whatever it is.
    pub guids: Vec<String>,
}

impl PrefabScene {
    pub fn node(&self, id: &str) -> Option<&SceneNode> {
        self.nodes.iter().find(|(k, _)| k == id).map(|(_, n)| n)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Doc {
    pub cls: String,
    pub id: String,
    pub stripped: bool,
    pub body: String,
}

/// Splits a Unity YAML file into its `--- !u!<class> &<id>` documents.
///
/// A header is a whole line, so only lines that start with `--- !u!` are matched. Running the pattern
/// over the full text instead tries every position of a file that can be megabytes long.
pub fn split_docs(text: &str) -> Vec<Doc> {
    let mut docs = Vec::new();
    let mut last: Option<(String, String, bool, usize)> = None;
    let mut resume = 0;
    for (at, _) in text.match_indices("--- !u!") {
        // Line starts as `^` sees them with the `m` flag; text inside an earlier header line is not one.
        let starts_line = at == 0 || text[..at].chars().next_back().is_some_and(|c| matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}'));
        if at < resume || !starts_line {
            continue;
        }
        let end = text[at..].find('\n').map_or(text.len(), |n| at + n);
        let Some(m) = re!(r"^--- !u!(\d+) &(-?\d+)([^\n]*)$").exec(&text[at..end]) else { continue };
        if let Some((cls, id, stripped, start)) = last.take() {
            docs.push(Doc { cls, id, stripped, body: text[start..at].to_string() });
        }
        last = Some((m.get(1).unwrap_or("").to_string(), m.get(2).unwrap_or("").to_string(), re!("stripped").test(m.get(3).unwrap_or("")), end));
        resume = end;
    }
    if let Some((cls, id, stripped, start)) = last {
        docs.push(Doc { cls, id, stripped, body: text[start..].to_string() });
    }
    docs
}

/// The fileID of `key: {fileID: N...` in a document body.
pub fn file_ref(body: &str, key: &str) -> Option<String> {
    js::group1(&format!(r"{key}: \{{fileID: (-?\d+)"), "", body).map(str::to_string)
}

/// The fileID and GUID of `key: {fileID: N, guid: G...`.
pub fn ref_guid(body: &str, key: &str) -> Option<FileRef> {
    let re = js::cached(&format!(r"{key}: \{{fileID: (-?\d+), guid: ({GUID})"), "");
    let m = re.exec(body)?;
    Some(FileRef { file_id: m.get(1)?.to_string(), guid: m.get(2)?.to_string() })
}

fn nums(body: &str, key: &str, keys: &[&str]) -> Option<Vec<f64>> {
    let line = js::group1(&format!(r"{key}: \{{([^}}]*)\}}"), "", body).filter(|l| !l.is_empty())?;
    let values: Vec<f64> = keys.iter().map(|k| js::group1(&format!(r"\b{k}: (-?[\d.eE+-]+)"), "", line).map(to_number).unwrap_or(f64::NAN)).collect();
    values.iter().all(|v| v.is_finite()).then_some(values)
}

fn array<const N: usize>(v: Vec<f64>) -> [f64; N] {
    std::array::from_fn(|i| v[i])
}

/// Material GUIDs listed under `m_Materials:`; an entry without a GUID is an empty string.
pub fn material_list(body: &str) -> Vec<String> {
    let Some(start) = body.find("m_Materials:") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in body[start..].split('\n').skip(1) {
        if !re!(r"^\s*- ").test(line) {
            break;
        }
        out.push(js::group1(&format!("guid: ({GUID})"), "", line).unwrap_or("").to_string());
    }
    out
}

struct Overrides {
    position: Option<Vec3>,
    rotation: Option<Quat>,
    scale: Option<Vec3>,
    materials: Vec<String>,
    name: Option<String>,
}

/// Largest material slot an override may name; a bigger index is garbage, not a slot.
const MAX_MATERIAL_SLOT: usize = 100_000;

/// Root transform overrides of an instance: the target that carries `m_LocalPosition` / `m_LocalRotation`.
fn instance_overrides(body: &str) -> Overrides {
    struct Modification {
        target: String,
        path: String,
        value: String,
        ref_guid: Option<String>,
    }
    let pattern = format!(r"- target: \{{fileID: (-?\d+)[^}}]*\}}\s*\n\s*propertyPath: ([^\n]+)\n\s*value: ([^\n]*)\n\s*objectReference: \{{fileID: (-?\d+)(?:, guid: ({GUID}))?");
    let mods: Vec<Modification> = js::cached(&pattern, "g")
        .find_all(body)
        .iter()
        .map(|m| Modification {
            target: m.get(1).unwrap_or("").to_string(),
            path: trim(m.get(2).unwrap_or("")).to_string(),
            value: trim(m.get(3).unwrap_or("")).to_string(),
            ref_guid: m.get(5).map(str::to_string),
        })
        .collect();
    // Unity writes position and rotation overrides for an instance's root; the first such target is it.
    let root = mods.iter().find(|m| re!(r"^m_Local(Position|Rotation)\.").test(&m.path)).map(|m| m.target.clone());
    let read = |prop: &str, keys: &[&str]| -> Option<Vec<f64>> {
        let values: Vec<Option<&str>> = keys
            .iter()
            .map(|k| mods.iter().find(|m| Some(&m.target) == root.as_ref() && m.path == format!("{prop}.{k}")).map(|m| m.value.as_str()))
            .collect();
        values.iter().all(|v| v.is_some_and(|v| to_number(v).is_finite())).then(|| values.iter().map(|v| to_number(v.unwrap_or(""))).collect())
    };
    let mut slots: Vec<Option<String>> = Vec::new();
    for m in &mods {
        let Some(index) = js::group1(r"^m_Materials\.Array\.data\[(\d+)\]$", "", &m.path) else {
            continue;
        };
        let (Some(guid), Ok(i)) = (&m.ref_guid, index.parse::<usize>()) else {
            continue;
        };
        if i > MAX_MATERIAL_SLOT {
            continue;
        }
        if slots.len() <= i {
            slots.resize(i + 1, None);
        }
        slots[i] = Some(guid.clone());
    }
    Overrides {
        position: read("m_LocalPosition", &["x", "y", "z"]).map(array),
        rotation: read("m_LocalRotation", &["x", "y", "z", "w"]).map(array),
        scale: read("m_LocalScale", &["x", "y", "z"]).map(array),
        materials: slots.into_iter().map(Option::unwrap_or_default).collect(),
        name: mods.iter().find(|m| m.path == "m_Name").map(|m| m.value.clone()).filter(|v| !v.is_empty()),
    }
}

fn unique(items: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for i in items {
        if !out.contains(&i) {
            out.push(i);
        }
    }
    out
}

pub fn parse_prefab_scene(prefab_text: &str) -> PrefabScene {
    let text = prefab_text.replace("\r\n", "\n");
    let docs = split_docs(&text);
    let mut game_objects: Vec<(&str, &Doc)> = Vec::new();
    for d in docs.iter().filter(|d| d.cls == "1" && !d.stripped) {
        match game_objects.iter_mut().find(|(k, _)| *k == d.id) {
            Some(slot) => slot.1 = d,
            None => game_objects.push((&d.id, d)),
        }
    }
    // GameObject id -> its Transform id.
    let mut transform_of: Vec<(String, String)> = Vec::new();
    let mut nodes: Vec<(String, SceneNode)> = Vec::new();
    let put_node = |nodes: &mut Vec<(String, SceneNode)>, node: SceneNode| match nodes.iter_mut().find(|(k, _)| *k == node.id) {
        Some(slot) => slot.1 = node,
        None => nodes.push((node.id.clone(), node)),
    };
    for d in &docs {
        if d.cls != "4" && d.cls != "224" {
            continue;
        }
        if d.stripped {
            // A transform of a nested instance that something here hangs under.
            put_node(&mut nodes, SceneNode { id: d.id.clone(), name: String::new(), parent: None, position: [0.0; 3], rotation: [0.0, 0.0, 0.0, 1.0], scale: [1.0; 3], active: true, instance: file_ref(&d.body, "m_PrefabInstance") });
            continue;
        }
        let go = file_ref(&d.body, "m_GameObject").unwrap_or_default();
        match transform_of.iter_mut().find(|(k, _)| *k == go) {
            Some(slot) => slot.1 = d.id.clone(),
            None => transform_of.push((go.clone(), d.id.clone())),
        }
        let go_body = game_objects.iter().find(|(k, _)| *k == go).map(|(_, d)| d.body.as_str()).unwrap_or("");
        let father = file_ref(&d.body, "m_Father");
        put_node(
            &mut nodes,
            SceneNode {
                id: d.id.clone(),
                name: re!(r"^\s*m_Name: (.*)$", "m").exec(go_body).and_then(|c| c.get(1)).map(|s| trim(s).to_string()).unwrap_or_default(),
                parent: father.filter(|f| f != "0"),
                position: nums(&d.body, "m_LocalPosition", &["x", "y", "z"]).map_or([0.0; 3], array),
                rotation: nums(&d.body, "m_LocalRotation", &["x", "y", "z", "w"]).map_or([0.0, 0.0, 0.0, 1.0], array),
                scale: nums(&d.body, "m_LocalScale", &["x", "y", "z"]).map_or([1.0; 3], array),
                active: !re!(r"^\s*m_IsActive: 0", "m").test(go_body),
                instance: None,
            },
        );
    }

    let mut renderers: Vec<(String, Vec<String>)> = Vec::new();
    for d in docs.iter().filter(|d| d.cls == "23" && !d.stripped) {
        let go = file_ref(&d.body, "m_GameObject").unwrap_or_default();
        let list = material_list(&d.body);
        match renderers.iter_mut().find(|(k, _)| *k == go) {
            Some(slot) => slot.1 = list,
            None => renderers.push((go, list)),
        }
    }
    let mut meshes = Vec::new();
    for d in &docs {
        if d.stripped || (d.cls != "33" && d.cls != "137") {
            continue;
        }
        let mesh = ref_guid(&d.body, "m_Mesh");
        let go = file_ref(&d.body, "m_GameObject").unwrap_or_default();
        let node = transform_of.iter().find(|(k, _)| *k == go).map(|(_, v)| v.clone());
        let (Some(mesh), Some(node)) = (mesh, node) else {
            continue;
        };
        let skinned = d.cls == "137";
        let materials = if skinned { material_list(&d.body) } else { renderers.iter().find(|(k, _)| *k == go).map(|(_, v)| v.clone()).unwrap_or_default() };
        meshes.push(SceneMesh { node, mesh, materials, skinned });
    }

    let mut instances = Vec::new();
    for d in docs.iter().filter(|d| d.cls == "1001") {
        let Some(source) = ref_guid(&d.body, "m_SourcePrefab").map(|r| r.guid) else {
            continue;
        };
        let parent = file_ref(&d.body, "m_TransformParent").filter(|p| p != "0");
        let o = instance_overrides(&d.body);
        instances.push(SceneInstance { id: d.id.clone(), source, parent, position: o.position, rotation: o.rotation, scale: o.scale, materials: o.materials, name: o.name });
    }

    let scripts = unique(docs.iter().filter(|d| d.cls == "114" && !d.stripped).filter_map(|d| ref_guid(&d.body, "m_Script").map(|r| r.guid)));
    let guids = unique(js::cached(&format!("guid: ({GUID})"), "g").find_all(&text).iter().filter_map(|m| m.get(1).map(str::to_string)));
    let particle_renderers: Vec<&Doc> = docs.iter().filter(|d| d.cls == "199" && !d.stripped).collect();
    PrefabScene {
        nodes,
        meshes,
        instances,
        scripts,
        particle_systems: docs.iter().filter(|d| d.cls == "198" && !d.stripped).count(),
        particle_materials: unique(particle_renderers.iter().flat_map(|d| material_list(&d.body)).filter(|g| !g.is_empty())),
        particle_meshes: unique(
            particle_renderers
                .iter()
                .flat_map(|d| ["m_Mesh", "m_Mesh1", "m_Mesh2", "m_Mesh3"].map(|k| ref_guid(&d.body, k).map(|r| r.guid).unwrap_or_default()))
                .filter(|g| !g.is_empty()),
        ),
        ui_elements: docs.iter().filter(|d| d.cls == "224" && !d.stripped).count(),
        guids,
    }
}

/// Mesh names an FBX's `.meta` gives its mesh fileIDs, when it lists them (`internalIDToNameTable`, or
/// `fileIDToRecycleName` before Unity 2019.3). Newer imports derive fileIDs from names and list
/// nothing: empty. Serialized as `[fileID, name]` pairs, like a JS `Map`.
pub fn fbx_mesh_names(meta_text: &str) -> Vec<(String, String)> {
    let text = meta_text.replace("\r\n", "\n");
    let mut names: Vec<(String, String)> = Vec::new();
    let mut put = |k: &str, v: &str| match names.iter_mut().find(|(n, _)| n == k) {
        Some(slot) => slot.1 = v.to_string(),
        None => names.push((k.to_string(), v.to_string())),
    };
    for m in re!(r"- first:\n\s*43: (-?\d+)\n\s*second: ([^\n]*)", "g").find_all(&text) {
        put(m.get(1).unwrap_or(""), trim(m.get(2).unwrap_or("")));
    }
    let recycle = re!(r"^ {2}fileIDToRecycleName:\n((?: {4}.*\n?)*)", "m").exec(&text).and_then(|c| c.get(1)).unwrap_or("").to_string();
    for m in re!(r"^ {4}(43\d{5}): (.*)$", "gm").find_all(&recycle) {
        put(m.get(1).unwrap_or(""), trim(m.get(2).unwrap_or("")));
    }
    names
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelImportScale {
    pub global_scale: f64,
    pub use_file_scale: bool,
}

/// Unity's import scale settings of a model (`globalScale`, `useFileScale`).
pub fn model_import_scale(meta_text: &str) -> ModelImportScale {
    let g = re!(r"^\s*globalScale: ([\d.eE+-]+)", "m").exec(meta_text).and_then(|c| c.get(1)).map_or(f64::NAN, to_number);
    ModelImportScale { global_scale: if g.is_finite() && g > 0.0 { g } else { 1.0 }, use_file_scale: !re!(r"^\s*useFileScale: 0", "m").test(meta_text) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The header pattern run over the whole text, as the documents were first split.
    fn split_docs_by_scanning(text: &str) -> Vec<Doc> {
        let mut docs = Vec::new();
        let mut last: Option<(String, String, bool, usize)> = None;
        for m in re!(r"^--- !u!(\d+) &(-?\d+)([^\n]*)$", "gm").find_all(text) {
            if let Some((cls, id, stripped, start)) = last.take() {
                docs.push(Doc { cls, id, stripped, body: text[start..m.start()].to_string() });
            }
            last = Some((m.get(1).unwrap_or("").to_string(), m.get(2).unwrap_or("").to_string(), re!("stripped").test(m.get(3).unwrap_or("")), m.end()));
        }
        if let Some((cls, id, stripped, start)) = last {
            docs.push(Doc { cls, id, stripped, body: text[start..].to_string() });
        }
        docs
    }

    fn shape(docs: &[Doc]) -> Vec<(String, String, bool, String)> {
        docs.iter().map(|d| (d.cls.clone(), d.id.clone(), d.stripped, d.body.clone())).collect()
    }

    #[test]
    fn documents_are_split_as_scanning_the_whole_text_does() {
        let texts = [
            "",
            "no headers here\n",
            "%YAML 1.1\n--- !u!1 &100\nGameObject:\n  m_Name: A\n--- !u!4 &200 stripped\nTransform:\n  x: 1\n",
            "--- !u!1 &-5\nbody\r\n--- !u!4 &6\r\nmore\r\n",
            "--- !u!1 &1 trailing words\nx\n--- !u!2 &2",
            "x --- !u!1 &1\nnot at the start of a line\n--- !u!3 &3\nok\n",
            "--- !u!abc &1\nbad class\n--- !u!5 &5\nok\n",
            "--- !u!1 &1\u{2028}--- !u!2 &2\nsecond header sits inside the first line\n--- !u!3 &3\nz\n",
            "a\r--- !u!1 &1\nafter a carriage return\n",
            "--- !u!1 &1\n--- !u!1 &2\n--- !u!1 &3\n",
            "--- !u!114 &11400000\nMonoBehaviour: {fileID: 0}\n  é ü 日本\n--- !u!1 &9\n",
        ];
        for text in texts {
            assert_eq!(shape(&split_docs(text)), shape(&split_docs_by_scanning(text)), "{text:?}");
        }
    }
}
