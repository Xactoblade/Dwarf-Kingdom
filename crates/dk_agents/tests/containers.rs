//! Container exit tests, run headlessly: a barrel or bin standing in a
//! stockpile swallows the fort's goods, so one tile holds a larder instead of
//! a single crop — and what's packed away is still there to eat, brew and
//! forge with.
//!
//! Storing goods is otherwise strictly one item per tile (`cell_free`), so
//! containers are what make a stockpile hold more than it has floor.

mod common;

use dk_agents::{container_capacity, ItemKind, ItemState, Sim};
use dk_world::path::Pos;

fn fort(seed: u64) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, 4);
    sim.invasions = false;
    (sim, raws)
}

/// A fort with a stockpile and a barrel already standing in it, as if a
/// carpenter had made one and a hauler had put it away.
fn fort_with_barrel(seed: u64) -> (Sim, dk_raws::Raws, usize) {
    let (mut sim, raws) = fort(seed);
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    sim.place_flat_stockpiles(cx, cy, 24);
    let cell = sim.stockpiles[0].cells().next().expect("a stockpile cell");
    sim.debug_spawn_item(ItemKind::Barrel, 0, cell);
    let barrel = sim.items.len() - 1;
    sim.items[barrel].state = ItemState::Stored { stockpile: 0 };
    (sim, raws, barrel)
}

#[test]
fn a_barrel_takes_food_and_a_bin_takes_goods_but_never_the_reverse() {
    // The whole rulebook: barrels are for food and drink, bins for worked
    // goods, and neither will hold the other's cargo.
    for food in [ItemKind::Meal, ItemKind::Crop, ItemKind::Berry, ItemKind::Drink] {
        assert!(container_capacity(ItemKind::Barrel, food) > 0, "a barrel holds {food:?}");
        assert_eq!(container_capacity(ItemKind::Bin, food), 0, "a bin is no place for {food:?}");
    }
    for good in [ItemKind::Bar, ItemKind::Cloth, ItemKind::Leather, ItemKind::Craft] {
        assert!(container_capacity(ItemKind::Bin, good) > 0, "a bin holds {good:?}");
        assert_eq!(container_capacity(ItemKind::Barrel, good), 0, "a barrel is no place for {good:?}");
    }
    // "Any number of units of brewed alcohol, but only a single stack": a
    // brew yields one stack, and that stack is what a cask holds.
    assert_eq!(
        container_capacity(ItemKind::Barrel, ItemKind::Drink),
        dk_agents::BATCH,
        "a cask holds exactly one brewing"
    );
    // And nothing holds the unbarrellable: stone, logs, the dead.
    for loose in [ItemKind::Boulder, ItemKind::Log, ItemKind::Corpse, ItemKind::Bed] {
        assert_eq!(container_capacity(ItemKind::Barrel, loose), 0);
        assert_eq!(container_capacity(ItemKind::Bin, loose), 0);
    }
    // Containers never nest.
    assert_eq!(container_capacity(ItemKind::Barrel, ItemKind::Bin), 0);
    assert_eq!(container_capacity(ItemKind::Bin, ItemKind::Barrel), 0);
}

#[test]
fn a_hauler_packs_loose_food_into_a_barrel() {
    let (mut sim, raws, barrel) = fort_with_barrel(6101);
    // A scatter of crops on the floor, away from the stockpile.
    let sp = sim.dwarves[0].pos;
    for _ in 0..5 {
        sim.debug_spawn_item(ItemKind::Crop, 0, sp);
    }
    assert!(sim.contents_of(barrel).is_empty(), "the barrel starts empty");

    let mut packed = 0;
    for _ in 0..8_000 {
        sim.step(&raws);
        packed = sim.contents_of(barrel).len();
        if packed >= 5 {
            break;
        }
    }
    assert_eq!(packed, 5, "every loose crop should end up in the barrel");
    // And they are IN it — sharing its tile, off the floor.
    let bpos = sim.items[barrel].pos;
    for c in sim.contents_of(barrel) {
        assert_eq!(sim.items[c].pos, bpos, "a packed crop rides on its barrel's tile");
        assert!(matches!(sim.items[c].state, ItemState::Inside { container } if container == barrel));
    }
}

#[test]
fn a_barrel_holds_more_than_its_tile_ever_could() {
    // The point of the whole mechanic. A stockpile stores strictly one item
    // per tile; a barrel on ONE tile holds a larder.
    let (mut sim, raws, barrel) = fort_with_barrel(6102);
    let cells: usize = sim.stockpiles.iter().map(|s| s.cells().count()).sum();
    let sp = sim.dwarves[0].pos;
    // More food than the stockpile has floor.
    for _ in 0..cells + 10 {
        sim.debug_spawn_item(ItemKind::Meal, 0, sp);
    }
    for _ in 0..25_000 {
        sim.step(&raws);
        if sim.contents_of(barrel).len() >= cells + 10 {
            break;
        }
    }
    assert!(
        sim.contents_of(barrel).len() > cells,
        "one barrel outholds the whole stockpile floor ({} meals in {cells} cells)",
        sim.contents_of(barrel).len()
    );
}

#[test]
fn a_dwarf_eats_what_is_packed_in_a_barrel() {
    // The nightmare this test exists to prevent: a fort starving beside a full
    // barrel because packed food fell out of every "find me food" query.
    let (mut sim, raws, barrel) = fort_with_barrel(6103);
    let bpos = sim.items[barrel].pos;
    for _ in 0..6 {
        sim.debug_spawn_item(ItemKind::Meal, 0, bpos);
        let m = sim.items.len() - 1;
        sim.items[m].state = ItemState::Inside { container: barrel };
    }
    assert_eq!(sim.contents_of(barrel).len(), 6, "a barrel of meals and nothing else");
    assert_eq!(sim.count_kind(ItemKind::Meal), 6);

    // Starve them: only the barrel's meals can save them.
    for d in &mut sim.dwarves {
        d.hunger = 95.0;
    }
    let mut ate = false;
    for _ in 0..10_000 {
        sim.step(&raws);
        if sim.contents_of(barrel).len() < 6 {
            ate = true;
            break;
        }
    }
    assert!(ate, "a hungry dwarf takes a meal out of the barrel");
    assert!(
        sim.dwarves.iter().any(|d| d.alive && d.hunger < 50.0),
        "and is fed by it"
    );
}

#[test]
fn a_brewer_reaches_a_crop_packed_in_a_barrel() {
    // Workshops pull their inputs through the same predicate as eating, so a
    // packed crop must still be brewable. The still is raised BEFORE the
    // stockpile claims the flat ground, so both land in the one region the
    // dwarves can walk (see survive_a_year.rs, which sets up the same way).
    let (mut sim, raws) = fort(6104);
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    let (still, _) = sim.find_flat_patch(cx, cy).expect("still site");
    assert!(sim.add_building(dk_agents::BuildingKind::Still, still));
    sim.place_flat_stockpiles(cx, cy, 24);
    let cell = sim.stockpiles[0].cells().next().expect("a stockpile cell");
    sim.debug_spawn_item(ItemKind::Barrel, 0, cell);
    let barrel = sim.items.len() - 1;
    sim.items[barrel].state = ItemState::Stored { stockpile: 0 };

    let bpos = sim.items[barrel].pos;
    for _ in 0..6 {
        sim.debug_spawn_item(ItemKind::Crop, 0, bpos);
        let c = sim.items.len() - 1;
        sim.items[c].state = ItemState::Inside { container: barrel };
    }
    // A second, empty cask for the drink to go home in — brewing needs one,
    // and the first is full of barley.
    sim.debug_spawn_item(ItemKind::Barrel, 0, sim.dwarves[0].pos);

    let mut brewed = false;
    for _ in 0..15_000 {
        sim.step(&raws);
        if sim.stats.drinks_brewed > 0 {
            brewed = true;
            break;
        }
    }
    assert!(brewed, "the brewer takes a crop out of the barrel and brews it");
}

#[test]
fn a_container_holds_one_kind_at_a_time() {
    let (mut sim, _raws, barrel) = fort_with_barrel(6105);
    let bpos = sim.items[barrel].pos;
    sim.debug_spawn_item(ItemKind::Meal, 0, bpos);
    let meal = sim.items.len() - 1;
    sim.items[meal].state = ItemState::Inside { container: barrel };

    // A barrel of meals is a barrel of meals — it won't also take crops.
    assert!(!sim.container_accepts_kind(barrel, ItemKind::Crop), "no mixing");
    assert!(sim.container_accepts_kind(barrel, ItemKind::Meal), "more meals are welcome");
}

#[test]
fn a_full_barrel_takes_no_more() {
    let (mut sim, _raws, barrel) = fort_with_barrel(6106);
    let bpos = sim.items[barrel].pos;
    let cap = container_capacity(ItemKind::Barrel, ItemKind::Meal);
    for _ in 0..cap - 1 {
        sim.debug_spawn_item(ItemKind::Meal, 0, bpos);
        let d = sim.items.len() - 1;
        sim.items[d].state = ItemState::Inside { container: barrel };
    }
    assert!(sim.container_accepts_kind(barrel, ItemKind::Meal), "one place left");
    sim.debug_spawn_item(ItemKind::Meal, 0, bpos);
    let last = sim.items.len() - 1;
    sim.items[last].state = ItemState::Inside { container: barrel };
    assert!(
        !sim.container_accepts_kind(barrel, ItemKind::Meal),
        "a barrel filled to its {cap} takes no more"
    );
}

#[test]
fn packed_goods_never_block_the_floor() {
    // Contents must not consume tile space — that is the entire storage win.
    let (mut sim, raws, barrel) = fort_with_barrel(6107);
    let bpos = sim.items[barrel].pos;
    for _ in 0..20 {
        sim.debug_spawn_item(ItemKind::Meal, 0, bpos);
        let m = sim.items.len() - 1;
        sim.items[m].state = ItemState::Inside { container: barrel };
    }
    // Twenty meals in one barrel, yet every other cell is still free: drop a
    // boulder in and it finds a home of its own.
    let sp = sim.dwarves[0].pos;
    sim.debug_spawn_item(ItemKind::Boulder, 0, sp);
    let rock = sim.items.len() - 1;
    let mut stored = false;
    for _ in 0..8_000 {
        sim.step(&raws);
        if matches!(sim.items[rock].state, ItemState::Stored { .. }) {
            stored = true;
            break;
        }
    }
    assert!(stored, "a barrel's contents leave the rest of the stockpile free");
}

#[test]
fn contents_ride_with_their_barrel_and_spill_if_it_is_destroyed() {
    let (mut sim, raws, barrel) = fort_with_barrel(6108);
    let bpos = sim.items[barrel].pos;
    sim.debug_spawn_item(ItemKind::Meal, 0, bpos);
    let meal = sim.items.len() - 1;
    sim.items[meal].state = ItemState::Inside { container: barrel };

    // Move the barrel: its contents follow it, wherever it went.
    let moved = Pos::new(bpos.x + 3, bpos.y, bpos.z);
    sim.items[barrel].pos = moved;
    sim.step(&raws);
    assert_eq!(sim.items[meal].pos, moved, "the meal rides with its barrel");

    // Destroy the barrel: the meal must not be left pointing at a ghost.
    sim.items[barrel].consumed = true;
    sim.step(&raws);
    assert_eq!(
        sim.items[meal].state,
        ItemState::OnGround,
        "a meal whose barrel is gone spills onto the floor, not into limbo"
    );
    assert!(sim.items[meal].active(), "and it is still real food");
}

#[test]
fn a_barrel_is_traded_with_its_contents_and_priced_for_them() {
    let (mut sim, raws, barrel) = fort_with_barrel(6109);
    let bpos = sim.items[barrel].pos;
    let empty = sim.stack_value(barrel, &raws);
    for _ in 0..10 {
        sim.debug_spawn_item(ItemKind::Meal, 0, bpos);
        let m = sim.items.len() - 1;
        sim.items[m].state = ItemState::Inside { container: barrel };
    }
    let full = sim.stack_value(barrel, &raws);
    assert!(
        full > empty,
        "a barrel of meals is worth more than the barrel ({full} vs {empty})"
    );

    // Selling the barrel sells the meals in it — no free goods left behind.
    let contents = sim.contents_of(barrel);
    sim.debug_consume_with_contents(barrel);
    assert!(!sim.items[barrel].active(), "the barrel left with the wagon");
    for c in contents {
        assert!(!sim.items[c].active(), "and so did what was inside it");
    }
}

#[test]
fn a_larder_already_on_the_floor_is_packed_once_a_barrel_arrives() {
    // A fort stores its food long before it ever works a barrel — and a save
    // from before containers existed is entirely stored-loose. That larder must
    // get packed away when a barrel finally stands in the pile, or containers
    // only ever help goods that happen to arrive later.
    let (mut sim, raws) = fort(6111);
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    sim.place_flat_stockpiles(cx, cy, 24);
    let sp = sim.dwarves[0].pos;
    for _ in 0..6 {
        sim.debug_spawn_item(ItemKind::Meal, 0, sp);
    }
    // Let the fort put them away the old way: one meal to a cell.
    for _ in 0..8_000 {
        sim.step(&raws);
        if sim.items.iter().filter(|i| i.kind == ItemKind::Meal).all(|i| {
            matches!(i.state, ItemState::Stored { .. })
        }) {
            break;
        }
    }
    let stored_loose = sim
        .items
        .iter()
        .filter(|i| i.kind == ItemKind::Meal && matches!(i.state, ItemState::Stored { .. }))
        .count();
    assert!(stored_loose >= 4, "the larder is on the floor, {stored_loose} meals");

    // Now a barrel arrives.
    let cell = sim.stockpiles[0]
        .cells()
        .find(|&c| !sim.items.iter().any(|i| i.active() && i.pos == c))
        .expect("a free cell for the barrel");
    sim.debug_spawn_item(ItemKind::Barrel, 0, cell);
    let barrel = sim.items.len() - 1;
    sim.items[barrel].state = ItemState::Stored { stockpile: 0 };

    let mut packed = 0;
    for _ in 0..20_000 {
        sim.step(&raws);
        packed = sim.contents_of(barrel).len();
        if packed >= stored_loose {
            break;
        }
    }
    assert!(
        packed >= stored_loose,
        "the standing larder is packed into the new barrel ({packed} of {stored_loose})"
    );
}

#[test]
fn a_barrel_someone_is_coming_for_cannot_be_sold() {
    // The trade guard spares a loaf a dwarf has claimed. A barrel must not be
    // a loophole around it.
    let (mut sim, raws, barrel) = fort_with_barrel(6112);
    let bpos = sim.items[barrel].pos;
    sim.debug_spawn_item(ItemKind::Meal, 0, bpos);
    let meal = sim.items.len() - 1;
    sim.items[meal].state = ItemState::Inside { container: barrel };

    // A dwarf claims the meal inside the barrel — they're on their way to eat.
    sim.items[meal].reserved_by = Some(0);

    sim.trade_partner = Some("the Amber Banners".to_string());
    let mut arrived = false;
    for _ in 0..dk_core::TICKS_PER_DAY * dk_core::DAYS_PER_SEASON * 2 {
        sim.step(&raws);
        if sim.caravan.is_some() {
            arrived = true;
            break;
        }
    }
    assert!(arrived, "a caravan is needed to try the sale");
    // Re-claim it: a season of stepping may have resolved the old claim.
    sim.items[meal].reserved_by = Some(0);

    let err = sim
        .execute_trade(&[barrel], &[0], &raws)
        .expect_err("selling a barrel whose meal is claimed must be refused");
    assert!(
        err.contains("already coming for"),
        "a claimed meal blocks the sale of its barrel, got: {err}"
    );
    assert!(sim.items[barrel].active(), "and the barrel stays");
    assert!(sim.items[meal].active(), "and so does the meal");
}

/// A still, a field of barley, and thirsty dwarves — everything but a cask.
fn brewing_fort(seed: u64, casks: usize) -> (Sim, dk_raws::Raws) {
    let (mut sim, raws) = fort(seed);
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    let (still, _) = sim.find_flat_patch(cx, cy).expect("still site");
    assert!(sim.add_building(dk_agents::BuildingKind::Still, still));
    sim.place_flat_stockpiles(cx, cy, 24);
    let sp = sim.dwarves[0].pos;
    for _ in 0..8 {
        sim.debug_spawn_item(ItemKind::Crop, 0, sp);
    }
    for _ in 0..casks {
        sim.debug_spawn_item(ItemKind::Barrel, 0, sp);
    }
    (sim, raws)
}

#[test]
fn no_empty_barrel_means_no_brewing() {
    // Dwarf Fortress's famous bind: "Brewers need a still, a brewable plant,
    // and one empty barrel per job." A fort with barley and no cask brews
    // nothing, however long it waits.
    let (mut sim, raws) = brewing_fort(6113, 0);
    for _ in 0..15_000 {
        sim.step(&raws);
    }
    assert_eq!(sim.stats.drinks_brewed, 0, "no cask, no drink");
    assert!(sim.count_kind(ItemKind::Crop) > 0, "and the barley sits unbrewed");
}

#[test]
fn a_brew_fills_a_cask_and_the_drink_is_never_loose() {
    let (mut sim, raws) = brewing_fort(6114, 1);
    let mut brewed = false;
    for _ in 0..15_000 {
        sim.step(&raws);
        if sim.stats.drinks_brewed > 0 {
            brewed = true;
            break;
        }
    }
    assert!(brewed, "one empty cask is enough to brew");
    // Every drop of it is in a barrel — drink never lies on the floor.
    for (i, it) in sim.items.iter().enumerate() {
        if it.active() && it.kind == ItemKind::Drink {
            assert!(
                matches!(it.state, ItemState::Inside { .. }),
                "drink {i} is loose on the ground, not in its cask"
            );
        }
    }
    // And that cask now holds a stack, so it is no longer empty for the next
    // brewing: one empty container per job.
    let barrel = sim
        .items
        .iter()
        .position(|i| i.active() && i.kind == ItemKind::Barrel)
        .expect("the cask");
    assert_eq!(sim.contents_of(barrel).len(), dk_agents::BATCH, "a stack to a cask");
    assert!(!sim.container_accepts_kind(barrel, ItemKind::Drink), "and it takes no more");
}

#[test]
fn an_emptied_cask_can_be_brewed_into_again() {
    // The loop that keeps a fort alive: dwarves drink a cask dry, which frees
    // it, and the brewer fills it again.
    let (mut sim, raws) = brewing_fort(6115, 1);
    for _ in 0..15_000 {
        sim.step(&raws);
        if sim.stats.drinks_brewed > 0 {
            break;
        }
    }
    let first = sim.stats.drinks_brewed;
    assert!(first > 0, "brewed once");

    // Drain the cask, as thirsty dwarves would.
    for i in 0..sim.items.len() {
        if sim.items[i].active() && sim.items[i].kind == ItemKind::Drink {
            sim.items[i].consumed = true;
        }
    }
    let mut again = false;
    for _ in 0..15_000 {
        sim.step(&raws);
        if sim.stats.drinks_brewed > first {
            again = true;
            break;
        }
    }
    assert!(again, "an emptied cask is an empty cask: the still runs again");
}

#[test]
fn an_embark_brings_its_booze_in_casks() {
    let (mut sim, raws) = fort(6116);
    sim.add_embark_supplies(&raws);
    assert!(sim.count_kind(ItemKind::Barrel) > 0, "the wagon carries casks");
    assert!(sim.count_kind(ItemKind::Drink) > 0, "with drink in them");
    for it in sim.items.iter().filter(|i| i.active() && i.kind == ItemKind::Drink) {
        assert!(
            matches!(it.state, ItemState::Inside { .. }),
            "a wagon carries no loose wine"
        );
    }
}

#[test]
fn a_carpenter_makes_the_containers_the_fort_needs() {
    let (mut sim, raws) = fort(6110);
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    sim.place_flat_stockpiles(cx, cy, 24);
    let (ca, _) = sim.find_flat_patch(cx, cy).expect("carpenter site");
    assert!(sim.add_building(dk_agents::BuildingKind::Carpenter, ca));
    let sp = sim.dwarves[0].pos;
    for _ in 0..8 {
        sim.debug_spawn_log(sp);
    }
    // Goods with nowhere to go: the fort should want a bin for them.
    for _ in 0..6 {
        sim.debug_spawn_item(ItemKind::Cloth, 0, sp);
    }
    let mut made_bin = false;
    for _ in 0..25_000 {
        sim.step(&raws);
        if sim.stats.bins_made > 0 {
            made_bin = true;
            break;
        }
    }
    assert!(made_bin, "a carpenter with logs and unpacked cloth works a bin");
}
