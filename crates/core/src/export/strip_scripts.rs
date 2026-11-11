//! Removes script components from a prefab's YAML (`src/core/export/stripScripts.ts`).

use std::collections::HashSet;

use serde::Serialize;

use crate::re;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Stripped {
    pub text: String,
    pub removed: usize,
}

/// Replaces each match of `re` by the text `edit` returns for it; `None` keeps the match as it was.
fn replace_with(re: &crate::js::Re, text: &str, mut edit: impl FnMut(&crate::js::Caps<'_>) -> Option<String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for m in re.find_all(text) {
        out.push_str(&text[last..m.start()]);
        match edit(&m) {
            Some(replacement) => out.push_str(&replacement),
            None => out.push_str(m.all()),
        }
        last = m.end();
    }
    out.push_str(&text[last..]);
    out
}

/// Removes the components of the given scripts (MonoBehaviours by script GUID) from a prefab's YAML:
/// their documents, the GameObjects' `- component:` lines pointing at them, and `m_AddedComponents`
/// entries adding them to nested prefabs. Everything else (Transforms, renderers, particle systems...)
/// stays.
pub fn strip_scripts(text: &str, scripts: &HashSet<String>) -> Stripped {
    // Unity YAML: "--- !u!<class> &<fileID>" starts each object.
    let docs = re!(r"(?=^--- !u!)", "m").split(text);
    let mut removed: HashSet<String> = HashSet::new();
    let mut kept: Vec<&str> = Vec::with_capacity(docs.len());
    for doc in docs {
        let head = re!(r"^--- !u!114 &(-?\d+)(?! stripped)").exec(doc);
        let script = head.as_ref().and_then(|_| re!(r"^ {2}m_Script: \{fileID: -?\d+, guid: ([0-9a-f]{32})", "m").exec(doc)).and_then(|c| c.get(1));
        match (head, script) {
            (Some(head), Some(script)) if scripts.contains(script) => {
                removed.insert(head.get(1).unwrap_or("").to_string());
            }
            _ => kept.push(doc),
        }
    }
    if removed.is_empty() {
        return Stripped { text: text.to_string(), removed: 0 };
    }
    let joined = kept.concat();
    let out = replace_with(re!(r"^ *- component: \{fileID: (-?\d+)\}\n", "gm"), &joined, |m| removed.contains(m.get(1).unwrap_or("")).then(String::new));
    let out = replace_with(
        re!(r"^( *)- targetCorrespondingSourceObject: .*\n\1 {2}insertIndex: .*\n\1 {2}addedObject: \{fileID: (-?\d+)\}\n", "gm"),
        &out,
        |m| removed.contains(m.get(2).unwrap_or("")).then(String::new),
    );
    // A list left without entries is written as [] by Unity.
    let out = replace_with(re!(r"^( *)(m_Component|m_AddedComponents):\n(?!\1[ -])", "gm"), &out, |m| Some(format!("{}{}: []\n", m.get(1).unwrap_or(""), m.get(2).unwrap_or(""))));
    Stripped { text: out, removed: removed.len() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_changes_when_no_script_matches() {
        let text = "%YAML 1.1\n--- !u!1 &100\nGameObject:\n  m_Component:\n  - component: {fileID: 101}\n";
        let out = strip_scripts(text, &HashSet::from(["a".repeat(32)]));
        assert_eq!(out, Stripped { text: text.into(), removed: 0 });
    }

    #[test]
    fn a_component_list_left_empty_becomes_an_empty_array() {
        let guid = "a".repeat(32);
        let text = format!("--- !u!1 &100\nGameObject:\n  m_Component:\n  - component: {{fileID: 102}}\n  m_Name: R\n--- !u!114 &102\nMonoBehaviour:\n  m_Script: {{fileID: 11500000, guid: {guid}, type: 3}}\n");
        let out = strip_scripts(&text, &HashSet::from([guid]));
        assert_eq!(out.removed, 1);
        assert_eq!(out.text, "--- !u!1 &100\nGameObject:\n  m_Component: []\n  m_Name: R\n");
    }
}
