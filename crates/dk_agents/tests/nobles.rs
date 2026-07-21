//! Nobility exit tests, run headlessly: a baron arises with population,
//! issues mandates, rewards fulfillment, and punishes failure — feeding
//! the stress pipeline like everything else.

mod common;

use dk_agents::{BuildingKind, ItemKind, MandateKind, Sim, ThoughtKind, BARONY_AT, MANDATE_DAYS};
use dk_core::TICKS_PER_DAY;
use dk_world::path::Pos;

fn barony_fort(seed: u64) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, BARONY_AT + 1);
    sim.invasions = false;
    sim.add_embark_supplies(&raws);
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    let barley = raws.plants.index_of("barley").unwrap();
    if let Some((fa, fb)) = sim.find_flat_patch(cx, cy) {
        sim.add_farm(fa, fb, barley);
    }
    if let Some((wa, _)) = sim.find_flat_patch(cx, cy) {
        sim.add_building(BuildingKind::Still, wa);
        sim.add_building(BuildingKind::Kitchen, Pos::new(wa.x + 1, wa.y, wa.z));
    }
    sim.place_flat_stockpiles(cx, cy, 27);
    (sim, raws)
}

#[test]
fn a_baron_arises_and_makes_demands() {
    let (mut sim, raws) = barony_fort(901);
    assert!(sim.baron.is_none());

    // Within a few days of fort life, the barony is claimed and a demand made.
    for _ in 0..(3 * TICKS_PER_DAY) {
        sim.step(&raws);
    }
    let baron = sim.baron.expect("a fort of this size attracts a baron");
    assert!(sim.dwarves[baron].alive);
    assert!(
        sim.dwarves[baron]
            .thoughts
            .iter()
            .any(|(_, t)| *t == ThoughtKind::BecameBaron),
        "elevation is a life event"
    );
    assert!(sim.log.iter().any(|(_, m)| m.contains("elevated to baron")));
    assert!(sim.mandate.is_some(), "barons do not sit idle");
    assert!(sim.log.iter().any(|(_, m)| m.contains("demands that")));
}

#[test]
fn a_fulfilled_mandate_pleases_the_baron() {
    let (mut sim, raws) = barony_fort(902);
    for _ in 0..(3 * TICKS_PER_DAY) {
        sim.step(&raws);
    }
    let baron = sim.baron.expect("baron appointed");
    // Set a concretely fulfillable demand (this fort cooks but does not
    // mine, so pin a cooking mandate rather than trusting the random roll).
    sim.mandate = Some(dk_agents::Mandate {
        kind: MandateKind::CookMeals,
        amount: 3,
        deadline: sim.clock.tick + (MANDATE_DAYS + 60) * TICKS_PER_DAY,
        baseline: sim.stats.meals_cooked,
        target: 0,
        violated: false,
    });
    let horizon = (MANDATE_DAYS + 60) * TICKS_PER_DAY;
    for _ in 0..horizon {
        sim.step(&raws);
        if sim.stats.mandates_met > 0 {
            break;
        }
    }
    assert!(
        sim.stats.mandates_met > 0,
        "a working fort should satisfy at least one mandate (failed: {})",
        sim.stats.mandates_failed
    );
    assert!(
        sim.dwarves[baron]
            .thoughts
            .iter()
            .any(|(_, t)| *t == ThoughtKind::MandateMet),
        "satisfaction is felt"
    );
    assert!(sim.log.iter().any(|(_, m)| m.contains("fulfilled")));
}

#[test]
fn defying_an_export_ban_is_punished_but_honouring_it_pleases_the_baron() {
    let (mut sim, raws) = barony_fort(904);
    for _ in 0..(3 * TICKS_PER_DAY) {
        sim.step(&raws);
    }
    let baron = sim.baron.expect("baron appointed");
    let granite = raws.materials.index_of("granite").unwrap();

    // A short export ban on granite. (Set directly — the random roll requires a
    // trade partner, which this fort has none of; the enforcement is the point.)
    let ban = |sim: &mut Sim| {
        sim.mandate = Some(dk_agents::Mandate {
            kind: MandateKind::ExportBan,
            amount: 0,
            deadline: sim.clock.tick + 2 * TICKS_PER_DAY,
            baseline: 0,
            target: granite,
            violated: false,
        });
    };

    // 1) Honour it: no forbidden sale, so the ban lapses met.
    ban(&mut sim);
    let met_before = sim.stats.mandates_met;
    for _ in 0..(3 * TICKS_PER_DAY) {
        sim.step(&raws);
        if sim.mandate.is_none() {
            break;
        }
    }
    assert_eq!(sim.stats.mandates_met, met_before + 1, "an honoured ban pleases the baron");
    assert!(sim.log.iter().any(|(_, m)| m.contains("edict held")));

    // 2) Defy it: sell granite boulders to a caravan, and the baron answers it.
    ban(&mut sim);
    let pos = sim.dwarves[0].pos;
    // Two granite boulders to sell (enough value to clear the merchant's margin
    // on one cheap good).
    sim.debug_spawn_boulder(granite, pos);
    sim.debug_spawn_boulder(granite, pos);
    let boulders: Vec<usize> = sim
        .items
        .iter()
        .enumerate()
        .filter(|(_, it)| it.active() && it.kind == ItemKind::Boulder && it.stuff == granite)
        .map(|(i, _)| i)
        .collect();
    sim.caravan = Some(dk_agents::Caravan {
        civ_name: "the Testers".into(),
        goods: vec![sim.items[boulders[0]].clone()], // a cheap thing to buy
        leaves_at: u64::MAX,
        traders: Vec::new(),
    });
    // Sell the banned granite (offering both boulders beats the margin on the good).
    sim.execute_trade(&boulders, &[0], &raws).expect("the sale goes through");
    assert!(sim.mandate.unwrap().violated, "selling the banned material is noted");

    let failed_before = sim.stats.mandates_failed;
    for _ in 0..(3 * TICKS_PER_DAY) {
        sim.step(&raws);
        if sim.mandate.is_none() {
            break;
        }
    }
    assert_eq!(sim.stats.mandates_failed, failed_before + 1, "defiance is punished");
}

#[test]
fn a_failed_mandate_means_a_beating() {
    let (mut sim, raws) = barony_fort(903);
    for _ in 0..(3 * TICKS_PER_DAY) {
        sim.step(&raws);
    }
    let baron = sim.baron.expect("baron appointed");
    // Force an impossible demand: a mountain of boulders with no miners.
    sim.mandate = Some(dk_agents::Mandate {
        kind: MandateKind::MineBoulders,
        amount: 500,
        deadline: sim.clock.tick + 2 * TICKS_PER_DAY,
        baseline: sim.stats.boulders_mined,
        target: 0,
        violated: false,
    });
    for _ in 0..(3 * TICKS_PER_DAY) {
        sim.step(&raws);
        if sim.stats.mandates_failed > 0 {
            break;
        }
    }
    assert_eq!(sim.stats.mandates_failed, 1, "the impossible demand fails");
    let punished = sim
        .dwarves
        .iter()
        .enumerate()
        .find(|(_, d)| d.thoughts.iter().any(|(_, t)| *t == ThoughtKind::Punished));
    let (culprit, victim) = punished.expect("someone answers for it");
    assert_ne!(culprit, baron, "the baron never blames themselves");
    assert!(victim.stress > 0.0, "injustice is stressful");
    assert!(
        victim.body.iter().any(|p| p.hp < p.max_hp),
        "the beating leaves bruises"
    );
    assert!(victim.alive, "justice stops short of murder");
    assert!(sim.log.iter().any(|(_, m)| m.contains("is beaten")));
    // The fort watched, and hated it.
    let disturbed = sim
        .dwarves
        .iter()
        .filter(|d| d.thoughts.iter().any(|(_, t)| *t == ThoughtKind::SawPunishment))
        .count();
    assert!(disturbed >= 2, "punishment poisons the room");
}
