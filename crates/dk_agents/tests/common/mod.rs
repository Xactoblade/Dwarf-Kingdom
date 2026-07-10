//! Shared fixtures for dk_agents integration tests.

use dk_raws::{MaterialCategory, MaterialDef, MaterialRegistry, PlantDef, PlantRegistry, Raws};

pub fn test_raws() -> Raws {
    let m = |id: &str, cat: MaterialCategory| MaterialDef {
        id: id.into(),
        name: id.into(),
        category: cat,
        color: [100, 100, 100],
        value: 1,
    };
    let materials = MaterialRegistry::from_defs(vec![
        m("loam", MaterialCategory::Soil),
        m("limestone", MaterialCategory::Sedimentary),
        m("granite", MaterialCategory::Igneous),
        m("hematite", MaterialCategory::Ore),
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
    Raws { materials, plants }
}
