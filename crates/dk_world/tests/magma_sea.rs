//! The magma sea: a lake of molten rock in the deepest reaches.

use dk_raws::{MaterialCategory, MaterialDef, MaterialRegistry};
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
    ]).unwrap()
}

#[test]
fn a_magma_sea_burns_in_the_deep() {
    let mats = regs();
    let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(5);
    let mut map = dk_world::generate(&mats, &mut rng, 96, 96, 32, 5);
    dk_world::carve_magma_sea(&mut map, 5);
    // Count magma tiles; they must be deep, below any cavern.
    let mut magma = 0;
    for y in 0..map.height {
        for x in 0..map.width {
            for z in 0..map.depth {
                if map.get(x, y, z).magma > 0 {
                    magma += 1;
                    assert!(z < map.depth / 4, "magma lies in the deepest reaches (z={z})");
                }
            }
        }
    }
    assert!(magma > 200, "the magma sea is broad ({magma} tiles)");
    // Deterministic.
    let mut rng2 = rand_chacha::ChaCha8Rng::seed_from_u64(5);
    let mut map2 = dk_world::generate(&mats, &mut rng2, 96, 96, 32, 5);
    dk_world::carve_magma_sea(&mut map2, 5);
    let count2: usize = (0..map2.width).flat_map(|x| (0..map2.height).flat_map(move |y| (0..map2.depth).map(move |z| (x,y,z))))
        .filter(|&(x,y,z)| map2.get(x,y,z).magma > 0).count();
    assert_eq!(magma, count2, "the sea is carved the same each time");
}
