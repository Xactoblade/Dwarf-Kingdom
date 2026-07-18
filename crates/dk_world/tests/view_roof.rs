//! The roof of the world: the highest z with anything in it.
//!
//! The renderer only reaches a few levels below the view for something to
//! draw, so a view above the roof is a black void — with the minimap still
//! cheerfully showing the fort, which reads as a broken game. The app clamps
//! the z-view here.

use rand_chacha::rand_core::SeedableRng;
use dk_raws::{MaterialCategory, MaterialDef, MaterialRegistry};
fn regs() -> MaterialRegistry {
    let m = |id: &str, cat: MaterialCategory| MaterialDef {
        id: id.into(), name: id.into(), category: cat, color: [100,100,100], value: 1,
        combat: Default::default(), is_flux: false,
    };
    MaterialRegistry::from_defs(vec![
        m("loam", MaterialCategory::Soil),
        m("limestone", MaterialCategory::Sedimentary),
        m("granite", MaterialCategory::Igneous),
        m("hematite", MaterialCategory::Ore),
    ]).unwrap()
}
#[test]
fn the_roof_of_the_world_is_where_the_terrain_stops() {
    let mats = regs();
    for seed in 0..4u64 {
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(seed);
        let map = dk_world::generate(&mats, &mut rng, 96, 96, 32, seed);
        let roof = map.highest_solid_z();
        assert!(roof < map.depth, "roof is on the map");
        // Nothing above the roof...
        for z in (roof + 1)..map.depth {
            for y in 0..map.height { for x in 0..map.width {
                assert_eq!(map.get(x,y,z).shape, dk_world::TileShape::Empty,
                    "seed {seed}: something solid above the roof at z={z}");
            }}
        }
        // ...and something at it.
        let any = (0..map.height).any(|y| (0..map.width)
            .any(|x| map.get(x,y,roof).shape != dk_world::TileShape::Empty));
        assert!(any, "seed {seed}: the roof itself has terrain");
        println!("seed {seed}: roof z={roof} of depth {}", map.depth);
    }
}
#[test]
fn an_unknown_material_index_never_panics_the_renderer() {
    let mats = regs();
    // NO_MATERIAL is what every empty tile carries; the renderer must survive it.
    let m = mats.get(dk_world::NO_MATERIAL);
    assert_eq!(m.id, "unknown");
    assert!(!mats.is_valid(dk_world::NO_MATERIAL));
    assert!(mats.is_valid(0));
    let _ = mats.get(9999).color; // and any other stray index
}
