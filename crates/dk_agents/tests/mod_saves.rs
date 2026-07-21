//! Slice 3 of the mod API: a fort save records which mods made it, and refuses
//! to load without them — with a clear, named message rather than a cryptic
//! "material X missing" from the remap.

mod common;

use dk_agents::{load_sim, save_sim, Sim};
use dk_raws::{ModManifest, Raws};

fn tmp_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("dk_modsave_{}_{}.bin", std::process::id(), name))
}

/// A copy of the test raws that declares one mod (no new content — just the
/// stamp, which is what the save records and checks).
fn modded_raws() -> Raws {
    let mut raws = common::test_raws();
    raws.mods.push(ModManifest {
        id: "cool_mod".into(),
        name: "Cool Mod".into(),
        version: "1.0.0".into(),
        target_game_version: String::new(),
        load_after: Vec::new(),
        overrides: Vec::new(),
    });
    raws
}

fn small_fort(raws: &Raws, seed: u64) -> Sim {
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 24, 24, 12, seed);
    Sim::new(map, raws, rng, 2)
}

#[test]
fn a_fort_saved_with_mods_wont_load_without_them() {
    let raws = modded_raws();
    let sim = small_fort(&raws, 1);
    let path = tmp_path("needs_mod");
    save_sim(&sim, &path, &raws).expect("save");

    // Same mods present -> loads fine.
    assert!(load_sim(&path, &raws).is_ok(), "loads with the mods it was made with");

    // Mods absent -> refused, and the message names the missing mod.
    let vanilla = common::test_raws();
    let err = match load_sim(&path, &vanilla) {
        Ok(_) => panic!("a modded fort must refuse to load unmodded"),
        Err(e) => e,
    };
    let msg = format!("{err:#}");
    assert!(msg.contains("cool_mod"), "the message names the needed mod: {msg}");
    assert!(msg.contains("1.0.0"), "and its version: {msg}");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn an_unmodded_fort_still_round_trips() {
    let raws = common::test_raws(); // no mods
    let sim = small_fort(&raws, 2);
    let path = tmp_path("vanilla");
    save_sim(&sim, &path, &raws).expect("save");
    assert!(load_sim(&path, &raws).is_ok(), "a vanilla fort loads in a vanilla game");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_different_mod_version_is_treated_as_a_different_mod() {
    let raws = modded_raws(); // cool_mod v1.0.0
    let sim = small_fort(&raws, 3);
    let path = tmp_path("version");
    save_sim(&sim, &path, &raws).expect("save");

    // Same id, bumped version -> refused (content may differ between versions).
    let mut bumped = common::test_raws();
    bumped.mods.push(ModManifest {
        id: "cool_mod".into(),
        name: "Cool Mod".into(),
        version: "1.1.0".into(),
        target_game_version: String::new(),
        load_after: Vec::new(),
        overrides: Vec::new(),
    });
    let err = match load_sim(&path, &bumped) {
        Ok(_) => panic!("a version bump must refuse to load"),
        Err(e) => e,
    };
    assert!(format!("{err:#}").contains("cool_mod"), "names the mismatched mod");
    let _ = std::fs::remove_file(&path);
}
