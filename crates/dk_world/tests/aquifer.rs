//! Aquifers: a shallow water-bearing layer that floods a dig. place_aquifer
//! reports which solid tiles are wet; the sim turns each into a spring when it
//! is opened.

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
    ])
    .unwrap()
}

#[test]
fn an_aquifer_is_wet_soil_or_sedimentary_below_the_surface() {
    let mats = regs();
    let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(7);
    let map = dk_world::generate(&mats, &mut rng, 96, 96, 32, 7);
    let aq = dk_world::place_aquifer(&map, &mats);
    assert!(!aq.is_empty(), "the map has a water-bearing band");
    for &p in &aq {
        let t = map.get(p.x as usize, p.y as usize, p.z as usize);
        assert!(t.is_solid(), "aquifer tiles are solid rock, not open air");
        assert!(
            matches!(mats.get(t.material).category, MaterialCategory::Soil | MaterialCategory::Sedimentary),
            "wet rock is soil or sedimentary, never igneous"
        );
        let top = map.surface_z(p.x as usize, p.y as usize).expect("a column has a top");
        assert!((p.z as usize) < top, "the aquifer lies below the surface");
    }
    // Deterministic for a given map.
    let mut rng2 = rand_chacha::ChaCha8Rng::seed_from_u64(7);
    let map2 = dk_world::generate(&mats, &mut rng2, 96, 96, 32, 7);
    assert_eq!(aq, dk_world::place_aquifer(&map2, &mats));
}
