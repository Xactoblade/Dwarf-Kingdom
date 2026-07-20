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

use dk_agents::{item_value, validate_economy, Item, ItemKind, ItemState};
use dk_raws::{EconomyConfig, Raws};
use dk_world::path::Pos;

fn data_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data")
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
