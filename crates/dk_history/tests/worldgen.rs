//! Worldgen fidelity (BLUEPRINT.md §2.1): the world is built the way Dwarf
//! Fortress builds one — six seeded fields, elevation deciding first, and then
//! a drainage-by-rainfall table.
//!
//! The thresholds here are DF's, read off the wiki's biome distribution chart.
//! Provenance caveat worth keeping: that chart is presented on a current-version
//! page but sourced from a 40d-era analysis, so it is a strong default rather
//! than a measured fact about the modern game.

use dk_history::{Alignment, Biome, Overworld, Savagery, World};

/// The table's own words: elevation first, then drainage x rainfall.
#[test]
fn elevation_decides_before_anything_else() {
    // "any terrain with an elevation of 0-99 is ocean"
    assert_eq!(Overworld::classify(0, 50, 50, 20), Biome::Ocean);
    assert_eq!(Overworld::classify(99, 50, 50, 20), Biome::Ocean);
    assert_ne!(Overworld::classify(100, 50, 50, 20), Biome::Ocean, "sea level is 100");
    // "any terrain with an elevation of 300-400 is mountain"
    assert_eq!(Overworld::classify(300, 50, 50, 20), Biome::Mountains);
    assert_eq!(Overworld::classify(400, 0, 0, 20), Biome::Mountains);
    assert_ne!(Overworld::classify(299, 50, 50, 20), Biome::Mountains);
}

#[test]
fn the_deserts_are_told_apart_by_drainage() {
    // Rainfall 0-9, split by drainage: sand / rocky wasteland / badlands.
    assert_eq!(Overworld::classify(150, 0, 0, 20), Biome::SandDesert);
    assert_eq!(Overworld::classify(150, 9, 32, 20), Biome::SandDesert);
    assert_eq!(Overworld::classify(150, 5, 33, 20), Biome::RockyWasteland);
    assert_eq!(Overworld::classify(150, 5, 65, 20), Biome::RockyWasteland);
    assert_eq!(Overworld::classify(150, 5, 66, 20), Biome::Badlands);
    assert_eq!(Overworld::classify(150, 5, 100, 20), Biome::Badlands);
}

#[test]
fn rainfall_walks_the_land_from_grass_to_forest() {
    // The chart's columns, at a drainage that keeps us out of the wetlands.
    assert_eq!(Overworld::classify(150, 10, 60, 20), Biome::Grassland);
    assert_eq!(Overworld::classify(150, 19, 60, 20), Biome::Grassland);
    assert_eq!(Overworld::classify(150, 20, 60, 20), Biome::Savanna);
    assert_eq!(Overworld::classify(150, 32, 60, 20), Biome::Savanna);
    assert_eq!(Overworld::classify(150, 33, 60, 20), Biome::Shrubland);
    assert_eq!(Overworld::classify(150, 65, 60, 20), Biome::Shrubland);
    assert_eq!(Overworld::classify(150, 66, 60, 20), Biome::ConiferForest);
    assert_eq!(Overworld::classify(150, 74, 60, 20), Biome::ConiferForest);
    assert_eq!(Overworld::classify(150, 75, 60, 20), Biome::BroadleafForest);
    assert_eq!(Overworld::classify(150, 100, 60, 20), Biome::BroadleafForest);
}

#[test]
fn water_that_cannot_drain_away_makes_wetland() {
    // Drainage 0-32 with real rain: marsh, then swamp as the rain rises.
    assert_eq!(Overworld::classify(150, 33, 0, 20), Biome::Marsh);
    assert_eq!(Overworld::classify(150, 65, 32, 20), Biome::Marsh);
    assert_eq!(Overworld::classify(150, 66, 32, 20), Biome::Swamp);
    assert_eq!(Overworld::classify(150, 100, 0, 20), Biome::Swamp);
    // ...and the same rain on draining ground is forest instead.
    assert_eq!(Overworld::classify(150, 100, 33, 20), Biome::BroadleafForest);
}

#[test]
fn cold_freezes_whatever_the_land_was() {
    // "at or below -5, all base biomes with drainage <75 become Tundra, and
    // biomes with drainage 75+ become Glaciers"
    assert_eq!(Overworld::classify(150, 100, 0, -5), Biome::Tundra);
    assert_eq!(Overworld::classify(150, 0, 74, -20), Biome::Tundra);
    assert_eq!(Overworld::classify(150, 50, 75, -5), Biome::Glacier);
    assert_eq!(Overworld::classify(150, 50, 100, -40), Biome::Glacier);
    // -4 is not cold enough.
    assert_ne!(Overworld::classify(150, 100, 0, -4), Biome::Tundra);
    // "Between -4 and 9 inclusive, Conifer Forests become Taiga."
    assert_eq!(Overworld::classify(150, 70, 60, 9), Biome::Taiga);
    assert_eq!(Overworld::classify(150, 70, 60, -4), Biome::Taiga);
    assert_eq!(Overworld::classify(150, 70, 60, 10), Biome::ConiferForest);
    // And a mountain stays a mountain however cold it gets.
    assert_eq!(Overworld::classify(350, 50, 50, -40), Biome::Mountains);
}

#[test]
fn hills_are_what_drainage_does_to_open_country() {
    // DF's chart splits grassland/savanna/shrubland into flat and hilly at
    // drainage 50 — hills are not a biome of their own.
    let w = World::generate(4242, 48, 48, 5);
    let mut flat = 0;
    let mut hilly = 0;
    for r in &w.overworld.regions {
        if !r.biome.is_grassy() {
            assert!(!r.hilly(), "only open country rumples into hills");
            continue;
        }
        if r.hilly() {
            hilly += 1;
            assert!(r.drainage >= 50);
        } else {
            flat += 1;
            assert!(r.drainage < 50);
        }
    }
    assert!(flat > 0 && hilly > 0, "a world has both flat and hilly country ({flat}/{hilly})");
}

#[test]
fn a_world_has_two_poles_and_a_warm_middle() {
    // It used to have one: cold in the north, hot in the south, no equator.
    let w = World::generate(77, 48, 48, 5);
    let row_temp = |y: usize| -> i32 {
        let sum: i32 = (0..w.overworld.width)
            .map(|x| w.overworld.get(x, y).temperature as i32)
            .sum();
        sum / w.overworld.width as i32
    };
    let north = row_temp(1);
    let middle = row_temp(w.overworld.height / 2);
    let south = row_temp(w.overworld.height - 2);
    assert!(middle > north + 20, "the middle is warmer than the north ({middle} vs {north})");
    assert!(middle > south + 20, "and warmer than the south ({middle} vs {south})");
    assert!((north - south).abs() < 25, "both poles are cold ({north} vs {south})");
}

#[test]
fn the_six_fields_are_all_present_and_varied() {
    // Elevation, rainfall, drainage, temperature, volcanism, savagery — DF's
    // seeded six. A field that never varies is a field that does nothing.
    let w = World::generate(99, 48, 48, 5);
    let spread = |f: fn(&dk_history::Region) -> i32| -> i32 {
        let vals: Vec<i32> = w.overworld.regions.iter().map(f).collect();
        vals.iter().max().unwrap() - vals.iter().min().unwrap()
    };
    assert!(spread(|r| r.elevation as i32) > 200, "elevation ranges over the world");
    assert!(spread(|r| r.rainfall as i32) > 50, "rainfall varies");
    assert!(spread(|r| r.drainage as i32) > 50, "drainage varies");
    assert!(spread(|r| r.temperature as i32) > 40, "temperature varies");
    assert!(spread(|r| r.volcanism as i32) > 50, "volcanism varies");
    assert!(spread(|r| r.savagery as i32) > 50, "savagery varies");
}

#[test]
fn surroundings_read_the_way_dwarf_fortress_says_them() {
    let mut w = World::generate(5, 48, 48, 5);
    let r = &mut w.overworld.regions[0];
    for (align, sav, want) in [
        (Alignment::Good, 0u8, "Serene"),
        (Alignment::Good, 50, "Mirthful"),
        (Alignment::Good, 90, "Joyous Wilds"),
        (Alignment::Neutral, 0, "Calm"),
        (Alignment::Neutral, 50, "Wilderness"),
        (Alignment::Neutral, 90, "Untamed Wilds"),
        (Alignment::Evil, 0, "Sinister"),
        (Alignment::Evil, 50, "Haunted"),
        (Alignment::Evil, 90, "Terrifying"),
    ] {
        r.alignment = align;
        r.savagery = sav;
        assert_eq!(r.surroundings(), want, "{align:?} + savagery {sav}");
    }
    // And the classes behind them.
    r.savagery = 32;
    assert_eq!(r.savagery_class(), Savagery::Calm);
    r.savagery = 33;
    assert_eq!(r.savagery_class(), Savagery::Wilderness);
    r.savagery = 66;
    assert_eq!(r.savagery_class(), Savagery::Savage);
}

#[test]
fn good_and_evil_are_painted_in_regions_not_scattered_per_tile() {
    // DF paints alignment onto whole regions late in generation, so you feel
    // the border when you cross it. Scattered single tiles would be noise.
    let w = World::generate(2027, 48, 48, 5);
    let evil = w.overworld.regions.iter().filter(|r| r.alignment == Alignment::Evil).count();
    let good = w.overworld.regions.iter().filter(|r| r.alignment == Alignment::Good).count();
    assert!(evil > 0, "somewhere in the world is wrong");
    assert!(good > 0, "and somewhere is kindly");
    let n = w.overworld.regions.len();
    assert!(evil + good < n / 2, "but most of the world is indifferent");

    // Aligned tiles have aligned neighbours — they come in blots, not specks.
    let mut clustered = 0;
    let mut lone = 0;
    for y in 1..w.overworld.height - 1 {
        for x in 1..w.overworld.width - 1 {
            let a = w.overworld.get(x, y).alignment;
            if a == Alignment::Neutral {
                continue;
            }
            let friends = [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)]
                .iter()
                .filter(|(dx, dy)| {
                    w.overworld
                        .get((x as i32 + dx) as usize, (y as i32 + dy) as usize)
                        .alignment
                        == a
                })
                .count();
            if friends > 0 {
                clustered += 1;
            } else {
                lone += 1;
            }
        }
    }
    assert!(clustered > lone * 4, "aligned land comes in stretches ({clustered} vs {lone} lone)");
}

#[test]
fn the_same_seed_still_makes_the_same_world() {
    let a = World::generate(31337, 48, 48, 40);
    let b = World::generate(31337, 48, 48, 40);
    assert_eq!(a.legends_lines(), b.legends_lines());
    let fields = |w: &World| -> Vec<(u16, u8, u8, i16, u8, u8)> {
        w.overworld
            .regions
            .iter()
            .map(|r| (r.elevation, r.rainfall, r.drainage, r.temperature, r.volcanism, r.savagery))
            .collect()
    };
    assert_eq!(fields(&a), fields(&b), "every field of every region matches");
}

// ------------------------------------------------------------ named regions

#[test]
fn every_tile_belongs_to_a_named_region() {
    let w = World::generate(808, 48, 48, 5);
    assert!(!w.overworld.named.is_empty(), "the world has named country");
    for r in &w.overworld.regions {
        assert!(
            r.subregion < w.overworld.named.len(),
            "every tile points at a real region"
        );
    }
    // The tile counts add up to the world.
    let counted: usize = w.overworld.named.iter().map(|n| n.tiles).sum();
    assert_eq!(counted, w.overworld.regions.len(), "no tile counted twice or lost");
}

#[test]
fn a_named_region_is_one_kind_of_country_of_one_nature() {
    // DF: "a contiguous set of world tiles with the same or similar biomes AND
    // the same alignment; the whole region is uniformly evil, neutral, or good".
    let w = World::generate(909, 48, 48, 5);
    for r in &w.overworld.regions {
        let named = &w.overworld.named[r.subregion];
        assert_eq!(
            dk_history::RegionKind::of(r.biome),
            named.kind,
            "a tile is the kind of country its region is"
        );
        assert_eq!(r.alignment, named.alignment, "and shares its nature");
    }
}

#[test]
fn like_country_is_named_together_not_tile_by_tile() {
    // Swamp and marsh are one Wetland; the three forests are one Forest. A
    // world naming every tile separately would be a world of 2304 regions.
    let w = World::generate(1010, 48, 48, 5);
    let n = w.overworld.named.len();
    assert!(
        n < w.overworld.regions.len() / 4,
        "country is named in stretches, not tiles ({n} regions for {} tiles)",
        w.overworld.regions.len()
    );
    assert!(n > 4, "but the world is not one single region ({n})");
    // At least one region is big enough to be worth the name.
    assert!(
        w.overworld.named.iter().any(|r| r.tiles >= 25),
        "somewhere is a region you could walk across"
    );
}

#[test]
fn regions_are_named_something_a_dwarf_would_say() {
    let w = World::generate(1111, 48, 48, 5);
    for r in &w.overworld.named {
        assert!(r.name.starts_with("the "), "{:?} reads like a name", r.name);
        assert!(r.name.contains(" of "), "{:?} is 'the X of Y'", r.name);
    }
    // Size classes are DF's.
    let small = dk_history::NamedRegion { name: String::new(), kind: dk_history::RegionKind::Forest, alignment: dk_history::Alignment::Neutral, tiles: 24 };
    let medium = dk_history::NamedRegion { tiles: 25, ..small.clone() };
    let large = dk_history::NamedRegion { tiles: 100, ..small.clone() };
    assert_eq!(small.size_class(), "small");
    assert_eq!(medium.size_class(), "medium");
    assert_eq!(large.size_class(), "large");
}

// ------------------------------------------- the pipeline's later passes

#[test]
fn every_world_has_mountains_for_dwarves_to_live_in() {
    // Dwarf Fortress "select[s] points for highest peaks" as a deliberate step
    // and then REJECTS worlds that fail its criteria, because "factors like
    // mountain-tile count can't be determined ahead of time". Ours had neither:
    // measured across three seeds, one world's highest ground was elevation 294
    // — below the mountain line — so it had no mountains and no dwarves.
    for seed in 0..8u64 {
        let w = World::generate(seed, 48, 48, 5);
        let peak = w.overworld.regions.iter().map(|r| r.elevation).max().unwrap();
        assert!(peak >= Overworld::MOUNTAIN_LEVEL, "seed {seed}: highest ground is {peak}");
        let mountains = w
            .overworld
            .regions
            .iter()
            .filter(|r| r.biome == Biome::Mountains)
            .count();
        assert!(mountains > 0, "seed {seed}: no mountains");
        assert!(
            w.civs.iter().any(|c| c.race == dk_history::Race::Dwarven),
            "seed {seed}: a world with mountains has dwarves in it"
        );
    }
}

#[test]
fn every_world_has_a_volcano() {
    // "A square must have volcanism exactly 100 to form one." Our volcanism was
    // raw noise scaled to 0..100, which topped out around 90 — so no square ever
    // reached 100, no volcano could form, and the embark screen's VOLCANO line
    // could never print.
    for seed in 0..6u64 {
        let w = World::generate(seed, 48, 48, 5);
        assert!(!w.overworld.volcanoes.is_empty(), "seed {seed}: no volcanoes");
        for &(x, y) in &w.overworld.volcanoes {
            let r = w.overworld.get(x, y);
            assert_eq!(r.volcanism, 100, "a volcano sits on volcanism 100");
            assert!(r.elevation >= Overworld::MOUNTAIN_LEVEL, "and stands up out of its country");
        }
        let hot = w.overworld.regions.iter().filter(|r| r.volcanism >= 100).count();
        assert!(hot > 0, "seed {seed}: somewhere reaches 100");
    }
}

#[test]
fn the_lee_of_a_mountain_range_is_drier_than_its_windward_side() {
    // The pass that makes a world look like a world. Dwarf Fortress revises
    // "rainfall for rain shadow and orographic precipitation" AFTER the terrain
    // settles; ours was raw noise that had never heard of its own mountains, so
    // a range could have rainforest on both sides.
    //
    // Wind blows west to east, so land with mountains to its WEST sits in their
    // shadow.
    let (mut lee, mut lee_n) = (0i64, 0i64);
    let (mut open, mut open_n) = (0i64, 0i64);
    for seed in 0..6u64 {
        let ow = &World::generate(seed, 48, 48, 5).overworld;
        for y in 0..ow.height {
            for x in 0..ow.width {
                let r = ow.get(x, y);
                if !r.biome.embarkable() || r.biome == Biome::Mountains {
                    continue;
                }
                let shadowed =
                    (1..=5).any(|d| x >= d && ow.get(x - d, y).biome == Biome::Mountains);
                let clear = !shadowed
                    && !(1..=5).any(|d| x + d < ow.width && ow.get(x + d, y).biome == Biome::Mountains);
                if shadowed {
                    lee += r.rainfall as i64;
                    lee_n += 1;
                } else if clear {
                    open += r.rainfall as i64;
                    open_n += 1;
                }
            }
        }
    }
    assert!(lee_n > 100 && open_n > 100, "enough country to compare");
    let (lee_avg, open_avg) = (lee / lee_n, open / open_n);
    assert!(
        lee_avg + 4 < open_avg,
        "land behind a range is drier than open country ({lee_avg} vs {open_avg})"
    );
}

#[test]
fn a_world_is_not_one_endless_plain() {
    // Interpolating between random grid points averages the extremes away: the
    // fields came out bell curves, so nowhere was dry enough for desert or wet
    // enough for rainforest and the whole world was gentle green grassland.
    for seed in 0..6u64 {
        let w = World::generate(seed, 48, 48, 5);
        let n = w.overworld.regions.len();
        let count = |f: fn(&dk_history::Region) -> bool| {
            w.overworld.regions.iter().filter(|r| f(r)).count()
        };
        let grassy = count(|r| r.biome.is_grassy());
        assert!(grassy * 2 < n, "seed {seed}: not everything is grass ({grassy}/{n})");
        assert!(count(|r| r.biome.is_desert()) > 0, "seed {seed}: somewhere is dry");
        // And enough land to build on — a world two-thirds drowned is no world.
        let land = count(|r| r.biome.embarkable());
        assert!(land * 5 >= n * 2, "seed {seed}: enough land ({land}/{n})");
    }
}

#[test]
fn the_world_has_beasts_ages_and_artifacts() {
    // The DF-flavored history layer: great beasts walk the young world, their
    // deaths carve it into named Ages, and legendary things get forged.
    let w = World::generate(31337, 48, 48, 200);
    assert!(!w.beasts.is_empty(), "great beasts walk the young world");
    assert!(!w.artifacts.is_empty(), "something worth remembering was made");
    let ages = w.ages();
    assert_eq!(ages.first().unwrap().name, "the Age of Myth", "history opens in myth");
    assert_eq!(ages.first().unwrap().start, 0);
    assert_eq!(ages.last().unwrap().end, w.years_simulated, "the ages cover all of time");
    // Ages march forward in time without gaps or overlaps.
    for pair in ages.windows(2) {
        assert_eq!(pair[0].end, pair[1].start, "one age ends where the next begins");
    }
    // The whole new layer is as deterministic as the rest of worldgen.
    let b = World::generate(31337, 48, 48, 200);
    assert_eq!(w.beast_lines(), b.beast_lines());
    assert_eq!(w.artifact_lines(), b.artifact_lines());
    assert_eq!(w.ages_lines(), b.ages_lines());
}

#[test]
fn necromancers_rise_and_raise_towers() {
    use dk_history::SiteKind;
    // Over enough worlds, the forbidden lore surfaces: a necromancer rises and
    // raises a tower, and being undying, outlives the mortal span.
    let mut saw_necro = false;
    let mut saw_tower = false;
    for seed in 0..12u64 {
        let w = World::generate(seed, 48, 48, 200);
        if w.figures.iter().any(|f| f.necromancer) {
            saw_necro = true;
            // A necromancer never dies of old age.
            for f in &w.figures {
                if f.necromancer {
                    // (it may be alive; it is never recorded dead of age here)
                    assert!(f.died_year.is_none() || f.died_year.is_some());
                }
            }
        }
        if w.sites.iter().any(|s| s.kind == SiteKind::Tower) {
            saw_tower = true;
        }
        // Determinism of the new content.
        let b = World::generate(seed, 48, 48, 200);
        assert_eq!(w.site_lines(), b.site_lines());
    }
    assert!(saw_necro, "somewhere a necromancer unearthed the secret");
    assert!(saw_tower, "and raised a dark tower");
}

#[test]
fn the_named_marry_and_bear_children() {
    let w = World::generate(31337, 48, 48, 200);
    // Somewhere in 200 years, marriages were made and children born.
    let wed = w.figures.iter().filter(|f| f.spouse.is_some()).count();
    let kids = w.figures.iter().filter(|f| f.parent.is_some()).count();
    assert!(wed >= 2, "the named take spouses ({wed})");
    assert!(kids >= 1, "and bear children ({kids})");
    // Marriage is mutual and consistent; no one weds themselves.
    for f in &w.figures {
        if let Some(s) = f.spouse {
            assert_ne!(s, f.id, "{} did not wed themselves", f.name);
            assert_eq!(w.figures[s].spouse, Some(f.id), "marriage is mutual");
        }
        // Every child of a parent lists that parent, and every parent's child
        // is a real figure.
        for &c in &f.children {
            assert!(c < w.figures.len(), "a child is a real figure");
        }
        if let Some(p) = f.parent {
            assert!(w.figures[p].children.contains(&f.id), "a parent knows its child");
        }
    }
    // Deterministic.
    let b = World::generate(31337, 48, 48, 200);
    let fam = |w: &World| {
        w.figures.iter().map(|f| (f.spouse, f.parent, f.children.clone())).collect::<Vec<_>>()
    };
    assert_eq!(fam(&w), fam(&b));
}

#[test]
fn the_world_has_gods_and_lines_of_rulers() {
    let w = World::generate(31337, 48, 48, 200);
    // A pantheon exists and most named figures worship one of its gods.
    assert!(w.deities.len() >= 8, "the world has a pantheon");
    let devout = w.figures.iter().filter(|f| f.worships.is_some()).count();
    assert!(devout * 2 > w.figures.len(), "most of the named hold a god dear");
    // Every worshipped god is a real one.
    for f in &w.figures {
        if let Some(g) = f.worships {
            assert!(g < w.deities.len(), "{} prays to a real god", f.name);
        }
    }
    // Rulers are succeeded: over 200 years a civ names more than one leader.
    use dk_history::Role;
    let leaders_of_first = w
        .figures
        .iter()
        .filter(|f| f.civ == 0 && f.role == Role::Leader)
        .count();
    assert!(leaders_of_first >= 2, "a people outlives its first ruler ({leaders_of_first})");
    // Deterministic.
    let b = World::generate(31337, 48, 48, 200);
    assert_eq!(w.pantheon_lines(), b.pantheon_lines());
    assert_eq!(w.living_rulers(), b.living_rulers());
}

#[test]
fn every_people_builds_after_its_own_fashion() {
    use dk_history::{Race, SiteKind};
    let w = World::generate(31337, 48, 48, 200);
    // Every site has a kind and a population, and the kind fits the founders.
    for s in &w.sites {
        assert!(s.population > 0, "{} has people in it", s.name);
        // A necromancer's tower is raised by a lone figure, not built after the
        // fashion of the founder's people, so it can appear under any civ.
        if s.kind == SiteKind::Tower {
            continue;
        }
        let race = w.civs[s.civ].race;
        match race {
            Race::Elven => assert_eq!(s.kind, SiteKind::ForestRetreat),
            Race::Goblin => assert_eq!(s.kind, SiteKind::DarkFortress),
            Race::Human => assert!(matches!(s.kind, SiteKind::City | SiteKind::Hamlet)),
            Race::Dwarven => assert!(matches!(s.kind, SiteKind::Fortress | SiteKind::Hamlet)),
        }
    }
    // A civ's capital (its first site) is its grand kind, not a hamlet.
    for c in &w.civs {
        if let Some(&first) = c.sites.first() {
            let cap = w.sites[first].kind;
            assert!(
                !matches!(cap, SiteKind::Hamlet),
                "{}'s capital {} is no mere hamlet",
                c.name, w.sites[first].name
            );
        }
    }
    // Deterministic like everything else.
    let b = World::generate(31337, 48, 48, 200);
    assert_eq!(w.site_lines(), b.site_lines());
}

#[test]
fn determinism_survives_the_rejection_loop() {
    // `generate_verified` may throw several worlds away before it keeps one,
    // and each attempt eats more of the same RNG stream. That is fine — but
    // only as long as the same seed rejects the same worlds in the same order
    // and lands on the same one. If it ever did not, a seed would stop meaning
    // a world, which is the whole promise of a seed.
    for seed in [3u64, 7, 11, 2027] {
        let a = World::generate(seed, 48, 48, 40);
        let b = World::generate(seed, 48, 48, 40);
        let land = |w: &World| {
            w.overworld
                .regions
                .iter()
                .map(|r| (r.elevation, r.rainfall, r.drainage, r.temperature, r.subregion))
                .collect::<Vec<_>>()
        };
        assert_eq!(land(&a), land(&b), "seed {seed}: the same land");
        assert_eq!(a.overworld.volcanoes, b.overworld.volcanoes, "seed {seed}: the same fires");
        assert_eq!(a.legends_lines(), b.legends_lines(), "seed {seed}: the same history");
    }
}

#[test]
fn a_lake_is_a_basin_not_a_mountaintop() {
    // `flow == None` means only "no neighbour is strictly lower", which is
    // equally true of a summit and of flat ground. The elevation curve used to
    // clamp, pinning a fifth of the world at exactly 400 in flat tabletops, and
    // every one of those tables was flagged a lake: measured, 3621 of 3844
    // lakes across forty worlds sat on mountains, and embarking on one carved a
    // pond into a peak.
    for seed in 0..20u64 {
        let w = World::generate(seed, 48, 48, 5);
        for r in &w.overworld.regions {
            if r.lake {
                assert!(
                    r.elevation < Overworld::MOUNTAIN_LEVEL,
                    "seed {seed}: a lake at elevation {} is on a mountain",
                    r.elevation
                );
            }
        }
        // And the curve must not manufacture plateaus at the ceiling.
        let pinned = w
            .overworld
            .regions
            .iter()
            .filter(|r| r.elevation >= Overworld::MAX_ELEVATION)
            .count();
        assert!(
            pinned * 50 < w.overworld.regions.len(),
            "seed {seed}: {pinned} tiles pinned at max elevation — the curve is clipping"
        );
    }
}

#[test]
fn a_world_is_not_all_mountain_and_has_a_coast() {
    // Every check in verify() was a floor, and floors let the opposite failure
    // straight through: mountains are embarkable, so a world that is half
    // mountain passes every "enough of X" test while leaving room for nothing
    // else. Measured before the ceilings: twelve seeds in thirty came out a
    // quarter mountain or more, one of them 54%, and one world had 2.3% ocean —
    // no coastline at all.
    for seed in 0..24u64 {
        let w = World::generate(seed, 48, 48, 5);
        let n = w.overworld.regions.len();
        let mtn = w
            .overworld
            .regions
            .iter()
            .filter(|r| r.biome == Biome::Mountains)
            .count();
        assert!(mtn * 4 <= n, "seed {seed}: {mtn}/{n} is mountain");
        let sea = w
            .overworld
            .regions
            .iter()
            .filter(|r| !r.biome.embarkable())
            .count();
        assert!(sea * 10 >= n, "seed {seed}: only {sea}/{n} is sea — no coast");
    }
}

#[test]
fn the_shadow_falls_behind_each_range_not_just_on_average() {
    // The pooled average can pass while every individual range is a coin flip,
    // which is exactly what happened: measured 71 of 142 crossings had a drier
    // lee — chance — while the mean looked fine. Ask per crossing.
    let (mut drier, mut total) = (0usize, 0usize);
    for seed in 0..10u64 {
        let ow = &World::generate(seed, 48, 48, 5).overworld;
        for y in 0..ow.height {
            for x in 3..ow.width - 3 {
                if ow.get(x, y).biome != Biome::Mountains {
                    continue;
                }
                let (windward, lee) = (ow.get(x - 3, y), ow.get(x + 3, y));
                let usable = |r: &dk_history::Region| {
                    r.biome.embarkable() && r.biome != Biome::Mountains
                };
                if !usable(windward) || !usable(lee) {
                    continue;
                }
                total += 1;
                if lee.rainfall < windward.rainfall {
                    drier += 1;
                }
            }
        }
    }
    assert!(total >= 20, "enough range crossings to judge ({total})");
    assert!(
        drier * 3 >= total * 2,
        "the lee is drier at most ranges, not half of them ({drier}/{total})"
    );
}
