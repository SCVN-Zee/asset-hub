//! FileIDs of the objects Unity makes when it imports a model (`src/core/unity/importedIds.ts`).
//!
//! Since Unity 2019.3 the `.meta` no longer lists them (`internalIDToNameTable` stays empty): each is
//! derived from the object's type and name, as the signed 64-bit xxHash64 of
//! `Type:<Type>-><Name><index>`, where the index counts objects of that type and name from 0.

use std::collections::HashMap;

const P1: u64 = 11_400_714_785_074_694_791;
const P2: u64 = 14_029_467_366_897_019_727;
const P3: u64 = 1_609_587_929_392_839_161;
const P4: u64 = 9_650_029_242_287_828_579;
const P5: u64 = 2_870_177_450_012_600_261;

fn round(acc: u64, input: u64) -> u64 {
    acc.wrapping_add(input.wrapping_mul(P2)).rotate_left(31).wrapping_mul(P1)
}

fn merge(acc: u64, v: u64) -> u64 {
    (acc ^ round(0, v)).wrapping_mul(P1).wrapping_add(P4)
}

/// xxHash64, seed 0.
pub fn xx_hash64(data: &[u8]) -> u64 {
    let n = data.len();
    let u64_at = |i: usize| u64::from_le_bytes(data[i..i + 8].try_into().expect("8 bytes"));
    let mut i = 0;
    let mut h;
    if n >= 32 {
        let mut v1 = P1.wrapping_add(P2);
        let mut v2 = P2;
        let mut v3 = 0u64;
        let mut v4 = 0u64.wrapping_sub(P1);
        while i + 32 <= n {
            v1 = round(v1, u64_at(i));
            v2 = round(v2, u64_at(i + 8));
            v3 = round(v3, u64_at(i + 16));
            v4 = round(v4, u64_at(i + 24));
            i += 32;
        }
        h = v1.rotate_left(1).wrapping_add(v2.rotate_left(7)).wrapping_add(v3.rotate_left(12)).wrapping_add(v4.rotate_left(18));
        for v in [v1, v2, v3, v4] {
            h = merge(h, v);
        }
    } else {
        h = P5;
    }
    h = h.wrapping_add(n as u64);
    while i + 8 <= n {
        h = (h ^ round(0, u64_at(i))).rotate_left(27).wrapping_mul(P1).wrapping_add(P4);
        i += 8;
    }
    if i + 4 <= n {
        let v = u32::from_le_bytes(data[i..i + 4].try_into().expect("4 bytes")) as u64;
        h = (h ^ v.wrapping_mul(P1)).rotate_left(23).wrapping_mul(P2).wrapping_add(P3);
        i += 4;
    }
    while i < n {
        h = (h ^ (data[i] as u64).wrapping_mul(P5)).rotate_left(11).wrapping_mul(P1);
        i += 1;
    }
    h ^= h >> 33;
    h = h.wrapping_mul(P2);
    h ^= h >> 29;
    h = h.wrapping_mul(P3);
    h ^= h >> 32;
    h
}

/// The fileID Unity gives an imported object (a model's mesh: type `Mesh`), as the `.prefab` writes it.
pub fn imported_object_id(kind: &str, name: &str, index: u32) -> String {
    (xx_hash64(format!("Type:{kind}->{name}{index}").as_bytes()) as i64).to_string()
}

/// Which of a model's meshes (by name, in file order) a mesh fileID stands for; repeated names count up
/// their index. `None` when no mesh has that fileID.
pub fn mesh_for_file_id<S: AsRef<str>>(names: &[S], file_id: &str) -> Option<usize> {
    let mut seen: HashMap<&str, u32> = HashMap::new();
    for (i, name) in names.iter().enumerate() {
        let name = name.as_ref();
        let index = seen.get(name).copied().unwrap_or(0);
        seen.insert(name, index + 1);
        if imported_object_id("Mesh", name, index) == file_id {
            return Some(i);
        }
    }
    None
}
