//! Stockpile filters (BLUEPRINT.md §2.3, "stockpiles with fine-grained
//! filters"): a pile can be told what it is for. A food pile takes no
//! boulders, a stone pile is no place for a meal, and the barrels that serve
//! the larder stand in the larder — never off among the beds.

mod common;

use dk_agents::{ItemKind, ItemState, Sim, StockCategory, StockFilter};
use dk_world::path::Pos;

fn fort(seed: u64) -> (Sim, dk_raws::Raws) {
    let raws = common::test_raws();
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, &raws, rng, 4);
    sim.invasions = false;
    (sim, raws)
}

/// Two piles side by side, each told what it is for. Returns their indices.
fn two_piles(sim: &mut Sim, a: StockCategory, b: StockCategory) -> (usize, usize) {
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    let (p, q) = sim.find_flat_patch(cx, cy).expect("pile ground");
    // Split the patch down the middle: one pile each.
    let mid = (p.x + q.x) / 2;
    sim.add_filtered_stockpile(p, Pos::new(mid, q.y, q.z), StockFilter::only(&[a]));
    sim.add_filtered_stockpile(Pos::new(mid + 1, p.y, p.z), q, StockFilter::only(&[b]));
    (0, 1)
}

#[test]
fn every_kind_is_listed_and_filed() {
    // ItemKind::ALL is hand-written, so prove it covers the enum: every kind
    // files into exactly one category, and no kind is missing from the list.
    assert_eq!(
        ItemKind::ALL.len(),
        26,
        "ItemKind::ALL must list every kind — add yours to it"
    );
    let mut seen = std::collections::BTreeSet::new();
    for k in ItemKind::ALL {
        assert!(seen.insert(format!("{k:?}")), "{k:?} listed twice in ItemKind::ALL");
        // Panics if a kind has no category.
        let _ = dk_agents::stock_category(k);
    }
    // Every category is reachable — a category nothing files into is dead.
    for c in StockCategory::ALL {
        assert!(
            ItemKind::ALL.iter().any(|&k| dk_agents::stock_category(k) == c),
            "nothing files into {c:?}"
        );
    }
}

#[test]
fn a_filter_takes_what_it_says_and_nothing_else() {
    let food = StockFilter::only(&[StockCategory::Food]);
    assert!(food.allows(StockCategory::Food));
    assert!(!food.allows(StockCategory::Stone));
    assert!(!food.takes_everything());

    let any = StockFilter::any();
    for c in StockCategory::ALL {
        assert!(any.allows(c), "an undirected pile takes {c:?}");
    }
    assert!(any.takes_everything());
}

#[test]
fn a_food_pile_refuses_stone_and_a_stone_pile_refuses_food() {
    let (mut sim, raws) = fort(7101);
    let (_food, _stone) = two_piles(&mut sim, StockCategory::Food, StockCategory::Stone);

    let sp = sim.dwarves[0].pos;
    sim.debug_spawn_item(ItemKind::Meal, 0, sp);
    let meal = sim.items.len() - 1;
    sim.debug_spawn_item(ItemKind::Boulder, 0, sp);
    let rock = sim.items.len() - 1;

    for _ in 0..12_000 {
        sim.step(&raws);
        let done = matches!(sim.items[meal].state, ItemState::Stored { .. })
            && matches!(sim.items[rock].state, ItemState::Stored { .. });
        if done {
            break;
        }
    }
    let meal_pile = sim.stockpile_at(sim.items[meal].pos).expect("the meal is in a pile");
    let rock_pile = sim.stockpile_at(sim.items[rock].pos).expect("the boulder is in a pile");
    assert!(
        sim.stockpiles[meal_pile].accepts.allows(StockCategory::Food),
        "the meal went to the food pile"
    );
    assert!(
        sim.stockpiles[rock_pile].accepts.allows(StockCategory::Stone),
        "the boulder went to the stone pile"
    );
    assert_ne!(meal_pile, rock_pile, "and they are different piles");
}

#[test]
fn a_good_no_pile_will_take_is_left_where_it_lies() {
    // Telling every pile what it is for is also how a player says "leave that
    // alone": a fort with only a stone pile never tidies its meals away.
    let (mut sim, raws) = fort(7102);
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    let (p, q) = sim.find_flat_patch(cx, cy).expect("pile ground");
    sim.add_filtered_stockpile(p, q, StockFilter::only(&[StockCategory::Stone]));

    let sp = sim.dwarves[0].pos;
    sim.debug_spawn_item(ItemKind::Meal, 0, sp);
    let meal = sim.items.len() - 1;
    for _ in 0..8_000 {
        sim.step(&raws);
    }
    assert_eq!(
        sim.items[meal].state,
        ItemState::OnGround,
        "no pile takes food, so the meal stays where it fell"
    );
}

#[test]
fn a_barrel_stands_in_the_larder_it_serves_not_among_the_beds() {
    // A container belongs where its cargo belongs. A food pile is a barrel's
    // home; a stone pile is not.
    let food = dk_agents::Stockpile {
        rect: dk_agents::Rect { z: 0, x0: 0, y0: 0, x1: 2, y1: 2 },
        accepts: StockFilter::only(&[StockCategory::Food]),
    };
    let stone = dk_agents::Stockpile {
        rect: dk_agents::Rect { z: 0, x0: 0, y0: 0, x1: 2, y1: 2 },
        accepts: StockFilter::only(&[StockCategory::Stone]),
    };
    let goods = dk_agents::Stockpile {
        rect: dk_agents::Rect { z: 0, x0: 0, y0: 0, x1: 2, y1: 2 },
        accepts: StockFilter::only(&[StockCategory::Goods]),
    };
    assert!(food.takes(ItemKind::Barrel), "a barrel serves the larder");
    assert!(!stone.takes(ItemKind::Barrel), "a barrel has no business in the stoneyard");
    assert!(goods.takes(ItemKind::Bin), "a bin serves the goods pile");
    assert!(!food.takes(ItemKind::Bin), "a bin is no place for a meal, nor a larder for a bin");
    // A furniture pile refuses casks — see
    // `a_furniture_pile_is_no_home_for_a_working_cask` for why.
}

#[test]
fn food_is_packed_into_a_barrel_standing_in_the_food_pile() {
    let (mut sim, raws) = fort(7103);
    let (food, _stone) = two_piles(&mut sim, StockCategory::Food, StockCategory::Stone);
    // A cask in the larder.
    let cell = sim.stockpiles[food].cells().next().expect("a larder cell");
    sim.debug_spawn_item(ItemKind::Barrel, 0, cell);
    let barrel = sim.items.len() - 1;
    sim.items[barrel].state = ItemState::Stored { stockpile: food };

    let sp = sim.dwarves[0].pos;
    for _ in 0..4 {
        sim.debug_spawn_item(ItemKind::Meal, 0, sp);
    }
    let mut packed = 0;
    for _ in 0..12_000 {
        sim.step(&raws);
        packed = sim.contents_of(barrel).len();
        if packed >= 4 {
            break;
        }
    }
    assert_eq!(packed, 4, "the larder's cask takes the larder's meals");
}

#[test]
fn painting_a_pile_over_another_corrects_it() {
    // There is no erase tool, so painting over a mis-categorised pile is the
    // player's only correction — the last word must win.
    let (mut sim, _raws) = fort(7105);
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    let (p, q) = sim.find_flat_patch(cx, cy).expect("pile ground");
    sim.add_filtered_stockpile(p, q, StockFilter::only(&[StockCategory::Stone]));
    // "No — this is the larder."
    sim.add_filtered_stockpile(p, q, StockFilter::only(&[StockCategory::Food]));

    let s = sim.stockpile_at(p).expect("a pile covers this tile");
    assert!(
        sim.stockpiles[s].accepts.allows(StockCategory::Food),
        "the pile painted last is the pile that counts"
    );
    assert!(!sim.stockpiles[s].accepts.allows(StockCategory::Stone));
}

#[test]
fn a_furniture_pile_is_no_home_for_a_working_cask() {
    // A cask hauled to the furniture pile is a cask nothing will ever fill:
    // food is only packed into casks whose pile wants food. Casks belong with
    // their cargo, so a furniture pile must refuse them outright.
    let furniture = dk_agents::Stockpile {
        rect: dk_agents::Rect { z: 0, x0: 0, y0: 0, x1: 2, y1: 2 },
        accepts: StockFilter::only(&[StockCategory::Furniture]),
    };
    assert!(!furniture.takes(ItemKind::Barrel), "no stranding the fort's casks");
    assert!(!furniture.takes(ItemKind::Bin));
    assert!(furniture.takes(ItemKind::Bed), "beds, though, are furniture");
    assert!(furniture.takes(ItemKind::Statue));
}

#[test]
fn an_undirected_pile_still_takes_everything() {
    // The old behavior must survive: a plain stockpile is unchanged.
    let (mut sim, raws) = fort(7104);
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    sim.place_flat_stockpiles(cx, cy, 24);
    assert!(
        sim.stockpiles.iter().all(|s| s.accepts.takes_everything()),
        "piles are undirected unless told otherwise"
    );
    let sp = sim.dwarves[0].pos;
    sim.debug_spawn_item(ItemKind::Meal, 0, sp);
    let meal = sim.items.len() - 1;
    sim.debug_spawn_item(ItemKind::Boulder, 0, sp);
    let rock = sim.items.len() - 1;
    for _ in 0..12_000 {
        sim.step(&raws);
        if matches!(sim.items[meal].state, ItemState::Stored { .. })
            && matches!(sim.items[rock].state, ItemState::Stored { .. })
        {
            break;
        }
    }
    assert!(matches!(sim.items[meal].state, ItemState::Stored { .. }), "meal put away");
    assert!(matches!(sim.items[rock].state, ItemState::Stored { .. }), "boulder put away");
}
