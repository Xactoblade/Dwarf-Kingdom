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
use bevy::render::render_asset::RenderAssetUsages;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::render::view::screenshot::{save_to_disk, Screenshot};
use bevy::window::{MonitorSelection, PresentMode, WindowMode};
use dk_agents::{
    item_value, AnimalKind, PlayerAction,
    load_sim, save_sim, BuildingKind, DesignationKind, Faction, FarmState, ItemKind, ItemState,
    SiegeLeader, SiegeRoster, Sim,
};
use dk_core::Season;
use dk_history::World;
use dk_raws::{MaterialCategory, Raws};
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
  Shift+J ... carpenter's workshop (logs -> barrels & instruments)\n\
  Shift+N ... tanner's shop (butchered hides -> leather)\n\
  Shift+P ... dig a well (thirsty dwarves draw water when drink runs out)\n\
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

/// Where the fort's dwarves have trodden — accumulates as they walk and slowly
/// fades, so busy routes wear the grass down to a bare-earth path. Render-only:
/// it never touches the simulation, so determinism is untouched.
#[derive(Resource, Default)]
struct Traffic(std::collections::HashMap<(i32, i32), f32>);

#[derive(Resource, Default)]
struct SimControl {
    paused: bool,
    speed: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UiKind {
    Mine,
    /// A stockpile told what it is for.
    Pile(dk_agents::StockCategory),
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
    Bedroom,
    Chop,
    Gather,
    Cancel,
}

impl UiKind {
    fn label(self) -> &'static str {
        match self {
            UiKind::Mine => "MINE",
            UiKind::Stairs => "STAIRS",
            UiKind::Channel => "CHANNEL",
            UiKind::Stockpile => "STOCKPILE",
            UiKind::Pile(_) => "PILE",
            UiKind::Farm => "FARM",
            UiKind::Pasture => "PASTURE",
            UiKind::Tavern => "TAVERN",
            UiKind::Temple => "TEMPLE",
            UiKind::Fishery => "FISHERY",
            UiKind::Hospital => "HOSPITAL",
            UiKind::Barracks => "BARRACKS",
            UiKind::Burrow => "BURROW",
            UiKind::Library => "LIBRARY",
            UiKind::Bedroom => "BEDROOM",
            UiKind::Chop => "CHOP",
            UiKind::Gather => "GATHER",
            UiKind::Cancel => "CANCEL",
        }
    }
}

#[derive(Resource, Default)]
struct UiMode(Option<(UiKind, Pos)>);

/// A tool the player can pick from the on-screen toolbar (or a keyboard
/// shortcut) and then apply by clicking the map — mouse-driven, like DF's
/// Steam UI. Rectangle tools take two map clicks; the rest place at one click.
#[derive(Clone, Copy, PartialEq)]
enum Tool {
    /// Two-click rectangle designation or zone (mine, stockpile, tavern, …).
    Rect(UiKind),
    /// Place a workshop/building at the clicked tile.
    Build(BuildingKind),
    /// Plan a constructed wall on the clicked tile.
    Wall,
    /// Smooth-and-engrave the clicked wall.
    Engrave,
    /// Mark the animal at the clicked tile for slaughter.
    Cull,
    /// Train the dog at the clicked tile for war.
    WarDog,
    /// Enlist (or dismiss) the dwarf at the clicked tile as a soldier.
    Enlist,
}

/// The currently selected toolbar tool and its pending rectangle anchor.
#[derive(Resource, Default)]
struct ActiveTool {
    tool: Option<Tool>,
    anchor: Option<Pos>,
    /// True while the mouse is over a toolbar button — map clicks are ignored.
    over_ui: bool,
}

/// An action offered by the clickable buttons on the embark / world-map screen.
#[derive(Clone, Copy, PartialEq)]
enum EmbarkAction {
    /// Skip browsing the world map — auto-pick a good spot and found a fort now.
    JustPlay,
    Embark,
    Adventure,
    NewWorld,
    Legends,
}

/// Set when an embark button is clicked; consumed by handle_input's embark
/// branch so a click does exactly what the matching key does.
#[derive(Resource, Default)]
struct PendingEmbark(Option<EmbarkAction>);

/// A clickable button on the embark screen.
#[derive(Component)]
struct EmbarkButton(EmbarkAction);

/// The embark button bar (shown only on the embark screen).
#[derive(Component)]
struct EmbarkBar;

/// Which toolbar category is expanded (its tools shown), if any. The bar is a
/// category launcher: click Dig/Zones/Workshops/Orders to reveal that group's
/// tools, DF-Steam style, instead of showing all ~33 at once.
#[derive(Resource, Default)]
struct OpenCategory(Option<u8>);

/// A top-level category button on the toolbar.
#[derive(Component)]
struct CategoryButton(u8);

/// Tracks a toggle button's last-frame pressed state so a click toggles once
/// (Interaction stays Pressed every frame the mouse is held).
#[derive(Component, Default)]
struct WasPressed(bool);

/// The category names, indexed by the `cat` field on each tool.
const CATEGORIES: &[&str] = &["Dig", "Zones", "Workshops", "Orders", "Piles"];

/// The dwarf whose info sheet is open (clicked with no tool selected).
#[derive(Resource, Default)]
struct SelectedDwarf(Option<usize>);

/// Root node of the click-a-dwarf info panel.
#[derive(Component)]
struct DwarfPanel;

/// The text inside the dwarf info panel.
#[derive(Component)]
struct DwarfPanelText;

/// Handle to the dynamic minimap texture (one pixel per map tile).
#[derive(Resource)]
struct Minimap(Handle<Image>);

/// Whether the Stocks (inventory) panel is open.
#[derive(Resource, Default)]
struct ShowStocks(bool);

/// The "Stocks" toggle button in the top-right.
#[derive(Component)]
struct StocksButton;

/// The "?" help button in the top-right (opens the controls reference).
#[derive(Component)]
struct HelpButton;

/// Root of the Stocks panel (toggled).
#[derive(Component)]
struct StocksPanel;

/// The text inside the Stocks panel.
#[derive(Component)]
struct StocksPanelText;

/// The fort's goods, grouped for the Stocks panel (label, the item kinds in it).
const STOCK_GROUPS: &[(&str, &[ItemKind])] = &[
    ("Food & Drink", &[ItemKind::Meal, ItemKind::Drink, ItemKind::Crop, ItemKind::Berry, ItemKind::Seed]),
    (
        "Raw materials",
        &[
            ItemKind::Boulder,
            ItemKind::Bar,
            ItemKind::Log,
            ItemKind::Wool,
            ItemKind::Cloth,
            ItemKind::Hide,
            ItemKind::Leather,
        ],
    ),
    (
        "Trade goods",
        &[
            ItemKind::Craft,
            ItemKind::Glass,
            ItemKind::CutGem,
            ItemKind::RoughGem,
            ItemKind::Statue,
            ItemKind::Instrument,
            ItemKind::Clothes,
        ],
    ),
    ("Furniture", &[ItemKind::Bed]),
    ("Containers", &[ItemKind::Barrel, ItemKind::Bin]),
    ("Military", &[ItemKind::Weapon, ItemKind::Armor]),
    ("Special", &[ItemKind::Artifact, ItemKind::Corpse]),
];

/// The yellow rectangle on the minimap showing the on-screen viewport.
#[derive(Component)]
struct MinimapViewport;

/// The minimap image node (toggled with the play screen).
#[derive(Component)]
struct MinimapContainer;

/// The minimap image node itself (side of the square in screen px).
const MINIMAP_PX: f32 = 190.0;

/// Marks a clickable toolbar button and carries what it does + how to describe it.
#[derive(Component, Clone)]
struct ToolButton {
    tool: Tool,
    label: &'static str,
    key: &'static str,
    tip: &'static str,
    /// Category index, for the button's accent colour.
    cat: u8,
}

/// The floating tooltip text node shown while hovering a toolbar button.
#[derive(Component)]
struct TooltipUi;

/// Root node of the bottom toolbar (toggled with the play screen).
#[derive(Component)]
struct ToolbarRoot;

/// Every tool the toolbar offers, grouped by category (cat: 0 dig, 1 zone,
/// 2 workshop, 3 order). The keyboard shortcuts still do the same thing.
const TOOLS: &[ToolButton] = &[
    // --- Dig & build terrain (cat 0)
    ToolButton { tool: Tool::Rect(UiKind::Mine), label: "Mine", key: "d", tip: "Dig out stone, carving tunnels and rooms", cat: 0 },
    ToolButton { tool: Tool::Rect(UiKind::Stairs), label: "Stairs", key: "x", tip: "Carve stairs up and down between z-levels", cat: 0 },
    ToolButton { tool: Tool::Rect(UiKind::Channel), label: "Channel", key: "h", tip: "Dig a channel: opens the floor, water pours in", cat: 0 },
    ToolButton { tool: Tool::Rect(UiKind::Chop), label: "Chop", key: "\u{21e7}X", tip: "Fell trees for logs (drag over a stand of trees)", cat: 0 },
    ToolButton { tool: Tool::Rect(UiKind::Gather), label: "Gather", key: "\u{21e7}G", tip: "Forage wild shrubs for edible berries (drag over a berry patch)", cat: 0 },
    ToolButton { tool: Tool::Wall, label: "Wall", key: "\u{21e7}B", tip: "Plan a constructed wall; masons haul stone and raise it", cat: 0 },
    ToolButton { tool: Tool::Engrave, label: "Engrave", key: "\u{21e7}D", tip: "Smooth a wall and carve a scene from the fort's history", cat: 0 },
    // --- Zones (cat 1)
    ToolButton { tool: Tool::Rect(UiKind::Stockpile), label: "Stockpile", key: "p", tip: "A zone where haulers stack loose goods", cat: 1 },
    ToolButton { tool: Tool::Rect(UiKind::Farm), label: "Farm", key: "f", tip: "A plot for planting and harvesting crops", cat: 1 },
    ToolButton { tool: Tool::Rect(UiKind::Pasture), label: "Pasture", key: "n", tip: "Graze livestock here", cat: 1 },
    ToolButton { tool: Tool::Rect(UiKind::Tavern), label: "Tavern", key: "o", tip: "Dwarves drink and shed stress here", cat: 1 },
    ToolButton { tool: Tool::Rect(UiKind::Temple), label: "Temple", key: "'", tip: "A place of worship for solace", cat: 1 },
    ToolButton { tool: Tool::Rect(UiKind::Fishery), label: "Fishery", key: "z", tip: "Fishers work the water beside this zone", cat: 1 },
    ToolButton { tool: Tool::Rect(UiKind::Hospital), label: "Hospital", key: "\u{21e7}H", tip: "The wounded rest here and mend far faster", cat: 1 },
    ToolButton { tool: Tool::Rect(UiKind::Barracks), label: "Barracks", key: "\u{21e7}I", tip: "Soldiers drill here to become veterans", cat: 1 },
    ToolButton { tool: Tool::Rect(UiKind::Burrow), label: "Burrow", key: "\u{21e7}Z", tip: "A safe room civilians flee to when the alarm sounds", cat: 1 },
    ToolButton { tool: Tool::Rect(UiKind::Library), label: "Library", key: "\u{21e7}L", tip: "Scholars write treatises here", cat: 1 },
    ToolButton { tool: Tool::Rect(UiKind::Bedroom), label: "Bedroom", key: "\u{21e7}R", tip: "Beds here become bedrooms — their owners wake happier", cat: 1 },
    // Piles: a stockpile told what it is for. The generic one above takes
    // anything; these take one class each, so the larder stays a larder.
    ToolButton { tool: Tool::Rect(UiKind::Stockpile), label: "Any", key: "p", tip: "A pile that takes whatever is brought to it", cat: 4 },
    ToolButton { tool: Tool::Rect(UiKind::Pile(dk_agents::StockCategory::Food)), label: "Food", key: "", tip: "Meals, drink, crops, seeds — barrels stand here", cat: 4 },
    ToolButton { tool: Tool::Rect(UiKind::Pile(dk_agents::StockCategory::Stone)), label: "Stone", key: "", tip: "Boulders, for the mason's reach", cat: 4 },
    ToolButton { tool: Tool::Rect(UiKind::Pile(dk_agents::StockCategory::Wood)), label: "Wood", key: "", tip: "Felled logs, for the carpenter's reach", cat: 4 },
    ToolButton { tool: Tool::Rect(UiKind::Pile(dk_agents::StockCategory::Bars)), label: "Bars", key: "", tip: "Smelted metal — bins stand here", cat: 4 },
    ToolButton { tool: Tool::Rect(UiKind::Pile(dk_agents::StockCategory::Goods)), label: "Goods", key: "", tip: "Crafts, cloth, leather, gems, glass — bins stand here", cat: 4 },
    ToolButton { tool: Tool::Rect(UiKind::Pile(dk_agents::StockCategory::Military)), label: "Arms", key: "", tip: "Weapons and armor for the squad", cat: 4 },
    ToolButton { tool: Tool::Rect(UiKind::Pile(dk_agents::StockCategory::Furniture)), label: "Furniture", key: "", tip: "Beds, statues, and empty casks", cat: 4 },
    ToolButton { tool: Tool::Rect(UiKind::Pile(dk_agents::StockCategory::Refuse)), label: "Refuse", key: "", tip: "The dead, until they are buried", cat: 4 },
    // --- Workshops (cat 2)
    ToolButton { tool: Tool::Build(BuildingKind::Still), label: "Still", key: "v", tip: "Brews crops into drink", cat: 2 },
    ToolButton { tool: Tool::Build(BuildingKind::Kitchen), label: "Kitchen", key: "k", tip: "Cooks meals", cat: 2 },
    ToolButton { tool: Tool::Build(BuildingKind::Craftsdwarf), label: "Crafts", key: "m", tip: "Turns stone into decorative trade goods", cat: 2 },
    ToolButton { tool: Tool::Build(BuildingKind::Loom), label: "Loom", key: "j", tip: "Weaves wool into cloth", cat: 2 },
    ToolButton { tool: Tool::Build(BuildingKind::Jeweler), label: "Jeweler", key: ";", tip: "Cuts rough gems into brilliant ones", cat: 2 },
    ToolButton { tool: Tool::Build(BuildingKind::Smelter), label: "Smelter", key: "\u{21e7}M", tip: "Smelts ore boulders into metal bars", cat: 2 },
    ToolButton { tool: Tool::Build(BuildingKind::Forge), label: "Forge", key: "\u{21e7}F", tip: "Forges bars into weapons and armor", cat: 2 },
    ToolButton { tool: Tool::Build(BuildingKind::Mason), label: "Mason", key: "\u{21e7}K", tip: "Carves stone into beds and statues", cat: 2 },
    ToolButton { tool: Tool::Build(BuildingKind::Carpenter), label: "Carpenter", key: "\u{21e7}J", tip: "Works logs into barrels and instruments", cat: 2 },
    ToolButton { tool: Tool::Build(BuildingKind::Clothier), label: "Clothier", key: "\u{21e7}C", tip: "Sews cloth into clothes", cat: 2 },
    ToolButton { tool: Tool::Build(BuildingKind::Tanner), label: "Tanner", key: "\u{21e7}N", tip: "Tans hides into leather", cat: 2 },
    ToolButton { tool: Tool::Build(BuildingKind::GlassFurnace), label: "Glass", key: "\u{21e7}G", tip: "Melts stone into blown glass", cat: 2 },
    ToolButton { tool: Tool::Build(BuildingKind::Well), label: "Well", key: "\u{21e7}P", tip: "Draw water when the drink runs out", cat: 2 },
    ToolButton { tool: Tool::Build(BuildingKind::Trap), label: "Trap", key: "\u{21e7}T", tip: "A weapon trap that shreds raiders", cat: 2 },
    ToolButton { tool: Tool::Build(BuildingKind::Tomb), label: "Tomb", key: "b", tip: "Bury the dead so their ghosts rest", cat: 2 },
    ToolButton { tool: Tool::Build(BuildingKind::Floodgate), label: "Gate", key: "g", tip: "A floodgate, opened and shut by a linked lever", cat: 2 },
    // --- Orders (cat 3)
    ToolButton { tool: Tool::Enlist, label: "Enlist", key: "i", tip: "Make the dwarf here a soldier (click again to dismiss)", cat: 3 },
    ToolButton { tool: Tool::Cull, label: "Cull", key: "u", tip: "Mark the animal here to be slaughtered for meat", cat: 3 },
    ToolButton { tool: Tool::WarDog, label: "War Dog", key: "\u{21e7}U", tip: "Train the dog here into a war beast", cat: 3 },
    ToolButton { tool: Tool::Rect(UiKind::Cancel), label: "Cancel", key: "c", tip: "Cancel designations in a rectangle", cat: 3 },
];

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
        if !s.songs.is_empty() {
            lines.push("=== Songs of the Fortress ===".to_string());
            for song in s.songs.iter().rev().take(40) {
                lines.push(song.clone());
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

/// The mid-point of a local map edge in the given world direction — where a
/// river crosses in or out to match the overworld's flow.
fn edge_point(dir: dk_history::Dir, w: usize, h: usize) -> (usize, usize) {
    use dk_history::Dir;
    match dir {
        Dir::N => (w / 2, 1),
        Dir::S => (w / 2, h - 2),
        Dir::W => (1, h / 2),
        Dir::E => (w - 2, h / 2),
    }
}

/// Cut the region's actual water onto its local map: a river only where the
/// overworld river network flows through (entering/exiting to match its
/// course), a lake if the region sits in a basin, and a scatter of ponds sized
/// to the biome. Post-gen map mutations that don't disturb the embark rng.
fn add_water_features(map: &mut dk_world::Map, region: &dk_history::Region, seed: u64) {
    use dk_history::Biome;
    let (w, h) = (map.width, map.height);
    let center = (w / 2, h / 2);
    if region.river {
        let from = region.river_in.map(|d| edge_point(d, w, h)).unwrap_or(center);
        let to = region.river_out.map(|d| edge_point(d, w, h)).unwrap_or(center);
        if from != to {
            dk_world::carve_river(map, seed, from, to);
        }
    }
    if region.lake {
        let r = w.min(h) as f32 * 0.17;
        dk_world::carve_lake(map, seed ^ 0xABCD, center.0, center.1, r);
    }
    let ponds = match region.biome {
        Biome::Swamp => 5,
        Biome::Forest => 2,
        Biome::Grassland | Biome::Hills | Biome::Tundra => 1,
        _ => 0,
    };
    if ponds > 0 {
        dk_world::carve_ponds(map, seed, ponds);
    }
}

/// How dramatic a region's local terrain relief is — so a mountain embark
/// climbs steep bare rock, hills roll, and the plains lie nearly flat.
fn relief_for(biome: dk_history::Biome) -> dk_world::Relief {
    use dk_history::Biome;
    use dk_world::Relief;
    match biome {
        Biome::Mountains => Relief::Mountainous,
        Biome::Hills => Relief::Hilly,
        Biome::Grassland | Biome::Desert => Relief::Flat,
        _ => Relief::Rolling,
    }
}

fn region_map(world: &World, raws: &Raws, region: (usize, usize)) -> dk_world::Map {
    let seed = world.seed ^ ((region.0 as u64) << 32 | region.1 as u64);
    let mut rng = dk_core::rng_from_seed(seed);
    let r = world.overworld.get(region.0, region.1);
    let style = surface_style(r.biome);
    let mut map = dk_world::generate_terrain(
        &raws.materials, &mut rng, MAP_W, MAP_H, MAP_D, seed, style, relief_for(r.biome),
    );
    add_water_features(&mut map, r, seed);
    map
}

fn embark(world: &World, raws: &Raws, region: (usize, usize)) -> Sim {
    // Each region is its own deterministic local map.
    let seed = world.seed ^ ((region.0 as u64) << 32 | region.1 as u64);
    let mut rng = dk_core::rng_from_seed(seed);
    let r = world.overworld.get(region.0, region.1);
    let biome = r.biome;
    let style = surface_style(biome);
    let mut map = dk_world::generate_terrain(
        &raws.materials, &mut rng, MAP_W, MAP_H, MAP_D, seed, style, relief_for(biome),
    );
    // Rivers, lakes and ponds — carved after gen; they don't disturb the embark
    // rng, so the dwarves rolled below are unchanged.
    add_water_features(&mut map, r, seed);
    // Seed the deep wonder-metal — and the doom of digging it too greedily.
    let breaches = raws
        .materials
        .index_of("adamantine")
        .map(|adam| dk_world::place_adamantine(&mut map, seed, adam))
        .unwrap_or_default();
    let mut sim = Sim::new(map, raws, rng, DWARF_COUNT);
    sim.adamantine_breaches = breaches.into_iter().collect();
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
    sim.plant_trees(trees, raws);
    // Scatter wild berry shrubs too — thickest where it's green and wet, bare
    // in the desert. Foragers gather them (Shift+G) for berries the fort can
    // eat straight, and a tended patch reseeds itself.
    let shrubs = {
        use dk_history::Biome;
        match biome {
            Biome::Forest | Biome::Swamp => 90,
            Biome::Grassland | Biome::Hills => 60,
            Biome::Desert | Biome::Mountains => 8,
            _ => 30,
        }
    };
    sim.plant_shrubs(shrubs);
    // Rarely (about one fort in ten), one of the founding seven keeps a dark
    // secret — a vampire, indistinguishable from any other dwarf until
    // fort-mates start turning up drained of blood.
    sim.maybe_curse_a_vampire();
    // Rarer still, a founder is a werebeast — a beast under the full moon.
    sim.maybe_curse_a_werebeast();
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
    // The fort is founded in the world's current year, and from here the two
    // clocks run together (see dk_agents::sync_world).
    sim.embark_world_year = world.years_simulated;
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
        // DK_SHOT_BIOME=mountain embarks in the highest-elevation region so the
        // mountainous terrain can be captured (screenshot builds only).
        let shot_biome = std::env::var("DK_SHOT_BIOME").unwrap_or_default();
        let region = if shot_biome == "mountain" {
            let mut best = default_region(&world);
            let mut best_e = -1.0f32;
            for y in 0..OW {
                for x in 0..OW {
                    let r = world.overworld.get(x, y);
                    if r.biome.embarkable() && r.elevation > best_e {
                        best_e = r.elevation;
                        best = (x, y);
                    }
                }
            }
            best
        } else if shot_biome == "river" {
            // The river region with the most through-flow (both edges) near
            // centre, to capture a full meandering course.
            let c = OW as i32 / 2;
            let mut best = default_region(&world);
            let mut best_d = i32::MAX;
            for y in 0..OW {
                for x in 0..OW {
                    let r = world.overworld.get(x, y);
                    if r.biome.embarkable() && r.river && r.river_in.is_some() && r.river_out.is_some() {
                        let d = (x as i32 - c).abs() + (y as i32 - c).abs();
                        if d < best_d {
                            best_d = d;
                            best = (x, y);
                        }
                    }
                }
            }
            best
        } else if shot_biome == "swamp" || shot_biome == "lake" {
            let want_lake = shot_biome == "lake";
            let c = OW as i32 / 2;
            let mut best = default_region(&world);
            let mut best_d = i32::MAX;
            for y in 0..OW {
                for x in 0..OW {
                    let r = world.overworld.get(x, y);
                    let hit = if want_lake { r.lake } else { r.biome == dk_history::Biome::Swamp };
                    if hit {
                        let d = (x as i32 - c).abs() + (y as i32 - c).abs();
                        if d < best_d {
                            best_d = d;
                            best = (x, y);
                        }
                    }
                }
            }
            best
        } else {
            default_region(&world)
        };
        let mut sim = embark(&world, &raws, region);
        demo_scenario(&mut sim, &raws);
        // DK_SHOT_SEASON=spring|summer|autumn|winter jumps the clock into that
        // season so the seasonal look can be captured (screenshot builds only).
        if let Ok(s) = std::env::var("DK_SHOT_SEASON") {
            let idx = match s.as_str() {
                "summer" => 1,
                "autumn" => 2,
                "winter" => 3,
                _ => 0,
            };
            sim.clock.tick = idx * dk_core::TICKS_PER_DAY * dk_core::DAYS_PER_SEASON
                + 2 * dk_core::TICKS_PER_DAY;
        }
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
        .insert_resource(Traffic::default())
        .insert_resource(SimControl { paused: false, speed: 1 })
        .insert_resource(UiMode::default())
        .insert_resource(ActiveTool::default())
        .insert_resource(OpenCategory(Some(0)))
        .insert_resource(SelectedDwarf::default())
        .insert_resource(PendingEmbark::default())
        .insert_resource(ShowStocks::default())
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
                    (
                        handle_toolbar,
                        handle_embark_buttons,
                        handle_category,
                        tool_escape,
                        toolbar_layout,
                    )
                        .chain(),
                    handle_mouse,
                    apply_active_tool,
                    select_dwarf,
                    accuse_selected,
                    update_dwarf_panel,
                    apply_embark_action,
                    handle_input,
                    handle_trade_input,
                    toolbar_visibility,
                    overlay_refresh,
                    redraw_tiles,
                )
                    .chain(),
                (
                    sync_agent_sprites,
                    position_cursor_sprite,
                    update_hud,
                    update_minimap,
                    update_stocks,
                    handle_help_button,
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
    mut images: ResMut<Assets<Image>>,
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

    // A floating tooltip that appears just above the toolbar on hover.
    commands.spawn((
        Text::new(""),
        TextFont { font_size: 15.0, ..default() },
        TextColor(Color::srgb(1.0, 0.95, 0.7)),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(10.0),
            bottom: Val::Px(46.0),
            padding: UiRect::axes(Val::Px(8.0), Val::Px(4.0)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.8)),
        TooltipUi,
    ));

    // Click-a-dwarf info sheet — a right-side panel, hidden until a dwarf is
    // clicked with no tool selected.
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(0.0),
                top: Val::Px(0.0),
                width: Val::Px(360.0),
                max_height: Val::Percent(88.0),
                padding: UiRect::all(Val::Px(10.0)),
                overflow: Overflow::clip(),
                ..default()
            },
            BackgroundColor(Color::srgba(0.06, 0.06, 0.09, 0.92)),
            Visibility::Hidden,
            DwarfPanel,
        ))
        .with_child((
            Text::new(""),
            TextFont { font_size: 14.0, ..default() },
            TextColor(Color::srgb(0.92, 0.92, 0.85)),
            DwarfPanelText,
        ));

    // The mouse-driven bottom toolbar: a bar of category launchers, with each
    // category's tools revealed in a panel above when it's opened.
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(0.0),
                left: Val::Px(0.0),
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::FlexStart,
                ..default()
            },
            ToolbarRoot,
        ))
        .with_children(|root| {
            // The tools panel (shown for the open category) sits above the bar.
            root.spawn((
                Node {
                    flex_direction: FlexDirection::Row,
                    flex_wrap: FlexWrap::Wrap,
                    column_gap: Val::Px(3.0),
                    row_gap: Val::Px(3.0),
                    padding: UiRect::all(Val::Px(4.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.05, 0.05, 0.07, 0.9)),
            ))
            .with_children(|panel| {
                for t in TOOLS {
                    panel
                        .spawn((
                            Button,
                            Node {
                                display: Display::None, // hidden until its category opens
                                padding: UiRect::axes(Val::Px(7.0), Val::Px(4.0)),
                                ..default()
                            },
                            BackgroundColor(tool_bg(t.cat, false, false)),
                            t.clone(),
                        ))
                        .with_children(|b| {
                            b.spawn((
                                Text::new(t.label),
                                TextFont { font_size: 13.0, ..default() },
                                TextColor(Color::srgb(0.95, 0.95, 0.92)),
                            ))
                            .with_child((
                                TextSpan::new(format!("  {}", t.key)),
                                TextFont { font_size: 11.0, ..default() },
                                TextColor(Color::srgb(0.75, 0.75, 0.55)),
                            ));
                        });
                }
            });
            // The always-visible category launcher bar.
            root.spawn((
                Node {
                    width: Val::Percent(100.0),
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(3.0),
                    padding: UiRect::all(Val::Px(4.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.04, 0.04, 0.06, 0.95)),
            ))
            .with_children(|bar| {
                for (i, name) in CATEGORIES.iter().enumerate() {
                    bar.spawn((
                        Button,
                        Node {
                            padding: UiRect::axes(Val::Px(12.0), Val::Px(5.0)),
                            ..default()
                        },
                        BackgroundColor(tool_bg(i as u8, false, false)),
                        CategoryButton(i as u8),
                        WasPressed::default(),
                    ))
                    .with_child((
                        Text::new(*name),
                        TextFont { font_size: 15.0, ..default() },
                        TextColor(Color::srgb(0.95, 0.95, 0.92)),
                    ));
                }
            });
        });

    // Minimap: a live top-down picture of the map (one pixel per tile) in the
    // lower-right corner, with a yellow outline showing the on-screen viewport.
    let minimap = Image::new_fill(
        Extent3d { width: MAP_W as u32, height: MAP_H as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        &[18, 18, 22, 255],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::all(),
    );
    let minimap = images.add(minimap);
    commands.insert_resource(Minimap(minimap.clone()));
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(8.0),
                bottom: Val::Px(60.0),
                width: Val::Px(MINIMAP_PX),
                height: Val::Px(MINIMAP_PX),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BorderColor(Color::srgb(0.5, 0.5, 0.55)),
            ImageNode::new(minimap),
            MinimapContainer,
        ))
        .with_child((
            Node {
                position_type: PositionType::Absolute,
                border: UiRect::all(Val::Px(1.5)),
                ..default()
            },
            BorderColor(Color::srgb(1.0, 0.95, 0.3)),
            MinimapViewport,
        ));

    // Clickable action buttons for the embark / world-map screen (shown only
    // there). Keyboard shortcuts still work.
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(0.0),
                left: Val::Px(0.0),
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Row,
                justify_content: JustifyContent::Center,
                column_gap: Val::Px(8.0),
                padding: UiRect::all(Val::Px(8.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.05, 0.05, 0.07, 0.92)),
            Visibility::Hidden,
            EmbarkBar,
        ))
        .with_children(|bar| {
            for (act, label, cat) in [
                (EmbarkAction::JustPlay, "\u{25b6} Just play (auto-pick a spot)", 1u8),
                (EmbarkAction::Embark, "Embark here  (Enter)", 1),
                (EmbarkAction::Adventure, "Adventure  (a)", 2),
                (EmbarkAction::NewWorld, "Forge a new world  (n)", 0),
                (EmbarkAction::Legends, "Legends  (y)", 3),
            ] {
                bar.spawn((
                    Button,
                    Node {
                        padding: UiRect::axes(Val::Px(14.0), Val::Px(7.0)),
                        ..default()
                    },
                    BackgroundColor(tool_bg(cat, false, false)),
                    EmbarkButton(act),
                ))
                .with_child((
                    Text::new(label),
                    TextFont { font_size: 16.0, ..default() },
                    TextColor(Color::srgb(0.95, 0.95, 0.92)),
                ));
            }
        });

    // "Stocks" toggle button (top-right) and the inventory panel it opens.
    commands
        .spawn((
            Button,
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(80.0),
                top: Val::Px(8.0),
                padding: UiRect::axes(Val::Px(12.0), Val::Px(5.0)),
                ..default()
            },
            BackgroundColor(tool_bg(2, false, false)),
            StocksButton,
            WasPressed::default(),
        ))
        .with_child((
            Text::new("Stocks"),
            TextFont { font_size: 15.0, ..default() },
            TextColor(Color::srgb(0.95, 0.95, 0.92)),
        ));
    commands
        .spawn((
            Button,
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(8.0),
                top: Val::Px(8.0),
                padding: UiRect::axes(Val::Px(13.0), Val::Px(5.0)),
                ..default()
            },
            BackgroundColor(tool_bg(3, false, false)),
            HelpButton,
            WasPressed::default(),
        ))
        .with_child((
            Text::new("?"),
            TextFont { font_size: 16.0, ..default() },
            TextColor(Color::srgb(0.95, 0.95, 0.92)),
        ));
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(8.0),
                top: Val::Px(44.0),
                width: Val::Px(300.0),
                max_height: Val::Percent(80.0),
                padding: UiRect::all(Val::Px(12.0)),
                overflow: Overflow::clip(),
                ..default()
            },
            BackgroundColor(Color::srgba(0.06, 0.06, 0.09, 0.94)),
            Visibility::Hidden,
            StocksPanel,
        ))
        .with_child((
            Text::new(""),
            TextFont { font_size: 14.0, ..default() },
            TextColor(Color::srgb(0.92, 0.92, 0.85)),
            StocksPanelText,
        ));
}

// ---------------------------------------------------------------- systems

fn run_sim(
    mut sim: ResMut<SimRes>,
    reg: Res<Registry>,
    control: Res<SimControl>,
    screen: Res<ScreenRes>,
    mut world: ResMut<WorldRes>,
    mut dirty: ResMut<MapDirty>,
    mut traffic: ResMut<Traffic>,
) {
    // The fortress keeps living while you read Legends. Embark has no sim,
    // and Adventure advances only when the player acts.
    if control.paused || matches!(screen.0, Screen::Embark | Screen::Adventure) {
        return;
    }
    let Some(sim) = sim.0.as_mut() else { return };
    sim.step(&reg.0);
    // Each year the fort lives, the world outside lives one too — and word of
    // it reaches the gates. Cheap until a year actually turns.
    dk_agents::sync_world(sim, &mut world.0);
    if sim.map_changed {
        sim.map_changed = false;
        dirty.0 = true;
    }
    // Foot traffic wears paths into the ground — a render-only overlay that
    // never feeds back into the sim, so determinism holds. Busy tiles climb
    // toward a cap; everywhere fades a little each tick.
    let t = &mut traffic.0;
    for v in t.values_mut() {
        *v *= 0.992;
    }
    t.retain(|_, v| *v > 0.03);
    for d in &sim.dwarves {
        if d.alive && d.faction == Faction::Fort {
            let e = t.entry((d.pos.x, d.pos.y)).or_insert(0.0);
            *e = (*e + 0.10).min(5.0);
        }
    }
    // Repaint the ground now and then so worn paths appear as they form.
    if sim.clock.tick % 20 == 0 {
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
    active: Res<ActiveTool>,
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

    // Left click: move the tile cursor to the clicked tile (unless the click
    // landed on a toolbar button).
    if buttons.just_pressed(MouseButton::Left) && !active.over_ui {
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

    // Live rubber-band: while a rectangle is being drawn (a toolbar tool's
    // anchor is down, or a keyboard mode is armed), the tile cursor follows the
    // mouse so the selection previews as you move — like DF's drag-select.
    // Only redraw when the tile actually changes, to keep it cheap.
    if (active.anchor.is_some() || mode.0.is_some()) && !active.over_ui {
        if let Ok(window) = windows.single() {
            if let Some(screen) = window.cursor_position() {
                if let Ok(world) = cam.viewport_to_world_2d(cam_global, screen) {
                    let tx = (world.x / TILE).round() as i32;
                    let ty = (world.y / TILE).round() as i32;
                    if (0..MAP_W as i32).contains(&tx)
                        && (0..MAP_H as i32).contains(&ty)
                        && (cursor.x != tx || cursor.y != ty)
                    {
                        cursor.x = tx;
                        cursor.y = ty;
                        dirty.0 = true;
                    }
                }
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

/// Apply a rectangle tool (designation or zone) to the sim — the shared body
/// behind both the keyboard shortcuts and the toolbar's mouse clicks.
fn apply_ui_rect(sim: &mut Sim, kind: UiKind, a: Pos, b: Pos) {
    match kind {
        UiKind::Mine => {
            sim.designate_rect(DesignationKind::Mine, a, b);
        }
        UiKind::Stairs => {
            sim.designate_rect(DesignationKind::Stairs, a, b);
        }
        UiKind::Channel => {
            sim.designate_rect(DesignationKind::Channel, a, b);
        }
        UiKind::Chop => {
            sim.designate_rect(DesignationKind::Chop, a, b);
        }
        UiKind::Gather => {
            sim.designate_rect(DesignationKind::Gather, a, b);
        }
        UiKind::Stockpile => sim.add_stockpile(a, b),
        UiKind::Pile(cat) => sim.add_filtered_stockpile(a, b, dk_agents::StockFilter::only(&[cat])),
        UiKind::Farm => {
            sim.add_farm(a, b, 0);
        }
        UiKind::Pasture => sim.add_pasture(a, b),
        UiKind::Tavern => sim.add_tavern(a, b),
        UiKind::Temple => sim.add_temple(a, b),
        UiKind::Fishery => sim.add_fishery(a, b),
        UiKind::Hospital => sim.add_hospital(a, b),
        UiKind::Barracks => sim.add_barracks(a, b),
        UiKind::Burrow => sim.add_burrow(a, b),
        UiKind::Library => sim.add_library(a, b),
        UiKind::Bedroom => sim.add_bedroom(a, b),
        UiKind::Cancel => {
            sim.cancel_rect(a, b);
        }
    }
}

/// Background colour for a tool button, brightened when hovered or active.
fn tool_bg(cat: u8, active: bool, hover: bool) -> Color {
    let [r, g, b] = match cat {
        0 => [0.55, 0.4, 0.3],
        1 => [0.3, 0.5, 0.4],
        2 => [0.4, 0.42, 0.55],
        _ => [0.5, 0.3, 0.3],
    };
    let k = if active { 0.55 } else if hover { 0.3 } else { 0.0 };
    Color::srgb(r + (1.0 - r) * k, g + (1.0 - g) * k, b + (1.0 - b) * k)
}

/// Tool buttons: clicking selects a tool (clicking the active one again
/// deselects); hovering shows its tooltip.
fn handle_toolbar(
    screen: Res<ScreenRes>,
    mut active: ResMut<ActiveTool>,
    mut buttons: Query<(&Interaction, &ToolButton, &mut BackgroundColor)>,
    cat_interactions: Query<&Interaction, With<CategoryButton>>,
    mut tip: Query<&mut Text, With<TooltipUi>>,
) {
    if screen.0 != Screen::Playing {
        active.over_ui = false;
        return;
    }
    let mut over = false;
    let mut hover_tip: Option<String> = None;
    for (interaction, tb, mut bg) in &mut buttons {
        let hovered = !matches!(interaction, Interaction::None);
        if hovered {
            over = true;
        }
        if matches!(interaction, Interaction::Pressed) {
            // Select this tool (idempotent while held). Deselect with Esc.
            active.tool = Some(tb.tool);
            active.anchor = None;
        }
        if hovered {
            hover_tip = Some(format!("{}  ({})   {}", tb.label, tb.key, tb.tip));
        }
        let is_active = active.tool == Some(tb.tool);
        *bg = BackgroundColor(tool_bg(tb.cat, is_active, hovered));
    }
    // Hovering a category launcher also counts as "over the UI".
    for interaction in &cat_interactions {
        if !matches!(interaction, Interaction::None) {
            over = true;
        }
    }
    active.over_ui = over;
    if let Ok(mut text) = tip.single_mut() {
        text.0 = hover_tip.unwrap_or_default();
    }
}

/// Embark-screen buttons: clicking one queues the matching action; the button
/// bar is shown only on the embark screen.
fn handle_embark_buttons(
    screen: Res<ScreenRes>,
    mut active: ResMut<ActiveTool>,
    mut pending: ResMut<PendingEmbark>,
    mut bar: Query<&mut Visibility, With<EmbarkBar>>,
    mut buttons: Query<(&Interaction, &EmbarkButton, &mut BackgroundColor)>,
) {
    let on_embark = screen.0 == Screen::Embark;
    if let Ok(mut vis) = bar.single_mut() {
        *vis = if on_embark { Visibility::Inherited } else { Visibility::Hidden };
    }
    if !on_embark {
        return;
    }
    let mut over = false;
    for (interaction, eb, mut bg) in &mut buttons {
        let hovered = !matches!(interaction, Interaction::None);
        if hovered {
            over = true;
        }
        if matches!(interaction, Interaction::Pressed) {
            pending.0 = Some(eb.0);
        }
        let cat = match eb.0 {
            EmbarkAction::JustPlay | EmbarkAction::Embark => 1,
            EmbarkAction::Adventure => 2,
            EmbarkAction::NewWorld => 0,
            EmbarkAction::Legends => 3,
        };
        *bg = BackgroundColor(tool_bg(cat, false, hovered));
    }
    // So clicking a button doesn't also move the world-map cursor.
    active.over_ui = over;
}

/// Category launchers: clicking one opens that group's tools (clicking the open
/// one closes it). The map beneath stays visible.
fn handle_category(
    screen: Res<ScreenRes>,
    mut open: ResMut<OpenCategory>,
    mut buttons: Query<(&Interaction, &CategoryButton, &mut WasPressed, &mut BackgroundColor)>,
) {
    if screen.0 != Screen::Playing {
        return;
    }
    for (interaction, cb, mut was, mut bg) in &mut buttons {
        let pressed = matches!(interaction, Interaction::Pressed);
        if pressed && !was.0 {
            open.0 = if open.0 == Some(cb.0) { None } else { Some(cb.0) };
        }
        was.0 = pressed;
        let is_open = open.0 == Some(cb.0);
        let hovered = !matches!(interaction, Interaction::None);
        *bg = BackgroundColor(tool_bg(cb.0, is_open, hovered));
    }
}

/// Show only the tools of the open category (collapse the rest).
fn toolbar_layout(open: Res<OpenCategory>, mut tools: Query<(&ToolButton, &mut Node)>) {
    if !open.is_changed() {
        return;
    }
    for (tb, mut node) in &mut tools {
        node.display = if open.0 == Some(tb.cat) { Display::Flex } else { Display::None };
    }
}

/// Escape steps back one level: clear the pending rectangle anchor, then the
/// selected tool, then close the open category — like right-click in DF.
fn tool_escape(
    keys: Res<ButtonInput<KeyCode>>,
    screen: Res<ScreenRes>,
    mut active: ResMut<ActiveTool>,
    mut open: ResMut<OpenCategory>,
    mut selected: ResMut<SelectedDwarf>,
) {
    if screen.0 != Screen::Playing || !keys.just_pressed(KeyCode::Escape) {
        return;
    }
    if selected.0.is_some() {
        selected.0 = None;
    } else if active.anchor.is_some() {
        active.anchor = None;
    } else if active.tool.is_some() {
        active.tool = None;
    } else if open.0.is_some() {
        open.0 = None;
    }
}

/// Apply the selected toolbar tool where the player clicks the map. Rectangle
/// tools take two clicks (anchor, then apply); the rest place at one click.
fn apply_active_tool(
    buttons: Res<ButtonInput<MouseButton>>,
    screen: Res<ScreenRes>,
    cursor: Res<Cursor>,
    view_z: Res<ViewZ>,
    mut active: ResMut<ActiveTool>,
    mut sim: ResMut<SimRes>,
    mut dirty: ResMut<MapDirty>,
) {
    if screen.0 != Screen::Playing || active.over_ui {
        return;
    }
    if !buttons.just_pressed(MouseButton::Left) {
        return;
    }
    let Some(tool) = active.tool else { return };
    let Some(sim) = sim.0.as_mut() else { return };
    // handle_mouse (chained before this) has already moved the cursor to the
    // clicked tile, so the cursor is exactly where the player clicked.
    let here = Pos::new(cursor.x, cursor.y, view_z.0);
    match tool {
        Tool::Build(kind) => {
            sim.add_building(kind, here);
        }
        Tool::Wall => {
            sim.designate_construction(here);
        }
        Tool::Engrave => {
            sim.designate_rect(DesignationKind::Smooth, here, here);
        }
        Tool::Cull => {
            sim.mark_nearest_animal(here);
        }
        Tool::WarDog => {
            sim.mark_nearest_for_war(here);
        }
        Tool::Enlist => {
            sim.toggle_soldier(here);
        }
        Tool::Rect(kind) => match active.anchor {
            Some(a) if a.z == here.z => {
                apply_ui_rect(sim, kind, a, here);
                active.anchor = None;
            }
            _ => active.anchor = Some(here),
        },
    }
    dirty.0 = true;
}

/// With no tool selected, clicking a tile opens that dwarf's info sheet;
/// clicking an empty tile closes it.
fn select_dwarf(
    buttons: Res<ButtonInput<MouseButton>>,
    screen: Res<ScreenRes>,
    active: Res<ActiveTool>,
    cursor: Res<Cursor>,
    view_z: Res<ViewZ>,
    sim: Res<SimRes>,
    mut selected: ResMut<SelectedDwarf>,
) {
    if screen.0 != Screen::Playing || active.over_ui || active.tool.is_some() {
        return;
    }
    if !buttons.just_pressed(MouseButton::Left) {
        return;
    }
    let Some(sim) = sim.0.as_ref() else { return };
    let here = Pos::new(cursor.x, cursor.y, view_z.0);
    // The dwarf standing on the clicked tile, if any (fort folk only).
    let found = sim.dwarves.iter().position(|d| {
        d.alive && d.faction == dk_agents::Faction::Fort && d.pos == here
    });
    // Clicking a dwarf opens their sheet; clicking bare ground closes it.
    if selected.0 != found {
        selected.0 = found;
    }
}

/// Shift+A brings the selected dwarf to justice on suspicion of vampirism —
/// the fort's one recourse against the hidden blood-drinker.
fn accuse_selected(
    keys: Res<ButtonInput<KeyCode>>,
    screen: Res<ScreenRes>,
    mut selected: ResMut<SelectedDwarf>,
    mut sim: ResMut<SimRes>,
    mut dirty: ResMut<MapDirty>,
) {
    if screen.0 != Screen::Playing {
        return;
    }
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    if !shift || !keys.just_pressed(KeyCode::KeyA) {
        return;
    }
    let Some(i) = selected.0 else { return };
    if let Some(sim) = sim.0.as_mut() {
        sim.accuse(i);
        selected.0 = None;
        dirty.0 = true;
    }
}

/// Populate and show/hide the dwarf info sheet.
fn update_dwarf_panel(
    selected: Res<SelectedDwarf>,
    sim: Res<SimRes>,
    reg: Res<Registry>,
    mut panel: Query<&mut Visibility, With<DwarfPanel>>,
    mut text: Query<&mut Text, With<DwarfPanelText>>,
) {
    if !selected.is_changed() {
        return;
    }
    let show = match (selected.0, sim.0.as_ref()) {
        (Some(i), Some(sim)) if i < sim.dwarves.len() => {
            if let Ok(mut t) = text.single_mut() {
                t.0 = format!("{}\n\n[Shift+A] accuse of vampirism", sim.biography(i, &reg.0));
            }
            true
        }
        _ => false,
    };
    if let Ok(mut vis) = panel.single_mut() {
        *vis = if show { Visibility::Inherited } else { Visibility::Hidden };
    }
}

/// Colour of a tile on the minimap: the topmost non-empty tile's material
/// (water shows blue), scanned from the surface down.
fn minimap_color(sim: &Sim, raws: &Raws, x: i32, y: i32) -> [u8; 3] {
    for z in (0..MAP_D as i32).rev() {
        let p = Pos::new(x, y, z);
        if let Some(t) = sim.map.tile_at(p) {
            if t.shape != TileShape::Empty {
                if sim.map.water_at(p) >= 3 {
                    return [40, 90, 160];
                }
                return raws.materials.get(t.material).color;
            }
        }
    }
    [18, 18, 22]
}

/// Repaint the minimap texture and move the yellow viewport rectangle to match
/// what's on screen.
fn update_minimap(
    screen: Res<ScreenRes>,
    sim: Res<SimRes>,
    reg: Res<Registry>,
    minimap: Res<Minimap>,
    mut images: ResMut<Assets<Image>>,
    windows: Query<&Window>,
    camera: Query<&Transform, With<Camera2d>>,
    mut container: Query<&mut Visibility, With<MinimapContainer>>,
    mut viewport: Query<&mut Node, With<MinimapViewport>>,
) {
    // Hide the minimap off the play screen.
    let playing = screen.0 == Screen::Playing;
    if let Ok(mut vis) = container.single_mut() {
        *vis = if playing { Visibility::Inherited } else { Visibility::Hidden };
    }
    if !playing {
        return;
    }
    let Some(sim) = sim.0.as_ref() else { return };

    // Repaint the map picture (one pixel per tile; row 0 = top = high map-y).
    if let Some(img) = images.get_mut(&minimap.0) {
        if let Some(data) = img.data.as_mut() {
            for y in 0..MAP_H {
                let row = MAP_H - 1 - y;
                for x in 0..MAP_W {
                    let c = minimap_color(sim, &reg.0, x as i32, y as i32);
                    let idx = (row * MAP_W + x) * 4;
                    data[idx] = c[0];
                    data[idx + 1] = c[1];
                    data[idx + 2] = c[2];
                    data[idx + 3] = 255;
                }
            }
        }
    }

    // Move the viewport outline to the camera's visible tile rectangle.
    let (Ok(window), Ok(cam)) = (windows.single(), camera.single()) else { return };
    let ppt = MINIMAP_PX / MAP_W as f32;
    let half_w = window.width() * 0.5 * cam.scale.x / TILE;
    let half_h = window.height() * 0.5 * cam.scale.y / TILE;
    let cx = cam.translation.x / TILE;
    let cy = cam.translation.y / TILE;
    let x0 = (cx - half_w).clamp(0.0, MAP_W as f32);
    let x1 = (cx + half_w).clamp(0.0, MAP_W as f32);
    let y0 = (cy - half_h).clamp(0.0, MAP_H as f32);
    let y1 = (cy + half_h).clamp(0.0, MAP_H as f32);
    if let Ok(mut node) = viewport.single_mut() {
        node.left = Val::Px(x0 * ppt);
        node.top = Val::Px((MAP_H as f32 - y1) * ppt);
        node.width = Val::Px((x1 - x0).max(2.0) * ppt);
        node.height = Val::Px((y1 - y0).max(2.0) * ppt);
    }
}

/// Drop into fortress play with a freshly founded/reclaimed sim: adopt its
/// map, recenter the camera and cursor, and switch to the Playing screen.
fn enter_fort_at(
    new_sim: Sim,
    sim: &mut SimRes,
    screen: &mut ScreenRes,
    cursor: &mut Cursor,
    view_z: &mut ViewZ,
    dirty: &mut MapDirty,
    camera: &mut Query<&mut Transform, With<Camera2d>>,
) {
    view_z.0 = new_sim.map.walk_surface_z(MAP_W / 2, MAP_H / 2).unwrap_or(MAP_D / 2) as i32;
    sim.0 = Some(new_sim);
    cursor.x = MAP_W as i32 / 2;
    cursor.y = MAP_H as i32 / 2;
    if let Ok(mut tf) = camera.single_mut() {
        tf.translation.x = MAP_W as f32 * TILE * 0.5;
        tf.translation.y = MAP_H as f32 * TILE * 0.5;
        tf.scale = Vec3::ONE;
    }
    screen.0 = Screen::Playing;
    dirty.0 = true;
}

/// Auto-pick a welcoming embark spot: an embarkable region nearest the middle
/// of the world, favouring the green, river-fed lands over bare hills.
fn pick_embark_region(world: &World) -> (usize, usize) {
    let c = OW as i32 / 2;
    let mut best: Option<((usize, usize), i32)> = None;
    for y in 0..OW {
        for x in 0..OW {
            let r = world.overworld.get(x, y);
            if !r.biome.embarkable() {
                continue;
            }
            // Lower score wins: distance from centre, minus a bonus for a river
            // or lake actually flowing through, so "just play" favours water.
            let dist = (x as i32 - c).abs() + (y as i32 - c).abs();
            let water_bonus = if r.river { 10 } else { 0 } + if r.lake { 5 } else { 0 };
            let score = dist - water_bonus;
            if best.map_or(true, |(_, b)| score < b) {
                best = Some(((x, y), score));
            }
        }
    }
    best.map_or((OW / 2, OW / 2), |(r, _)| r)
}

/// Reclaim a retired fortress at `region` if one endures there, otherwise found
/// a fresh colony.
fn found_fort(world: &World, raws: &Raws, region: (usize, usize)) -> Sim {
    let reclaimed = fort_path(region).exists().then(|| {
        load_sim(&fort_path(region), raws)
            .map_err(|e| error!("reclaim failed: {e:#}"))
            .ok()
            .filter(|s| (s.map.width, s.map.height, s.map.depth) == (MAP_W, MAP_H, MAP_D))
    });
    match reclaimed {
        Some(Some(mut loaded)) => {
            loaded.home_region = Some(region);
            loaded
        }
        _ => embark(world, raws, region),
    }
}

/// Perform a queued embark-screen action from a button click — the same effect
/// as the matching key, so the world map is fully mouse-drivable.
fn apply_embark_action(
    mut pending: ResMut<PendingEmbark>,
    reg: Res<Registry>,
    mut world: ResMut<WorldRes>,
    mut screen: ResMut<ScreenRes>,
    mut legends: ResMut<LegendsState>,
    mut sim: ResMut<SimRes>,
    mut cursor: ResMut<Cursor>,
    mut view_z: ResMut<ViewZ>,
    mut dirty: ResMut<MapDirty>,
    mut camera: Query<&mut Transform, With<Camera2d>>,
) {
    if screen.0 != Screen::Embark {
        pending.0 = None;
        return;
    }
    let Some(act) = pending.0.take() else { return };
    let region = ((cursor.x as usize / 2).min(OW - 1), (cursor.y as usize / 2).min(OW - 1));
    let embarkable = world.0.overworld.get(region.0, region.1).biome.embarkable();
    match act {
        EmbarkAction::JustPlay => {
            // Skip the world map entirely: pick a good spot and found a fort.
            let region = pick_embark_region(&world.0);
            let new_sim = found_fort(&world.0, &reg.0, region);
            enter_fort_at(new_sim, &mut sim, &mut screen, &mut cursor, &mut view_z, &mut dirty, &mut camera);
        }
        EmbarkAction::Legends => {
            legends.from = Screen::Embark;
            legends.scroll = 0;
            screen.0 = Screen::Legends;
            dirty.0 = true;
        }
        EmbarkAction::NewWorld => {
            let seed = fresh_world_seed();
            persist_world_seed(seed);
            world.0 = World::generate(seed, OW, OW, HISTORY_YEARS);
            cursor.x = 0;
            cursor.y = 0;
            dirty.0 = true;
        }
        EmbarkAction::Embark if embarkable => {
            let new_sim = found_fort(&world.0, &reg.0, region);
            enter_fort_at(new_sim, &mut sim, &mut screen, &mut cursor, &mut view_z, &mut dirty, &mut camera);
        }
        EmbarkAction::Adventure if embarkable => {
            let mut new_sim = embark(&world.0, &reg.0, region);
            if new_sim.begin_adventure(&reg.0).is_some() {
                new_sim.adv_region = Some(region);
                enter_fort_at(new_sim, &mut sim, &mut screen, &mut cursor, &mut view_z, &mut dirty, &mut camera);
                screen.0 = Screen::Adventure;
            }
        }
        // Chose an unembarkable tile (ocean/mountains) — no-op, like the key.
        _ => {}
    }
}

/// The colour a stockpile paints its floor: one hue per class of goods, so a
/// glance at the fort tells you where the larder ends and the stoneyard
/// begins. A pile that takes everything keeps the plain blue it always had.
fn pile_tint(accepts: dk_agents::StockFilter) -> [f32; 3] {
    use dk_agents::StockCategory as C;
    if accepts.takes_everything() {
        return [0.25, 0.45, 0.9];
    }
    match accepts.categories().first() {
        Some(C::Food) => [0.35, 0.75, 0.35],
        Some(C::Stone) => [0.55, 0.55, 0.6],
        Some(C::Wood) => [0.55, 0.38, 0.2],
        Some(C::Bars) => [0.75, 0.7, 0.35],
        Some(C::Goods) => [0.75, 0.5, 0.85],
        Some(C::Military) => [0.85, 0.3, 0.3],
        Some(C::Furniture) => [0.4, 0.65, 0.8],
        Some(C::Refuse) => [0.45, 0.35, 0.3],
        None => [0.3, 0.3, 0.3],
    }
}

/// A plain plural name for an item kind, for the Stocks list.
fn item_kind_name(k: ItemKind) -> &'static str {
    match k {
        ItemKind::Meal => "prepared meals",
        ItemKind::Drink => "drinks",
        ItemKind::Crop => "crops",
        ItemKind::Berry => "foraged berries",
        ItemKind::Seed => "seeds",
        ItemKind::Boulder => "stone boulders",
        ItemKind::Bar => "metal bars",
        ItemKind::Log => "logs",
        ItemKind::Wool => "raw wool",
        ItemKind::Cloth => "cloth",
        ItemKind::Hide => "raw hides",
        ItemKind::Leather => "leather",
        ItemKind::Craft => "stone crafts",
        ItemKind::Glass => "blown glass",
        ItemKind::CutGem => "cut gems",
        ItemKind::RoughGem => "rough gems",
        ItemKind::Statue => "statues",
        ItemKind::Barrel => "barrels",
        ItemKind::Bin => "bins",
        ItemKind::Instrument => "instruments",
        ItemKind::Clothes => "sets of clothes",
        ItemKind::Bed => "beds",
        ItemKind::Weapon => "weapons",
        ItemKind::Armor => "suits of armor",
        ItemKind::Artifact => "artifacts",
        ItemKind::Corpse => "corpses (unburied)",
    }
}

/// The Stocks button toggles a categorized inventory panel; populate it while
/// it's open.
fn update_stocks(
    screen: Res<ScreenRes>,
    mut show: ResMut<ShowStocks>,
    mut active: ResMut<ActiveTool>,
    sim: Res<SimRes>,
    mut button: Query<
        (&Interaction, &mut WasPressed, &mut BackgroundColor, &mut Visibility),
        (With<StocksButton>, Without<StocksPanel>),
    >,
    mut panel: Query<&mut Visibility, (With<StocksPanel>, Without<StocksButton>)>,
    mut text: Query<&mut Text, With<StocksPanelText>>,
) {
    let playing = screen.0 == Screen::Playing;
    if let Ok((interaction, mut was, mut bg, mut vis)) = button.single_mut() {
        *vis = if playing { Visibility::Inherited } else { Visibility::Hidden };
        if playing {
            let hovered = !matches!(interaction, Interaction::None);
            let pressed = matches!(interaction, Interaction::Pressed);
            if pressed && !was.0 {
                show.0 = !show.0;
            }
            was.0 = pressed;
            if hovered {
                active.over_ui = true;
            }
            *bg = BackgroundColor(tool_bg(2, show.0, hovered));
        }
    }
    let open = playing && show.0;
    if let Ok(mut vis) = panel.single_mut() {
        *vis = if open { Visibility::Inherited } else { Visibility::Hidden };
    }
    if open {
        if let Some(sim) = sim.0.as_ref() {
            let mut s = String::from("STOCKS\n");
            for (group, kinds) in STOCK_GROUPS {
                let mut rows = String::new();
                for &k in *kinds {
                    let n = sim.count_kind(k);
                    if n > 0 {
                        rows.push_str(&format!("   {n:>4}  {}\n", item_kind_name(k)));
                    }
                }
                if !rows.is_empty() {
                    s.push_str(&format!("\n{group}\n{rows}"));
                }
            }
            if let Ok(mut t) = text.single_mut() {
                t.0 = s;
            }
        }
    }
}

/// The "?" button opens the controls-reference (Help) screen, like F1.
fn handle_help_button(
    mut screen: ResMut<ScreenRes>,
    mut legends: ResMut<LegendsState>,
    mut active: ResMut<ActiveTool>,
    mut button: Query<
        (&Interaction, &mut WasPressed, &mut BackgroundColor, &mut Visibility),
        With<HelpButton>,
    >,
) {
    let playing = screen.0 == Screen::Playing;
    if let Ok((interaction, mut was, mut bg, mut vis)) = button.single_mut() {
        *vis = if playing { Visibility::Inherited } else { Visibility::Hidden };
        if playing {
            let hovered = !matches!(interaction, Interaction::None);
            let pressed = matches!(interaction, Interaction::Pressed);
            if pressed && !was.0 {
                legends.from = Screen::Playing;
                screen.0 = Screen::Help;
            }
            was.0 = pressed;
            if hovered {
                active.over_ui = true;
            }
            *bg = BackgroundColor(tool_bg(3, false, hovered));
        }
    }
}

/// Show the toolbar and tooltip only on the play screen.
fn toolbar_visibility(
    screen: Res<ScreenRes>,
    mut roots: Query<&mut Visibility, With<ToolbarRoot>>,
    mut tips: Query<&mut Visibility, (With<TooltipUi>, Without<ToolbarRoot>)>,
) {
    if !screen.is_changed() {
        return;
    }
    let vis = if screen.0 == Screen::Playing { Visibility::Inherited } else { Visibility::Hidden };
    for mut v in &mut roots {
        *v = vis;
    }
    for mut v in &mut tips {
        *v = vis;
    }
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
                // The world's own clock is now the authority on what year it
                // is — it advances alongside the fort's (see sync_world), so
                // adding the fort's year on top would count it twice.
                let year = world.0.years_simulated;
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
                    UiKind::Pile(cat) => {
                        sim.0.add_filtered_stockpile(anchor, here, dk_agents::StockFilter::only(&[cat]))
                    }
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
                    UiKind::Bedroom => sim.0.add_bedroom(anchor, here),
                    UiKind::Chop => {
                        sim.0.designate_rect(DesignationKind::Chop, anchor, here);
                    }
                    UiKind::Gather => {
                        sim.0.designate_rect(DesignationKind::Gather, anchor, here);
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
    // Shift+G: designate wild shrubs for foraging (two-press rectangle; only
    // tiles with a berry shrub are marked).
    if shift && keys.just_pressed(KeyCode::KeyG) {
        let here = cursor.pos(view_z.0);
        match mode.0 {
            Some((UiKind::Gather, anchor)) if anchor.z == here.z => {
                sim.0.designate_rect(DesignationKind::Gather, anchor, here);
                mode.0 = None;
            }
            _ => mode.0 = Some((UiKind::Gather, here)),
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
    // Shift+J: build a carpenter's workshop (Shift keeps it clear of the loom).
    if shift && keys.just_pressed(KeyCode::KeyJ) {
        let here = cursor.pos(view_z.0);
        if sim.0.add_building(BuildingKind::Carpenter, here) {
            info!("built a Carpenter's Workshop at {:?}", here);
        } else {
            warn!("can't build a Carpenter's Workshop there");
        }
        dirty.0 = true;
    }
    // Shift+N: build a tanner's shop (Shift keeps it clear of the pasture designation).
    if shift && keys.just_pressed(KeyCode::KeyN) {
        let here = cursor.pos(view_z.0);
        if sim.0.add_building(BuildingKind::Tanner, here) {
            info!("built a Tanner's Shop at {:?}", here);
        } else {
            warn!("can't build a Tanner's Shop there");
        }
        dirty.0 = true;
    }
    // Shift+P: dig a well (Shift keeps it clear of the stockpile designation).
    if shift && keys.just_pressed(KeyCode::KeyP) {
        let here = cursor.pos(view_z.0);
        if sim.0.add_building(BuildingKind::Well, here) {
            info!("dug a Well at {:?}", here);
        } else {
            warn!("can't dig a Well there");
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
/// The meadow's grass palettes — base greens laid out in soft patches, each
/// shaded further by a per-tile mottle.
const GRASS_TYPES: [[f32; 3]; 4] = [
    [0.27, 0.56, 0.22], // meadow green
    [0.23, 0.53, 0.21], // lush deep green
    [0.33, 0.55, 0.21], // dry olive
    [0.26, 0.57, 0.28], // cool fescue
];
/// Coat colours so a herd isn't a rank of identical beasts.
const COW_COATS: [[f32; 3]; 4] = [
    [1.00, 0.96, 0.90], // pale
    [0.72, 0.52, 0.36], // brown
    [0.52, 0.44, 0.40], // dark
    [0.90, 0.82, 0.70], // fawn
];
const SHEEP_COATS: [[f32; 3]; 3] = [
    [1.00, 0.98, 0.94], // white
    [0.80, 0.78, 0.74], // grey
    [0.56, 0.53, 0.50], // black
];
const DOG_COATS: [[f32; 3]; 4] = [
    [0.70, 0.52, 0.36], // brown
    [0.47, 0.40, 0.36], // near-black
    [0.90, 0.80, 0.62], // tan
    [0.84, 0.84, 0.82], // grey
];

/// Gentle per-citizen tints (near white) so a fort's dwarves read as
/// individuals in their own homespun rather than a rank of identical figures.
const DWARF_TINTS: [[f32; 3]; 6] = [
    [1.00, 0.92, 0.80], // warm tan
    [0.85, 0.90, 1.00], // cool blue-grey
    [0.97, 0.84, 0.84], // dusty rose
    [0.86, 0.96, 0.86], // sage
    [1.00, 0.87, 0.70], // ochre
    [0.91, 0.86, 0.97], // lavender
];

/// Textured ground sprites, chosen per-tile so the field doesn't read as a
/// flat colored grid. Grass, bare earth, and stone each have a few variants.
const GRASS_SPRITES: [&str; 3] = ["grass_a", "grass_b", "grass_c"];
const DIRT_SPRITES: [&str; 2] = ["dirt_a", "dirt_b"];
const ROCK_SPRITES: [&str; 2] = ["rock_a", "rock_b"];

/// Wildflower colors sprinkled sparsely through the grass.
const FLOWER_COLORS: [[f32; 3]; 4] = [
    [0.85, 0.22, 0.24], // red poppy
    [0.95, 0.82, 0.28], // yellow buttercup
    [0.64, 0.36, 0.80], // violet
    [0.93, 0.93, 0.88], // white daisy
];

/// The topmost non-empty tile of a neighbouring column at or just below
/// `view_z` — its material and water. Lets the renderer bleed that terrain
/// across the seam (a river sunk a level below still counts as a wet edge).
fn surface_at(sim: &Sim, x: i32, y: i32, view_z: i32) -> Option<(u16, u8)> {
    for dz in 0..4 {
        let z = view_z - dz;
        if z < 0 {
            break;
        }
        if let Some(t) = sim.map.tile_at(Pos::new(x, y, z)) {
            if t.shape != TileShape::Empty {
                return Some((t.material, t.water));
            }
        }
    }
    None
}

fn tile_visual(
    sim: &Sim,
    raws: &Raws,
    x: i32,
    y: i32,
    view_z: i32,
    selection: Option<(Pos, Pos)>,
    traffic: &std::collections::HashMap<(i32, i32), f32>,
) -> (Color, &'static str) {
    const DIM: [f32; 4] = [1.0, 0.55, 0.34, 0.20];
    let mut rgb = [0.02, 0.02, 0.03];
    let mut glyph = "block";
    let season = sim.clock.season();
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
        // Water fills its tile wherever it's found — including a river sunk a
        // level or two below the surface — depth-graded from pale shallows to
        // dark blue, dimmed by how far down it lies.
        if tile.water > 0 {
            let d = (tile.water as f32 / 7.0).min(1.0);
            let mut wcol = mix([0.36, 0.62, 0.68], [0.07, 0.22, 0.62], d);
            // Shallows: water touching land is paler and greener at the shore.
            let mut land = 0;
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                match surface_at(sim, x + dx, y + dy, z) {
                    Some((_, nw)) if nw > 0 => {}
                    Some(_) => land += 1,
                    None => {}
                }
            }
            if land > 0 {
                wcol = mix(wcol, [0.44, 0.60, 0.50], 0.18 + 0.12 * land as f32);
            }
            // Winter locks the surface into pale ice.
            if matches!(season, Season::Winter) {
                wcol = mix(wcol, [0.74, 0.83, 0.89], 0.55);
            }
            rgb = [wcol[0] * factor, wcol[1] * factor, wcol[2] * factor];
            glyph = "water";
            break;
        }
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
    // Connected walls: a solid tile is shaded by how buried it is — rock ringed
    // by more rock sits in shadow, while an exposed face at the edge of a dig
    // catches the light — so wall masses read as one connected relief instead
    // of a grid of identical squares. (Off-map neighbours count as more rock.)
    if sim.map.tile_at(here).is_some_and(|t| t.is_solid()) {
        let mut open = 0;
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let solid = sim
                .map
                .tile_at(Pos::new(x + dx, y + dy, view_z))
                .map_or(true, |n| n.is_solid());
            if !solid {
                open += 1;
            }
        }
        let lit = 0.60 + 0.15 * open as f32; // 0.60 buried .. 1.20 isolated
        rgb = [rgb[0] * lit, rgb[1] * lit, rgb[2] * lit];
    }
    // Grassy groundcover over open, grass-bearing soil (loam and clay of the
    // plains and forests — never the bare desert sand): a meadow of a few grass
    // types, dappled with wildflowers, and turning with the seasons — fresh in
    // spring, gold in autumn, snow-dusted in winter. Purely cosmetic and
    // deterministic per-tile; trees, farms, zones and buildings all paint over
    // it. (x, y are in-bounds and non-negative here, so the hashes stay small
    // and positive.)
    if sim.map.water_at(here) == 0 && sim.map.walkable(here) {
        if let Some(t) = sim.map.tile_at(here) {
            let m = raws.materials.get(t.material);
            // Textured ground sprites (a few variants each) only stand in for a
            // plain floor; ramps and stairs keep their own shape but still take
            // the colour below.
            let is_floor = t.shape == TileShape::Floor;
            if m.category == MaterialCategory::Soil && m.id != "sand" {
                // Grass: its colour varies in broad, soft PATCHES (coarse
                // coords, not per-tile) so the meadow reads as a cohesive field
                // rather than a checkerboard, while the sprite texture still
                // varies tile to tile. Takes the season's cast — and in winter a
                // snow blanket that all but hides it.
                let kind = ((x / 5) * 6151 + (y / 5) * 3079).rem_euclid(4) as usize;
                let mottle = ((x / 3) * 7919 + (y / 3) * 1049).rem_euclid(8) as f32 / 7.0;
                let base = GRASS_TYPES[kind];
                let green = [
                    base[0] + 0.03 * mottle,
                    base[1] + 0.05 * mottle,
                    base[2] + 0.03 * mottle,
                ];
                let (green, cover) = match season {
                    Season::Spring => (mix(green, [0.40, 0.72, 0.32], 0.28), 0.72),
                    Season::Summer => (green, 0.72),
                    Season::Autumn => (mix(green, [0.68, 0.50, 0.18], 0.55), 0.72),
                    Season::Winter => (mix(green, [0.90, 0.93, 0.97], 0.82), 0.85),
                };
                rgb = mix(rgb, green, cover);
                let wear = traffic.get(&(x, y)).copied().unwrap_or(0.0);
                if is_floor && wear > 0.7 {
                    // Trodden bare: a worn-earth path where the fort walks most.
                    let k = ((wear - 0.7) / 2.4).clamp(0.0, 0.82);
                    rgb = mix(rgb, [0.34, 0.26, 0.16], k);
                    glyph = DIRT_SPRITES[(x * 40_503 + y * 1259).rem_euclid(2) as usize];
                } else if is_floor {
                    glyph = GRASS_SPRITES[(x * 6151 + y * 3079).rem_euclid(3) as usize];
                    // Ground clutter breaks up the meadow: the odd leafy bush or
                    // a stone, and in the warm seasons wildflowers — sparse, so
                    // it dapples rather than crowds.
                    let c = (x * 769 + y * 1109).rem_euclid(27);
                    if c == 0 {
                        glyph = "bush";
                        rgb = mix(rgb, [0.14, 0.33, 0.13], 0.5);
                    } else if c == 1 {
                        glyph = "stone";
                        rgb = mix(rgb, [0.50, 0.48, 0.44], 0.6);
                    } else if matches!(season, Season::Spring | Season::Summer) {
                        let f = x * 1_299_709 + y * 1301;
                        if f.rem_euclid(13) == 0 {
                            let bloom = FLOWER_COLORS
                                [(f / 13).rem_euclid(FLOWER_COLORS.len() as i32) as usize];
                            rgb = mix(rgb, bloom, 0.55);
                            glyph = "crop";
                        }
                    }
                }
            } else if is_floor {
                // Bare ground keeps the terrain's own colour but gains texture:
                // pebbled earth on soil/sand, cracked stone on rock — with the
                // occasional loose stone scattered on top.
                glyph = if m.category == MaterialCategory::Soil {
                    DIRT_SPRITES[(x * 40_503 + y * 1259).rem_euclid(2) as usize]
                } else {
                    ROCK_SPRITES[(x * 15_731 + y * 789).rem_euclid(2) as usize]
                };
                if (x * 769 + y * 1109).rem_euclid(19) == 0 {
                    glyph = "stone";
                    rgb = mix(rgb, [0.52, 0.50, 0.46], 0.45);
                }
            }
            // De-box: bleed neighbouring terrain across the seam so the hard
            // tile boundaries between grass, earth, rock and water soften into
            // ragged, organic edges. Only plain floors, and only some tiles (a
            // per-tile roll), so the bleed is uneven and natural — a damp sandy
            // bank where water laps the shore, a wash of the other stone or
            // soil where two grounds meet.
            if t.shape == TileShape::Floor && (x * 271 + y * 331).rem_euclid(7) < 4 {
                let here_cat = m.category;
                let mut bank = false;
                for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    let Some((nmat, nwater)) = surface_at(sim, x + dx, y + dy, view_z) else {
                        continue;
                    };
                    if nwater > 0 {
                        rgb = mix(rgb, [0.60, 0.55, 0.42], 0.30);
                        bank = true;
                    } else {
                        let nm = raws.materials.get(nmat);
                        if nm.category != here_cat {
                            let c = nm.color;
                            let nc = [
                                c[0] as f32 / 255.0 * 0.55,
                                c[1] as f32 / 255.0 * 0.55,
                                c[2] as f32 / 255.0 * 0.55,
                            ];
                            rgb = mix(rgb, nc, 0.22);
                        }
                    }
                }
                // Reeds fringe a grassy bank (not the desert sand or bare rock),
                // and die back to none under winter's ice.
                let grassy_bank = bank
                    && here_cat == MaterialCategory::Soil
                    && m.id != "sand"
                    && !matches!(season, Season::Winter);
                if grassy_bank && (x * 457 + y * 613).rem_euclid(3) == 0 {
                    glyph = "reed";
                    rgb = mix(rgb, [0.19, 0.42, 0.18], 0.5);
                }
            }
        }
    }
    if let Some(farm) = sim.farms.get(&here) {
        let (tint, k) = match farm.state {
            FarmState::Fallow => ([0.3, 0.4, 0.18], 0.4),
            FarmState::Growing { .. } => ([0.25, 0.6, 0.2], 0.45),
            FarmState::Grown => ([0.45, 0.9, 0.3], 0.55),
        };
        rgb = mix(rgb, tint, k);
        glyph = "farm";
    }
    if let Some(species) = sim.tree_species(here) {
        // A tree standing on the surface, its canopy turning with the season —
        // green in the warm months, ablaze in autumn (each wood its own hue),
        // bare in winter — unless it's an evergreen. Amber once marked to fell.
        // Its silhouette follows its species: a spiky conifer, a weeping
        // willow, a slender birch, or the broad round canopy of the rest.
        let mat = raws.materials.get(species);
        glyph = match mat.id.as_str() {
            "pine" => "tree_conifer",
            "willow" => "tree_willow",
            "birch" => "tree_birch",
            _ => "tree",
        };
        let marked = matches!(
            sim.designations.get(&here).map(|d| d.kind),
            Some(DesignationKind::Chop)
        );
        if marked {
            rgb = mix(rgb, [0.85, 0.5, 0.12], 0.75);
        } else {
            let wood = mat.color;
            let wood = [wood[0] as f32 / 255.0, wood[1] as f32 / 255.0, wood[2] as f32 / 255.0];
            let evergreen = mat.id == "pine";
            let canopy = if evergreen {
                // A conifer holds its dark needles all year, snow-laden in winter.
                let green = mix([0.10, 0.34, 0.15], wood, 0.15);
                if matches!(season, Season::Winter) {
                    mix(green, [0.80, 0.85, 0.90], 0.30)
                } else {
                    green
                }
            } else {
                match season {
                    Season::Spring => mix([0.20, 0.56, 0.18], wood, 0.2),
                    Season::Summer => mix([0.16, 0.5, 0.14], wood, 0.25),
                    // Autumn fire, pulled strongly toward the wood's own color
                    // so maple burns red, birch yellows, oak goes russet.
                    Season::Autumn => mix([0.74, 0.42, 0.10], wood, 0.45),
                    // Bare branches under snow.
                    Season::Winter => mix([0.42, 0.34, 0.27], [0.82, 0.85, 0.90], 0.35),
                }
            };
            rgb = mix(rgb, canopy, 0.8);
        }
    }
    if sim.shrub_at(here) {
        // A wild berry shrub — low and berry-red, or amber once a forager has
        // marked it to be gathered.
        let marked = matches!(
            sim.designations.get(&here).map(|d| d.kind),
            Some(DesignationKind::Gather)
        );
        let tint = if marked { [0.85, 0.5, 0.12] } else { [0.5, 0.18, 0.32] };
        rgb = mix(rgb, tint, 0.7);
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
            BuildingKind::Carpenter => [0.55, 0.4, 0.22],
            BuildingKind::Tanner => [0.6, 0.45, 0.3],
            BuildingKind::Well => [0.3, 0.5, 0.85],
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
            BuildingKind::Craftsdwarf => "ws_bench",
            BuildingKind::Loom => "ws_loom",
            BuildingKind::Jeweler => "artifact",
            BuildingKind::Forge => "ws_forge",
            BuildingKind::Smelter => "ws_furnace",
            BuildingKind::Mason => "ws_mason",
            BuildingKind::Clothier => "ws_cloth",
            BuildingKind::Carpenter => "ws_carpenter",
            BuildingKind::Tanner => "ws_hides",
            BuildingKind::Well => "ws_well",
            BuildingKind::GlassFurnace => "ws_furnace",
            BuildingKind::Trap => "weapon",
        };
    }
    // Relief shadows: light falls from the upper-left, so a wall throws a soft
    // shadow onto the open ground at its lower-right. Only open (walkable) tiles
    // catch a shadow; walls, water and magma below take their own overlays. This
    // gives the flat grid a sense of raised stone and depth.
    if sim.map.walkable(here) && sim.map.water_at(here) == 0 {
        let mut shade = 0.0f32;
        for (dx, dy, s) in [(-1, 0, 0.13f32), (0, 1, 0.13), (-1, 1, 0.08)] {
            let np = Pos::new(x + dx, y + dy, view_z);
            if sim.map.tile_at(np).is_some_and(|t| t.is_solid()) {
                shade += s;
            }
        }
        if shade > 0.0 {
            let f = 1.0 - shade;
            rgb = [rgb[0] * f, rgb[1] * f, rgb[2] * f];
        }
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
    // A pile is tinted by what it is for, so a fort's storage reads at a
    // glance: the larder green, the stoneyard grey, the armoury red. An
    // undirected pile keeps the old blue.
    if let Some(s) = sim.stockpile_at(here) {
        rgb = mix(rgb, pile_tint(sim.stockpiles[s].accepts), 0.35);
    }
    if sim.pastures.iter().any(|p| p.contains(here)) {
        rgb = mix(rgb, [0.35, 0.6, 0.25], 0.3);
    }
    if sim.bedroom_at(here) {
        rgb = mix(rgb, [0.85, 0.6, 0.4], 0.28);
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
    active: Res<ActiveTool>,
    traffic: Res<Traffic>,
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
            // Shade by elevation so the land reads as relief: valleys sit
            // darker, high ground brightens, and the highest peaks catch a
            // dusting of snow — mountains stand out at a glance.
            if !matches!(region.biome, dk_history::Biome::Ocean) {
                let e = region.elevation;
                let shade = (0.72 + 0.5 * (e - 0.35)).clamp(0.55, 1.15);
                rgb = [rgb[0] * shade, rgb[1] * shade, rgb[2] * shade];
                if e > 0.78 {
                    let snow = ((e - 0.78) / 0.22).clamp(0.0, 0.6);
                    rgb = mix(rgb, [0.92, 0.94, 0.98], snow);
                }
            }
            // A river threads blue across the land; a basin holds a lake — so
            // you can see the water and settle beside it.
            if region.lake {
                rgb = mix(rgb, [0.14, 0.34, 0.72], 0.72);
            } else if region.river {
                rgb = mix(rgb, [0.22, 0.46, 0.82], 0.62);
            }
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
    // Preview the rectangle being drawn — from the keyboard mode's anchor, or
    // the toolbar tool's anchor — to the cursor.
    let selection = mode
        .0
        .map(|(_, anchor)| anchor)
        .or(active.anchor)
        .map(|anchor| (anchor, cursor.pos(view_z.0)));
    for (t, mut sprite) in &mut tiles {
        let (color, glyph) =
            tile_visual(sim, &reg.0, t.x as i32, t.y as i32, view_z.0, selection, &traffic.0);
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
        ItemKind::Berry => "berries".to_string(),
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
        ItemKind::Log => format!("{} log", raws.materials.get(it.stuff).name),
        ItemKind::Barrel => "wooden barrel".to_string(),
        ItemKind::Bin => "wooden bin".to_string(),
        ItemKind::Statue => format!("{} statue", raws.materials.get(it.stuff).name),
        ItemKind::Instrument => "musical instrument".to_string(),
        ItemKind::Hide => "raw hide".to_string(),
        ItemKind::Leather => "leather".to_string(),
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
        ItemKind::Berry => Color::srgb(0.72, 0.16, 0.42),
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
        ItemKind::Log => item_material_color(raws, stuff),
        ItemKind::Barrel => Color::srgb(0.62, 0.44, 0.24),
        ItemKind::Bin => Color::srgb(0.50, 0.37, 0.21),
        ItemKind::Statue => item_material_color(raws, stuff),
        ItemKind::Instrument => Color::srgb(0.72, 0.52, 0.3),
        ItemKind::Hide => Color::srgb(0.66, 0.5, 0.36),
        ItemKind::Leather => Color::srgb(0.55, 0.38, 0.24),
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
                        // Traders wear the road's gold dust; beasts loom dark;
                        // soldiers take a steel sheen; other citizens each wear
                        // their own homespun tint so the fort reads as a crowd
                        // of individuals.
                        sprite.color = if d.beast {
                            Color::srgb(0.5, 0.1, 0.15)
                        } else if d.soldier {
                            Color::srgb(0.7, 0.8, 1.0)
                        } else {
                            match d.faction {
                                Faction::Visitor => Color::srgb(1.0, 0.85, 0.55),
                                _ => {
                                    let t = DWARF_TINTS[i % DWARF_TINTS.len()];
                                    Color::srgb(t[0], t[1], t[2])
                                }
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
            // A packed item sits on its container's tile: draw the barrel, not
            // the meals inside it. Carried goods ride out of sight too.
            Some(it)
                if it.active()
                    && it.pos.z == view_z.0
                    && !matches!(
                        it.state,
                        ItemState::Carried { .. } | ItemState::Inside { .. }
                    ) =>
            {
                tf.translation.x = it.pos.x as f32 * TILE;
                tf.translation.y = it.pos.y as f32 * TILE;
                match &tileset.0 {
                    Some(ts) => {
                        let glyph = match it.kind {
                            ItemKind::Boulder => "boulder",
                            ItemKind::Seed => "seed",
                            ItemKind::Crop => "crop",
                            ItemKind::Berry => "crop",
                            ItemKind::Meal => "meal",
                            ItemKind::Drink => "drink",
                            ItemKind::Artifact => "artifact",
                            ItemKind::Corpse => "dwarf",
                            ItemKind::Craft => "i_craft",
                            ItemKind::Wool => "i_wool",
                            ItemKind::Cloth => "i_cloth",
                            ItemKind::RoughGem | ItemKind::CutGem => "i_gem",
                            ItemKind::Weapon => "weapon",
                            ItemKind::Glass => "i_glass",
                            ItemKind::Bar => "i_bar",
                            ItemKind::Armor => "i_armor",
                            ItemKind::Bed => "i_bed",
                            ItemKind::Clothes => "i_clothes",
                            ItemKind::Log => "i_log",
                            ItemKind::Barrel => "i_barrel",
                            ItemKind::Bin => "i_bin",
                            ItemKind::Statue => "i_statue",
                            ItemKind::Instrument => "i_instrument",
                            ItemKind::Hide => "i_leather",
                            ItemKind::Leather => "i_leather",
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
                    // Each beast wears its own coat, so a herd looks like one.
                    let coat = match a.kind {
                        AnimalKind::Cow => COW_COATS[i % COW_COATS.len()],
                        AnimalKind::Sheep => SHEEP_COATS[i % SHEEP_COATS.len()],
                        AnimalKind::Dog => DOG_COATS[i % DOG_COATS.len()],
                    };
                    Color::srgb(coat[0], coat[1], coat[2])
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
            // Price a container with its contents, exactly as `Sim::trade`
            // does — otherwise the screen quotes 45 for a barrel the sim
            // charges 525 for, and the player ships a cellar unawares.
            let offered: u32 = trade.offer.iter().map(|&i| sim.stack_value(i, &reg.0)).sum();
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
                // A full barrel must read as a full barrel: the contents go
                // with it, so name them and price them in.
                let held = if dk_agents::is_container(it.kind) {
                    let c = sim.contents_of(i);
                    match c.first() {
                        Some(&f) => format!(
                            " [{} {}]",
                            c.len(),
                            item_kind_name(sim.items[f].kind)
                        ),
                        None => String::new(),
                    }
                } else {
                    String::new()
                };
                right.push_str(&format!(
                    "{cur}{sel} {}{} ({})\n",
                    item_label(&reg.0, it),
                    held,
                    sim.stack_value(i, &reg.0)
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
    // Say what a pile is for — "stockpile (food)" — so the player can tell
    // their larder from their stoneyard without guessing at the tint.
    if let Some(s) = sim.0.stockpile_at(here) {
        let accepts = sim.0.stockpiles[s].accepts;
        let what = if accepts.takes_everything() {
            "anything".to_string()
        } else {
            accepts
                .categories()
                .iter()
                .map(|c| c.name())
                .collect::<Vec<_>>()
                .join(", ")
        };
        under = format!("{under} · stockpile ({what})");
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
    // A container and its contents share a tile, so describe the container —
    // never one of the things packed inside it. Contents are summarized below.
    if let Some((idx, it)) = sim
        .0
        .items
        .iter()
        .enumerate()
        .filter(|(_, i)| {
            i.active()
                && i.pos == here
                && !matches!(i.state, ItemState::Carried { .. } | ItemState::Inside { .. })
        })
        .min_by_key(|(_, i)| !dk_agents::is_container(i.kind))
    {
        let what = match it.kind {
            ItemKind::Boulder => format!("{} boulder", reg.0.materials.get(it.stuff).name),
            ItemKind::Seed => format!("{} seeds", reg.0.plants.get(it.stuff).name),
            ItemKind::Crop => reg.0.plants.get(it.stuff).name.clone(),
            ItemKind::Berry => "a heap of berries".to_string(),
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
            ItemKind::Log => format!("{} log", reg.0.materials.get(it.stuff).name),
            ItemKind::Barrel => "wooden barrel".to_string(),
            ItemKind::Bin => "wooden bin".to_string(),
            ItemKind::Statue => format!("{} statue (a work of art)", reg.0.materials.get(it.stuff).name),
            ItemKind::Instrument => "musical instrument (trade good)".to_string(),
            ItemKind::Hide => "raw hide".to_string(),
            ItemKind::Leather => "leather (trade good)".to_string(),
        };
        let what = if it.quality > 0 && it.kind != ItemKind::Artifact {
            format!("{} {what}", dk_agents::quality_name(it.quality))
        } else {
            what
        };
        // "wooden barrel (12 prepared meals)" — a container is only as
        // interesting as what it holds, so say so.
        let what = if dk_agents::is_container(it.kind) {
            let held = sim.0.contents_of(idx);
            match held.first() {
                Some(&f) => format!(
                    "{what} ({} {})",
                    held.len(),
                    item_kind_name(sim.0.items[f].kind)
                ),
                None => format!("{what} (empty)"),
            }
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
             harvested {}   cooked {}   brewed {}   gems {}/{}   migrants {}   raiders {} ({} slain, {} drowned)   beasts slain {}   veterans {}   armed {}   armored {}   poems {}   songs {}{}\n\
             Build & dig from the toolbar below (or press its key) -- click a tool, then click the map.   l:lever  t:pull   F1: full controls\n\
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
            sim.0.songs.len(),
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
