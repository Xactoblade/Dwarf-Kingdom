//! Cavern layer: a great open cave dug deep in the rock, that a fort breaks
//! into by digging down.

use dk_raws::{MaterialCategory, MaterialDef, MaterialRegistry};
use dk_world::TileShape;
use rand_chacha::rand_core::SeedableRng;

fn regs() -> MaterialRegistry {
    let m = |id: &str, cat: MaterialCategory| MaterialDef {
        id: id.into(), name: id.into(), category: cat, color: [100, 100, 100], value: 1,
        combat: Default::default(), is_flux: false,
    };
    MaterialRegistry::from_defs(vec![
        m("loam", MaterialCategory::Soil),
        m("limestone", MaterialCategory::Sedimentary),
        m("granite", MaterialCategory::Igneous),
    ])
    .unwrap()
}

#[test]
fn a_cavern_is_open_cave_deep_in_the_rock() {
    let mats = regs();
    let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(9);
    let mut map = dk_world::generate(&mats, &mut rng, 96, 96, 32, 9);
    let floors = dk_world::carve_caverns(&mut map, 9);
    assert!(!floors.is_empty(), "the deep holds a cavern");
    for &p in &floors {
        let (x, y, z) = (p.x as usize, p.y as usize, p.z as usize);
        assert!(map.get(x, y, z).shape.is_walkable(), "the cavern floor is walkable");
        assert_eq!(
            map.get(x, y, z + 1).shape,
            TileShape::Empty,
            "there is open air to stand in above the cavern floor"
        );
        let top = map.surface_z(x, y).expect("a column has a top");
        assert!(z < top, "the cavern lies below the surface");
    }
    // A cavern is a broad space, not a few stray tiles.
    assert!(floors.len() > 200, "the cavern is broad ({} floor tiles)", floors.len());
    // Deterministic carve.
    let mut rng2 = rand_chacha::ChaCha8Rng::seed_from_u64(9);
    let mut map2 = dk_world::generate(&mats, &mut rng2, 96, 96, 32, 9);
    assert_eq!(floors, dk_world::carve_caverns(&mut map2, 9));
}
