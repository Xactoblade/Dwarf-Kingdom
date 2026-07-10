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
    load_sim, save_sim, BuildingKind, DesignationKind, FarmState, ItemKind, ItemState, Sim,
};
use dk_raws::Raws;
use dk_world::path::Pos;
use dk_world::TileShape;
use std::path::{Path, PathBuf};

const TILE: f32 = 12.0;
const MAP_W: usize = 96;
const MAP_H: usize = 96;
const MAP_D: usize = 32;
const WORLD_SEED: u64 = 20260710;
const DWARF_COUNT: usize = 7;

// ---------------------------------------------------------------- resources

#[derive(Resource)]
struct Registry(Raws);

#[derive(Resource)]
struct SimRes(Sim);

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
    Stockpile,
    Farm,
    Cancel,
}

impl UiKind {
    fn label(self) -> &'static str {
        match self {
            UiKind::Mine => "MINE",
            UiKind::Stairs => "STAIRS",
            UiKind::Stockpile => "STOCKPILE",
            UiKind::Farm => "FARM",
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

fn screenshot_mode_on() -> bool {
    std::env::var_os("DK_SCREENSHOT").is_some()
}

fn main() {
    let raws = Raws::load(&data_dir()).expect("failed to load raws");
    let mut rng = dk_core::rng_from_seed(WORLD_SEED);
    let map = dk_world::generate(&raws.materials, &mut rng, MAP_W, MAP_H, MAP_D, WORLD_SEED);
    let mut sim = Sim::new(map, &raws, rng, DWARF_COUNT);
    sim.add_embark_supplies(&raws);
    if screenshot_mode_on() {
        demo_scenario(&mut sim, &raws);
    }
    let start_z = sim
        .map
        .walk_surface_z(MAP_W / 2, MAP_H / 2)
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
            DefaultPlugins.set(WindowPlugin {
                primary_window: Some(window),
                ..default()
            }),
            FrameTimeDiagnosticsPlugin::default(),
        ))
        .insert_resource(ClearColor(Color::srgb(0.04, 0.04, 0.06)))
        .insert_resource(Time::<Fixed>::from_hz(sim_hz))
        .insert_resource(Registry(raws))
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
                handle_mouse,
                handle_input,
                overlay_refresh,
                redraw_tiles,
                sync_agent_sprites,
                position_cursor_sprite,
                update_hud,
                screenshot_mode,
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
    }
    sim.place_flat_stockpiles(cx, cy, 36);
}

fn setup(mut commands: Commands) {
    let center = Vec3::new(
        MAP_W as f32 * TILE * 0.5,
        MAP_H as f32 * TILE * 0.5,
        999.0,
    );
    commands.spawn((Camera2d, Transform::from_translation(center)));

    for y in 0..MAP_H {
        for x in 0..MAP_W {
            commands.spawn((
                Sprite {
                    color: Color::BLACK,
                    custom_size: Some(Vec2::splat(TILE - 1.0)),
                    ..default()
                },
                Transform::from_xyz(x as f32 * TILE, y as f32 * TILE, 0.0),
                TileSprite { x, y },
            ));
        }
    }

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
    mut dirty: ResMut<MapDirty>,
) {
    if control.paused {
        return;
    }
    sim.0.step(&reg.0);
    if sim.0.map_changed {
        sim.0.map_changed = false;
        dirty.0 = true;
    }
}

/// Farm growth and stockpile contents change tile tints slowly; refresh the
/// overlay layer once a second instead of every frame.
fn overlay_refresh(
    time: Res<Time>,
    mut timer: ResMut<OverlayRefresh>,
    mut dirty: ResMut<MapDirty>,
) {
    if timer.0.tick(time.delta()).just_finished() {
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
}

#[allow(clippy::too_many_arguments)]
fn handle_input(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    reg: Res<Registry>,
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

    // Rectangle modes. First press anchors, second press applies.
    for (key, kind) in [
        (KeyCode::KeyD, UiKind::Mine),
        (KeyCode::KeyX, UiKind::Stairs),
        (KeyCode::KeyP, UiKind::Stockpile),
        (KeyCode::KeyF, UiKind::Farm),
        (KeyCode::KeyC, UiKind::Cancel),
    ] {
        if !keys.just_pressed(key) {
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
                    UiKind::Stockpile => sim.0.add_stockpile(anchor, here),
                    UiKind::Farm => {
                        sim.0.add_farm(anchor, here, 0);
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
    ] {
        if keys.just_pressed(key) {
            let here = cursor.pos(view_z.0);
            if sim.0.add_building(kind, here) {
                info!("built a {} at {:?}", kind.name(), here);
            } else {
                warn!("can't build a {} there", kind.name());
            }
            dirty.0 = true;
        }
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
            tf.scale *= 0.8;
        }
        if keys.just_pressed(KeyCode::Minus) {
            tf.scale *= 1.25;
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
                sim.0 = loaded;
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

fn mix(base: [f32; 3], tint: [f32; 3], k: f32) -> [f32; 3] {
    [
        base[0] * (1.0 - k) + tint[0] * k,
        base[1] * (1.0 - k) + tint[1] * k,
        base[2] * (1.0 - k) + tint[2] * k,
    ]
}

/// Color for a map position as seen from `view_z`, including overlays.
fn tile_color(
    sim: &Sim,
    raws: &Raws,
    x: i32,
    y: i32,
    view_z: i32,
    selection: Option<(Pos, Pos)>,
) -> Color {
    const DIM: [f32; 4] = [1.0, 0.55, 0.34, 0.20];
    let mut rgb = [0.02, 0.02, 0.03];
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
            TileShape::Solid => 1.0,
            TileShape::Ramp => 0.8,
            TileShape::Stairs => 0.7,
            TileShape::Floor => 0.55,
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
    }
    if let Some(b) = sim.building_at(here) {
        let tint = match b.kind {
            BuildingKind::Still => [0.85, 0.5, 0.22],
            BuildingKind::Kitchen => [0.8, 0.25, 0.2],
        };
        rgb = mix(rgb, tint, 0.6);
    }
    if sim.designations.contains_key(&here) {
        rgb = mix(rgb, [1.0, 0.62, 0.12], 0.45);
    }
    if sim.stockpile_at(here).is_some() {
        rgb = mix(rgb, [0.25, 0.45, 0.9], 0.35);
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
    Color::srgb(rgb[0], rgb[1], rgb[2])
}

fn redraw_tiles(
    mut dirty: ResMut<MapDirty>,
    sim: Res<SimRes>,
    reg: Res<Registry>,
    view_z: Res<ViewZ>,
    cursor: Res<Cursor>,
    mode: Res<UiMode>,
    mut tiles: Query<(&TileSprite, &mut Sprite)>,
) {
    if !dirty.0 {
        return;
    }
    dirty.0 = false;
    let selection = mode.0.map(|(_, anchor)| (anchor, cursor.pos(view_z.0)));
    for (ts, mut sprite) in &mut tiles {
        sprite.color = tile_color(&sim.0, &reg.0, ts.x as i32, ts.y as i32, view_z.0, selection);
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
    }
}

fn sync_agent_sprites(
    mut commands: Commands,
    sim: Res<SimRes>,
    reg: Res<Registry>,
    view_z: Res<ViewZ>,
    mut pools: ResMut<SpritePools>,
    mut sprites: Query<
        (&mut Transform, &mut Sprite, &mut Visibility),
        (Without<TileSprite>, Without<CursorSprite>),
    >,
) {
    while pools.dwarves.len() < sim.0.dwarves.len() {
        pools.dwarves.push(
            commands
                .spawn((
                    Sprite {
                        color: Color::srgb(0.93, 0.79, 0.55),
                        custom_size: Some(Vec2::splat(TILE * 0.72)),
                        ..default()
                    },
                    Transform::from_xyz(0.0, 0.0, 2.0),
                    Visibility::Hidden,
                ))
                .id(),
        );
    }
    while pools.items.len() < sim.0.items.len() {
        pools.items.push(
            commands
                .spawn((
                    Sprite {
                        color: Color::WHITE,
                        custom_size: Some(Vec2::splat(TILE * 0.4)),
                        ..default()
                    },
                    Transform::from_xyz(0.0, 0.0, 1.5),
                    Visibility::Hidden,
                ))
                .id(),
        );
    }

    for (i, &e) in pools.dwarves.iter().enumerate() {
        let Ok((mut tf, _, mut vis)) = sprites.get_mut(e) else { continue };
        match sim.0.dwarves.get(i) {
            Some(d) if d.alive && d.pos.z == view_z.0 => {
                tf.translation.x = d.pos.x as f32 * TILE;
                tf.translation.y = d.pos.y as f32 * TILE;
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
                sprite.color = item_color(&reg.0, it.kind, it.stuff);
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
    view_z: Res<ViewZ>,
    cursor: Res<Cursor>,
    control: Res<SimControl>,
    mode: Res<UiMode>,
    diagnostics: Res<DiagnosticsStore>,
    mut q: Query<&mut Text, With<HudText>>,
) {
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
            let mut s = format!(
                "\n{} — {} · happiness {:.0} · hunger {:.0} thirst {:.0}",
                d.name,
                d.task_name(),
                d.happiness,
                d.hunger,
                d.thirst
            );
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
    let status = if control.paused {
        "PAUSED".to_string()
    } else {
        format!("speed {}", control.speed)
    };
    let mode_txt = mode
        .0
        .map(|(k, _)| format!("   [{} — move cursor, press key again to apply]", k.label()))
        .unwrap_or_default();

    for mut text in &mut q {
        text.0 = format!(
            "Dwarf Kingdom :: Phase 2 :: Survive a Year\n\
             z {} / {}   cursor ({}, {})   {}\n\
             Year {}, {} {}   {}   {:.0} fps\n\
             dwarves {} ({} idle, {} lost)   meals {}   drinks {}   crops {}   jobs {}\n\
             harvested {}   cooked {}   brewed {}   migrants {}\n\
             d:mine x:stairs f:farm p:stockpile v:still k:kitchen c:cancel   space:pause 1/2/3:speed   [ ]:z   F5/F9:save/load   Q:quit{}{}",
            view_z.0,
            MAP_D - 1,
            cursor.x,
            cursor.y,
            under,
            cal.year(),
            cal.season().name(),
            cal.day_of_season(),
            status,
            fps,
            alive,
            idle,
            sim.0.stats.deaths,
            sim.0.count_kind(ItemKind::Meal),
            sim.0.count_kind(ItemKind::Drink),
            sim.0.count_kind(ItemKind::Crop),
            sim.0.pending_designations(),
            sim.0.stats.crops_harvested,
            sim.0.stats.meals_cooked,
            sim.0.stats.drinks_brewed,
            sim.0.stats.migrants_arrived,
            mode_txt,
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
