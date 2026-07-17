//! Guards the audio crash we shipped once: every event sound must be a WAV the
//! game can actually decode. Bevy plays sounds through rodio, whose WAV support
//! is `hound`; if a sound file is missing, truncated, or not PCM, rodio panics
//! ("UnrecognizedFormat") the first time an event tries to play it — killing the
//! game a few minutes in. Opening each file with hound here catches that at CI
//! time instead. (The `wav` feature on the `bevy` dependency is what makes rodio
//! able to decode these at runtime — keep it enabled.)

use std::path::Path;

/// The sound names play_event_sounds looks up. Kept in sync by eye; the test
/// also sweeps the directory so an added-but-unlisted file is still checked.
const EVENT_SOUNDS: &[&str] = &[
    "hit", "horn", "chime", "bell", "toll", "hiss", "doom", "fanfare",
];

fn sounds_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/sounds")
}

#[test]
fn every_event_sound_exists_and_decodes() {
    let dir = sounds_dir();
    for name in EVENT_SOUNDS {
        let path = dir.join(format!("{name}.wav"));
        assert!(path.is_file(), "missing sound asset: {}", path.display());
        let reader = hound::WavReader::open(&path)
            .unwrap_or_else(|e| panic!("{name}.wav is not a readable WAV ({e}) — rodio would panic on it"));
        let spec = reader.spec();
        assert_eq!(
            spec.sample_format,
            hound::SampleFormat::Int,
            "{name}.wav must be PCM (integer) samples, not float"
        );
        assert!(spec.channels >= 1, "{name}.wav has no channels");
    }
}

#[test]
fn every_wav_in_the_sounds_dir_decodes() {
    // Anything dropped into assets/sounds must also be decodable, so a new sound
    // added later can't quietly reintroduce the crash.
    let dir = sounds_dir();
    let entries = std::fs::read_dir(&dir).expect("assets/sounds exists");
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("wav") {
            hound::WavReader::open(&path).unwrap_or_else(|e| {
                panic!("{} is not a decodable WAV ({e})", path.display())
            });
        }
    }
}
