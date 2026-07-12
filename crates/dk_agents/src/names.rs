//! Name generation — our own syllable corpus.

use rand::Rng;
use rand_chacha::ChaCha8Rng;

const ONSETS: [&str; 14] = [
    "Bal", "Dor", "Thrun", "Kaz", "Bel", "Mor", "Vond", "Grim", "Nar", "Bof",
    "Dur", "Tor", "Hald", "Skar",
];

const CODAS: [&str; 12] = [
    "in", "ek", "rim", "gar", "din", "li", "na", "ra", "grum", "dis", "mund", "vik",
];

pub fn dwarf_name(rng: &mut ChaCha8Rng) -> String {
    let onset = ONSETS[rng.gen_range(0..ONSETS.len())];
    let coda = CODAS[rng.gen_range(0..CODAS.len())];
    format!("{onset}{coda}")
}

const ARTIFACT_A: [&str; 10] = [
    "Thunder", "Ember", "Whisper", "Oath", "Winter", "Iron", "Dawn", "Shadow", "Anvil", "River",
];
const ARTIFACT_B: [&str; 10] = [
    "gates", "song", "bind", "crown", "heart", "ward", "vow", "gleam", "root", "call",
];

/// Names for strange-mood masterworks.
pub fn artifact_name(rng: &mut ChaCha8Rng) -> String {
    format!(
        "{}{}",
        ARTIFACT_A[rng.gen_range(0..ARTIFACT_A.len())],
        ARTIFACT_B[rng.gen_range(0..ARTIFACT_B.len())]
    )
}

const BEAST_ON: [&str; 12] = [
    "Ngul", "Vor", "Xoth", "Gra", "Ulm", "Zeph", "Kra", "Thûl", "Oga", "Ssur", "Mor", "Yg",
];
const BEAST_END: [&str; 10] = [
    "goth", "raxis", "moth", "zhul", "vane", "koth", "grash", "ulon", "reth", "xix",
];
const BEAST_FORM: [&str; 10] = [
    "a great scaled serpent",
    "a towering four-armed horror",
    "a bloated eyeless mass",
    "a gaunt beast of ash and sinew",
    "a shelled thing of many legs",
    "a winged terror wreathed in smoke",
    "a hulking beast of black iron hide",
    "a writhing knot of tentacles",
    "a spined leviathan",
    "a faceless giant of stone",
];

/// A forgotten beast's name and its terrible form.
pub fn beast_name(rng: &mut ChaCha8Rng) -> (String, &'static str) {
    let name = format!(
        "{}{}",
        BEAST_ON[rng.gen_range(0..BEAST_ON.len())],
        BEAST_END[rng.gen_range(0..BEAST_END.len())]
    );
    (name, BEAST_FORM[rng.gen_range(0..BEAST_FORM.len())])
}
