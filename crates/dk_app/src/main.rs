//! Dwarf Kingdom — Phase 2: Survive a Year.
//!
//! Controls:
//!   Mouse ........ left-click: move cursor   wheel: zoom
//!                  right/middle-drag: pan the map
//!   Arrow keys ... move cursor        W/A/S/E ...... pan camera (D designates)
//!   [ / ] ........ z-level down/up    - / = ........ zoom out/in
//!   d / x ........ designate mine / stairs (press to anchor, again to apply)
//!   p / f ........ place stockpile / farm plot (same two-press flow)
//!   c ............ cancel designations in a rectangle
//!   v / k ........ build still / kitchen at the cursor
//!   Esc .......... cancel current designation mode
//!   Space ........ pause    . ........ single-step while paused
//!   1 / 2 / 3 .... sim speed (normal / fast / blazing)
//!   F5 / F9 ...... save / load       Q ............ quit
//!
//! Set DK_SCREENSHOT=1 to run a scripted demo in a fixed-size window,
//! capture `phase0.png`, and exit (used for automated verification).

use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::input::mouse::{MouseMotion, MouseWheel};
use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};
use bevy::window::{MonitorSelection, PresentMode, WindowMode};
use dk_agents::{
    item_value, AnimalKind, PlayerAction,
    load_sim, save_sim, BuildingKind, DesignationKind, Faction, FarmState, ItemKind, ItemState,
    SiegeLeader, SiegeRoster, Sim,
};
use dk_history::World;
use dk_raws::Raws;
use dk_world::path::Pos;
use dk_world::TileShape;
use std::path::{Path, PathBuf};

const TILE: f32 = 16.0;
const MAP_W: usize = 96;
const MAP_H: usize = 96;
const MAP_D: usize = 32;
const WORLD_SEED: u64 = 20260710;
const DWARF_COUNT: usize = 7;
/// Overworld regions (rendered 2x on the 96x96 tile grid).
const OW: usize = 48;
const HISTORY_YEARS: u32 = 80;
/// Lines per page in the Legends viewer.
const LEGENDS_PAGE: usize = 30;

/// The controls reference shown on the Help overlay (F1). Uses `::` rather
/// than em-dashes, which the HUD font renders as a box.
const HELP_TEXT: &str = "\
Dwarf Kingdom :: Controls   (F1 or Esc to close)\n\
\n\
CAMERA & VIEW\n\
  W A S D / drag ... pan      mouse wheel ... zoom      [ ] ... change z-level\n\
  space ... pause      1 2 3 ... game speed\n\
\n\
DIG & BUILD (cursor = arrow keys or click)\n\
  d ... mine        x ... stairs      h ... channel     Shift+D ... engrave a wall\n\
  Shift+X ... fell trees for logs (drag over a stand of trees)\n\
  Shift+B ... build a wall (masons haul stone and raise it)\n\
  v ... still   k ... kitchen   m ... craftsdwarf   j ... loom   ; ... jeweler\n\
  Shift+M ... smelter (ore -> metal bars)   Shift+F ... forge (bars -> weapons & armor)\n\
  Shift+K ... mason's workshop (stone -> beds; a bed rests its owner better)\n\
  Shift+C ... clothier's shop (cloth -> clothes; a dressed dwarf frets less)\n\
  Shift+G ... glass furnace   Shift+T ... weapon trap   b ... tomb\n\
  g ... floodgate   l ... lever   t ... pull lever\n\
\n\
ZONES & LABOR\n\
  f ... farm     p ... stockpile    n ... pasture    o ... tavern    ' ... temple\n\
  z ... fishery      Shift+H ... hospital (the wounded mend here, faster)\n\
  Shift+Z ... burrow (safe room)     F2 ... sound/lift the alarm (civilians hide)\n\
  Shift+L ... library (scholars write treatises, read them in Legends)\n\
  u ... cull an animal      Shift+U ... war-train a dog\n\
  i ... enlist/dismiss a soldier      Shift+I ... barracks (soldiers drill here)\n\
  c ... cancel designations\n\
\n\
FORTRESS\n\
  r ... trade with a caravan     y ... Legends & your fort's poetry\n\
  F5 ... save     F9 ... load     F8 ... retire the fortress     Q ... quit\n\
\n\
ADVENTURE MODE (found a lone hero with 'a' on the embark map)\n\
  arrows ... move / attack      [ ] ... climb stairs      . ... wait\n\
  p ... pick up a fallen foe's weapon      c ... recruit a companion\n\
  g ... journey to the next land      y ... Legends      Esc ... abandon the quest\n\
\n\
The world only moves when you do in adventure mode. In the fortress, your\n\
dwarves decide how to do the jobs you designate. Put the cursor on a dwarf,\n\
an item, or an engraved wall to read about it in the status bar.";

// ---------------------------------------------------------------- resources

#[derive(Resource)]
struct Registry(Raws);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Embark,
    Playing,
    Adventure,
    Legends,
    Trade,
    Help,
}

#[derive(Resource)]
struct ScreenRes(Screen);

#[derive(Resource)]
struct WorldRes(World);

/// Legends viewer state: scroll offset and which screen to return to.
#[derive(Resource)]
struct LegendsState {
    scroll: usize,
    from: Screen,
}

/// Trade screen state: which column, cursor row, and the selected deal.
#[derive(Resource, Default)]
struct TradeState {
    /// 0 = the caravan's wagon, 1 = your stores.
    side: usize,
    cursor: usize,
    offer: std::collections::BTreeSet<usize>,
    request: std::collections::BTreeSet<usize>,
    message: String,
}

/// Whether a saved fortress existed at launch (embark-screen F9 hint).
#[derive(Resource)]
struct HasSave(bool);

/// Event sounds keyed by name, when assets/sounds/ exists. Absent = silent.
#[derive(Resource, Default)]
struct SoundBank(std::collections::HashMap<&'static str, Handle<AudioSource>>);

/// How far through the sim log the sound system has played.
#[derive(Resource, Default)]
struct LogCursor(usize);

/// Loaded sprite-sheet, when data/tileset.ron is present. Absent = flat
/// colored squares (the pre-graphics look).
#[derive(Resource, Default)]
struct Tileset(Option<TilesetHandles>);

struct TilesetHandles {
    image: Handle<Image>,
    layout: Handle<TextureAtlasLayout>,
    glyphs: std::collections::HashMap<String, usize>,
    tinted: std::collections::HashSet<String>,
}

impl TilesetHandles {
    fn index(&self, glyph: &str) -> usize {
        self.glyphs.get(glyph).copied().unwrap_or(0)
    }

    fn is_tinted(&self, glyph: &str) -> bool {
        self.tinted.contains(glyph)
    }

    /// A sprite showing `glyph`, sized to one map tile.
    fn sprite(&self, glyph: &str) -> Sprite {
        let mut sp = Sprite::from_atlas_image(
            self.image.clone(),
            TextureAtlas { layout: self.layout.clone(), index: self.index(glyph) },
        );
        sp.custom_size = Some(Vec2::splat(TILE));
        sp
    }
}

#[derive(Resource)]
struct SimRes(Option<Sim>);

#[derive(Resource)]
struct ViewZ(i32);

#[derive(Resource)]
struct Cursor {
    x: i32,
    y: i32,
}

impl Cursor {
    fn pos(&self, z: i32) -> Pos {
        Pos::new(self.x, self.y, z)
    }
}

#[derive(Resource)]
struct MapDirty(bool);

#[derive(Resource, Default)]
struct SimControl {
    paused: bool,
    speed: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UiKind {
    Mine,
    Stairs,
    Channel,
    Stockpile,
    Farm,
    Pasture,
    Tavern,
    Temple,
    Fishery,
    Hospital,
    Barracks,
    Burrow,
    Library,
    Chop,
    Cancel,
}

impl UiKind {
    fn label(self) -> &'static str {
        match self {
            UiKind::Mine => "MINE",
            UiKind::Stairs => "STAIRS",
            UiKind::Channel => "CHANNEL",
            UiKind::Stockpile => "STOCKPILE",
            UiKind::Farm => "FARM",
            UiKind::Pasture => "PASTURE",
            UiKind::Tavern => "TAVERN",
            UiKind::Temple => "TEMPLE",
            UiKind::Fishery => "FISHERY",
            UiKind::Hospital => "HOSPITAL",
            UiKind::Barracks => "BARRACKS",
            UiKind::Burrow => "BURROW",
            UiKind::Library => "LIBRARY",
            UiKind::Chop => "CHOP",
            UiKind::Cancel => "CANCEL",
        }
    }
}

#[derive(Resource, Default)]
struct UiMode(Option<(UiKind, Pos)>);

#[derive(Resource)]
struct MoveRepeat(Timer);

#[derive(Resource)]
struct OverlayRefresh(Timer);

#[derive(Resource, Default)]
struct ShotState {
    frames: u32,
    taken: bool,
    started_at: f64,
}

#[derive(Resource, Default)]
struct SpritePools {
    dwarves: Vec<Entity>,
    items: Vec<Entity>,
    animals: Vec<Entity>,
}

// ---------------------------------------------------------------- components

#[derive(Component)]
struct TileSprite {
    x: usize,
    y: usize,
}

#[derive(Component)]
struct CursorSprite;

#[derive(Component)]
struct HudText;

/// Borrow shims so system bodies written against `SimRes(Sim)` (field access
/// via `.0`) keep working now that SimRes holds an Option.
struct SimRef<'a>(&'a Sim);
struct SimMut<'a>(&'a mut Sim);

// ---------------------------------------------------------------- setup

fn data_dir() -> PathBuf {
    let candidates = [
        PathBuf::from("data"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data"),
    ];
    for c in candidates {
        if c.join("materials").is_dir() {
            return c;
        }
    }
    panic!("could not locate the data/ directory with material raws");
}

fn save_path() -> PathBuf {
    PathBuf::from("saves/world.bin")
}

fn world_seed_path() -> PathBuf {
    PathBuf::from("saves/world_seed")
}

/// The seed for this game's world. Persisted so a world is stable across
/// launches (your saves stay valid); a fresh install — or the "new world"
/// command — rolls a different one. Screenshot/CI mode always uses the fixed
/// seed for reproducible verification.
fn resolve_world_seed() -> u64 {
    if screenshot_mode_on() {
        return WORLD_SEED;
    }
    if let Ok(s) = std::fs::read_to_string(world_seed_path()) {
        if let Ok(seed) = s.trim().parse::<u64>() {
            return seed;
        }
    }
    let seed = fresh_world_seed();
    persist_world_seed(seed);
    seed
}

/// A brand-new random world seed from the wall clock, bit-spread so successive
/// worlds look nothing alike.
fn fresh_world_seed() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(WORLD_SEED);
    (nanos ^ 0x9E37_79B9_7F4A_7C15).wrapping_mul(0xD1B5_4A32_D192_ED03) | 1
}

fn persist_world_seed(seed: u64) {
    let _ = std::fs::create_dir_all("saves");
    let _ = std::fs::write(world_seed_path(), seed.to_string());
}

/// A retired fortress is kept in its own file, named for its region, so it
/// endures in the world and can be reclaimed by returning to that spot.
fn fort_path(region: (usize, usize)) -> PathBuf {
    PathBuf::from(format!("saves/fort_{}_{}.bin", region.0, region.1))
}

/// The assets directory lives at the workspace root, not the app crate.
fn assets_dir() -> String {
    let candidates = [
        PathBuf::from("assets"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets"),
    ];
    for c in &candidates {
        if c.is_dir() {
            // Bevy joins relative paths onto the app crate's manifest dir,
            // so hand it an absolute path.
            if let Ok(abs) = std::fs::canonicalize(c) {
                return abs.to_string_lossy().into_owned();
            }
        }
    }
    "assets".to_string()
}

fn screenshot_mode_on() -> bool {
    std::env::var_os("DK_SCREENSHOT").is_some()
}

/// Build the fortress sim for a chosen overworld region.
/// The deterministic local map for an overworld region.
/// The full Legends scroll: the active fortress's own anthology of poetry
/// first (latest works up top), then the recorded history of the world.
fn legends_all(sim: Option<&Sim>, world: &World) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(s) = sim {
        if !s.poems.is_empty() {
            lines.push("=== The Fortress Anthology ===".to_string());
            for p in s.poems.iter().rev().take(40) {
                lines.push(p.clone());
            }
            lines.push(String::new());
        }
        if !s.treatises.is_empty() {
            lines.push("=== The Library ===".to_string());
            for t in s.treatises.iter().rev().take(40) {
                lines.push(t.clone());
            }
            lines.push(String::new());
        }
    }
    lines.extend(world.legends_lines());
    lines
}

/// How a region's overworld biome paints its local surface soil.
fn surface_style(biome: dk_history::Biome) -> dk_world::SurfaceStyle {
    use dk_history::Biome;
    use dk_world::SurfaceStyle;
    match biome {
        Biome::Desert => SurfaceStyle::Sandy,
        Biome::Swamp => SurfaceStyle::Clayey,
        Biome::Grassland | Biome::Forest | Biome::Hills => SurfaceStyle::Loamy,
        _ => SurfaceStyle::Default,
    }
}

/// Whether a region's biome is wet enough to run a river across its map.
fn has_river(biome: dk_history::Biome) -> bool {
    use dk_history::Biome;
    matches!(biome, Biome::Grassland | Biome::Forest | Biome::Swamp)
}

fn region_map(world: &World, raws: &Raws, region: (usize, usize)) -> dk_world::Map {
    let seed = world.seed ^ ((region.0 as u64) << 32 | region.1 as u64);
    let mut rng = dk_core::rng_from_seed(seed);
    let biome = world.overworld.get(region.0, region.1).biome;
    let style = surface_style(biome);
    let mut map = dk_world::generate_styled(&raws.materials, &mut rng, MAP_W, MAP_H, MAP_D, seed, style);
    if has_river(biome) {
        dk_world::carve_river(&mut map, seed);
    }
    map
}

fn embark(world: &World, raws: &Raws, region: (usize, usize)) -> Sim {
    // Each region is its own deterministic local map.
    let seed = world.seed ^ ((region.0 as u64) << 32 | region.1 as u64);
    let mut rng = dk_core::rng_from_seed(seed);
    let biome = world.overworld.get(region.0, region.1).biome;
    let style = surface_style(biome);
    let mut map = dk_world::generate_styled(&raws.materials, &mut rng, MAP_W, MAP_H, MAP_D, seed, style);
    // A river runs through wetter lands — carved after gen; it draws no RNG,
    // so the dwarves rolled below are unchanged.
    if has_river(biome) {
        dk_world::carve_river(&mut map, seed);
    }
    let mut sim = Sim::new(map, raws, rng, DWARF_COUNT);
    sim.home_region = Some(region);
    sim.add_embark_supplies(raws);
    sim.add_starting_dogs();
    // Scatter a woodland across the surface — dense in forests, sparse on the
    // plains, bare in the desert. Woodcutters fell them (Shift+X) for logs.
    let trees = {
        use dk_history::Biome;
        match biome {
            Biome::Forest => 240,
            Biome::Grassland | Biome::Swamp | Biome::Hills => 120,
            Biome::Desert | Biome::Mountains => 20,
            _ => 60,
        }
    };
    sim.plant_trees(trees);
    // Rarely (about one fort in ten), one of the founding seven keeps a dark
    // secret — a vampire, indistinguishable from any other dwarf until
    // fort-mates start turning up drained of blood.
    sim.maybe_curse_a_vampire();
    // Caravans come from the nearest friendly neighbors.
    sim.trade_partner = world
        .nearest_friendly_civ(region.0, region.1)
        .map(|c| c.name.clone());
    // Wire the nearest hostile civ's grudge-bearers as siege leaders.
    if let Some((civ_name, leaders)) = world.siege_pack(region.0, region.1) {
        sim.siege_roster = Some(SiegeRoster {
            civ_name,
            leaders: leaders
                .into_iter()
                .map(|(name, grudge)| SiegeLeader { name, grudge })
                .collect(),
        });
    }
    sim
}

/// Nearest embarkable region to the world's center (screenshot auto-embark).
fn default_region(world: &World) -> (usize, usize) {
    let (cx, cy) = (OW / 2, OW / 2);
    let mut best = (cx, cy);
    let mut best_d = usize::MAX;
    for y in 0..OW {
        for x in 0..OW {
            if world.overworld.get(x, y).biome.embarkable() {
                let d = x.abs_diff(cx) + y.abs_diff(cy);
                if d < best_d {
                    best_d = d;
                    best = (x, y);
                }
            }
        }
    }
    best
}

fn main() {
    let raws = Raws::load(&data_dir()).expect("failed to load raws");
    let world = World::generate(resolve_world_seed(), OW, OW, HISTORY_YEARS);
    // DK_SHOT_SCREEN=embark verifies the embark map instead of the fort.
    let shot_embark = std::env::var("DK_SHOT_SCREEN").is_ok_and(|v| v == "embark");
    let (screen, sim) = if screenshot_mode_on() && !shot_embark {
        let mut sim = embark(&world, &raws, default_region(&world));
        demo_scenario(&mut sim, &raws);
        (Screen::Playing, Some(sim))
    } else {
        (Screen::Embark, None)
    };
    let start_z = sim
        .as_ref()
        .and_then(|s| s.map.walk_surface_z(MAP_W / 2, MAP_H / 2))
        .unwrap_or(MAP_D / 2) as i32;
    let sim_hz = if screenshot_mode_on() { 180.0 } else { dk_core::SIM_HZ };

    let window = if screenshot_mode_on() {
        Window {
            title: "Dwarf Kingdom".into(),
            resolution: (1100.0_f32, 860.0_f32).into(),
            present_mode: PresentMode::AutoNoVsync,
            ..default()
        }
    } else {
        Window {
            title: "Dwarf Kingdom".into(),
            mode: WindowMode::BorderlessFullscreen(MonitorSelection::Primary),
            present_mode: PresentMode::AutoVsync,
            ..default()
        }
    };

    App::new()
        .add_plugins((
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(window),
                    ..default()
                })
                .set(AssetPlugin {
                    file_path: assets_dir(),
                    ..default()
                }),
            FrameTimeDiagnosticsPlugin::default(),
        ))
        .insert_resource(ClearColor(Color::srgb(0.04, 0.04, 0.06)))
        .insert_resource(Time::<Fixed>::from_hz(sim_hz))
        .insert_resource(Registry(raws))
        .insert_resource(WorldRes(world))
        .insert_resource(ScreenRes(screen))
        .insert_resource(LegendsState { scroll: 0, from: Screen::Embark })
        .insert_resource(TradeState::default())
        .insert_resource(HasSave(save_path().exists()))
        .insert_resource(SimRes(sim))
        .insert_resource(ViewZ(start_z))
        .insert_resource(Cursor { x: MAP_W as i32 / 2, y: MAP_H as i32 / 2 })
        .insert_resource(MapDirty(true))
        .insert_resource(SimControl { paused: false, speed: 1 })
        .insert_resource(UiMode::default())
        .insert_resource(MoveRepeat(Timer::from_seconds(0.08, TimerMode::Repeating)))
        .insert_resource(OverlayRefresh(Timer::from_seconds(1.0, TimerMode::Repeating)))
        .insert_resource(ShotState::default())
        .insert_resource(SpritePools::default())
        .add_systems(Startup, setup)
        .add_systems(FixedUpdate, run_sim)
        .add_systems(
            Update,
            (
                (
                    handle_mouse,
                    handle_input,
                    handle_trade_input,
                    overlay_refresh,
                    redraw_tiles,
                )
                    .chain(),
                (
                    sync_agent_sprites,
                    position_cursor_sprite,
                    update_hud,
                    play_event_sounds,
                    screenshot_mode,
                )
                    .chain(),
            )
                .chain(),
        )
        .run();
}

/// Scripted demo for automated verification: dig scenario + food industry.
fn demo_scenario(sim: &mut Sim, raws: &Raws) {
    let cx = sim.map.width as i32 / 2;
    let cy = sim.map.height as i32 / 2;
    let Some(wz) = sim.map.walk_surface_z(cx as usize, cy as usize) else { return };
    let wz = wz as i32;
    // Staircase into stone + a room.
    let (rx0, rx1, ry0, ry1) = (cx + 1, cx + 5, cy - 2, cy + 2);
    let room_z = (ry0..=ry1)
        .flat_map(|y| (rx0..=rx1).map(move |x| (x, y)))
        .chain(std::iter::once((cx, cy)))
        .filter_map(|(x, y)| sim.map.surface_z(x as usize, y as usize))
        .map(|z| z as i32)
        .min()
        .map(|s| (s - 3).max(2))
        .unwrap_or(wz - 3);
    for z in room_z..=wz {
        sim.designate_rect(DesignationKind::Stairs, Pos::new(cx, cy, z), Pos::new(cx, cy, z));
    }
    sim.designate_rect(
        DesignationKind::Mine,
        Pos::new(rx0, ry0, room_z),
        Pos::new(rx1, ry1, room_z),
    );
    // Food industry.
    if let Some((fa, fb)) = sim.find_flat_patch(cx, cy) {
        sim.add_farm(fa, fb, 0);
    }
    if raws.plants.len() > 1 {
        if let Some((fa, fb)) = sim.find_flat_patch(cx, cy) {
            sim.add_farm(fa, fb, 1);
        }
    }
    if let Some((wa, _)) = sim.find_flat_patch(cx, cy) {
        sim.add_building(BuildingKind::Still, wa);
        sim.add_building(BuildingKind::Kitchen, Pos::new(wa.x + 1, wa.y, wa.z));
        sim.add_building(BuildingKind::Craftsdwarf, Pos::new(wa.x + 2, wa.y, wa.z));
        sim.add_building(BuildingKind::Loom, Pos::new(wa.x + 3, wa.y, wa.z));
        sim.add_building(BuildingKind::Jeweler, Pos::new(wa.x + 4, wa.y, wa.z));
    }
    // A tavern and a temple so the demo shows the social/spiritual hubs.
    if let Some((va, vb)) = sim.find_flat_patch(cx, cy) {
        sim.add_tavern(va, vb);
    }
    if let Some((ea, eb)) = sim.find_flat_patch(cx, cy) {
        sim.add_temple(ea, eb);
    }
    // A pasture with a small herd so the demo shows livestock.
    if let Some((pa, pb)) = sim.find_flat_patch(cx, cy) {
        sim.add_pasture(pa, pb);
        let c = pa;
        sim.add_animal(AnimalKind::Cow, c, true);
        sim.add_animal(AnimalKind::Sheep, Pos::new(c.x + 1, c.y, c.z), true);
        sim.add_animal(AnimalKind::Sheep, Pos::new(c.x, c.y + 1, c.z), false);
        // A guard dog, already war-trained, to show off the sprite.
        let dog = sim.add_animal(AnimalKind::Dog, Pos::new(c.x + 2, c.y, c.z), true);
        sim.animals[dog].war = true;
    }
    sim.place_flat_stockpiles(cx, cy, 36);
}

fn setup(
    mut commands: Commands,
    reg: Res<Registry>,
    asset_server: Res<AssetServer>,
    mut layouts: ResMut<Assets<TextureAtlasLayout>>,
) {
    // Load the sprite sheet if data/tileset.ron declared one.
    let tileset = reg.0.tileset.as_ref().map(|def| {
        let image: Handle<Image> = asset_server.load(def.image.clone());
        let layout = layouts.add(TextureAtlasLayout::from_grid(
            UVec2::splat(def.tile_px),
            def.columns,
            def.rows,
            None,
            None,
        ));
        TilesetHandles {
            image,
            layout,
            glyphs: def.glyphs.clone(),
            tinted: def.tinted.iter().cloned().collect(),
        }
    });

    let center = Vec3::new(
        MAP_W as f32 * TILE * 0.5,
        MAP_H as f32 * TILE * 0.5,
        999.0,
    );
    commands.spawn((Camera2d, Transform::from_translation(center)));

    for y in 0..MAP_H {
        for x in 0..MAP_W {
            let sprite = match &tileset {
                Some(ts) => {
                    let mut sp = ts.sprite("block");
                    sp.color = Color::BLACK;
                    sp
                }
                None => Sprite {
                    color: Color::BLACK,
                    custom_size: Some(Vec2::splat(TILE - 1.0)),
                    ..default()
                },
            };
            commands.spawn((
                sprite,
                Transform::from_xyz(x as f32 * TILE, y as f32 * TILE, 0.0),
                TileSprite { x, y },
            ));
        }
    }
    commands.insert_resource(Tileset(tileset));

    // Event sounds are optional: no directory, no sound, no errors. The
    // automated screenshot/CI harness has no audio output device, where
    // bevy_audio panics decoding the first queued sound — so stay silent there.
    let mut bank = std::collections::HashMap::new();
    let sound_dir = std::path::Path::new(&assets_dir()).join("sounds");
    if sound_dir.is_dir() && !screenshot_mode_on() {
        for name in ["hit", "horn", "chime", "bell", "toll", "hiss", "doom", "fanfare"] {
            if sound_dir.join(format!("{name}.wav")).is_file() {
                bank.insert(name, asset_server.load(format!("sounds/{name}.wav")));
            }
        }
    }
    commands.insert_resource(SoundBank(bank));
    commands.insert_resource(LogCursor(0));

    commands.spawn((
        Sprite {
            color: Color::srgba(1.0, 0.95, 0.3, 0.55),
            custom_size: Some(Vec2::splat(TILE)),
            ..default()
        },
        Transform::from_xyz(0.0, 0.0, 5.0),
        CursorSprite,
    ));

    commands.spawn((
        Text::new(""),
        TextFont { font_size: 15.0, ..default() },
        TextColor(Color::srgb(0.9, 0.9, 0.85)),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(10.0),
            top: Val::Px(8.0),
            ..default()
        },
        HudText,
    ));
}

// ---------------------------------------------------------------- systems

fn run_sim(
    mut sim: ResMut<SimRes>,
    reg: Res<Registry>,
    control: Res<SimControl>,
    screen: Res<ScreenRes>,
    mut dirty: ResMut<MapDirty>,
) {
    // The fortress keeps living while you read Legends. Embark has no sim,
    // and Adventure advances only when the player acts.
    if control.paused || matches!(screen.0, Screen::Embark | Screen::Adventure) {
        return;
    }
    let Some(sim) = sim.0.as_mut() else { return };
    sim.step(&reg.0);
    if sim.map_changed {
        sim.map_changed = false;
        dirty.0 = true;
    }
}

/// Farm growth and stockpile contents change tile tints slowly; refresh the
/// overlay layer once a second instead of every frame.
fn overlay_refresh(
    time: Res<Time>,
    screen: Res<ScreenRes>,
    mut timer: ResMut<OverlayRefresh>,
    mut dirty: ResMut<MapDirty>,
) {
    // Farm growth/stockpile tints only change while Playing; embark and
    // legends screens are static and need no periodic recolor.
    if screen.0 == Screen::Playing && timer.0.tick(time.delta()).just_finished() {
        dirty.0 = true;
    }
}

fn handle_mouse(
    buttons: Res<ButtonInput<MouseButton>>,
    mut wheel: EventReader<MouseWheel>,
    mut motion: EventReader<MouseMotion>,
    windows: Query<&Window>,
    mut camera: Query<(&Camera, &GlobalTransform, &mut Transform), With<Camera2d>>,
    mut cursor: ResMut<Cursor>,
    mode: Res<UiMode>,
    mut dirty: ResMut<MapDirty>,
) {
    let Ok((cam, cam_global, mut cam_tf)) = camera.single_mut() else { return };

    // Wheel: zoom toward the current view center.
    let mut zoom = 0.0f32;
    for ev in wheel.read() {
        zoom += ev.y;
    }
    if zoom.abs() > 0.01 {
        let factor = if zoom > 0.0 { 0.9 } else { 1.1 };
        cam_tf.scale = (cam_tf.scale * factor).clamp(
            Vec3::splat(0.2),
            Vec3::splat(4.0),
        );
    }

    // Right/middle drag: pan.
    let dragging = buttons.pressed(MouseButton::Right) || buttons.pressed(MouseButton::Middle);
    let mut delta = Vec2::ZERO;
    for ev in motion.read() {
        delta += ev.delta;
    }
    if dragging && delta != Vec2::ZERO {
        cam_tf.translation.x -= delta.x * cam_tf.scale.x;
        cam_tf.translation.y += delta.y * cam_tf.scale.y;
    }

    // Left click: move the tile cursor to the clicked tile.
    if buttons.just_pressed(MouseButton::Left) {
        let Ok(window) = windows.single() else { return };
        let Some(screen) = window.cursor_position() else { return };
        let Ok(world) = cam.viewport_to_world_2d(cam_global, screen) else { return };
        let tx = (world.x / TILE).round() as i32;
        let ty = (world.y / TILE).round() as i32;
        if (0..MAP_W as i32).contains(&tx) && (0..MAP_H as i32).contains(&ty) {
            cursor.x = tx;
            cursor.y = ty;
            if mode.0.is_some() {
                dirty.0 = true;
            }
        }
    }

    // Keep the view over the map: never pan (or zoom out) into the void past
    // its edges — the map fills the frame and stops at its banks, like DF.
    if let Ok(window) = windows.single() {
        clamp_camera_to_map(&mut cam_tf, window.width(), window.height());
    }
}

/// Hold the camera over the map like DF: the map always fills the frame and you
/// scroll to its edges, never past them into the void. Enforced every frame so
/// it also corrects a fresh recenter's default zoom to fit the actual window.
fn clamp_camera_to_map(tf: &mut Transform, win_w: f32, win_h: f32) {
    let map_w = MAP_W as f32 * TILE;
    let map_h = MAP_H as f32 * TILE;
    // Cap zoom-out at the scale where the view still fits inside the map on
    // both axes — beyond it the void would show. Independent of monitor size,
    // so a wide 4K screen zooms in enough to fill just like a laptop does.
    let fit = (map_w / win_w).min(map_h / win_h);
    let s = tf.scale.x.min(fit).max(0.2);
    tf.scale = Vec3::new(s, s, tf.scale.z);
    let half_vw = win_w * 0.5 * s;
    let half_vh = win_h * 0.5 * s;
    tf.translation.x = if half_vw * 2.0 >= map_w {
        map_w * 0.5
    } else {
        tf.translation.x.clamp(half_vw, map_w - half_vw)
    };
    tf.translation.y = if half_vh * 2.0 >= map_h {
        map_h * 0.5
    } else {
        tf.translation.y.clamp(half_vh, map_h - half_vh)
    };
}

#[allow(clippy::too_many_arguments)]
fn handle_input(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    reg: Res<Registry>,
    mut world: ResMut<WorldRes>,
    mut screen: ResMut<ScreenRes>,
    mut legends: ResMut<LegendsState>,
    mut repeat: ResMut<MoveRepeat>,
    mut cursor: ResMut<Cursor>,
    mut view_z: ResMut<ViewZ>,
    mut dirty: ResMut<MapDirty>,
    mut sim: ResMut<SimRes>,
    mut control: ResMut<SimControl>,
    mut mode: ResMut<UiMode>,
    mut fixed_time: ResMut<Time<Fixed>>,
    mut camera: Query<&mut Transform, With<Camera2d>>,
    mut exit: EventWriter<AppExit>,
) {
    if screen.0 == Screen::Trade {
        return; // handle_trade_input owns this screen
    }
    // ---- Help overlay: a controls reference, openable from any screen.
    if screen.0 == Screen::Help {
        if keys.just_pressed(KeyCode::F1) || keys.just_pressed(KeyCode::Escape) {
            screen.0 = legends.from;
            dirty.0 = true;
        }
        if keys.just_pressed(KeyCode::KeyQ) {
            exit.write(AppExit::Success);
        }
        return;
    }
    if keys.just_pressed(KeyCode::F1) {
        legends.from = screen.0;
        screen.0 = Screen::Help;
        dirty.0 = true;
        return;
    }
    // ---- Legends screen: scroll and close.
    if screen.0 == Screen::Legends {
        // The view never scrolls past the last full page.
        let total = legends_all(sim.0.as_ref(), &world.0).len();
        let max_scroll = total.saturating_sub(LEGENDS_PAGE);
        let held = |key: KeyCode, keys: &ButtonInput<KeyCode>, repeat: &MoveRepeat| {
            keys.just_pressed(key) || (keys.pressed(key) && repeat.0.just_finished())
        };
        if held(KeyCode::ArrowDown, &keys, &repeat) {
            legends.scroll = (legends.scroll + 1).min(max_scroll);
        }
        if held(KeyCode::ArrowUp, &keys, &repeat) {
            legends.scroll = legends.scroll.saturating_sub(1);
        }
        if keys.just_pressed(KeyCode::PageDown) {
            legends.scroll = (legends.scroll + LEGENDS_PAGE).min(max_scroll);
        }
        if keys.just_pressed(KeyCode::PageUp) {
            legends.scroll = legends.scroll.saturating_sub(LEGENDS_PAGE);
        }
        if keys.just_pressed(KeyCode::KeyY) || keys.just_pressed(KeyCode::Escape) {
            screen.0 = if sim.0.is_some() { legends.from } else { Screen::Embark };
            dirty.0 = true;
        }
        if keys.just_pressed(KeyCode::KeyQ) {
            exit.write(AppExit::Success);
        }
        repeat.0.tick(time.delta());
        return;
    }

    // ---- Embark screen: pick a region and found the fortress.
    if screen.0 == Screen::Embark {
        repeat.0.tick(time.delta());
        let step_ok = repeat.0.just_finished();
        let mut dx = 0i32;
        let mut dy = 0i32;
        for ((mx, my), key) in [
            ((-2i32, 0i32), KeyCode::ArrowLeft),
            ((2, 0), KeyCode::ArrowRight),
            ((0, 2), KeyCode::ArrowUp),
            ((0, -2), KeyCode::ArrowDown),
        ] {
            if keys.just_pressed(key) || (keys.pressed(key) && step_ok) {
                dx += mx;
                dy += my;
            }
        }
        if dx != 0 || dy != 0 {
            cursor.x = (cursor.x + dx).clamp(0, MAP_W as i32 - 1);
            cursor.y = (cursor.y + dy).clamp(0, MAP_H as i32 - 1);
        }
        if keys.just_pressed(KeyCode::KeyY) {
            legends.from = Screen::Embark;
            legends.scroll = 0;
            screen.0 = Screen::Legends;
            dirty.0 = true;
            return;
        }
        let mut enter_fort = |new_sim: Sim,
                              sim: &mut SimRes,
                              screen: &mut ScreenRes,
                              cursor: &mut Cursor,
                              view_z: &mut ViewZ,
                              dirty: &mut MapDirty,
                              camera: &mut Query<&mut Transform, With<Camera2d>>| {
            view_z.0 = new_sim
                .map
                .walk_surface_z(MAP_W / 2, MAP_H / 2)
                .unwrap_or(MAP_D / 2) as i32;
            sim.0 = Some(new_sim);
            cursor.x = MAP_W as i32 / 2;
            cursor.y = MAP_H as i32 / 2;
            // Undo any embark-screen panning/zooming: center on the fort.
            if let Ok(mut tf) = camera.single_mut() {
                tf.translation.x = MAP_W as f32 * TILE * 0.5;
                tf.translation.y = MAP_H as f32 * TILE * 0.5;
                tf.scale = Vec3::ONE;
            }
            screen.0 = Screen::Playing;
            dirty.0 = true;
        };
        if keys.just_pressed(KeyCode::Enter) {
            let region = ((cursor.x as usize / 2).min(OW - 1), (cursor.y as usize / 2).min(OW - 1));
            if world.0.overworld.get(region.0, region.1).biome.embarkable() {
                // A fortress retired to this region is reclaimed as it was;
                // otherwise a new colony is founded here.
                let reclaimed = fort_path(region).exists().then(|| {
                    load_sim(&fort_path(region), &reg.0)
                        .map_err(|e| error!("reclaim failed: {e:#}"))
                        .ok()
                        .filter(|s| {
                            (s.map.width, s.map.height, s.map.depth) == (MAP_W, MAP_H, MAP_D)
                        })
                });
                let new_sim = match reclaimed {
                    Some(Some(mut loaded)) => {
                        info!("reclaimed the retired fortress at {region:?}");
                        loaded.home_region = Some(region);
                        loaded
                    }
                    _ => embark(&world.0, &reg.0, region),
                };
                enter_fort(new_sim, &mut sim, &mut screen, &mut cursor, &mut view_z, &mut dirty, &mut camera);
            }
        }
        // 'a': walk this world as a lone adventurer instead.
        if keys.just_pressed(KeyCode::KeyA) {
            let region = ((cursor.x as usize / 2).min(OW - 1), (cursor.y as usize / 2).min(OW - 1));
            if world.0.overworld.get(region.0, region.1).biome.embarkable() {
                let mut new_sim = embark(&world.0, &reg.0, region);
                if new_sim.begin_adventure(&reg.0).is_some() {
                    new_sim.adv_region = Some(region);
                    enter_fort(new_sim, &mut sim, &mut screen, &mut cursor, &mut view_z, &mut dirty, &mut camera);
                    screen.0 = Screen::Adventure;
                }
            }
        }
        // Continue a saved fortress straight from the embark screen.
        if keys.just_pressed(KeyCode::F9) {
            match load_sim(&save_path(), &reg.0) {
                Ok(loaded)
                    if (loaded.map.width, loaded.map.height, loaded.map.depth)
                        != (MAP_W, MAP_H, MAP_D) =>
                {
                    error!("load failed: save has different map dimensions");
                }
                Ok(loaded) => {
                    info!("loaded saved fortress from {}", save_path().display());
                    enter_fort(loaded, &mut sim, &mut screen, &mut cursor, &mut view_z, &mut dirty, &mut camera);
                }
                Err(e) => error!("load failed: {e:#}"),
            }
        }
        // 'n': forge a whole new world — a fresh random seed, so its lands,
        // peoples, and eight decades of history are unlike this one.
        if keys.just_pressed(KeyCode::KeyN) {
            let seed = fresh_world_seed();
            persist_world_seed(seed);
            world.0 = World::generate(seed, OW, OW, HISTORY_YEARS);
            cursor.x = 0;
            cursor.y = 0;
            dirty.0 = true;
            info!("forged a new world (seed {seed})");
        }
        if keys.just_pressed(KeyCode::KeyQ) {
            exit.write(AppExit::Success);
        }
        return;
    }

    // ---- Adventure: turn-based control of a single hero.
    if screen.0 == Screen::Adventure {
        let mut acted: Option<PlayerAction> = None;
        for (key, action) in [
            (KeyCode::ArrowLeft, PlayerAction::Move(-1, 0)),
            (KeyCode::ArrowRight, PlayerAction::Move(1, 0)),
            (KeyCode::ArrowUp, PlayerAction::Move(0, 1)),
            (KeyCode::ArrowDown, PlayerAction::Move(0, -1)),
            (KeyCode::BracketRight, PlayerAction::Climb(1)),
            (KeyCode::BracketLeft, PlayerAction::Climb(-1)),
            (KeyCode::Period, PlayerAction::Wait),
            (KeyCode::KeyP, PlayerAction::Grab),
        ] {
            if keys.just_pressed(key) {
                acted = Some(action);
                break;
            }
        }
        if let (Some(action), Some(sim_inner)) = (acted, sim.0.as_mut()) {
            sim_inner.player_step(action, &reg.0);
            // The view follows the hero.
            if let Some(hero) = sim_inner.player {
                let p = sim_inner.dwarves[hero].pos;
                view_z.0 = p.z;
                cursor.x = p.x;
                cursor.y = p.y;
                if let Ok(mut tf) = camera.single_mut() {
                    tf.translation.x = p.x as f32 * TILE;
                    tf.translation.y = p.y as f32 * TILE;
                }
            }
            dirty.0 = true;
        }
        // 'c': recruit an adjacent townsfolk as a travelling companion.
        if keys.just_pressed(KeyCode::KeyC) {
            if let Some(sim_inner) = sim.0.as_mut() {
                sim_inner.recruit_companion();
                dirty.0 = true;
            }
            return;
        }
        // 'g': journey to the next land — the nearest embarkable region in
        // whichever direction the cursor was last nudged (default: east).
        if keys.just_pressed(KeyCode::KeyG) {
            if let Some(sim_inner) = sim.0.as_mut() {
                if let Some((rx, ry)) = sim_inner.adv_region {
                    // Try the four neighbors, preferring one that's land.
                    let neighbors = [
                        (rx + 1, ry),
                        (rx.wrapping_sub(1), ry),
                        (rx, ry + 1),
                        (rx, ry.wrapping_sub(1)),
                    ];
                    let dest = neighbors.into_iter().find(|&(nx, ny)| {
                        nx < OW && ny < OW && world.0.overworld.get(nx, ny).biome.embarkable()
                    });
                    if let Some(dest) = dest {
                        let new_map = region_map(&world.0, &reg.0, dest);
                        sim_inner.relocate_player(new_map, &reg.0);
                        sim_inner.adv_region = Some(dest);
                        if let Some(hero) = sim_inner.player {
                            let p = sim_inner.dwarves[hero].pos;
                            view_z.0 = p.z;
                            cursor.x = p.x;
                            cursor.y = p.y;
                            if let Ok(mut tf) = camera.single_mut() {
                                tf.translation.x = p.x as f32 * TILE;
                                tf.translation.y = p.y as f32 * TILE;
                            }
                        }
                        dirty.0 = true;
                    }
                }
            }
            return;
        }
        if keys.just_pressed(KeyCode::KeyY) {
            legends.from = Screen::Adventure;
            legends.scroll = 0;
            screen.0 = Screen::Legends;
            dirty.0 = true;
            return;
        }
        if keys.just_pressed(KeyCode::Escape) {
            // The saga ends: inscribe the hero's deeds into the world's
            // annals so they live on in Legends alongside the ancient feats.
            if let Some(s) = sim.0.as_ref() {
                let year = world.0.years_simulated + s.clock.year() as u32;
                for deed in &s.deeds {
                    world.0.record_deed(year, deed.clone());
                }
            }
            // Back to the world map; the adventure ends.
            sim.0 = None;
            screen.0 = Screen::Embark;
            if let Ok(mut tf) = camera.single_mut() {
                tf.translation.x = MAP_W as f32 * TILE * 0.5;
                tf.translation.y = MAP_H as f32 * TILE * 0.5;
                tf.scale = Vec3::ONE;
            }
            dirty.0 = true;
            return;
        }
        if keys.just_pressed(KeyCode::KeyQ) {
            exit.write(AppExit::Success);
        }
        return;
    }

    // ---- Playing.
    // F8: retire the fortress. It is preserved in its own file, keyed to its
    // region, so it endures in the world and can be reclaimed by embarking
    // there again. Then return to the overworld.
    if keys.just_pressed(KeyCode::F8) {
        if let Some(s) = sim.0.as_ref() {
            match s.home_region {
                Some(region) => {
                    match save_sim(s, &fort_path(region), &reg.0) {
                        Ok(()) => info!("the fortress at {region:?} is retired to history"),
                        Err(e) => error!("retire failed: {e:#}"),
                    }
                    sim.0 = None;
                    screen.0 = Screen::Embark;
                    if let Ok(mut tf) = camera.single_mut() {
                        tf.translation.x = MAP_W as f32 * TILE * 0.5;
                        tf.translation.y = MAP_H as f32 * TILE * 0.5;
                        tf.scale = Vec3::ONE;
                    }
                    dirty.0 = true;
                }
                None => error!("this fortress has no home region; cannot retire"),
            }
        }
        return;
    }
    if keys.just_pressed(KeyCode::KeyY) {
        legends.from = Screen::Playing;
        legends.scroll = 0;
        screen.0 = Screen::Legends;
        dirty.0 = true;
        return;
    }
    let Some(sim_inner) = sim.0.as_mut() else { return };
    // The rest of this system was written against `sim.0.<field>` when
    // SimRes held a bare Sim; keep that shape via a local binding.
    let mut sim = SimMut(sim_inner);

    // Cursor movement with hold-to-repeat.
    repeat.0.tick(time.delta());
    let step_ok = repeat.0.just_finished();
    let mut dx = 0i32;
    let mut dy = 0i32;
    for ((mx, my), key) in [
        ((-1i32, 0i32), KeyCode::ArrowLeft),
        ((1, 0), KeyCode::ArrowRight),
        ((0, 1), KeyCode::ArrowUp),
        ((0, -1), KeyCode::ArrowDown),
    ] {
        if keys.just_pressed(key) || (keys.pressed(key) && step_ok) {
            dx += mx;
            dy += my;
        }
    }
    if dx != 0 || dy != 0 {
        cursor.x = (cursor.x + dx).clamp(0, MAP_W as i32 - 1);
        cursor.y = (cursor.y + dy).clamp(0, MAP_H as i32 - 1);
        if mode.0.is_some() {
            dirty.0 = true;
        }
    }

    // Z-level.
    if keys.just_pressed(KeyCode::BracketLeft) && view_z.0 > 0 {
        view_z.0 -= 1;
        dirty.0 = true;
    }
    if keys.just_pressed(KeyCode::BracketRight) && view_z.0 < MAP_D as i32 - 1 {
        view_z.0 += 1;
        dirty.0 = true;
    }

    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    // Rectangle modes. First press anchors, second press applies. Shift is
    // reserved for shifted commands (Shift+F forge, Shift+U war-dog), so a
    // shifted press never triggers these plain designations.
    for (key, kind) in [
        (KeyCode::KeyD, UiKind::Mine),
        (KeyCode::KeyX, UiKind::Stairs),
        (KeyCode::KeyP, UiKind::Stockpile),
        (KeyCode::KeyF, UiKind::Farm),
        (KeyCode::KeyN, UiKind::Pasture),
        (KeyCode::KeyO, UiKind::Tavern),
        (KeyCode::Quote, UiKind::Temple),
        (KeyCode::KeyZ, UiKind::Fishery),
        (KeyCode::KeyH, UiKind::Channel),
        (KeyCode::KeyC, UiKind::Cancel),
    ] {
        if !keys.just_pressed(key) || shift {
            continue;
        }
        let here = cursor.pos(view_z.0);
        match mode.0 {
            Some((active, anchor)) if active == kind && anchor.z == here.z => {
                match kind {
                    UiKind::Mine => {
                        sim.0.designate_rect(DesignationKind::Mine, anchor, here);
                    }
                    UiKind::Stairs => {
                        sim.0.designate_rect(DesignationKind::Stairs, anchor, here);
                    }
                    UiKind::Channel => {
                        sim.0.designate_rect(DesignationKind::Channel, anchor, here);
                    }
                    UiKind::Stockpile => sim.0.add_stockpile(anchor, here),
                    UiKind::Farm => {
                        sim.0.add_farm(anchor, here, 0);
                    }
                    UiKind::Pasture => sim.0.add_pasture(anchor, here),
                    UiKind::Tavern => sim.0.add_tavern(anchor, here),
                    UiKind::Temple => sim.0.add_temple(anchor, here),
                    UiKind::Fishery => sim.0.add_fishery(anchor, here),
                    UiKind::Hospital => sim.0.add_hospital(anchor, here),
                    UiKind::Barracks => sim.0.add_barracks(anchor, here),
                    UiKind::Burrow => sim.0.add_burrow(anchor, here),
                    UiKind::Library => sim.0.add_library(anchor, here),
                    UiKind::Chop => {
                        sim.0.designate_rect(DesignationKind::Chop, anchor, here);
                    }
                    UiKind::Cancel => {
                        sim.0.cancel_rect(anchor, here);
                    }
                }
                mode.0 = None;
            }
            // Changed z since anchoring (rects are per z-level): re-anchor
            // here instead of silently dropping the designation.
            _ => mode.0 = Some((kind, here)),
        }
        dirty.0 = true;
    }
    if keys.just_pressed(KeyCode::Escape) && mode.0.is_some() {
        mode.0 = None;
        dirty.0 = true;
    }

    // Buildings: instant placement at the cursor.
    for (key, kind) in [
        (KeyCode::KeyV, BuildingKind::Still),
        (KeyCode::KeyK, BuildingKind::Kitchen),
        (KeyCode::KeyM, BuildingKind::Craftsdwarf),
        (KeyCode::KeyJ, BuildingKind::Loom),
        (KeyCode::Semicolon, BuildingKind::Jeweler),
        (KeyCode::KeyG, BuildingKind::Floodgate),
        (KeyCode::KeyB, BuildingKind::Tomb),
    ] {
        if keys.just_pressed(key) && !shift {
            let here = cursor.pos(view_z.0);
            if sim.0.add_building(kind, here) {
                info!("built a {} at {:?}", kind.name(), here);
            } else {
                warn!("can't build a {} there", kind.name());
            }
            dirty.0 = true;
        }
    }
    // Shift+X: designate trees for chopping (two-press rectangle; only tiles
    // with a tree are marked). Shift keeps it clear of the plain 'x' stairs.
    if shift && keys.just_pressed(KeyCode::KeyX) {
        let here = cursor.pos(view_z.0);
        match mode.0 {
            Some((UiKind::Chop, anchor)) if anchor.z == here.z => {
                sim.0.designate_rect(DesignationKind::Chop, anchor, here);
                mode.0 = None;
            }
            _ => mode.0 = Some((UiKind::Chop, here)),
        }
        dirty.0 = true;
    }
    // Shift+H: designate a hospital zone (two-press rectangle, like a tavern).
    if shift && keys.just_pressed(KeyCode::KeyH) {
        let here = cursor.pos(view_z.0);
        match mode.0 {
            Some((UiKind::Hospital, anchor)) if anchor.z == here.z => {
                sim.0.add_hospital(anchor, here);
                mode.0 = None;
            }
            _ => mode.0 = Some((UiKind::Hospital, here)),
        }
        dirty.0 = true;
    }
    // Shift+Z: designate a burrow (safe room civilians flee to on the alarm).
    if shift && keys.just_pressed(KeyCode::KeyZ) {
        let here = cursor.pos(view_z.0);
        match mode.0 {
            Some((UiKind::Burrow, anchor)) if anchor.z == here.z => {
                sim.0.add_burrow(anchor, here);
                mode.0 = None;
            }
            _ => mode.0 = Some((UiKind::Burrow, here)),
        }
        dirty.0 = true;
    }
    // F2: sound or lift the civilian alarm.
    if keys.just_pressed(KeyCode::F2) {
        let on = sim.0.toggle_alarm();
        info!("alarm {}", if on { "sounded" } else { "lifted" });
        dirty.0 = true;
    }
    // Shift+B: plan a constructed wall on the floor tile at the cursor.
    if shift && keys.just_pressed(KeyCode::KeyB) {
        let here = cursor.pos(view_z.0);
        if sim.0.designate_construction(here) {
            info!("planned a wall at {:?}", here);
        } else {
            warn!("can't build a wall there (needs open floor)");
        }
        dirty.0 = true;
    }
    // Shift+D: smooth & engrave the wall at the cursor.
    if shift && keys.just_pressed(KeyCode::KeyD) {
        let here = cursor.pos(view_z.0);
        if sim.0.designate_rect(DesignationKind::Smooth, here, here) > 0 {
            info!("marked a wall for engraving");
        } else {
            warn!("can't engrave there (needs a bare wall)");
        }
        dirty.0 = true;
    }
    // Shift+F: build a forge (Shift keeps it clear of the farm designation).
    if shift && keys.just_pressed(KeyCode::KeyF) {
        let here = cursor.pos(view_z.0);
        if sim.0.add_building(BuildingKind::Forge, here) {
            info!("built a Forge at {:?}", here);
        } else {
            warn!("can't build a Forge there");
        }
        dirty.0 = true;
    }
    // Shift+M: build a smelter (Shift keeps it clear of the craftsdwarf's shop).
    if shift && keys.just_pressed(KeyCode::KeyM) {
        let here = cursor.pos(view_z.0);
        if sim.0.add_building(BuildingKind::Smelter, here) {
            info!("built a Smelter at {:?}", here);
        } else {
            warn!("can't build a Smelter there");
        }
        dirty.0 = true;
    }
    // Shift+K: build a mason's workshop (Shift keeps it clear of the kitchen).
    if shift && keys.just_pressed(KeyCode::KeyK) {
        let here = cursor.pos(view_z.0);
        if sim.0.add_building(BuildingKind::Mason, here) {
            info!("built a Mason's Workshop at {:?}", here);
        } else {
            warn!("can't build a Mason's Workshop there");
        }
        dirty.0 = true;
    }
    // Shift+C: build a clothier's shop (Shift keeps it clear of designate-cancel).
    if shift && keys.just_pressed(KeyCode::KeyC) {
        let here = cursor.pos(view_z.0);
        if sim.0.add_building(BuildingKind::Clothier, here) {
            info!("built a Clothier's Shop at {:?}", here);
        } else {
            warn!("can't build a Clothier's Shop there");
        }
        dirty.0 = true;
    }
    // Shift+G: build a glass furnace (Shift keeps it clear of the floodgate).
    if shift && keys.just_pressed(KeyCode::KeyG) {
        let here = cursor.pos(view_z.0);
        if sim.0.add_building(BuildingKind::GlassFurnace, here) {
            info!("built a Glass Furnace at {:?}", here);
        } else {
            warn!("can't build a Glass Furnace there");
        }
        dirty.0 = true;
    }
    // Shift+T: lay a weapon trap on the floor for raiders to blunder into.
    if shift && keys.just_pressed(KeyCode::KeyT) {
        let here = cursor.pos(view_z.0);
        if sim.0.add_building(BuildingKind::Trap, here) {
            info!("laid a weapon trap at {:?}", here);
        } else {
            warn!("can't lay a trap there (needs open floor)");
        }
        dirty.0 = true;
    }
    // Shift+L designates a library (two-press rectangle); plain 'l' links a
    // lever to a floodgate.
    if shift && keys.just_pressed(KeyCode::KeyL) {
        let here = cursor.pos(view_z.0);
        match mode.0 {
            Some((UiKind::Library, anchor)) if anchor.z == here.z => {
                sim.0.add_library(anchor, here);
                mode.0 = None;
            }
            _ => mode.0 = Some((UiKind::Library, here)),
        }
        dirty.0 = true;
    } else if keys.just_pressed(KeyCode::KeyL) {
        let here = cursor.pos(view_z.0);
        match sim.0.add_lever(here) {
            Some(gate) => info!("lever placed, linked to floodgate at {:?}", gate),
            None => warn!("no floodgate to link (build one with 'g' first)"),
        }
        dirty.0 = true;
    }
    // 'i': enlist/dismiss the fort dwarf under the cursor as a soldier.
    // Shift+I instead designates a barracks (two-press rectangle) where idle
    // soldiers drill between battles.
    if shift && keys.just_pressed(KeyCode::KeyI) {
        let here = cursor.pos(view_z.0);
        match mode.0 {
            Some((UiKind::Barracks, anchor)) if anchor.z == here.z => {
                sim.0.add_barracks(anchor, here);
                mode.0 = None;
            }
            _ => mode.0 = Some((UiKind::Barracks, here)),
        }
        dirty.0 = true;
    } else if keys.just_pressed(KeyCode::KeyI) {
        let here = cursor.pos(view_z.0);
        match sim.0.toggle_soldier(here) {
            Some(true) => info!("enlisted a soldier"),
            Some(false) => info!("dismissed a soldier"),
            None => warn!("no citizen at the cursor to enlist"),
        }
        dirty.0 = true;
    }
    // 'u': cull the nearest animal for slaughter. Shift+U instead war-trains
    // the nearest dog into a fortress guardian.
    if keys.just_pressed(KeyCode::KeyU) {
        let here = cursor.pos(view_z.0);
        let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
        if shift {
            if sim.0.mark_nearest_for_war(here).is_none() {
                warn!("no untrained dog to war-train");
            }
        } else if sim.0.mark_nearest_animal(here).is_none() {
            warn!("no animal to slaughter");
        }
        dirty.0 = true;
    }
    if !shift && keys.just_pressed(KeyCode::KeyT) {
        let here = cursor.pos(view_z.0);
        if sim.0.pull_lever(here) {
            info!("lever pulled");
        } else {
            warn!("no lever at the cursor");
        }
        dirty.0 = true;
    }

    // Time controls.
    if keys.just_pressed(KeyCode::Space) {
        control.paused = !control.paused;
    }
    if keys.just_pressed(KeyCode::Period) && control.paused {
        sim.0.step(&reg.0);
        if sim.0.map_changed {
            sim.0.map_changed = false;
            dirty.0 = true;
        }
    }
    for (key, speed, hz) in [
        (KeyCode::Digit1, 1u8, 20.0),
        (KeyCode::Digit2, 2, 60.0),
        (KeyCode::Digit3, 3, 180.0),
    ] {
        if keys.just_pressed(key) {
            control.speed = speed;
            fixed_time.set_timestep_hz(hz);
        }
    }

    // Camera pan / zoom (keyboard; mouse also pans/zooms).
    if let Ok(mut tf) = camera.single_mut() {
        let pan = 300.0 * time.delta_secs() * tf.scale.x;
        if keys.pressed(KeyCode::KeyA) {
            tf.translation.x -= pan;
        }
        if keys.pressed(KeyCode::KeyS) {
            tf.translation.y -= pan;
        }
        if keys.pressed(KeyCode::KeyW) {
            tf.translation.y += pan;
        }
        if keys.pressed(KeyCode::KeyE) {
            tf.translation.x += pan;
        }
        if keys.just_pressed(KeyCode::Equal) {
            tf.scale = (tf.scale * 0.8).clamp(Vec3::splat(0.2), Vec3::splat(4.0));
        }
        if keys.just_pressed(KeyCode::Minus) {
            tf.scale = (tf.scale * 1.25).clamp(Vec3::splat(0.2), Vec3::splat(4.0));
        }
    }

    // Save / load.
    if keys.just_pressed(KeyCode::F5) {
        match save_sim(&sim.0, &save_path(), &reg.0) {
            Ok(()) => info!("world saved to {}", save_path().display()),
            Err(e) => error!("save failed: {e:#}"),
        }
    }
    if keys.just_pressed(KeyCode::F9) {
        match load_sim(&save_path(), &reg.0) {
            Ok(loaded)
                if (loaded.map.width, loaded.map.height, loaded.map.depth)
                    != (MAP_W, MAP_H, MAP_D) =>
            {
                error!(
                    "load failed: save is {}x{}x{}, this build expects {}x{}x{}",
                    loaded.map.width, loaded.map.height, loaded.map.depth, MAP_W, MAP_H, MAP_D
                );
            }
            Ok(loaded) => {
                *sim.0 = loaded;
                cursor.x = cursor.x.min(MAP_W as i32 - 1);
                cursor.y = cursor.y.min(MAP_H as i32 - 1);
                view_z.0 = view_z.0.min(MAP_D as i32 - 1);
                dirty.0 = true;
                info!("world loaded from {}", save_path().display());
            }
            Err(e) => error!("load failed: {e:#}"),
        }
    }

    if keys.just_pressed(KeyCode::KeyQ) {
        exit.write(AppExit::Success);
    }
}

/// Trade screen input — its own system to keep handle_input under Bevy's
/// system-param limit.
fn handle_trade_input(
    keys: Res<ButtonInput<KeyCode>>,
    reg: Res<Registry>,
    mut screen: ResMut<ScreenRes>,
    mut trade: ResMut<TradeState>,
    mut sim: ResMut<SimRes>,
    mut dirty: ResMut<MapDirty>,
) {
    // 'r' during play opens the negotiation (when a caravan is visiting).
    if screen.0 == Screen::Playing
        && keys.just_pressed(KeyCode::KeyR)
        && sim.0.as_ref().is_some_and(|s| s.caravan.is_some())
    {
        *trade = TradeState::default(); // a fresh deal every visit
        screen.0 = Screen::Trade;
        dirty.0 = true;
        return;
    }
    if screen.0 != Screen::Trade {
        return;
    }
    let Some(sim_inner) = sim.0.as_mut() else {
        *trade = TradeState::default();
        screen.0 = Screen::Playing;
        return;
    };
    // Caravan left mid-negotiation? Abandon the deal entirely — stale
    // selections must never carry into the next caravan's visit.
    let Some(caravan_len) = sim_inner.caravan.as_ref().map(|c| c.goods.len()) else {
        *trade = TradeState::default();
        screen.0 = Screen::Playing;
        dirty.0 = true;
        return;
    };
    let yours = tradeable_items(sim_inner);
    // The fort keeps living while you haggle: items get eaten, hauled, and
    // reserved under you. Prune dead selections and keep the cursor in
    // bounds every frame, or Space can index past the end and panic.
    let yours_set: std::collections::BTreeSet<usize> = yours.iter().copied().collect();
    trade.offer.retain(|i| yours_set.contains(i));
    trade.request.retain(|&g| g < caravan_len);
    let col_len = if trade.side == 0 { caravan_len } else { yours.len() };
    trade.cursor = trade.cursor.min(col_len.saturating_sub(1));

    if keys.just_pressed(KeyCode::Tab)
        || keys.just_pressed(KeyCode::ArrowLeft)
        || keys.just_pressed(KeyCode::ArrowRight)
    {
        trade.side = 1 - trade.side;
        trade.cursor = 0;
    }
    if keys.just_pressed(KeyCode::ArrowDown) && col_len > 0 {
        trade.cursor = (trade.cursor + 1).min(col_len - 1);
    }
    if keys.just_pressed(KeyCode::ArrowUp) {
        trade.cursor = trade.cursor.saturating_sub(1);
    }
    if keys.just_pressed(KeyCode::Space) && col_len > 0 {
        if trade.side == 0 {
            let g = trade.cursor;
            if !trade.request.remove(&g) {
                trade.request.insert(g);
            }
        } else {
            let i = yours[trade.cursor];
            if !trade.offer.remove(&i) {
                trade.offer.insert(i);
            }
        }
        trade.message.clear();
    }
    if keys.just_pressed(KeyCode::Enter) {
        let offer: Vec<usize> = trade.offer.iter().copied().collect();
        let request: Vec<usize> = trade.request.iter().copied().collect();
        match sim_inner.execute_trade(&offer, &request, &reg.0) {
            Ok(()) => {
                trade.offer.clear();
                trade.request.clear();
                trade.cursor = 0;
                trade.message = "The merchants shake on it.".to_string();
                dirty.0 = true;
            }
            Err(e) => trade.message = e,
        }
    }
    if keys.just_pressed(KeyCode::Escape) || keys.just_pressed(KeyCode::KeyR) {
        trade.offer.clear();
        trade.request.clear();
        trade.message.clear();
        screen.0 = Screen::Playing;
        dirty.0 = true;
    }
}

/// Play sounds for new fort-log events. The log is the game's narrator;
/// this system is its voice.
fn play_event_sounds(
    mut commands: Commands,
    sim: Res<SimRes>,
    bank: Res<SoundBank>,
    sources: Res<Assets<AudioSource>>,
    mut cursor: ResMut<LogCursor>,
) {
    let Some(sim) = sim.0.as_ref() else {
        cursor.0 = 0;
        return;
    };
    if bank.0.is_empty() {
        return;
    }
    let len = sim.log.len();
    if len < cursor.0 {
        // A new fort or a loaded save: don't replay history.
        cursor.0 = len;
        return;
    }
    let mut played = 0;
    for (_, msg) in sim.log.iter().skip(cursor.0) {
        if played >= 2 {
            break; // never a wall of noise in one frame
        }
        let key = if msg.contains("strikes") {
            Some("hit")
        } else if msg.contains("raiding party") {
            Some("horn")
        } else if msg.contains("strange mood") || msg.contains("has created") {
            Some("chime")
        } else if msg.contains("caravan") && msg.contains("arrived") {
            Some("bell")
        } else if msg.contains("laid to rest") || msg.contains("at peace") {
            Some("toll")
        } else if msg.contains("obsidian") {
            Some("hiss")
        } else if msg.contains("drowned")
            || msg.contains("incinerated")
            || msg.contains("falls dead")
            || msg.contains("bled out")
        {
            Some("doom")
        } else if msg.contains("elevated to baron") {
            Some("fanfare")
        } else {
            None
        };
        // Only play a sound whose asset has finished loading — decoding a
        // not-yet-loaded AudioSource panics deep inside bevy_audio (rodio's
        // Decoder::new(..).unwrap()). Skipping silently is the safe choice.
        if let Some(handle) = key.and_then(|k| bank.0.get(k)) {
            if sources.get(handle).is_some() {
                commands.spawn((
                    AudioPlayer::new(handle.clone()),
                    PlaybackSettings::DESPAWN,
                ));
                played += 1;
            }
        }
    }
    cursor.0 = len;
}

fn mix(base: [f32; 3], tint: [f32; 3], k: f32) -> [f32; 3] {
    [
        base[0] * (1.0 - k) + tint[0] * k,
        base[1] * (1.0 - k) + tint[1] * k,
        base[2] * (1.0 - k) + tint[2] * k,
    ]
}

/// Color and glyph for a map position as seen from `view_z`.
fn tile_visual(
    sim: &Sim,
    raws: &Raws,
    x: i32,
    y: i32,
    view_z: i32,
    selection: Option<(Pos, Pos)>,
) -> (Color, &'static str) {
    const DIM: [f32; 4] = [1.0, 0.55, 0.34, 0.20];
    let mut rgb = [0.02, 0.02, 0.03];
    let mut glyph = "block";
    for (levels_down, factor) in DIM.iter().enumerate() {
        let z = view_z - levels_down as i32;
        if z < 0 {
            break;
        }
        let p = Pos::new(x, y, z);
        let Some(tile) = sim.map.tile_at(p) else { break };
        if tile.shape == TileShape::Empty {
            continue;
        }
        let [r, g, b] = raws.materials.get(tile.material).color;
        let shade = match tile.shape {
            TileShape::Solid | TileShape::Gate => 1.0,
            TileShape::Ramp => 0.8,
            TileShape::Stairs => 0.7,
            TileShape::Floor => 0.55,
            TileShape::Empty => unreachable!(),
        };
        glyph = match tile.shape {
            TileShape::Solid => "wall",
            TileShape::Gate => "gate",
            TileShape::Ramp => "ramp",
            TileShape::Stairs => "stairs",
            TileShape::Floor => "floor",
            TileShape::Empty => unreachable!(),
        };
        rgb = [
            r as f32 / 255.0 * factor * shade,
            g as f32 / 255.0 * factor * shade,
            b as f32 / 255.0 * factor * shade,
        ];
        break;
    }

    let here = Pos::new(x, y, view_z);
    if let Some(farm) = sim.farms.get(&here) {
        let (tint, k) = match farm.state {
            FarmState::Fallow => ([0.3, 0.4, 0.18], 0.4),
            FarmState::Growing { .. } => ([0.25, 0.6, 0.2], 0.45),
            FarmState::Grown => ([0.45, 0.9, 0.3], 0.55),
        };
        rgb = mix(rgb, tint, k);
        glyph = "farm";
    }
    if sim.tree_at(here) {
        // A tree standing on the surface — leafy green, or amber once a
        // woodcutter has marked it to be felled.
        let marked = matches!(
            sim.designations.get(&here).map(|d| d.kind),
            Some(DesignationKind::Chop)
        );
        let tint = if marked { [0.85, 0.5, 0.12] } else { [0.16, 0.5, 0.14] };
        rgb = mix(rgb, tint, 0.75);
        glyph = "crop";
    }
    if let Some(b) = sim.building_at(here) {
        let tint = match b.kind {
            BuildingKind::Still => [0.85, 0.5, 0.22],
            BuildingKind::Kitchen => [0.8, 0.25, 0.2],
            BuildingKind::Floodgate => [0.55, 0.55, 0.6],
            BuildingKind::Lever { .. } => [0.9, 0.85, 0.3],
            BuildingKind::Tomb => [0.6, 0.55, 0.75],
            BuildingKind::Craftsdwarf => [0.7, 0.6, 0.35],
            BuildingKind::Loom => [0.55, 0.7, 0.72],
            BuildingKind::Jeweler => [0.75, 0.55, 0.85],
            BuildingKind::Forge => [0.95, 0.45, 0.2],
            BuildingKind::Smelter => [0.8, 0.32, 0.12],
            BuildingKind::Mason => [0.62, 0.6, 0.55],
            BuildingKind::Clothier => [0.7, 0.6, 0.8],
            BuildingKind::GlassFurnace => [0.5, 0.85, 0.85],
            BuildingKind::Trap => [0.85, 0.2, 0.2],
        };
        rgb = mix(rgb, tint, 0.6);
        glyph = match b.kind {
            BuildingKind::Still => "still",
            BuildingKind::Kitchen => "kitchen",
            BuildingKind::Floodgate => "gate",
            BuildingKind::Lever { .. } => "lever",
            BuildingKind::Tomb => "tomb",
            BuildingKind::Craftsdwarf => "artifact",
            BuildingKind::Loom => "still",
            BuildingKind::Jeweler => "artifact",
            BuildingKind::Forge => "weapon",
            BuildingKind::Smelter => "still",
            BuildingKind::Mason => "artifact",
            BuildingKind::Clothier => "still",
            BuildingKind::GlassFurnace => "still",
            BuildingKind::Trap => "weapon",
        };
    }
    let water = sim.map.water_at(here);
    if water > 0 {
        let k = 0.25 + 0.08 * water as f32;
        rgb = mix(rgb, [0.15, 0.35, 0.9], k.min(0.85));
    }
    let magma = sim.map.magma_at(here);
    if magma > 0 {
        let k = 0.5 + 0.06 * magma as f32;
        rgb = mix(rgb, [1.0, 0.32, 0.02], k.min(0.95));
    }
    if sim.designations.contains_key(&here) {
        rgb = mix(rgb, [1.0, 0.62, 0.12], 0.45);
    }
    // An engraved wall catches a soft golden sheen.
    if sim.engravings.contains_key(&here) {
        rgb = mix(rgb, [0.95, 0.85, 0.5], 0.35);
    }
    // A planned wall is outlined in slate blue until it's raised.
    if sim.constructions.contains_key(&here) {
        rgb = mix(rgb, [0.4, 0.55, 0.7], 0.5);
    }
    if sim.stockpile_at(here).is_some() {
        rgb = mix(rgb, [0.25, 0.45, 0.9], 0.35);
    }
    if sim.pastures.iter().any(|p| p.contains(here)) {
        rgb = mix(rgb, [0.35, 0.6, 0.25], 0.3);
    }
    if sim.tavern_at(here) {
        rgb = mix(rgb, [0.7, 0.45, 0.75], 0.3);
    }
    if sim.fishery_at(here) {
        rgb = mix(rgb, [0.2, 0.7, 0.7], 0.28);
    }
    if sim.temple_at(here) {
        rgb = mix(rgb, [0.85, 0.8, 0.5], 0.28);
    }
    if sim.hospital_at(here) {
        rgb = mix(rgb, [0.9, 0.35, 0.35], 0.28);
    }
    if sim.barracks_at(here) {
        rgb = mix(rgb, [0.55, 0.5, 0.4], 0.32);
    }
    if sim.burrow_at(here) {
        rgb = mix(rgb, [0.4, 0.7, 0.55], 0.3);
    }
    if sim.library_at(here) {
        rgb = mix(rgb, [0.7, 0.6, 0.9], 0.3);
    }
    if let Some((a, b)) = selection {
        if view_z == a.z
            && x >= a.x.min(b.x)
            && x <= a.x.max(b.x)
            && y >= a.y.min(b.y)
            && y <= a.y.max(b.y)
        {
            rgb = mix(rgb, [0.3, 1.0, 0.4], 0.35);
        }
    }
    // The sky's mood washes over the whole map.
    match sim.weather {
        dk_agents::Weather::Rain => rgb = mix(rgb, [0.35, 0.42, 0.55], 0.18),
        dk_agents::Weather::Snow => rgb = mix(rgb, [0.85, 0.88, 0.95], 0.22),
        dk_agents::Weather::Clear => {}
    }
    (Color::srgb(rgb[0], rgb[1], rgb[2]), glyph)
}

fn redraw_tiles(
    mut dirty: ResMut<MapDirty>,
    sim: Res<SimRes>,
    reg: Res<Registry>,
    world: Res<WorldRes>,
    screen: Res<ScreenRes>,
    tileset: Res<Tileset>,
    view_z: Res<ViewZ>,
    cursor: Res<Cursor>,
    mode: Res<UiMode>,
    mut tiles: Query<(&TileSprite, &mut Sprite)>,
) {
    if !dirty.0 {
        return;
    }
    dirty.0 = false;
    if screen.0 == Screen::Embark {
        // The 48x48 overworld fills the 96x96 grid at 2x scale.
        for (t, mut sprite) in &mut tiles {
            let (rx, ry) = (t.x / 2, t.y / 2);
            let region = world.0.overworld.get(rx.min(OW - 1), ry.min(OW - 1));
            let [r, g, b] = region.biome.color();
            let mut rgb = [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0];
            let mut glyph = "block";
            // Mark civilization sites.
            if world.0.sites.iter().any(|st| st.region == (rx, ry) && !st.ruined) {
                rgb = [0.95, 0.9, 0.5];
                glyph = "artifact";
            }
            if let Some(handles) = &tileset.0 {
                if let Some(atlas) = sprite.texture_atlas.as_mut() {
                    atlas.index = handles.index(glyph);
                }
                sprite.color = if glyph == "artifact" {
                    Color::WHITE
                } else {
                    Color::srgb(rgb[0], rgb[1], rgb[2])
                };
            } else {
                sprite.color = Color::srgb(rgb[0], rgb[1], rgb[2]);
            }
        }
        return;
    }
    let Some(sim) = sim.0.as_ref() else { return };
    let selection = mode.0.map(|(_, anchor)| (anchor, cursor.pos(view_z.0)));
    for (t, mut sprite) in &mut tiles {
        let (color, glyph) =
            tile_visual(sim, &reg.0, t.x as i32, t.y as i32, view_z.0, selection);
        if let Some(handles) = &tileset.0 {
            if let Some(atlas) = sprite.texture_atlas.as_mut() {
                atlas.index = handles.index(glyph);
            }
            // Grayscale glyphs take the material color; full-color art
            // renders as painted.
            sprite.color = if handles.is_tinted(glyph) { color } else { Color::WHITE };
        } else {
            sprite.color = color;
        }
    }
}

/// Fort items eligible for trade, in a stable display order.
fn tradeable_items(sim: &Sim) -> Vec<usize> {
    sim.items
        .iter()
        .enumerate()
        .filter(|(_, it)| {
            it.active()
                && it.kind != ItemKind::Corpse // the dead are not for sale
                && it.reserved_by.is_none()
                && matches!(it.state, ItemState::Stored { .. } | ItemState::OnGround)
        })
        .map(|(i, _)| i)
        .collect()
}

fn item_label(raws: &Raws, it: &dk_agents::Item) -> String {
    let base = match it.kind {
        ItemKind::Boulder => format!("{} boulder", raws.materials.get(it.stuff).name),
        ItemKind::Seed => format!("{} seeds", raws.plants.get(it.stuff).name),
        ItemKind::Crop => raws.plants.get(it.stuff).name.clone(),
        ItemKind::Meal => "prepared meal".to_string(),
        ItemKind::Drink => "mug of drink".to_string(),
        ItemKind::Artifact => it.name.clone().unwrap_or_else(|| "artifact".to_string()),
        ItemKind::Corpse => it.name.clone().unwrap_or_else(|| "remains".to_string()),
        ItemKind::Craft => format!("{} craft", raws.materials.get(it.stuff).name),
        ItemKind::Wool => "raw wool".to_string(),
        ItemKind::Cloth => "bolt of cloth".to_string(),
        ItemKind::RoughGem => format!("rough {}", dk_agents::gem_name(it.stuff)),
        ItemKind::CutGem => format!("cut {}", dk_agents::gem_name(it.stuff)),
        ItemKind::Weapon => format!("{} weapon", raws.materials.get(it.stuff).name),
        ItemKind::Glass => "blown glass".to_string(),
        ItemKind::Bar => format!("{} bar", raws.materials.get(it.stuff).name),
        ItemKind::Armor => format!("{} armor", raws.materials.get(it.stuff).name),
        ItemKind::Bed => format!("{} bed", raws.materials.get(it.stuff).name),
        ItemKind::Clothes => "set of clothes".to_string(),
        ItemKind::Log => "wooden log".to_string(),
    };
    // A crafted good wears its quality; an artifact's name already says it.
    if it.quality > 0 && it.kind != ItemKind::Artifact {
        format!("{} {base}", dk_agents::quality_name(it.quality))
    } else {
        base
    }
}

fn item_color(raws: &Raws, kind: ItemKind, stuff: u16) -> Color {
    let lighten = |c: [u8; 3]| {
        let l = |v: u8| (v as f32 / 255.0 * 1.3).min(1.0);
        Color::srgb(l(c[0]), l(c[1]), l(c[2]))
    };
    match kind {
        ItemKind::Boulder => lighten(raws.materials.get(stuff).color),
        ItemKind::Seed | ItemKind::Crop => lighten(raws.plants.get(stuff).color),
        ItemKind::Meal => Color::srgb(0.9, 0.62, 0.3),
        ItemKind::Drink => Color::srgb(0.78, 0.55, 0.16),
        ItemKind::Artifact => Color::srgb(1.0, 0.85, 0.25),
        ItemKind::Corpse => Color::srgb(0.75, 0.8, 0.9),
        ItemKind::Craft => item_material_color(raws, stuff),
        ItemKind::Wool => Color::srgb(0.92, 0.9, 0.82),
        ItemKind::Cloth => Color::srgb(0.6, 0.55, 0.85),
        ItemKind::RoughGem | ItemKind::CutGem => {
            let [r, g, b] = dk_agents::gem_color(stuff);
            let l = |v: u8| (v as f32 / 255.0).min(1.0);
            Color::srgb(l(r), l(g), l(b))
        }
        ItemKind::Weapon => Color::srgb(0.8, 0.82, 0.88),
        ItemKind::Glass => Color::srgb(0.6, 0.9, 0.88),
        ItemKind::Bar => Color::srgb(0.72, 0.74, 0.8),
        ItemKind::Armor => Color::srgb(0.62, 0.66, 0.78),
        ItemKind::Bed => item_material_color(raws, stuff),
        ItemKind::Clothes => Color::srgb(0.85, 0.5, 0.7),
        ItemKind::Log => Color::srgb(0.5, 0.35, 0.18),
    }
}

/// A lightened material color, for crafts and boulders.
fn item_material_color(raws: &Raws, stuff: u16) -> Color {
    let [r, g, b] = raws.materials.get(stuff).color;
    let l = |v: u8| (v as f32 / 255.0 * 1.3).min(1.0);
    Color::srgb(l(r), l(g), l(b))
}

fn sync_agent_sprites(
    mut commands: Commands,
    sim: Res<SimRes>,
    reg: Res<Registry>,
    screen: Res<ScreenRes>,
    tileset: Res<Tileset>,
    view_z: Res<ViewZ>,
    mut pools: ResMut<SpritePools>,
    mut sprites: Query<
        (&mut Transform, &mut Sprite, &mut Visibility),
        (Without<TileSprite>, Without<CursorSprite>),
    >,
) {
    // On the embark map there are no creatures to draw.
    if screen.0 == Screen::Embark {
        for &e in pools.dwarves.iter().chain(pools.items.iter()) {
            if let Ok((_, _, mut vis)) = sprites.get_mut(e) {
                *vis = Visibility::Hidden;
            }
        }
        return;
    }
    let Some(sim) = sim.0.as_ref() else { return };
    let sim = SimRef(sim);
    while pools.dwarves.len() < sim.0.dwarves.len() {
        let sprite = match &tileset.0 {
            Some(ts) => ts.sprite("dwarf"),
            None => Sprite {
                color: Color::srgb(0.93, 0.79, 0.55),
                custom_size: Some(Vec2::splat(TILE * 0.72)),
                ..default()
            },
        };
        pools.dwarves.push(
            commands
                .spawn((sprite, Transform::from_xyz(0.0, 0.0, 2.0), Visibility::Hidden))
                .id(),
        );
    }
    while pools.items.len() < sim.0.items.len() {
        let sprite = match &tileset.0 {
            Some(ts) => {
                let mut sp = ts.sprite("boulder");
                sp.custom_size = Some(Vec2::splat(TILE * 0.8));
                sp
            }
            None => Sprite {
                color: Color::WHITE,
                custom_size: Some(Vec2::splat(TILE * 0.4)),
                ..default()
            },
        };
        pools.items.push(
            commands
                .spawn((sprite, Transform::from_xyz(0.0, 0.0, 1.5), Visibility::Hidden))
                .id(),
        );
    }
    while pools.animals.len() < sim.0.animals.len() {
        let sprite = match &tileset.0 {
            Some(ts) => ts.sprite("cow"),
            None => Sprite {
                color: Color::srgb(0.8, 0.7, 0.55),
                custom_size: Some(Vec2::splat(TILE * 0.7)),
                ..default()
            },
        };
        pools.animals.push(
            commands
                .spawn((sprite, Transform::from_xyz(0.0, 0.0, 1.8), Visibility::Hidden))
                .id(),
        );
    }

    for (i, &e) in pools.dwarves.iter().enumerate() {
        let Ok((mut tf, mut sprite, mut vis)) = sprites.get_mut(e) else { continue };
        match sim.0.dwarves.get(i) {
            Some(d) if d.ghost && d.pos.z == view_z.0 => {
                tf.translation.x = d.pos.x as f32 * TILE;
                tf.translation.y = d.pos.y as f32 * TILE;
                if let Some(ts) = &tileset.0 {
                    if let Some(atlas) = sprite.texture_atlas.as_mut() {
                        atlas.index = ts.index("dwarf");
                    }
                }
                sprite.color = Color::srgba(0.8, 0.9, 1.0, 0.45);
                *vis = Visibility::Visible;
            }
            Some(d) if d.alive && d.pos.z == view_z.0 => {
                tf.translation.x = d.pos.x as f32 * TILE;
                tf.translation.y = d.pos.y as f32 * TILE;
                tf.scale = Vec3::splat(if d.beast { 1.6 } else { 1.0 });
                match &tileset.0 {
                    Some(ts) => {
                        // Full-color figures; the glyph carries the faction.
                        if let Some(atlas) = sprite.texture_atlas.as_mut() {
                            atlas.index = ts.index(match d.faction {
                                Faction::Fort | Faction::Visitor => "dwarf",
                                Faction::Hostile => "raider",
                            });
                        }
                        // Traders wear the road's gold dust; beasts loom dark.
                        sprite.color = if d.beast {
                            Color::srgb(0.5, 0.1, 0.15)
                        } else if d.soldier {
                            Color::srgb(0.7, 0.8, 1.0) // steel sheen
                        } else {
                            match d.faction {
                                Faction::Visitor => Color::srgb(1.0, 0.85, 0.55),
                                _ => Color::WHITE,
                            }
                        };
                    }
                    None => {
                        sprite.color = match d.faction {
                            Faction::Fort => Color::srgb(0.93, 0.79, 0.55),
                            Faction::Hostile => Color::srgb(0.85, 0.25, 0.25),
                            Faction::Visitor => Color::srgb(0.95, 0.85, 0.4),
                        };
                    }
                }
                *vis = Visibility::Visible;
            }
            _ => *vis = Visibility::Hidden,
        }
    }
    for (i, &e) in pools.items.iter().enumerate() {
        let Ok((mut tf, mut sprite, mut vis)) = sprites.get_mut(e) else { continue };
        match sim.0.items.get(i) {
            Some(it)
                if it.active()
                    && it.pos.z == view_z.0
                    && !matches!(it.state, ItemState::Carried { .. }) =>
            {
                tf.translation.x = it.pos.x as f32 * TILE;
                tf.translation.y = it.pos.y as f32 * TILE;
                match &tileset.0 {
                    Some(ts) => {
                        let glyph = match it.kind {
                            ItemKind::Boulder => "boulder",
                            ItemKind::Seed => "seed",
                            ItemKind::Crop => "crop",
                            ItemKind::Meal => "meal",
                            ItemKind::Drink => "drink",
                            ItemKind::Artifact => "artifact",
                            ItemKind::Corpse => "dwarf",
                            ItemKind::Craft => "artifact",
                            ItemKind::Wool => "crop",
                            ItemKind::Cloth => "artifact",
                            ItemKind::RoughGem | ItemKind::CutGem => "artifact",
                            ItemKind::Weapon => "weapon",
                            ItemKind::Glass => "artifact",
                            ItemKind::Bar => "boulder",
                            ItemKind::Armor => "weapon",
                            ItemKind::Bed => "artifact",
                            ItemKind::Clothes => "artifact",
                            ItemKind::Log => "boulder",
                        };
                        if let Some(atlas) = sprite.texture_atlas.as_mut() {
                            atlas.index = ts.index(glyph);
                        }
                        sprite.color = if it.kind == ItemKind::Corpse {
                            Color::srgba(0.75, 0.8, 0.9, 0.8) // the pale dead
                        } else if it.kind == ItemKind::Craft {
                            item_material_color(&reg.0, it.stuff) // stone-tinted goods
                        } else if matches!(it.kind, ItemKind::RoughGem | ItemKind::CutGem) {
                            item_color(&reg.0, it.kind, it.stuff) // gem-colored
                        } else if ts.is_tinted(glyph) {
                            item_color(&reg.0, it.kind, it.stuff)
                        } else {
                            Color::WHITE
                        };
                    }
                    None => {
                        sprite.color = item_color(&reg.0, it.kind, it.stuff);
                    }
                }
                *vis = Visibility::Visible;
            }
            _ => *vis = Visibility::Hidden,
        }
    }
    for (i, &e) in pools.animals.iter().enumerate() {
        let Ok((mut tf, mut sprite, mut vis)) = sprites.get_mut(e) else { continue };
        match sim.0.animals.get(i) {
            Some(a) if a.alive && a.pos.z == view_z.0 => {
                tf.translation.x = a.pos.x as f32 * TILE;
                tf.translation.y = a.pos.y as f32 * TILE;
                let glyph = match a.kind {
                    AnimalKind::Cow => "cow",
                    AnimalKind::Sheep => "sheep",
                    AnimalKind::Dog => "dog",
                };
                if let Some(ts) = &tileset.0 {
                    if let Some(atlas) = sprite.texture_atlas.as_mut() {
                        atlas.index = ts.index(glyph);
                    }
                }
                // Calves are smaller; the marked flash red, war dogs steel-blue.
                let scale = if a.is_adult() { 1.0 } else { 0.6 };
                tf.scale = Vec3::splat(scale);
                sprite.color = if a.marked || a.war_marked {
                    Color::srgb(1.0, 0.5, 0.5)
                } else if a.war {
                    Color::srgb(0.6, 0.75, 1.0)
                } else {
                    Color::WHITE
                };
                *vis = Visibility::Visible;
            }
            _ => *vis = Visibility::Hidden,
        }
    }
}

fn position_cursor_sprite(
    cursor: Res<Cursor>,
    mut q: Query<&mut Transform, With<CursorSprite>>,
) {
    for mut tf in &mut q {
        tf.translation.x = cursor.x as f32 * TILE;
        tf.translation.y = cursor.y as f32 * TILE;
    }
}

fn update_hud(
    sim: Res<SimRes>,
    reg: Res<Registry>,
    world: Res<WorldRes>,
    screen: Res<ScreenRes>,
    has_save: Res<HasSave>,
    scroll: Res<LegendsState>,
    trade: Res<TradeState>,
    view_z: Res<ViewZ>,
    cursor: Res<Cursor>,
    control: Res<SimControl>,
    mode: Res<UiMode>,
    diagnostics: Res<DiagnosticsStore>,
    mut q: Query<&mut Text, With<HudText>>,
) {
    match screen.0 {
        Screen::Embark => {
            let (rx, ry) = ((cursor.x as usize / 2).min(OW - 1), (cursor.y as usize / 2).min(OW - 1));
            let region = world.0.overworld.get(rx, ry);
            let site = world
                .0
                .sites
                .iter()
                .find(|st| st.region == (rx, ry) && !st.ruined)
                .map(|st| {
                    format!("   here: {} ({})", st.name, world.0.civs[st.civ].name)
                })
                .unwrap_or_default();
            let enemy = world
                .0
                .nearest_hostile_civ(rx, ry)
                .map(|c| format!("nearest threat: {} of the {}", c.name, c.race.name()))
                .unwrap_or_default();
            let reclaimable = region.biome.embarkable() && fort_path((rx, ry)).exists();
            let ok = if !region.biome.embarkable() {
                "cannot embark on ocean"
            } else if reclaimable {
                "Enter: reclaim the retired fortress here"
            } else {
                "Enter: embark here"
            };
            let resume = if has_save.0 { "   F9: continue your saved fortress" } else { "" };
            for mut text in &mut q {
                text.0 = format!(
                    "Dwarf Kingdom :: Choose your embark   (world seed {})\n\
                     {} years of history · {} civilizations · {} sites · {} named figures\n\
                     region ({}, {}) — {}{}\n\
                     {}\n\
                     arrows/click: move   y: Legends   n: forge a new world   {}{}",
                    world.0.seed,
                    world.0.years_simulated,
                    world.0.civs.len(),
                    world.0.sites.len(),
                    world.0.figures.len(),
                    rx,
                    ry,
                    region.biome.name(),
                    site,
                    enemy,
                    ok,
                    resume,
                );
            }
            return;
        }
        Screen::Legends => {
            // The world is immutable; rebuild the text only when the player
            // scrolls or the screen was just opened.
            if !scroll.is_changed() && !screen.is_changed() {
                return;
            }
            let lines = legends_all(sim.0.as_ref(), &world.0);
            let top = scroll.scroll.min(lines.len().saturating_sub(1));
            let body: String = lines
                .iter()
                .skip(top)
                .take(LEGENDS_PAGE)
                .map(|l| format!("\n{l}"))
                .collect();
            for mut text in &mut q {
                text.0 = format!(
                    "Dwarf Kingdom :: Legends & Anthology — {} entries (up/down to scroll, y/Esc to close)\n{}",
                    lines.len(),
                    body
                );
            }
            return;
        }
        Screen::Help => {
            if !screen.is_changed() {
                return;
            }
            for mut text in &mut q {
                text.0 = HELP_TEXT.to_string();
            }
            return;
        }
        Screen::Adventure => {
            let Some(sim) = sim.0.as_ref() else { return };
            let Some(hero) = sim.player else { return };
            let d = &sim.dwarves[hero];
            let quest = match &sim.quest {
                Some((name, false)) => format!("Quest: slay {name}"),
                Some((name, true)) => format!("Quest complete — {name} is slain!"),
                None => "Wander freely.".to_string(),
            };
            let status = if d.alive {
                format!(
                    "{} — torso {} · blood {:.0} · hunger {:.0} thirst {:.0}",
                    d.name, d.body[1].hp, d.blood, d.hunger, d.thirst
                )
            } else {
                format!("{} has fallen. Their deeds are remembered. (Esc)", d.name)
            };
            let log_tail: String = sim
                .log
                .iter()
                .rev()
                .take(4)
                .map(|(_, m)| format!("\n> {m}"))
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            for mut text in &mut q {
                text.0 = format!(
                    "Dwarf Kingdom :: Adventure\n{status}\n{quest}\n\
                     arrows: move/attack   [ ]: stairs   .: wait   p: pick up   c: recruit   g: journey on   y: Legends   Esc: abandon   Q: quit{log_tail}"
                );
            }
            return;
        }
        Screen::Trade => {
            let Some(sim) = sim.0.as_ref() else { return };
            let Some(caravan) = sim.caravan.as_ref() else { return };
            let yours = tradeable_items(sim);
            let offered: u32 = trade.offer.iter().map(|&i| item_value(&sim.items[i], &reg.0)).sum();
            let asked: u32 = trade
                .request
                .iter()
                .filter(|&&g| g < caravan.goods.len())
                .map(|&g| item_value(&caravan.goods[g], &reg.0))
                .sum();
            let need = (asked as f32 * dk_agents::TRADE_MARGIN).ceil() as u32;
            let mut left = format!("THEIR WAGON ({})\n", caravan.civ_name);
            for (g, it) in caravan.goods.iter().enumerate() {
                let sel = if trade.request.contains(&g) { "[x]" } else { "[ ]" };
                let cur = if trade.side == 0 && trade.cursor == g { ">" } else { " " };
                left.push_str(&format!(
                    "{cur}{sel} {} ({})\n",
                    item_label(&reg.0, it),
                    item_value(it, &reg.0)
                ));
            }
            let mut right = "YOUR STORES\n".to_string();
            // A 24-row window that follows the cursor.
            const WINDOW: usize = 24;
            let start = if trade.side == 1 {
                trade.cursor.saturating_sub(WINDOW / 2).min(yours.len().saturating_sub(WINDOW))
            } else {
                0
            };
            if start > 0 {
                right.push_str(&format!("  ... {start} above ...\n"));
            }
            for (row, &i) in yours.iter().enumerate().skip(start).take(WINDOW) {
                let it = &sim.items[i];
                let sel = if trade.offer.contains(&i) { "[x]" } else { "[ ]" };
                let cur = if trade.side == 1 && trade.cursor == row { ">" } else { " " };
                right.push_str(&format!(
                    "{cur}{sel} {} ({})\n",
                    item_label(&reg.0, it),
                    item_value(it, &reg.0)
                ));
            }
            let below = yours.len().saturating_sub(start + WINDOW);
            if below > 0 {
                right.push_str(&format!("  ... {below} below ...\n"));
            }
            for mut text in &mut q {
                text.0 = format!(
                    "Dwarf Kingdom :: Trading with {}\n\
                     offering {offered} · they ask {need} (their price {asked} + the road)\n\
                     tab/arrows: switch column & move   space: select   Enter: strike the deal   Esc: walk away\n\
                     {}\n\n{left}\n{right}",
                    caravan.civ_name, trade.message
                );
            }
            return;
        }
        Screen::Playing => {}
    }
    let Some(sim) = sim.0.as_ref() else { return };
    let sim = SimRef(sim);
    // When the fortress has fallen, the HUD gives way to its epitaph.
    if sim.0.fallen() {
        for mut text in &mut q {
            text.0 = format!(
                "Dwarf Kingdom :: The Fortress Has Fallen\n\n{}\n\n\
                 F9: load a save    Q: quit",
                sim.0.epitaph()
            );
        }
        return;
    }
    let here = cursor.pos(view_z.0);
    let mut under = match sim.0.map.tile_at(here) {
        Some(t) if t.shape != TileShape::Empty => {
            format!("{} {}", reg.0.materials.get(t.material).name, t.shape.name())
        }
        _ => "open air".to_string(),
    };
    if let Some(farm) = sim.0.farms.get(&here) {
        let plant = reg.0.plants.get(farm.crop);
        under = format!(
            "{under} · {} farm ({})",
            plant.name,
            match farm.state {
                FarmState::Fallow => "fallow".to_string(),
                FarmState::Growing { progress } => format!(
                    "growing {}%",
                    progress * 100 / (plant.grow_days * dk_core::TICKS_PER_DAY as u32).max(1)
                ),
                FarmState::Grown => "ready to harvest".to_string(),
            }
        );
    }
    if let Some(b) = sim.0.building_at(here) {
        under = format!("{under} · {}", b.kind.name());
    }
    if let Some(scene) = sim.0.engravings.get(&here) {
        under = format!("{under} · {scene}");
    }
    let water = sim.0.map.water_at(here);
    if water > 0 {
        under = format!("{under} · water {water}/7");
    }
    let magma = sim.0.map.magma_at(here);
    if magma > 0 {
        under = format!("{under} · MAGMA {magma}/7");
    }
    if let Some(a) = sim.0.animal_at(here) {
        let stage = if a.is_adult() { "" } else { " (calf)" };
        let mark = if a.marked { " [marked to cull]" } else { "" };
        under = format!("{under} · {}{}{}", a.kind.name(), stage, mark);
    }
    if let Some(it) = sim
        .0
        .items
        .iter()
        .find(|i| i.active() && i.pos == here && !matches!(i.state, ItemState::Carried { .. }))
    {
        let what = match it.kind {
            ItemKind::Boulder => format!("{} boulder", reg.0.materials.get(it.stuff).name),
            ItemKind::Seed => format!("{} seeds", reg.0.plants.get(it.stuff).name),
            ItemKind::Crop => reg.0.plants.get(it.stuff).name.clone(),
            ItemKind::Meal => "prepared meal".to_string(),
            ItemKind::Drink => "mug of drink".to_string(),
            ItemKind::Artifact => it
                .name
                .clone()
                .unwrap_or_else(|| "a legendary artifact".to_string()),
            ItemKind::Corpse => it
                .name
                .clone()
                .unwrap_or_else(|| "remains".to_string()),
            ItemKind::Craft => format!("{} craft (trade good)", reg.0.materials.get(it.stuff).name),
            ItemKind::Wool => "raw wool".to_string(),
            ItemKind::Cloth => "bolt of cloth (trade good)".to_string(),
            ItemKind::RoughGem => format!("rough {}", dk_agents::gem_name(it.stuff)),
            ItemKind::CutGem => format!("cut {} (trade good)", dk_agents::gem_name(it.stuff)),
            ItemKind::Weapon => format!("{} weapon", reg.0.materials.get(it.stuff).name),
            ItemKind::Glass => "blown glass (trade good)".to_string(),
            ItemKind::Bar => format!("{} bar (trade good)", reg.0.materials.get(it.stuff).name),
            ItemKind::Armor => format!("{} armor", reg.0.materials.get(it.stuff).name),
            ItemKind::Bed => format!("{} bed (trade good)", reg.0.materials.get(it.stuff).name),
            ItemKind::Clothes => "set of clothes (trade good)".to_string(),
            ItemKind::Log => "wooden log".to_string(),
        };
        let what = if it.quality > 0 && it.kind != ItemKind::Artifact {
            format!("{} {what}", dk_agents::quality_name(it.quality))
        } else {
            what
        };
        under = format!("{under} · {what}");
    }

    // Dwarf inspection panel.
    let dwarf_panel = sim
        .0
        .dwarves
        .iter()
        .find(|d| d.alive && d.pos == here)
        .map(|d| {
            let wounds = if d.is_wounded() { " · WOUNDED" } else { "" };
            let role = if d.soldier { " · SOLDIER" } else { "" };
            let mut s = format!(
                "\n{} — {} · happiness {:.0} stress {:.0} · hunger {:.0} thirst {:.0} · blood {:.0}{}",
                d.name,
                d.task_name(),
                d.happiness,
                d.stress,
                d.hunger,
                d.thirst,
                d.blood,
                wounds
            );
            s.push_str(role);
            let idx = sim
                .0
                .dwarves
                .iter()
                .position(|x| std::ptr::eq(x, d))
                .unwrap_or(0);
            s.push_str(&format!("\n  {}", sim.0.biography(idx, &reg.0)));
            for (_, t) in d.thoughts.iter().rev().take(3) {
                s.push_str(&format!("\n  · {} ({:+.0})", t.text(), t.delta()));
            }
            s
        })
        .unwrap_or_default();

    let alive = sim.0.alive_dwarves();
    let idle = sim.0.dwarves.iter().filter(|d| d.alive && d.is_idle()).count();
    let fps = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|d| d.smoothed())
        .unwrap_or(0.0);
    let cal = &sim.0.clock;
    let mut status = if control.paused {
        "PAUSED".to_string()
    } else {
        format!("speed {}", control.speed)
    };
    if sim.0.caravan.is_some() {
        status.push_str("   CARAVAN VISITING :: press r to trade");
    }
    if let Some(b) = sim.0.baron {
        if let Some(d) = sim.0.dwarves.get(b) {
            status.push_str(&format!("   baron: {}", d.name));
        }
    }
    if let Some(m) = &sim.0.mandate {
        let days_left = m.deadline.saturating_sub(sim.0.clock.tick) / dk_core::TICKS_PER_DAY;
        status.push_str(&format!(
            "   MANDATE: {} ({} days left)",
            m.kind.describe(m.amount),
            days_left
        ));
    }
    let mode_txt = mode
        .0
        .map(|(k, _)| format!("   [{} — move cursor, press key again to apply]", k.label()))
        .unwrap_or_default();
    let alarm_txt = if sim.0.alarm { " [SOUNDED]" } else { "" };
    let vampire_txt = if sim.0.stats.drained > 0 {
        format!("   ** a vampire walks among us: {} drained **", sim.0.stats.drained)
    } else {
        String::new()
    };
    let log_tail = sim
        .0
        .log
        .iter()
        .rev()
        .take(2)
        .map(|(_, m)| format!("\n> {m}"))
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();

    for mut text in &mut q {
        text.0 = format!(
            "Dwarf Kingdom\n\
             z {} / {}   cursor ({}, {})   {}\n\
             Year {}, {} {} ({})   {}   {:.0} fps\n\
             dwarves {} ({} idle, {} lost)   meals {}   drinks {}   crops {}   crafts {}   cloth {}   livestock {}   jobs {}\n\
             harvested {}   cooked {}   brewed {}   gems {}/{}   migrants {}   raiders {} ({} slain, {} drowned)   beasts slain {}   veterans {}   armed {}   armored {}   poems {}{}\n\
             d:mine D:engrave x:stairs h:channel f:farm p:stockpile n:pasture o:tavern ':temple z:fishery H:hospital Z:burrow L:library u:cull U:war-dog i:enlist I:barracks v:still k:kitchen m:crafts j:loom ;:jeweler M:smelter K:mason C:clothier F:forge G:glass T:trap B:wall b:tomb g:gate l:lever t:pull c:cancel\n\
             space:pause 1/2/3:speed   [ ]:z   r:trade y:legends   F1:help   F2:alarm{}   F5/F9:save/load   F8:retire   Q:quit{}{}{}",
            view_z.0,
            MAP_D - 1,
            cursor.x,
            cursor.y,
            under,
            cal.year(),
            cal.season().name(),
            cal.day_of_season(),
            sim.0.weather.name(),
            status,
            fps,
            alive,
            idle,
            sim.0.stats.deaths,
            sim.0.count_kind(ItemKind::Meal),
            sim.0.count_kind(ItemKind::Drink),
            sim.0.count_kind(ItemKind::Crop),
            sim.0.count_kind(ItemKind::Craft),
            sim.0.count_kind(ItemKind::Cloth),
            sim.0.alive_animals(),
            sim.0.pending_designations(),
            sim.0.stats.crops_harvested,
            sim.0.stats.meals_cooked,
            sim.0.stats.drinks_brewed,
            sim.0.stats.gems_found,
            sim.0.stats.gems_cut,
            sim.0.stats.migrants_arrived,
            sim.0.alive_hostiles(),
            sim.0.stats.raiders_slain,
            sim.0.stats.drownings,
            sim.0.stats.beasts_slain,
            sim.0.veterans(),
            sim.0.armed_soldiers(),
            sim.0.armored_soldiers(),
            sim.0.poems.len(),
            vampire_txt,
            alarm_txt,
            mode_txt,
            log_tail,
            dwarf_panel,
        );
    }
}

/// Automated verification: DK_SCREENSHOT=1 runs the scripted demo, captures a
/// frame, logs raw fps, and exits.
fn screenshot_mode(
    mut state: ResMut<ShotState>,
    mut commands: Commands,
    mut exit: EventWriter<AppExit>,
    time: Res<Time<Real>>,
) {
    if !screenshot_mode_on() {
        return;
    }
    state.frames += 1;
    if state.frames == 1 {
        state.started_at = time.elapsed_secs_f64();
    }
    if state.frames == 600 && !state.taken {
        state.taken = true;
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(Path::new("phase0.png").to_path_buf()));
        info!("screenshot requested");
    }
    if state.frames >= 660 {
        let elapsed = time.elapsed_secs_f64() - state.started_at;
        info!(
            "measured {:.1} fps over {} frames ({:.2}s)",
            (state.frames - 1) as f64 / elapsed,
            state.frames - 1,
            elapsed
        );
        exit.write(AppExit::Success);
    }
}
