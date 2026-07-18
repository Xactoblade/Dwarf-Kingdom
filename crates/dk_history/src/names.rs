//! Name generation for civilizations, sites, and historical figures.
//! All syllable corpora are original to Dwarf Kingdom.

use crate::Race;
use rand::Rng;
use rand_chacha::ChaCha8Rng;

fn pick<'a>(rng: &mut ChaCha8Rng, list: &[&'a str]) -> &'a str {
    list[rng.gen_range(0..list.len())]
}

// ---------------------------------------------------------------- figures

const DWARF_ON: [&str; 12] = [
    "Bal", "Dor", "Thrun", "Kaz", "Bel", "Mor", "Vond", "Grim", "Nar", "Dur", "Tor", "Skar",
];
const DWARF_END: [&str; 10] = ["din", "grum", "mund", "vik", "rek", "gar", "na", "dis", "rim", "li"];

const HUMAN_ON: [&str; 10] = ["Ald", "Ber", "Cas", "Ed", "Hal", "Jor", "Mar", "Os", "Ro", "Wil"];
const HUMAN_END: [&str; 8] = ["win", "ric", "mond", "sa", "ther", "ban", "el", "ard"];

const ELF_ON: [&str; 10] = ["Ae", "Cael", "Elu", "Fae", "Ithi", "Lora", "Nym", "Sae", "Thal", "Yll"];
const ELF_END: [&str; 8] = ["riel", "wen", "thas", "nor", "lil", "vane", "dir", "mae"];

const GOBLIN_ON: [&str; 12] = [
    "Sno", "Grak", "Uz", "Bash", "Mog", "Zag", "Krug", "Nur", "Skab", "Drub", "Gna", "Ruk",
];
const GOBLIN_END: [&str; 10] = ["dub", "gash", "tuk", "mar", "zob", "nak", "gril", "shak", "ur", "bog"];

const GOBLIN_EPITHET: [&str; 10] = [
    "Skullcracker",
    "the Vile",
    "Bonechewer",
    "the Merciless",
    "Gutripper",
    "the Cruel",
    "Wolfbane",
    "the Festering",
    "Doomherald",
    "Threefinger",
];

pub fn figure_name(rng: &mut ChaCha8Rng, race: Race) -> String {
    match race {
        Race::Dwarven => format!("{}{}", pick(rng, &DWARF_ON), pick(rng, &DWARF_END)),
        Race::Human => format!("{}{}", pick(rng, &HUMAN_ON), pick(rng, &HUMAN_END)),
        Race::Elven => format!("{}{}", pick(rng, &ELF_ON), pick(rng, &ELF_END)),
        Race::Goblin => format!(
            "{}{} {}",
            pick(rng, &GOBLIN_ON),
            pick(rng, &GOBLIN_END),
            pick(rng, &GOBLIN_EPITHET)
        ),
    }
}

// ------------------------------------------------------------------- civs

const CIV_A: [&str; 12] = [
    "Broken", "Iron", "Silent", "Crimson", "Amber", "Hollow", "Golden", "Ashen", "Storm",
    "Deep", "Wild", "Pale",
];
const CIV_B_BY_RACE: [(&str, &[&str]); 4] = [
    ("dwarf", &["Hammers", "Anvils", "Delvings", "Beards", "Vaults", "Peaks"]),
    ("human", &["Banners", "Crowns", "Roads", "Shields", "Fields", "Towers"]),
    ("elf", &["Boughs", "Glades", "Songs", "Leaves", "Rivers", "Moons"]),
    ("goblin", &["Fangs", "Claws", "Maws", "Chains", "Spites", "Scars"]),
];

pub fn civ_name(rng: &mut ChaCha8Rng, race: Race) -> String {
    let idx = match race {
        Race::Dwarven => 0,
        Race::Human => 1,
        Race::Elven => 2,
        Race::Goblin => 3,
    };
    format!("the {} {}", pick(rng, &CIV_A), pick(rng, CIV_B_BY_RACE[idx].1))
}

// ------------------------------------------------------------------ sites

const SITE_A: [&str; 12] = [
    "Bronze", "Oaken", "Stone", "Raven", "Ember", "Frost", "Moss", "Thorn", "Salt", "Cinder",
    "High", "Low",
];
const SITE_B: [&str; 12] = [
    "gate", "hold", "spire", "hollow", "reach", "fort", "haven", "moor", "crag", "watch",
    "ford", "burrow",
];

pub fn site_name(rng: &mut ChaCha8Rng, _race: Race) -> String {
    format!("{}{}", pick(rng, &SITE_A), pick(rng, &SITE_B))
}

// ---------------------------------------------------------------- regions

/// The word a stretch of country goes by — "the Forest of Whispering".
/// Deliberately evocative rather than descriptive: a region's name should be
/// something a dwarf would say, not a label.
const REGION_OF: [&str; 24] = [
    "Whispering",
    "Sorrow",
    "the Long Dusk",
    "Iron Rain",
    "Quiet Bones",
    "the Pale Sun",
    "Wandering",
    "Old Grief",
    "Silver Thunder",
    "the Last Word",
    "Bitter Song",
    "Hollow Winds",
    "the Grey Watch",
    "Broken Promises",
    "Amber Light",
    "the Deep Hush",
    "Rusted Hope",
    "Singing Stones",
    "the Cold Vigil",
    "Glass Water",
    "Fading Echoes",
    "the Slow Fire",
    "Crooked Roads",
    "Mourning Doves",
];

pub fn region_name(rng: &mut ChaCha8Rng, kind: &str) -> String {
    format!("the {} of {}", kind, pick(rng, &REGION_OF))
}

// --------------------------------------------------------------- megabeasts

const BEAST_ON: [&str; 14] = [
    "Ssz", "Ang", "Vor", "Uth", "Ngar", "Koru", "Zmey", "Axu", "Grend", "Mor", "Tyr", "Oku",
    "Bael", "Xar",
];
const BEAST_END: [&str; 12] = [
    "oth", "ax", "uth", "gan", "mor", "ith", "ug", "and", "esh", "oros", "ull", "yx",
];
const BEAST_EPITHET: [&str; 12] = [
    "the Ashen",
    "the World-Ender",
    "Deepgnawer",
    "the Scaled Doom",
    "Fireborn",
    "the Old Terror",
    "Stonecrusher",
    "the Winged Night",
    "the Unbroken",
    "Bloodmaw",
    "the Sky's Ruin",
    "the Sleepless",
];

/// A megabeast's name: an ancient, guttural word and a title of dread.
pub fn beast_name(rng: &mut ChaCha8Rng) -> String {
    format!(
        "{}{} {}",
        pick(rng, &BEAST_ON),
        pick(rng, &BEAST_END),
        pick(rng, &BEAST_EPITHET)
    )
}

// ---------------------------------------------------------------- artifacts

const ART_ON: [&str; 12] = [
    "Ng", "Kel", "Zol", "Dur", "Bomr", "Ast", "Uzol", "Ing", "Ral", "Thob", "Vel", "Osz",
];
const ART_MID: [&str; 8] = ["ol", "az", "ar", "um", "esh", "ir", "od", "un"];
const ART_END: [&str; 8] = ["tar", "mun", "shu", "kil", "grath", "dim", "los", "reth"];

/// An artifact's proper name — a made-up dwarven word, "Ngoltar", "Kelazmun".
pub fn artifact_name(rng: &mut ChaCha8Rng) -> String {
    let mut s = String::from(pick(rng, &ART_ON));
    if rng.gen_ratio(1, 2) {
        s.push_str(pick(rng, &ART_MID));
    }
    s.push_str(pick(rng, &ART_END));
    s
}

// ------------------------------------------------------------------- world

const WORLD_EPITHET: [&str; 16] = [
    "Echoes", "Legends", "Wonder", "Mist", "the Long Dusk", "Iron", "Whispers",
    "the Deep", "Ash", "Dawn", "Sorrows", "the Endless Song", "Bronze", "Storms",
    "Quiet Stars", "the Old Roads",
];

/// The name a whole world goes by — "Ustolgrath, the World of Echoes".
pub fn world_name(rng: &mut ChaCha8Rng) -> String {
    let mut proper = String::from(pick(rng, &ART_ON));
    proper.push_str(pick(rng, &ART_MID));
    proper.push_str(pick(rng, &ART_END));
    format!("{}, the World of {}", proper, pick(rng, &WORLD_EPITHET))
}

// ------------------------------------------------------------------ deities

const DEITY_ON: [&str; 14] = [
    "A", "Ka", "Lo", "Ma", "O", "Sa", "The", "U", "Ve", "Za", "Il", "Nu", "Ra", "Xe",
];
const DEITY_MID: [&str; 10] = ["la", "ru", "na", "mo", "ri", "sha", "tho", "le", "va", "zu"];
const DEITY_END: [&str; 10] = ["th", "en", "sis", "ar", "il", "ux", "am", "dun", "ok", "eph"];

/// A god's name — lofty and made of open, resonant syllables.
pub fn deity_name(rng: &mut ChaCha8Rng) -> String {
    let mut s = String::from(pick(rng, &DEITY_ON));
    s.push_str(pick(rng, &DEITY_MID));
    if rng.gen_ratio(1, 2) {
        s.push_str(pick(rng, &DEITY_MID));
    }
    s.push_str(pick(rng, &DEITY_END));
    s
}

const ART_MATERIAL: [&str; 8] =
    ["steel", "silver", "gold", "copper", "bronze", "obsidian", "platinum", "adamantine"];
const ART_ITEM: [&str; 12] = [
    "battle axe",
    "short sword",
    "war hammer",
    "shield",
    "crown",
    "scepter",
    "amulet",
    "statue",
    "goblet",
    "breastplate",
    "ring",
    "mace",
];

/// What an artifact is — "an adamantine crown", "a steel battle axe".
pub fn artifact_kind(rng: &mut ChaCha8Rng) -> String {
    let mat = pick(rng, &ART_MATERIAL);
    let item = pick(rng, &ART_ITEM);
    let article = if matches!(mat.chars().next(), Some('a' | 'e' | 'i' | 'o' | 'u')) {
        "an"
    } else {
        "a"
    };
    format!("{} {} {}", article, mat, item)
}
