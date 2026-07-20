//! Slice 1 of the mod API: external mod folders can ADD materials/plants on top
//! of the base data/, and a content fingerprint detects when the content set
//! changed (so a modded world is rebuilt, not silently mis-generated). These
//! tests cover the loader semantics and the fingerprint; the world-rebuild wiring
//! lives in dk_app.

use dk_raws::{
    CombatStats, EconomyConfig, MaterialCategory, MaterialDef, MaterialRegistry, PlantDef,
    PlantRegistry, Raws,
};
use std::path::{Path, PathBuf};

fn base_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data")
}

/// A unique temp dir for one test's mod, cleaned up on drop.
struct TmpMod(PathBuf);
impl TmpMod {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("dk_modtest_{}_{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&dir);
        TmpMod(dir)
    }
    /// Write a `mod.ron` and one materials file holding `materials_ron` (a RON list).
    fn with_materials(self, id: &str, materials_ron: &str) -> Self {
        std::fs::create_dir_all(self.0.join("materials")).unwrap();
        std::fs::write(
            self.0.join("mod.ron"),
            format!("ModManifest(id:\"{id}\", name:\"{id} Pack\", version:\"1.0.0\")"),
        )
        .unwrap();
        std::fs::write(self.0.join("materials").join("m.ron"), materials_ron).unwrap();
        self
    }
    fn path(&self) -> PathBuf {
        self.0.clone()
    }
}
impl Drop for TmpMod {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_mod_folder_adds_its_materials_on_top_of_the_base() {
    let base = base_dir();
    let m = TmpMod::new("add").with_materials(
        "test_stone",
        "[ MaterialDef(id:\"orichalcum\", name:\"Orichalcum\", category: Igneous, color:(200,150,80), value:25) ]",
    );
    let raws = Raws::load_with_mods(&base, &[m.path()]).expect("load base + mod");

    assert!(raws.materials.index_of("orichalcum").is_some(), "the mod's stone is loaded");
    assert!(raws.materials.index_of("granite").is_some(), "base stone still present");
    assert_eq!(raws.mods.len(), 1, "the mod is recorded");
    assert_eq!(raws.mods[0].id, "test_stone");
    assert_eq!(raws.mods[0].version, "1.0.0");
}

#[test]
fn a_mod_redefining_a_base_id_is_a_hard_error() {
    let base = base_dir();
    // No silent override this slice: colliding with a base id must fail loudly.
    let m = TmpMod::new("collide").with_materials(
        "collider",
        "[ MaterialDef(id:\"granite\", name:\"Impostor\", category: Igneous, color:(1,1,1), value:9) ]",
    );
    let err = match Raws::load_with_mods(&base, &[m.path()]) {
        Ok(_) => panic!("a duplicate id must error, not load"),
        Err(e) => e,
    };
    let msg = format!("{err:#}");
    assert!(msg.contains("granite") || msg.contains("duplicate"), "error names the clash: {msg}");
}

#[test]
fn base_only_load_has_no_mods_and_a_stable_hash() {
    let base = base_dir();
    let a = Raws::load(&base).expect("base load");
    let b = Raws::load(&base).expect("base load again");
    assert!(a.mods.is_empty(), "an unmodded game has no mods");
    assert_eq!(a.content_hash(), b.content_hash(), "the same content hashes the same");
}

#[test]
fn adding_a_material_changes_the_content_hash() {
    let base = base_dir();
    let base_hash = Raws::load(&base).unwrap().content_hash();
    let m = TmpMod::new("hash").with_materials(
        "hash_stone",
        "[ MaterialDef(id:\"orichalcum\", name:\"Orichalcum\", category: Igneous, color:(200,150,80), value:25) ]",
    );
    let modded = Raws::load_with_mods(&base, &[m.path()]).unwrap();
    assert_ne!(base_hash, modded.content_hash(), "a modded content set is a different world");
}

// --- content_hash must be sensitive to CATEGORY, not just ids (critique blocker) ---

fn mat(id: &str, cat: MaterialCategory) -> MaterialDef {
    MaterialDef {
        id: id.into(),
        name: id.into(),
        category: cat,
        color: [1, 2, 3],
        value: 1,
        combat: CombatStats::default(),
        is_flux: false,
    }
}

fn raws_with(mats: Vec<MaterialDef>) -> Raws {
    Raws {
        materials: MaterialRegistry::from_defs(mats).unwrap(),
        plants: PlantRegistry::from_defs(vec![PlantDef {
            id: "barley".into(),
            name: "Barley".into(),
            color: [1, 1, 1],
            grow_days: 10,
            seasons: vec![0],
            brewable: true,
        }])
        .unwrap(),
        tileset: None,
        economy: EconomyConfig::default(),
        mods: Vec::new(),
    }
}

#[test]
fn content_hash_distinguishes_a_category_change_even_with_the_same_id() {
    // Worldgen draws from indices_in_category(...), so re-tagging a stone's
    // category shifts the rng stream while leaving its id unchanged. If the hash
    // covered ids only, this divergence would go undetected.
    let igneous = raws_with(vec![mat("stone", MaterialCategory::Igneous)]);
    let ore = raws_with(vec![mat("stone", MaterialCategory::Ore)]);
    let igneous_again = raws_with(vec![mat("stone", MaterialCategory::Igneous)]);

    assert_ne!(
        igneous.content_hash(),
        ore.content_hash(),
        "same id, different category must hash differently"
    );
    assert_eq!(
        igneous.content_hash(),
        igneous_again.content_hash(),
        "identical content hashes identically"
    );
}
