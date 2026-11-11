//! Dependencies of an animation clip, and the models that may carry its rig (`src/core/deps/anim.ts`).

use std::collections::HashSet;

use crate::deps::context::{entry_size, unique, ByPath, DepContext, GuidKind, OrderedSet};
use crate::deps::types::{DependencyReport, DependencyWarning, PartKind, PartRef, ReportKind};
use crate::errors::SourceError;
use crate::js::{self, len16, slice_from, slice_units};
use crate::re;
use crate::registry::{base_name, dir_name};
use crate::unity::anim_clip::{parse_anim_clip, ClipKind};

const MAX_RIGS: usize = 8;

/// Folder names that say nothing about which character a clip is for.
const GENERIC: &[&str] = &[
    "animation", "animations", "anim", "anims", "animator", "animators", "clip", "clips", "motion", "motions", "enemy", "enemies", "character", "characters", "model", "models", "fbx", "fbxs", "art", "assets",
    "prefab", "prefabs", "old", "new", "common", "basic", "controller", "controllers", "override",
];

/// Words of a clip's name that say what it does, not who does it ("Customer_Idle": "idle").
const ACTIONS: &[&str] = &[
    "idle", "run", "running", "walk", "walking", "jog", "sprint", "attack", "atk", "melee", "skill", "cast", "spell", "shoot", "fire", "aim", "reload", "throw", "hit", "hurt", "damage", "die", "dying", "death",
    "dead", "stun", "knockback", "jump", "fall", "land", "landing", "spawn", "appear", "disappear", "intro", "outro", "pick", "pickup", "carry", "stack", "drop", "push", "pull", "punch", "kick", "block",
    "dodge", "dash", "roll", "slide", "climb", "swim", "fly", "flying", "sit", "sitting", "stand", "standing", "talk", "wave", "dance", "victory", "win", "lose", "fail", "cheer", "celebrate", "happy", "sad",
    "angry", "eat", "eating", "drink", "sleep", "work", "working", "cut", "chop", "dig", "mine", "move", "turn", "left", "right", "front", "back", "forward", "backward", "start", "end", "loop", "pose", "tpose",
    "action", "emote", "react", "reaction", "fast", "slow", "combo", "charge", "swing", "slash", "stab", "catch", "hold", "open", "close", "upgrade",
];

/// Models named in the clip's own name, slipped in right after the best guess (the rig check then picks among them).
const NAMED_EXTRA: usize = 2;

/// Animator controllers read to find the ones playing a clip, nearest first (a few KB each).
const CONTROLLERS_READ: usize = 24;

/// `name.replace(/\.[^.]+$/, '')`: drops the last extension.
fn strip_ext(name: &str) -> &str {
    match name.rfind('.') {
        Some(i) if i + 1 < name.len() => &name[..i],
        _ => name,
    }
}

/// `s.toLowerCase().split(/[^a-z0-9]+/)`, without the empty pieces.
fn words(s: &str) -> Vec<String> {
    s.to_lowercase().split(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit())).filter(|w| !w.is_empty()).map(str::to_string).collect()
}

/// Words that can name a character: longer than two letters and not generic.
fn split(s: &str) -> Vec<String> {
    words(s).into_iter().filter(|w| w.len() > 2 && !GENERIC.contains(&w.as_str())).collect()
}

/// Words of a file name that can name something ("Customer_Red.fbx": customer, red), trailing digits
/// and generic words left out.
fn name_words(file: &str) -> Vec<String> {
    words(strip_ext(file))
        .into_iter()
        .map(|w| w.trim_end_matches(|c: char| c.is_ascii_digit()).to_string())
        .filter(|w| w.len() > 2 && !GENERIC.contains(&w.as_str()))
        .collect()
}

/// Models whose file name has one of `named` ("customer" for "Customer_Red.fbx"), most words first,
/// then those whose name starts with one. Animation-only files are left out.
fn named_models(named: &[String], models: &[String]) -> Vec<String> {
    if named.is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<(&String, usize, bool)> = models
        .iter()
        .filter(|p| !base_name(p).contains('@'))
        .map(|p| {
            let own = name_words(base_name(p));
            let hits = named.iter().filter(|w| own.iter().any(|o| o.contains(w.as_str()))).count();
            let starts = named.iter().any(|w| own.first().is_some_and(|o| o.starts_with(w.as_str())));
            (p, hits, starts)
        })
        .filter(|m| m.1 > 0)
        .collect();
    hits.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.cmp(&a.2)).then(len16(a.0).cmp(&len16(b.0))));
    hits.into_iter().map(|m| m.0.clone()).collect()
}

/// Folders shared from the root between two folder lists.
fn shared_prefix(a: &[&str], b: &[&str]) -> usize {
    a.iter().zip(b).take_while(|(x, y)| x == y).count()
}

/// Models that may carry the rig a clip animates, most likely first, and only related ones. The clip's
/// folders name the character ("Enemy/Goblin_Melee/Skill1.anim" and "FBXs/Enemy/Enemy_Goblin.fbx"); its
/// file name only names the action ("Idle" would match every character's idle animation file). Models
/// sharing no character word with the clip are kept only from its own folder; with none related at all,
/// the closest few are offered. Animation-only files (Model@Run.fbx, FBX under an Animation folder) come
/// after models. When the clip's name also names someone ("Customer_Idle" in a folder named after the
/// game), the models named so are tried right after the best guess. Animator controllers that play the
/// clip (`controllers`) say best whose it is: models they name go first.
pub fn rig_candidates(anim_path: &str, root: &str, by_path: &ByPath, limit: usize, controllers: &[String]) -> Vec<String> {
    let root_units = len16(root);
    let anim_dir = dir_name(anim_path);
    let folders: Vec<&str> = slice_from_units(anim_dir, root_units).split('/').collect();
    // The clip's folder and the one above it ("Player/WeaponAnims", "Enemy/Goblin_Melee").
    let character: HashSet<String> = folders[folders.len().saturating_sub(2)..].iter().flat_map(|f| split(f)).collect();
    let action: HashSet<String> = split(strip_ext(base_name(anim_path))).into_iter().collect();
    let near: Vec<&str> = anim_dir.split('/').collect();
    let assets = format!("{root}Assets/");
    let mut scored: Vec<(String, i64, bool)> = Vec::new();
    for path in by_path.keys() {
        if !re!(r"\.fbx$", "i").test(path) || !path.starts_with(&assets) {
            continue;
        }
        let folder = dir_name(path);
        let parts: Vec<&str> = folder.split('/').collect();
        let shared = shared_prefix(&near, &parts);
        // Words of the model's name and its own folder ("Characters/Goblin/Model.fbx").
        let own: HashSet<String> = split(strip_ext(base_name(path))).into_iter().chain(split(base_name(folder))).collect();
        // "Frog" names "Frogking"; but "WeaponAnims" does not name every weapon: only a model word may contain the clip's.
        let character_hits = character.iter().filter(|w| own.contains(*w) || own.iter().any(|o| o.contains(w.as_str()))).count();
        let action_hits = action.iter().filter(|w| own.contains(*w)).count();
        let animation_only = base_name(path).contains('@') || re!(r"/anim(ation)?s?/", "i").test(slice_from_units(path, root_units));
        let same_folder = folder == anim_dir;
        let score = character_hits as i64 * 15 + action_hits as i64 * 2 + shared as i64 * 2 + if same_folder { 10 } else { 0 } - if animation_only { 8 } else { 0 };
        scored.push((path.to_string(), score, character_hits > 0 || same_folder));
    }
    scored.sort_by(|a, b| b.1.cmp(&a.1).then(len16(&a.0).cmp(&len16(&b.0))));
    let related: Vec<&(String, i64, bool)> = scored.iter().filter(|s| s.2).collect();
    let ranked: Vec<String> = if related.is_empty() { scored.iter().take(3).map(|s| s.0.clone()).collect() } else { related.iter().map(|s| s.0.clone()).collect() };
    let all: Vec<String> = scored.iter().map(|s| s.0.clone()).collect();
    let clip_words: Vec<String> = name_words(base_name(anim_path)).into_iter().filter(|w| !ACTIONS.contains(&w.as_str()) && !character.contains(w)).collect();
    let by_controller = named_models(&unique(controllers.iter().flat_map(|c| name_words(base_name(c)))), &all);
    let by_name = named_models(&clip_words, &all);
    let mut out = OrderedSet::default();
    out.extend(by_controller.into_iter().take(NAMED_EXTRA));
    out.extend(ranked.iter().take(1).cloned());
    out.extend(by_name.into_iter().take(NAMED_EXTRA));
    out.extend(ranked.iter().skip(1).cloned());
    out.into_vec().into_iter().take(limit).collect()
}

/// `text.slice(units)` for a number of UTF-16 units.
fn slice_from_units(text: &str, units: usize) -> &str {
    let mut used = 0;
    for (i, c) in text.char_indices() {
        if used >= units {
            return &text[i..];
        }
        used += c.len_utf16();
    }
    if used >= units {
        slice_from(text, text.len())
    } else {
        ""
    }
}

/// A clip needs only what it references by GUID (sprites of a sprite animation, objects in events).
pub async fn resolve_animation_dependencies<C: DepContext>(anim_path: &str, ctx: &C) -> Result<DependencyReport, SourceError> {
    let mut files: OrderedSet<String> = OrderedSet::default();
    files.add(anim_path.to_string());
    let mut warnings: Vec<DependencyWarning> = Vec::new();
    let meta = format!("{anim_path}.meta");
    if ctx.by_path().has(&meta) {
        files.add(meta);
    } else {
        warnings.push(DependencyWarning::NoMeta);
    }
    let text = ctx.read_texts(&[anim_path.to_string()]).await?.remove(anim_path).unwrap_or_default();
    let guids: Vec<String> = unique(re!(r"guid: ([0-9a-f]{32})", "g").find_all(&text).iter().filter_map(|m| m.get(1).map(str::to_string))).into_iter().filter(|g| !re!(r"^0{16}").test(g)).collect();
    ctx.prepare(&guids, GuidKind::Texture, &[]).await?;
    let unresolved: Vec<String> = guids.iter().filter(|g| ctx.path_for_guid(g).is_none()).cloned().collect();
    ctx.prepare(&unresolved, GuidKind::Other, &[]).await?;
    let mut parts: Vec<PartRef> = Vec::new();
    for guid in &guids {
        let path = ctx.path_for_guid(guid);
        let unsure = path.is_none() && ctx.unsure(guid).unwrap_or(false);
        if path.is_none() && !unsure {
            continue;
        }
        if let Some(p) = &path {
            files.add(p.clone());
            let m = format!("{p}.meta");
            if ctx.by_path().has(&m) {
                files.add(m);
            }
        }
        parts.push(PartRef {
            kind: PartKind::Other,
            guid: guid.clone(),
            name: path.as_deref().map_or_else(|| slice_units(guid, 0, 8).to_string(), |p| base_name(p).to_string()),
            size: path.as_deref().and_then(|p| entry_size(ctx.by_path().get(p))),
            missing: path.is_none(),
            unsure: unsure.then_some(true),
            path,
            uses: 1,
        });
    }
    // Only clips that move transforms need a rig to be seen.
    let info = parse_anim_clip(&text);
    let rigs = if matches!(info.kind, ClipKind::Generic | ClipKind::Legacy) {
        rig_candidates(anim_path, ctx.root(), ctx.by_path(), MAX_RIGS, &controllers_playing(anim_path, ctx).await?)
    } else {
        Vec::new()
    };
    let mut report = DependencyReport::new(ReportKind::None);
    report.partial = parts.iter().any(|p| p.unsure == Some(true)).then_some(true);
    report.warnings = warnings;
    report.files = files.into_vec();
    report.parts = Some(parts);
    report.rigs = Some(rigs);
    Ok(report)
}

/// Animator controllers (of the nearest few) whose states play this clip, found by its GUID.
///
/// The TypeScript kept the controller texts it had read for the commit; here every read goes through
/// the context, whose blob cache gives the same saving.
pub async fn controllers_playing<C: DepContext>(anim_path: &str, ctx: &C) -> Result<Vec<String>, SourceError> {
    let meta = format!("{anim_path}.meta");
    if !ctx.by_path().has(&meta) {
        return Ok(Vec::new());
    }
    let texts = ctx.read_texts(std::slice::from_ref(&meta)).await?;
    let Some(guid) = js::group1(r"guid: ([0-9a-f]{32})", "", texts.get(&meta).map(String::as_str).unwrap_or("")).map(str::to_string) else { return Ok(Vec::new()) };
    let near: Vec<&str> = dir_name(anim_path).split('/').collect();
    let mut nearest: Vec<(&str, usize)> = ctx
        .by_path()
        .keys()
        .filter(|p| re!(r"\.(controller|overrideController)$", "i").test(p) && p.starts_with(ctx.root()))
        .map(|p| (p, shared_prefix(&near, &dir_name(p).split('/').collect::<Vec<_>>())))
        .collect();
    nearest.sort_by(|a, b| b.1.cmp(&a.1).then(len16(a.0).cmp(&len16(b.0))));
    let nearest: Vec<String> = nearest.into_iter().take(CONTROLLERS_READ).map(|c| c.0.to_string()).collect();
    if nearest.is_empty() {
        return Ok(Vec::new());
    }
    let texts = ctx.read_texts(&nearest).await?;
    Ok(nearest.into_iter().filter(|p| texts.get(p).is_some_and(|t| t.contains(&guid))).collect())
}
