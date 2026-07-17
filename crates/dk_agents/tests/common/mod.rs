//! Shared fixtures for dk_agents integration tests.

use dk_raws::{CombatStats, MaterialCategory, MaterialDef, MaterialRegistry, PlantDef, PlantRegistry, Raws};

pub fn test_raws() -> Raws {
    let m = |id: &str, cat: MaterialCategory, combat: CombatStats| MaterialDef {
        id: id.into(),
        name: id.into(),
        category: cat,
        color: [100, 100, 100],
        value: 1,
        combat,
    };
    let stone = CombatStats::default();
    // Hematite forges into the fort's iron: a real weapon metal, so combat
    // tests have something with an edge to swing.
    let iron = CombatStats { sharpness: 1.0, density: 7.8, hardness: 100.0 };
    let wood = CombatStats { sharpness: 0.1, density: 0.7, hardness: 8.0 };
    let materials = MaterialRegistry::from_defs(vec![
        m("loam", MaterialCategory::Soil, stone),
        m("limestone", MaterialCategory::Sedimentary, stone),
        m("granite", MaterialCategory::Igneous, stone),
        m("hematite", MaterialCategory::Ore, iron),
        // Two wood species so tests can exercise the wooden-goods chain.
        m("oak", MaterialCategory::Wood, wood),
        m("pine", MaterialCategory::Wood, wood),
    ])
    .unwrap();
    let plants = PlantRegistry::from_defs(vec![
        PlantDef {
            id: "barley".into(),
            name: "Barley".into(),
            color: [196, 168, 98],
            grow_days: 10,
            seasons: vec![0, 1, 2],
            brewable: true,
        },
        PlantDef {
            id: "potato".into(),
            name: "Potato".into(),
            color: [168, 140, 92],
            grow_days: 8,
            seasons: vec![0, 1, 2],
            brewable: false,
        },
    ])
    .unwrap();
    Raws { materials, plants, tileset: None }
}
