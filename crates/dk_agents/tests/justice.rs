//! Justice, run headlessly: a fort can accuse a suspected vampire. Accuse the
//! true vampire and the killings end; accuse an innocent and they die for
//! nothing while the real horror walks free.

mod common;

use dk_agents::{Faction, Sim};

fn fort(seed: u64, n: usize) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, n);
    sim.invasions = false;
    (sim, raws)
}

#[test]
fn accusing_the_true_vampire_ends_the_terror() {
    let (mut sim, _raws) = fort(8801, 4);
    sim.curse_a_vampire();
    let vamp = sim.dwarves.iter().position(|d| d.vampire).expect("a vampire was cursed");
    assert!(sim.accuse(vamp), "the accused was the vampire");
    assert!(!sim.dwarves[vamp].alive, "the vampire is put to death");
    assert!(
        !sim.dwarves.iter().any(|d| d.alive && d.vampire),
        "no vampire remains among the fort"
    );
}

#[test]
fn accusing_an_innocent_spares_the_real_vampire() {
    let (mut sim, _raws) = fort(8802, 5);
    sim.curse_a_vampire();
    let vamp = sim.dwarves.iter().position(|d| d.vampire).unwrap();
    let innocent = (0..sim.dwarves.len())
        .find(|&i| i != vamp && sim.dwarves[i].alive && sim.dwarves[i].faction == Faction::Fort)
        .unwrap();
    assert!(!sim.accuse(innocent), "an innocent was accused");
    assert!(!sim.dwarves[innocent].alive, "the innocent hangs all the same");
    assert!(sim.dwarves[vamp].alive && sim.dwarves[vamp].vampire, "the true vampire lives on");
}

#[test]
fn the_biography_betrays_a_vampire_once_blood_is_spilled() {
    let (mut sim, raws) = fort(8803, 3);
    sim.curse_a_vampire();
    let vamp = sim.dwarves.iter().position(|d| d.vampire).unwrap();
    // Before any death, no tell.
    assert!(!sim.biography(vamp, &raws).contains("eat, drink, or sleep"));
    // Once the fort has lost blood, folk begin to whisper.
    sim.stats.drained = 1;
    assert!(
        sim.biography(vamp, &raws).contains("eat, drink, or sleep"),
        "the tell surfaces once a killing is known"
    );
}
