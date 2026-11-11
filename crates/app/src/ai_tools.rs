//! What the AI pet can do in the hub (`src/shared/aiTools.ts`, `src/main/ai.ts`).
//!
//! Codex calls these (dynamic tools); the app hands each call to the hub, which acts on what the user
//! is looking at and answers. Nothing here writes to GitHub or to disk.

use serde_json::{json, Value};

use crate::dto::Language;

pub const AI_TOOL_NAMES: [&str; 7] = ["name_words", "search_assets", "open_asset", "asset_info", "add_to_chest", "add_to_collection", "offer_choices"];

/// The hub's type tabs, in tab order: what `search_assets` can be limited to. The registry of asset
/// handlers (the hidden ones left out) is the source; this list stands in for it until the core
/// handlers are ported, and must follow the tab order of `src/core/handlers`.
pub const ASSET_TYPES: [&str; 9] = ["fbx", "prefab", "vfx", "texture", "sprite", "material", "audio", "animation", "font"];

fn language_name(language: Language) -> &'static str {
    match language {
        Language::En => "English",
        Language::Vi => "Vietnamese",
        Language::Ko => "Korean",
    }
}

/// What the assistant is told about itself and its limits.
pub fn instructions(language: Language) -> String {
    let language_name = language_name(language);
    [
        r##"You are the assistant inside Asset Hub, a desktop app where game artists and developers at Supercent browse Unity projects on GitHub"##.to_owned(),
        r##"(models, prefabs, VFX, textures, sprites, materials, audio, animations, fonts), preview them and export them as .unitypackage files."##.to_owned(),
        format!("Answer in {} unless the user writes in another language.", language_name),
        r##"You live in a small chat bubble in the corner of the app, like a pet: reply in one or two short sentences (under 40 words), friendly and to the point."##.to_owned(),
        r##"No headings, tables or long lists; at most three short bullet points when a list is really needed."##.to_owned(),
        r##"You cannot run commands, read or write files, or browse the web, and you never change anything on GitHub."##.to_owned(),
        r##"Use the tools to act in the hub. search_assets shows its results in the hub itself, so do not list them back:"##.to_owned(),
        r##"name the best one or two and say the rest are on screen. Open an asset when the user wants to see it; use refs exactly as the tools return them."##.to_owned(),
        r##"Add to the chest or a collection only when asked; exporting is done by the user with the Export button."##.to_owned(),
        r##"Where to search: if the user names a repo ("in repo pizza", "trong luna_pizza"), search with repo set to that name — every branch of it, no question."##.to_owned(),
        r##"If they say this branch, all branches or every repo, use where=branch, where=repo or where=all — no question. If an earlier message in the"##.to_owned(),
        r##"conversation already settled where, keep using it. Only when nothing tells you where, call offer_choices with "this branch" and"##.to_owned(),
        r##""all branches" in their language (e.g. "Branch này", "Mọi branch") and other=true, and ask where in one line."##.to_owned(),
        r##"Questions in general: ask only when the answer is truly unknown and changes what you do; then offer 2-4 choices instead of asking in words,"##.to_owned(),
        r##"at most once per request. If a tool says a name is ambiguous, offer the candidates as choices."##.to_owned(),
        r##"Search keywords: file names are English words, often joined (FarmMachine, CuttingMachine). Unless the user gave a plain English file-name word,"##.to_owned(),
        r##"first call name_words for the same repo/scope and type, pick every word from that list that could mean what the user wants, and search them"##.to_owned(),
        r##"all at once separated by " | " (any matches), plus the plain English word for it, since joined names hide words from that list"##.to_owned(),
        r##"(NewXRayMachine lists as "ray", "machine"): "máy cắt lúa" → "cuttingmachine | harvester | farm machine | tractor"; "máy x-quang" → "xray | x ray | scanner"."##.to_owned(),
        r##"Words inside one alternative must all match, so keep each alternative to one idea. Search once like that; retry only if nothing comes up."##.to_owned(),
        r##"If the user says a result is not what they meant, look at the other results (asset_info) or search again with different names, not the same ones."##.to_owned(),
        r##"Type tabs: fbx (models), prefab, vfx (effects), texture, sprite, material, audio, animation, font. Search with short English keywords as in file names."##.to_owned(),
    ]
    .join(" ")
}

/// An asset as the tools name it: "owner/repo@branch:path/in/repo".
const REF: &str = r#"An asset reference exactly as returned by search_assets ("owner/repo@branch:path")."#;

/// Tool definitions for Codex; `types` are the asset types the hub knows (its tabs).
pub fn tool_specs(types: &[&str]) -> Value {
    json!([
        {
            "type": "function",
            "name": "name_words",
            "description": "The words asset file names are actually made of in a scope (repo / branch, optionally one type), most used first. Call it before searching for anything described by meaning, in another language or by look, then search with words from this list.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "type": { "type": "string", "enum": types },
                    "where": { "type": "string", "enum": ["branch", "repo", "all"] },
                    "repo": { "type": "string", "description": "A repo the user named; part of its name is enough" }
                },
                "additionalProperties": false
            }
        },
        {
            "type": "function",
            "name": "search_assets",
            "description": "Search Unity assets and show the results in the hub (it fills the search box and picks the type tab). Searches the open branch, every branch of the open repo, or every indexed repo. Understands synonyms and typos; use short English keywords as in file names (e.g. \"fire\", \"explosion smoke\"). Returns the best matches with refs.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Keywords; \" | \" between alternatives matches any of them, e.g. \"fire | flame | burn\"" },
                    "type": { "type": "string", "enum": types, "description": "Only this asset type (tab), e.g. vfx for effects" },
                    "where": { "type": "string", "enum": ["branch", "repo", "all"], "description": "branch: the open branch (default); repo: every branch of the open repo; all: every repo" },
                    "repo": { "type": "string", "description": "A repo the user named; part of its name is enough (\"pizza\"). Opens that repo in the hub and searches every branch of it (where is then ignored)." }
                },
                "required": ["query"],
                "additionalProperties": false
            }
        },
        {
            "type": "function",
            "name": "open_asset",
            "description": "Open an asset in the hub: its repo and branch, with the preview and details shown.",
            "inputSchema": { "type": "object", "properties": { "ref": { "type": "string", "description": REF } }, "required": ["ref"], "additionalProperties": false }
        },
        {
            "type": "function",
            "name": "asset_info",
            "description": "Details of an asset: size, measured stats (triangles, textures…), whether it fits the playable budget, and what it depends on.",
            "inputSchema": { "type": "object", "properties": { "ref": { "type": "string", "description": REF } }, "required": ["ref"], "additionalProperties": false }
        },
        {
            "type": "function",
            "name": "add_to_chest",
            "description": "Add assets of the open branch to the chest (the export list). The user exports it themselves.",
            "inputSchema": {
                "type": "object",
                "properties": { "refs": { "type": "array", "items": { "type": "string", "description": REF }, "minItems": 1, "maxItems": 20 } },
                "required": ["refs"],
                "additionalProperties": false
            }
        },
        {
            "type": "function",
            "name": "add_to_collection",
            "description": "Add assets to a collection by name (made when it does not exist). \"Favourites\" is the starred list.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "refs": { "type": "array", "items": { "type": "string", "description": REF }, "minItems": 1, "maxItems": 20 },
                    "collection": { "type": "string" }
                },
                "required": ["refs", "collection"],
                "additionalProperties": false
            }
        },
        {
            "type": "function",
            "name": "offer_choices",
            "description": "Show reply buttons under your message, so the user answers with one click. Use it instead of asking in words when a request is unclear and the answer changes what you do. Then end your turn with a one-line question. A clicked button sends its label as the user's reply.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "options": { "type": "array", "items": { "type": "string", "maxLength": 40 }, "minItems": 2, "maxItems": 4, "description": "Short labels in the user's language" },
                    "other": { "type": "boolean", "description": "Add an \"Other…\" button that lets the user type their own answer" }
                },
                "required": ["options"],
                "additionalProperties": false
            }
        }
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defines_exactly_the_named_tools() {
        let specs = tool_specs(&["vfx"]);
        let names: Vec<&str> = specs.as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(names, AI_TOOL_NAMES);
        assert_eq!(specs[1]["inputSchema"]["properties"]["type"]["enum"], json!(["vfx"]));
        assert_eq!(specs[1]["inputSchema"]["required"], json!(["query"]));
    }

    #[test]
    fn tells_the_assistant_the_language_and_its_limits() {
        let text = instructions(Language::Vi);
        assert!(text.contains("Answer in Vietnamese unless the user writes in another language."));
        assert!(text.contains("You cannot run commands, read or write files, or browse the web, and you never change anything on GitHub."));
        assert!(text.contains("Type tabs: fbx (models)"));
        assert!(instructions(Language::Ko).contains("Answer in Korean"));
    }

    #[test]
    fn offers_the_tabs_of_the_visible_asset_types() {
        for t in ASSET_TYPES {
            assert!(instructions(Language::En).contains(t), "{t}");
        }
    }
}
