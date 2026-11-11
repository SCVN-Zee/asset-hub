//! Display helpers shared by the Rust side (`src/core/format.ts`).

use crate::js::to_fixed;

const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];

/// Project path without its first two folders, which are the same for most assets:
/// `Assets/Supercent/X/a.fbx` becomes `../X/a.fbx`.
pub fn short_path(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() > 3 {
        format!("../{}", parts[2..].join("/"))
    } else {
        path.to_string()
    }
}

/// A size in bytes as `1.5 KB`; an unknown, negative or non-finite size is an em dash.
pub fn format_bytes(bytes: Option<f64>) -> String {
    let Some(mut value) = bytes.filter(|b| b.is_finite() && *b >= 0.0) else {
        return "—".to_string();
    };
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    let digits = if unit == 0 || value >= 100.0 { 0 } else { 1 };
    format!("{} {}", to_fixed(value, digits), UNITS[unit])
}
