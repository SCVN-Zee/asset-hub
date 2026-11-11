//! Timings against the budgets of the port: classify a large listing, search 20,000 assets per query,
//! write a unitypackage of 200 files, and the `regress` expression engine against `regex` on the
//! patterns of the classify path.
//!
//! Run with `cargo bench -p asset_hub_core --bench core_bench` (release profile, no framework: a
//! warm-up pass, then the median of several timed runs).

use std::hint::black_box;
use std::time::{Duration, Instant};

use asset_hub_core::export::unitypackage::{plan_package, unity_package_tar, PackageFile, PackageLayout};
use asset_hub_core::handlers::create_default_registry;
use asset_hub_core::js::Re;
use asset_hub_core::search::AssetSearch;
use asset_hub_core::types::{Asset, EntryType, TreeEntry};

/// Small deterministic generator, so every run times the same input.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn pick<'a, T>(&mut self, list: &'a [T]) -> &'a T {
        &list[(self.next() % list.len() as u64) as usize]
    }
}

fn median(mut runs: Vec<Duration>) -> Duration {
    runs.sort();
    runs[runs.len() / 2]
}

fn time<T>(runs: usize, mut f: impl FnMut() -> T) -> Duration {
    black_box(f());
    median((0..runs).map(|_| {
        let start = Instant::now();
        black_box(f());
        start.elapsed()
    }).collect())
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

const NOUNS: &[&str] = &["Fish", "Shark", "Tuna", "Boat", "Rock", "Tree", "Hero", "Wall", "Door", "Chest", "Sword", "Shield", "Barrel", "Crate", "Lamp", "Torch", "Tower", "Bridge", "Gate", "House"];
const ADJ: &[&str] = &["Big", "Small", "Old", "Red", "Blue", "Green", "Dark", "Light", "Wood", "Stone", "Metal", "Gold", "Ice", "Fire", "Cartoon", "Low", "High", "Broken", "Clean", "Dirty"];
const FOLDERS: &[&str] = &["Art", "Models", "Props", "Characters", "Environment", "Textures", "Materials", "Audio", "Fonts", "Prefabs", "Animations", "Fx"];
const EXTS: &[&str] = &["fbx", "png", "mat", "prefab", "anim", "wav", "ttf", "jpg", "cs", "shader", "asset", "unity", "psd", "mp3", "txt"];

fn listing(count: usize) -> Vec<TreeEntry> {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let mut out = Vec::with_capacity(count * 2);
    for i in 0..count {
        let path = format!("Assets/{}/{}/{}/{}{}{:02}.{}", rng.pick(FOLDERS), rng.pick(FOLDERS), rng.pick(NOUNS), rng.pick(ADJ), rng.pick(NOUNS), i % 97, rng.pick(EXTS));
        out.push(TreeEntry { path: format!("{path}.meta"), kind: EntryType::Blob, sha: format!("{:040x}", i * 2 + 1), size: Some(120), lfs: None, lfs_size: None, lfs_oid: None });
        out.push(TreeEntry { path, kind: EntryType::Blob, sha: format!("{:040x}", i * 2), size: Some(2048 + (i as u64 % 5000)), lfs: None, lfs_size: None, lfs_oid: None });
    }
    out
}

fn bench_classify() {
    let registry = create_default_registry();
    for files in [10_000usize, 100_000] {
        let entries = listing(files);
        let t = time(5, || registry.collect_assets(&entries));
        let assets = registry.collect_assets(&entries).len();
        println!("classify      {:>7} entries ({assets} assets): {:>8.2} ms total, {:>6.0} ns per entry", entries.len(), ms(t), t.as_nanos() as f64 / entries.len() as f64);
    }
}

fn bench_search() {
    let registry = create_default_registry();
    let assets: Vec<Asset> = registry.collect_assets(&listing(40_000)).into_iter().take(20_000).collect();
    assert_eq!(assets.len(), 20_000);
    let start = Instant::now();
    let search = AssetSearch::new(&assets, None);
    println!("search        index of {} assets: {:.1} ms", assets.len(), ms(start.elapsed()));

    // Distinct queries (the engine keeps recent answers): words, plurals, typos, groups, folder names, two terms.
    let mut rng = Rng(0xdead_beef_cafe_f00d);
    let mut queries: Vec<String> = Vec::new();
    let mut round = 0;
    while queries.len() < 150 {
        round += 1;
        let noun = rng.pick(NOUNS).to_lowercase();
        let adj = rng.pick(ADJ).to_lowercase();
        let q = match round % 5 {
            0 => noun,
            1 => format!("{noun}s"),
            2 => format!("{}{}", &noun[..noun.len() - 1], "x"),
            3 => format!("{adj} {noun}"),
            _ => format!("{} {}", rng.pick(FOLDERS).to_lowercase(), noun),
        };
        if !queries.contains(&q) {
            queries.push(q);
        }
    }
    let mut slow: Vec<(Duration, &String)> = Vec::new();
    let mut times: Vec<Duration> = queries
        .iter()
        .map(|q| {
            let start = Instant::now();
            black_box(search.search(q));
            let took = start.elapsed();
            if std::env::var_os("BENCH_VERBOSE").is_some() {
                println!("  query {q:?}: {:.2} ms", ms(took));
            }
            slow.push((took, q));
            took
        })
        .collect();
    slow.sort_by_key(|a| std::cmp::Reverse(a.0));
    for (took, q) in slow.iter().take(3) {
        println!("search        slowest: {q:?} {:.2} ms", ms(*took));
    }
    times.sort();
    let mean = times.iter().sum::<Duration>().as_secs_f64() * 1000.0 / times.len() as f64;
    let p = |q: f64| ms(times[((times.len() as f64 * q) as usize).min(times.len() - 1)]);
    println!("search        {} cold queries over 20,000 assets: mean {mean:.2} ms, median {:.2} ms, p95 {:.2} ms, max {:.2} ms (budget 5 ms)", times.len(), p(0.5), p(0.95), p(1.0));
    let again = time(5, || queries.iter().map(|q| search.search(q).assets.len()).sum::<usize>());
    println!("search        same queries answered from the result cache: {:.4} ms each", ms(again) / queries.len() as f64);
}

fn bench_package() {
    let mut rng = Rng(0x1234_5678_9abc_def1);
    let files: Vec<PackageFile> = (0..200)
        .map(|i| {
            let asset: Vec<u8> = (0..64 * 1024).map(|_| (rng.next() & 0xff) as u8).collect();
            let meta = format!("fileFormatVersion: 2\nguid: {:032x}\n", i + 1).into_bytes();
            PackageFile { path: format!("proj/Assets/Art/Models/Model{i}.fbx"), asset, meta, preview: None }
        })
        .collect();
    let t = time(5, || {
        let plan = plan_package(&files, "proj/", PackageLayout::Source, "repo");
        unity_package_tar(&plan, 1_793_000_000).expect("tar").len()
    });
    println!("unitypackage  200 files of 64 KiB (12.5 MiB): plan + tar {:.2} ms", ms(t));
}

/// The patterns of the classify path, on every path of a large listing.
fn bench_engines() {
    let patterns = [
        r"(sdf|font)[^/]*\.asset$|\/fonts?[^/]*\/[^/]*\.asset$",
        r"\.(png|jpe?g|tga|psd|gif|bmp|tiff?)$",
        r"\.(ttf|otf|ttc)$",
        r"\.(wav|mp3|ogg)$",
        r"\.anim$",
        r"\.fbx$",
        r"\.mat$",
        r"\.prefab$",
    ];
    let paths: Vec<String> = listing(50_000).into_iter().map(|e| e.path).collect();
    let regress: Vec<Re> = patterns.iter().map(|p| Re::with_flags(p, "i")).collect();
    let regex: Vec<regex::Regex> = patterns.iter().map(|p| regex::RegexBuilder::new(p).case_insensitive(true).build().expect("pattern")).collect();
    let a = time(5, || paths.iter().map(|p| regress.iter().filter(|r| r.test(p)).count()).sum::<usize>());
    let b = time(5, || paths.iter().map(|p| regex.iter().filter(|r| r.is_match(p)).count()).sum::<usize>());
    let same = paths.iter().all(|p| regress.iter().zip(&regex).all(|(r, x)| r.test(p) == x.is_match(p)));
    let n = (paths.len() * patterns.len()) as f64;
    println!("engines       {} paths x {} patterns: regress {:.1} ms ({:.0} ns per test), regex {:.1} ms ({:.0} ns per test), same answers: {same}", paths.len(), patterns.len(), ms(a), a.as_nanos() as f64 / n, ms(b), b.as_nanos() as f64 / n);
    // Compiling matters too: patterns are built once per process, but a run-time pattern is built per use.
    let c = time(5, || patterns.iter().map(|p| Re::with_flags(p, "i")).collect::<Vec<_>>());
    let d = time(5, || patterns.iter().map(|p| regex::RegexBuilder::new(p).case_insensitive(true).build().expect("pattern")).collect::<Vec<_>>());
    println!("engines       compile 8 patterns: regress {:.3} ms, regex {:.3} ms", ms(c), ms(d));
}

fn main() {
    bench_classify();
    bench_search();
    bench_package();
    bench_engines();
}
