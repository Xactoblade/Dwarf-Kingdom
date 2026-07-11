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
