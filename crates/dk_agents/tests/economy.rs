//! Slice 1 of the economy layer: prices moved out of a hardcoded `item_value`
//! match and into `data/economy/prices.ron`, with no change in what anything is
//! worth. These tests lock that "no change" promise from two sides:
//!
//!  * the GOLDEN test pins `item_value`'s output to the exact numbers the old
//!    hardcoded match produced (for value-1 materials, so it depends only on the
//!    price formula, never on shipped material data), and
//!  * the PARITY test pins the shipped `prices.ron` to `EconomyConfig::default`
//!    — the canonical table in code that the golden numbers assume.
//!
//! Together they guarantee the real game values every good exactly as before,
//! for any material, while the numbers now live in a data file you can tune.

mod common;

use dk_agents::{
    item_value, raider_wave_size, validate_economy, Caravan, Item, ItemKind, ItemState, Sim,
};
use dk_raws::{EconomyConfig, Raws};
use dk_world::path::Pos;

/// A caravan parked at the fort carrying exactly `goods`, so trade math is
/// deterministic. No traders — `execute_trade` then drops bought goods at the
/// first citizen's feet.
fn test_caravan(goods: Vec<Item>) -> Caravan {
    Caravan {
        civ_name: "the Testing Company".into(),
        goods,
        leaves_at: u64::MAX,
        traders: Vec::new(),
    }
}

fn data_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data")
}

/// A small fort with a known-empty inventory, so wealth math starts from zero.
/// `invasions` off keeps it out of the stochastic siege/migration paths.
fn empty_fort(raws: &Raws, seed: u64) -> Sim {
    let mut rng = dk_core::rng_from_seed(seed);
    let map = dk_world::generate(&raws.materials, &mut rng, 32, 32, 16, seed);
    let mut sim = Sim::new(map, raws, rng, 4);
    sim.invasions = false;
    sim.items.clear();
    sim
}

/// An item of `kind` made of material/gem index `stuff` at the given quality.
fn item(kind: ItemKind, stuff: u16, quality: u8) -> Item {
    Item {
        kind,
        stuff,
        name: None,
        pos: Pos::new(0, 0, 0),
        state: ItemState::OnGround,
        reserved_by: None,
        consumed: false,
        quality,
        made_at: 0,
        variant: 0,
    }
}

/// The exact worth `item_value` produced before prices were data-driven, for a
/// material whose `value` is 1 (as every material in `test_raws` is). If any of
/// these move, the refactor changed a price — which Slice 1 promised it would
/// not.
#[test]
fn item_value_matches_the_old_hardcoded_prices() {
    let raws = common::test_raws(); // all materials have value 1
    // (kind, stuff, quality) -> expected worth
    let cases: &[(ItemKind, u16, u8, u32)] = &[
        // material-derived: value(1) * mat_coeff + flat
        (ItemKind::Boulder, 0, 0, 3),
        (ItemKind::Craft, 0, 0, 16),
        (ItemKind::Weapon, 3, 0, 30),
        (ItemKind::Bar, 3, 0, 18),
        (ItemKind::Armor, 3, 0, 40),
        (ItemKind::Shield, 3, 0, 26),
        (ItemKind::Bed, 4, 0, 26),
        (ItemKind::Log, 4, 0, 8),
        (ItemKind::Statue, 0, 0, 55),
        (ItemKind::Artifact, 0, 0, 55),
        // flat-priced: material ignored
        (ItemKind::Seed, 0, 0, 3),
        (ItemKind::Crop, 0, 0, 5),
        (ItemKind::Meal, 0, 0, 8),
        (ItemKind::Drink, 0, 0, 8),
        (ItemKind::Corpse, 0, 0, 0),
        (ItemKind::BodyPart, 0, 0, 0),
        (ItemKind::BoneCraft, 0, 0, 10),
        (ItemKind::Wool, 0, 0, 4),
        (ItemKind::Cloth, 0, 0, 18),
        (ItemKind::Glass, 0, 0, 85),
        (ItemKind::Clothes, 0, 0, 40),
        (ItemKind::Barrel, 0, 0, 45),
        (ItemKind::Bin, 0, 0, 30),
        (ItemKind::Instrument, 0, 0, 55),
        (ItemKind::Hide, 0, 0, 6),
        (ItemKind::Leather, 0, 0, 30),
        (ItemKind::Berry, 0, 0, 4),
        // gems: priced by rarity tier in code, not the table
        (ItemKind::RoughGem, 16, 0, 4),   // agate, tier 1: 4 * 1
        (ItemKind::CutGem, 16, 0, 24),    // agate, tier 1: 24 * 1
        (ItemKind::CutGem, 6, 0, 144),    // diamond, tier 6: 24 * 6
        // quality multiplier: base + base * quality / 2 (masterwork craft)
        (ItemKind::Craft, 0, 5, 56),      // 16 + 16*5/2
        (ItemKind::Statue, 0, 3, 137),    // 55 + 55*3/2
    ];
    for &(kind, stuff, quality, expected) in cases {
        let got = item_value(&item(kind, stuff, quality), &raws);
        assert_eq!(
            got, expected,
            "{kind:?} (stuff {stuff}, quality {quality}) should be worth {expected}, got {got}"
        );
    }
}

/// The shipped price file must equal the canonical table in code. If they drift,
/// the real game and every test/tool that uses `EconomyConfig::default` disagree
/// on what things cost. An intentional balance change means updating both.
#[test]
fn shipped_prices_match_the_canonical_default() {
    let raws = Raws::load(&data_dir()).expect("load real raws");
    assert_eq!(
        raws.economy,
        EconomyConfig::default(),
        "data/economy/prices.ron has drifted from EconomyConfig::default()"
    );
}

/// Every priced item kind has a row, so nothing is ever silently worth nothing.
#[test]
fn validate_economy_accepts_the_shipped_and_default_price_lists() {
    let raws = Raws::load(&data_dir()).expect("load real raws");
    validate_economy(&raws).expect("shipped economy should be complete");
    validate_economy(&common::test_raws()).expect("default economy should be complete");
}

/// A missing row is caught loudly, not passed over as a free good.
#[test]
fn validate_economy_rejects_a_missing_row() {
    let mut raws = common::test_raws();
    raws.economy.kind_prices.retain(|p| p.kind != "Statue");
    let err = validate_economy(&raws).expect_err("a missing Statue row should fail validation");
    assert!(
        err.to_string().contains("Statue"),
        "the error should name the missing kind, got: {err}"
    );
}

/// The margin is data now: change it and trade terms change. (Guards the wiring,
/// not the value.)
#[test]
fn trade_margin_comes_from_data() {
    let raws = Raws::load(&data_dir()).expect("load real raws");
    assert_eq!(raws.economy.trade_margin, 1.2);
}

// ------------------------------------------------------------- Slice 2: wealth

/// Fortress wealth sums each active item's value exactly once — no double-count,
/// and consumed items are invisible.
#[test]
fn fortress_wealth_sums_every_active_item_once() {
    let raws = common::test_raws(); // every material is value 1
    let mut sim = empty_fort(&raws, 7);
    assert_eq!(sim.fortress_wealth(&raws), 0, "an empty fort is worth nothing");

    // 3 granite boulders (1*3 = 3 each) + a masterwork statue (1*15+40 = 55,
    // then quality 5: 55 + 55*5/2 = 192).
    let granite = raws.materials.index_of("granite").unwrap();
    sim.items.push(item(ItemKind::Boulder, granite, 0));
    sim.items.push(item(ItemKind::Boulder, granite, 0));
    sim.items.push(item(ItemKind::Boulder, granite, 0));
    sim.items.push(item(ItemKind::Statue, granite, 5));
    assert_eq!(sim.fortress_wealth(&raws), 3 + 3 + 3 + 192);

    // A consumed item is worth nothing to the fort.
    let mut ghost = item(ItemKind::Statue, granite, 5);
    ghost.consumed = true;
    sim.items.push(ghost);
    assert_eq!(sim.fortress_wealth(&raws), 3 + 3 + 3 + 192, "consumed goods don't count");
}

/// The barrel invariant from the scope doc: a fort's wealth is the same whether
/// its wine sits loose on the floor or packed inside a barrel. (Guards against
/// the double-count / O(n²) trap of summing `stack_value` over all items.)
#[test]
fn wealth_is_the_same_whether_goods_are_loose_or_packed() {
    let raws = common::test_raws();

    let mut loose = empty_fort(&raws, 11);
    loose.items.push(item(ItemKind::Barrel, 0, 0));
    for _ in 0..3 {
        loose.items.push(item(ItemKind::Drink, 0, 0)); // OnGround
    }

    let mut packed = empty_fort(&raws, 11);
    packed.items.push(item(ItemKind::Barrel, 0, 0)); // index 0
    for _ in 0..3 {
        let mut d = item(ItemKind::Drink, 0, 0);
        d.state = ItemState::Inside { container: 0 };
        packed.items.push(d);
    }

    assert_eq!(loose.fortress_wealth(&raws), packed.fortress_wealth(&raws));
    // sanity: barrel 45 + 3 drinks * 8 = 69
    assert_eq!(loose.fortress_wealth(&raws), 45 + 3 * 8);
}

/// The siege wave scales with wealth and is capped, so a rich fort is besieged
/// harder but a masterwork hoard can't summon an endless horde.
#[test]
fn raider_wave_size_scales_with_wealth_and_caps() {
    assert_eq!(raider_wave_size(0), 1, "a penniless fort still draws a token raid");
    assert_eq!(raider_wave_size(150), 1, "50 common rocks go unnoticed");
    assert_eq!(raider_wave_size(1500), 2);
    assert_eq!(raider_wave_size(1715), 2, "one masterwork gold statue nudges it up");
    assert_eq!(raider_wave_size(4500), 4);
    assert_eq!(raider_wave_size(6000), 5, "a rich fort draws the full wave");
    assert_eq!(raider_wave_size(1_000_000), 5, "and no more than the full wave");
}

/// Slice 2's exit test: a fort holding one masterwork gold statue is worth more
/// — and draws a bigger siege — than a fort holding fifty rocks. Wealth, not
/// clutter, is what puts a target on the fort.
#[test]
fn a_gold_statue_outdraws_a_pile_of_rocks() {
    let raws = Raws::load(&data_dir()).expect("load real raws"); // gold is value 30 here
    let granite = raws.materials.index_of("granite").unwrap();
    let gold = raws.materials.index_of("native_gold").unwrap();

    let mut rock_fort = empty_fort(&raws, 3);
    for _ in 0..50 {
        rock_fort.debug_spawn_boulder(granite, Pos::new(1, 1, 1));
    }

    let mut gold_fort = empty_fort(&raws, 3);
    gold_fort.items.push(item(ItemKind::Statue, gold, 5)); // masterwork gold statue

    let rock_wealth = rock_fort.fortress_wealth(&raws);
    let gold_wealth = gold_fort.fortress_wealth(&raws);
    assert_eq!(rock_wealth, 150, "50 granite boulders at value 1: 50 * 3");
    assert_eq!(gold_wealth, 1715, "gold statue: 30*15+40 = 490, masterwork x3.5");

    assert!(gold_wealth > rock_wealth, "the gold statue is the richer fort");
    assert!(
        raider_wave_size(gold_wealth) > raider_wave_size(rock_wealth),
        "and it draws a larger siege ({} vs {} raiders)",
        raider_wave_size(gold_wealth),
        raider_wave_size(rock_wealth),
    );
}

/// The cached figure the HUD and siege read matches a fresh recompute.
#[test]
fn recompute_wealth_updates_the_cache() {
    let raws = common::test_raws();
    let mut sim = empty_fort(&raws, 5);
    assert_eq!(sim.wealth(), 0);
    let granite = raws.materials.index_of("granite").unwrap();
    sim.items.push(item(ItemKind::Statue, granite, 0)); // 55
    assert_eq!(sim.wealth(), 0, "the cache is stale until recomputed");
    sim.recompute_wealth(&raws);
    assert_eq!(sim.wealth(), 55, "recompute picks up the new statue");
}

// ---------------------------------------------------------- Slice 3: trade credit

/// The required price: the good's value plus the merchant's margin, rounded up.
fn required_price(value: u32, raws: &Raws) -> i64 {
    (value as f32 * raws.economy.trade_margin).ceil() as i64
}

/// Over-pay a caravan and the surplus is banked as goodwill rather than thrown
/// away; the ledger records both sides of the deal.
#[test]
fn overpaying_a_caravan_banks_the_surplus_as_credit() {
    let raws = common::test_raws();
    let mut sim = empty_fort(&raws, 21);
    let granite = raws.materials.index_of("granite").unwrap();
    // Offer three statues (55 each = 165) for one craft (16). Wild over-payment.
    sim.items.push(item(ItemKind::Statue, granite, 0)); // idx 0
    sim.items.push(item(ItemKind::Statue, granite, 0)); // idx 1
    sim.items.push(item(ItemKind::Statue, granite, 0)); // idx 2
    sim.caravan = Some(test_caravan(vec![item(ItemKind::Craft, granite, 0)]));

    let required = required_price(16, &raws); // ceil(16 * 1.2) = 20
    assert_eq!(sim.trade_credit, 0);
    sim.execute_trade(&[0, 1, 2], &[0], &raws).expect("over-payment is accepted");

    assert_eq!(sim.trade_credit, 165 - required, "the surplus over the ask is banked");
    assert_eq!(sim.stats.value_exported, 165, "all offered value is exported");
    assert_eq!(sim.stats.value_imported, 16, "the craft's value is imported");
    assert_eq!(sim.stats.trades_completed, 1);
}

/// Banked goodwill can buy goods outright, with nothing offered in return — the
/// caravan tab from last season pays this season's bill.
#[test]
fn banked_credit_buys_goods_with_nothing_offered() {
    let raws = common::test_raws();
    let mut sim = empty_fort(&raws, 22);
    let granite = raws.materials.index_of("granite").unwrap();
    sim.trade_credit = 100;
    sim.caravan = Some(test_caravan(vec![item(ItemKind::Craft, granite, 0)])); // value 16

    let required = required_price(16, &raws); // 20
    sim.execute_trade(&[], &[0], &raws).expect("credit alone covers the price");

    assert_eq!(sim.trade_credit, 100 - required, "credit is drawn down by the price");
    assert_eq!(sim.stats.value_exported, 0, "no goods left the fort");
    assert_eq!(sim.stats.value_imported, 16);
    assert!(
        sim.items.iter().any(|it| it.active() && it.kind == ItemKind::Craft),
        "the bought craft lands in the fort"
    );
}

/// A trade the fort can't cover — even counting credit — is refused, and a
/// refusal never touches the balance.
#[test]
fn an_unaffordable_trade_is_refused_and_leaves_credit_untouched() {
    let raws = common::test_raws();
    let mut sim = empty_fort(&raws, 23);
    let granite = raws.materials.index_of("granite").unwrap();
    sim.trade_credit = 5;
    sim.caravan = Some(test_caravan(vec![item(ItemKind::Statue, granite, 0)])); // value 55

    let err = sim.execute_trade(&[], &[0], &raws).unwrap_err();
    assert!(!err.is_empty());
    assert_eq!(sim.trade_credit, 5, "a scoffed-at offer doesn't spend the balance");
    assert_eq!(sim.stats.trades_completed, 0);
    assert_eq!(sim.stats.value_imported, 0);
}

/// Goodwill on account is wealth too, so a fort can't launder its masterworks
/// into credit to duck a wealth-scaled siege.
#[test]
fn banked_credit_counts_toward_fortress_wealth() {
    let raws = common::test_raws();
    let mut sim = empty_fort(&raws, 24);
    assert_eq!(sim.fortress_wealth(&raws), 0);
    sim.trade_credit = 250;
    assert_eq!(sim.fortress_wealth(&raws), 250, "credit on account counts as wealth");
    // and on top of real goods
    let granite = raws.materials.index_of("granite").unwrap();
    sim.items.push(item(ItemKind::Statue, granite, 0)); // 55
    assert_eq!(sim.fortress_wealth(&raws), 250 + 55);
}
