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
