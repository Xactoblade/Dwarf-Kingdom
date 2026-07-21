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
        self.write(id, &[], materials_ron)
    }

    /// Like `with_materials`, but the manifest declares `overrides`.
    fn with_override(self, id: &str, overrides: &[&str], materials_ron: &str) -> Self {
        self.write(id, overrides, materials_ron)
    }

    /// Write a `mod.ron` and one gems file holding `gems_ron` (a RON list).
    fn with_gems(self, id: &str, gems_ron: &str) -> Self {
        std::fs::create_dir_all(self.0.join("gems")).unwrap();
        std::fs::write(
            self.0.join("mod.ron"),
            format!("ModManifest(id:\"{id}\", name:\"{id} Pack\", version:\"1.0.0\")"),
        )
        .unwrap();
        std::fs::write(self.0.join("gems").join("g.ron"), gems_ron).unwrap();
        self
    }

    fn write(self, id: &str, overrides: &[&str], materials_ron: &str) -> Self {
        std::fs::create_dir_all(self.0.join("materials")).unwrap();
        let ov = overrides
            .iter()
            .map(|s| format!("\"{s}\""))
            .collect::<Vec<_>>()
            .join(", ");
        std::fs::write(
            self.0.join("mod.ron"),
            format!("ModManifest(id:\"{id}\", name:\"{id} Pack\", version:\"1.0.0\", overrides:[{ov}])"),
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
fn an_undeclared_duplicate_id_is_a_hard_error_naming_both_sources() {
    let base = base_dir();
    // A mod redefining a base id WITHOUT declaring the override is an accidental
    // clash and must fail loudly, naming the mod and what it clashed with.
    let m = TmpMod::new("collide").with_materials(
        "collider",
        "[ MaterialDef(id:\"granite\", name:\"Impostor\", category: Igneous, color:(1,1,1), value:9) ]",
    );
    let err = match Raws::load_with_mods(&base, &[m.path()]) {
        Ok(_) => panic!("an undeclared duplicate id must error, not load"),
        Err(e) => e,
    };
    let msg = format!("{err:#}");
    assert!(msg.contains("granite"), "error names the id: {msg}");
    assert!(msg.contains("collider"), "error names the offending mod: {msg}");
    assert!(msg.contains("core"), "error names what it clashed with: {msg}");
}

#[test]
fn a_declared_override_replaces_a_base_material_in_place() {
    let base = base_dir();
    let base_raws = Raws::load(&base).unwrap();
    let granite_idx = base_raws.materials.index_of("granite").unwrap();
    let base_value = base_raws.materials.get(granite_idx).value;

    // Same category (so the world doesn't reshape), a declared override, new value.
    let m = TmpMod::new("recolor").with_override(
        "recolor",
        &["granite"],
        "[ MaterialDef(id:\"granite\", name:\"Granite\", category: Igneous, color:(10,20,30), value:99) ]",
    );
    let raws = Raws::load_with_mods(&base, &[m.path()]).expect("a declared override loads");

    assert_eq!(
        raws.materials.index_of("granite"),
        Some(granite_idx),
        "an override replaces in place — the index (and saved references) don't move"
    );
    assert_eq!(raws.materials.get(granite_idx).value, 99, "the override's fields win");
    assert_ne!(base_value, 99, "and they really differ from the base");
    // Same-id, same-category override does NOT reshape the world.
    assert_eq!(
        raws.content_hash(),
        base_raws.content_hash(),
        "recolouring a stone (same id+category) keeps the same world"
    );
}

// --- gems: the first former-const content axis, now data-driven ---

#[test]
fn base_gems_match_the_canonical_list_in_order() {
    // The shipped data/gems/gems.ron must equal the code mirror canonical_gems(),
    // which reproduces the old GEM_KINDS const in order — so every existing world
    // seed strikes the same gems.
    let raws = Raws::load(&base_dir()).unwrap();
    let canon = dk_raws::canonical_gems();
    assert_eq!(raws.gems.len(), canon.len(), "the base ships exactly the canonical gems");
    for (i, g) in canon.iter().enumerate() {
        assert_eq!(raws.gems.get(i as u16), g, "gem #{i} matches canonical (order + fields)");
    }
}

#[test]
fn a_mod_can_add_a_gem() {
    let base = base_dir();
    let m = TmpMod::new("shinies").with_gems(
        "shinies",
        "[ GemDef(id:\"starstone\", name:\"starstone\", color:(90,90,255), value_tier:6) ]",
    );
    let raws = Raws::load_with_mods(&base, &[m.path()]).expect("load base + gem mod");
    let idx = raws.gems.index_of("starstone").expect("the mod's gem is present");
    assert_eq!(raws.gems.name(idx), "starstone");
    assert_eq!(raws.gems.value_tier(idx), 6);
    assert!(raws.gems.index_of("ruby").is_some(), "base gems remain");
    assert_eq!(raws.gems.len(), dk_raws::canonical_gems().len() + 1, "one gem added");
}

// --- weapons: the second former-enum content axis, now data-driven ---

#[test]
fn base_weapons_match_the_canonical_list_in_order() {
    // data/weapons/weapons.ron must equal canonical_weapons() (the old
    // WeaponKind::ALL order), so a saved weapon's variant index and the forge's
    // melee draw are unchanged.
    let raws = Raws::load(&base_dir()).unwrap();
    let canon = dk_raws::canonical_weapons();
    assert_eq!(raws.weapons.len(), canon.len(), "the base ships exactly the canonical weapons");
    for (i, w) in canon.iter().enumerate() {
        assert_eq!(raws.weapons.get(i as u16), w, "weapon #{i} matches canonical");
    }
    // The melee subset is the first five (crossbow is the ranged one at the end),
    // so the forge's gen_range draw is byte-identical to the old MELEE_WEAPONS.
    assert_eq!(raws.weapons.melee_indices(), vec![0, 1, 2, 3, 4]);
    assert!(raws.weapons.is_ranged(raws.weapons.index_of("crossbow").unwrap()));
}

#[test]
fn a_mod_can_add_a_weapon() {
    let base = base_dir();
    let m = TmpMod::new("warhammers");
    std::fs::create_dir_all(m.path().join("weapons")).unwrap();
    std::fs::write(
        m.path().join("mod.ron"),
        "ModManifest(id:\"warhammers\", name:\"Warhammers\", version:\"1.0.0\")",
    )
    .unwrap();
    std::fs::write(
        m.path().join("weapons").join("w.ron"),
        "[ WeaponDef(id:\"greathammer\", name:\"greathammer\", damage_type: Blunt, heft: 3.0, verb:\"pulverizes\") ]",
    )
    .unwrap();
    let raws = Raws::load_with_mods(&base, &[m.path()]).expect("load base + weapon mod");
    let idx = raws.weapons.index_of("greathammer").expect("the mod's weapon is present");
    assert_eq!(raws.weapons.heft(idx), 3.0);
    assert!(!raws.weapons.is_ranged(idx), "a melee weapon joins the melee pool");
    assert!(raws.weapons.melee_indices().contains(&idx));
    assert!(raws.weapons.index_of("sword").is_some(), "base weapons remain");
}

#[test]
fn required_category_validator_catches_an_emptied_category() {
    // Base has soil/sedimentary/igneous, so it passes.
    let base = Raws::load(&base_dir()).unwrap();
    assert!(dk_raws::validate_required_categories(&base).is_ok());

    // A content set with no Soil material fails, naming the empty category.
    let only_igneous = raws_with(vec![mat("basalt", MaterialCategory::Igneous)]);
    let err = dk_raws::validate_required_categories(&only_igneous).unwrap_err();
    assert!(format!("{err:#}").contains("Soil"), "names the empty category");
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
        gems: dk_raws::GemRegistry::from_defs(dk_raws::canonical_gems()).unwrap(),
        weapons: dk_raws::WeaponRegistry::from_defs(dk_raws::canonical_weapons()).unwrap(),
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
