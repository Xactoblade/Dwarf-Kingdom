//! Dwarf Kingdom — Phase 0: z-level tile viewer over a data-driven map.
//!
//! Controls:
//!   Arrow keys ... move cursor        W/A/S/D ...... pan camera
//!   [ / ] ........ z-level down/up    - / = ........ zoom out/in
//!   F5 / F9 ...... save / load map    Esc .......... quit
//!
//! Set DK_SCREENSHOT=1 to capture `phase0.png` after ~1.5s and exit
//! (used for automated verification).

use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};
use dk_core::Calendar;
use dk_raws::MaterialRegistry;
use dk_world::{Map, TileShape};
use std::path::{Path, PathBuf};

const TILE: f32 = 12.0;
const MAP_W: usize = 64;
const MAP_H: usize = 64;
const MAP_D: usize = 24;
const WORLD_SEED: u64 = 20260710;

// ---------------------------------------------------------------- resources

#[derive(Resource)]
struct Registry(MaterialRegistry);

#[derive(Resource)]
struct WorldMap(Map);

#[derive(Resource)]
struct ViewZ(usize);

#[derive(Resource)]
struct Cursor {
    x: usize,
    y: usize,
}

/// Set when the map or view z changes so the tile layer re-colors itself.
#[derive(Resource)]
struct MapDirty(bool);

#[derive(Resource)]
struct SimClock(Calendar);

#[derive(Resource)]
struct MoveRepeat(Timer);

#[derive(Resource, Default)]
struct ShotState {
    frames: u32,
    taken: bool,
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
    // Works when run from the workspace root (cargo run) or from an
    // installed location next to the binary.
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

fn main() {
    let registry = MaterialRegistry::load_dir(&data_dir().join("materials"))
        .expect("failed to load material raws");
    let mut rng = dk_core::rng_from_seed(WORLD_SEED);
    let map = dk_world::generate(&registry, &mut rng, MAP_W, MAP_H, MAP_D, WORLD_SEED);
    let start_z = map.surface_z(MAP_W / 2, MAP_H / 2).unwrap_or(MAP_D / 2);

    App::new()
        .add_plugins((
            DefaultPlugins.set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Dwarf Kingdom — Phase 0".into(),
                    resolution: (1100.0, 860.0).into(),
                    ..default()
                }),
                ..default()
            }),
            FrameTimeDiagnosticsPlugin::default(),
        ))
        .insert_resource(ClearColor(Color::srgb(0.04, 0.04, 0.06)))
        .insert_resource(Time::<Fixed>::from_hz(dk_core::SIM_HZ))
        .insert_resource(Registry(registry))
        .insert_resource(WorldMap(map))
        .insert_resource(ViewZ(start_z))
        .insert_resource(Cursor { x: MAP_W / 2, y: MAP_H / 2 })
        .insert_resource(MapDirty(true))
        .insert_resource(SimClock(Calendar::default()))
        .insert_resource(MoveRepeat(Timer::from_seconds(0.08, TimerMode::Repeating)))
        .insert_resource(ShotState::default())
        .add_systems(Startup, setup)
        .add_systems(FixedUpdate, advance_clock)
        .add_systems(
            Update,
            (
                handle_input,
                redraw_tiles,
                position_cursor_sprite,
                update_hud,
                screenshot_mode,
            ),
        )
        .run();
}

fn setup(mut commands: Commands) {
    let center = Vec3::new(
        MAP_W as f32 * TILE * 0.5,
        MAP_H as f32 * TILE * 0.5,
        999.0,
    );
    commands.spawn((Camera2d, Transform::from_translation(center)));

    // One sprite per (x, y) column; re-colored whenever the view changes.
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

    // Cursor overlay.
    commands.spawn((
        Sprite {
            color: Color::srgba(1.0, 0.95, 0.3, 0.55),
            custom_size: Some(Vec2::splat(TILE)),
            ..default()
        },
        Transform::from_xyz(0.0, 0.0, 5.0),
        CursorSprite,
    ));

    // HUD.
    commands.spawn((
        Text::new(""),
        TextFont {
            font_size: 15.0,
            ..default()
        },
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

fn advance_clock(mut clock: ResMut<SimClock>) {
    clock.0.advance();
}

fn handle_input(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut repeat: ResMut<MoveRepeat>,
    mut cursor: ResMut<Cursor>,
    mut view_z: ResMut<ViewZ>,
    mut dirty: ResMut<MapDirty>,
    mut map: ResMut<WorldMap>,
    mut camera: Query<&mut Transform, With<Camera2d>>,
    mut exit: EventWriter<AppExit>,
) {
    // Cursor movement with hold-to-repeat.
    repeat.0.tick(time.delta());
    let step_ok = repeat.0.just_finished();
    let mut mv = |dx: i64, dy: i64, key: KeyCode, keys: &ButtonInput<KeyCode>| -> (i64, i64) {
        if keys.just_pressed(key) || (keys.pressed(key) && step_ok) {
            (dx, dy)
        } else {
            (0, 0)
        }
    };
    let mut dx = 0i64;
    let mut dy = 0i64;
    for (d, key) in [
        ((-1i64, 0i64), KeyCode::ArrowLeft),
        ((1, 0), KeyCode::ArrowRight),
        ((0, 1), KeyCode::ArrowUp),
        ((0, -1), KeyCode::ArrowDown),
    ] {
        let (mx, my) = mv(d.0, d.1, key, &keys);
        dx += mx;
        dy += my;
    }
    if dx != 0 || dy != 0 {
        cursor.x = (cursor.x as i64 + dx).clamp(0, MAP_W as i64 - 1) as usize;
        cursor.y = (cursor.y as i64 + dy).clamp(0, MAP_H as i64 - 1) as usize;
    }

    // Z-level.
    if keys.just_pressed(KeyCode::BracketLeft) && view_z.0 > 0 {
        view_z.0 -= 1;
        dirty.0 = true;
    }
    if keys.just_pressed(KeyCode::BracketRight) && view_z.0 < MAP_D - 1 {
        view_z.0 += 1;
        dirty.0 = true;
    }

    // Camera pan / zoom.
    if let Ok(mut tf) = camera.single_mut() {
        let pan = 300.0 * time.delta_secs() * tf.scale.x;
        if keys.pressed(KeyCode::KeyA) {
            tf.translation.x -= pan;
        }
        if keys.pressed(KeyCode::KeyD) {
            tf.translation.x += pan;
        }
        if keys.pressed(KeyCode::KeyW) {
            tf.translation.y += pan;
        }
        if keys.pressed(KeyCode::KeyS) {
            tf.translation.y -= pan;
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
        match map.0.save(&save_path()) {
            Ok(()) => info!("world saved to {}", save_path().display()),
            Err(e) => error!("save failed: {e:#}"),
        }
    }
    if keys.just_pressed(KeyCode::F9) {
        match Map::load(&save_path()) {
            Ok(loaded) => {
                map.0 = loaded;
                dirty.0 = true;
                info!("world loaded from {}", save_path().display());
            }
            Err(e) => error!("load failed: {e:#}"),
        }
    }

    if keys.just_pressed(KeyCode::Escape) {
        exit.write(AppExit::Success);
    }
}

/// Color for a map position as seen from `view_z`: the tile itself if solid,
/// otherwise the first solid tile below, dimmed by distance ("depth fog").
fn tile_color(map: &Map, reg: &MaterialRegistry, x: usize, y: usize, view_z: usize) -> Color {
    const DIM: [f32; 4] = [1.0, 0.55, 0.34, 0.20];
    for (levels_down, factor) in DIM.iter().enumerate() {
        if levels_down > view_z {
            break;
        }
        let z = view_z - levels_down;
        let tile = map.get(x, y, z);
        if tile.shape == TileShape::Solid {
            let [r, g, b] = reg.get(tile.material).color;
            return Color::srgb(
                r as f32 / 255.0 * factor,
                g as f32 / 255.0 * factor,
                b as f32 / 255.0 * factor,
            );
        }
    }
    Color::srgb(0.02, 0.02, 0.03)
}

fn redraw_tiles(
    mut dirty: ResMut<MapDirty>,
    map: Res<WorldMap>,
    reg: Res<Registry>,
    view_z: Res<ViewZ>,
    mut tiles: Query<(&TileSprite, &mut Sprite)>,
) {
    if !dirty.0 {
        return;
    }
    dirty.0 = false;
    for (ts, mut sprite) in &mut tiles {
        sprite.color = tile_color(&map.0, &reg.0, ts.x, ts.y, view_z.0);
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
    map: Res<WorldMap>,
    reg: Res<Registry>,
    view_z: Res<ViewZ>,
    cursor: Res<Cursor>,
    clock: Res<SimClock>,
    diagnostics: Res<DiagnosticsStore>,
    mut q: Query<&mut Text, With<HudText>>,
) {
    let tile = map.0.get(cursor.x, cursor.y, view_z.0);
    let under = match tile.shape {
        TileShape::Solid => reg.0.get(tile.material).name.clone(),
        TileShape::Empty => match map.0.surface_z(cursor.x, cursor.y) {
            Some(z) if z < view_z.0 => format!(
                "Open air ({} below at z{})",
                reg.0.get(map.0.get(cursor.x, cursor.y, z).material).name,
                z
            ),
            _ => "Open air".to_string(),
        },
    };
    let fps = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|d| d.smoothed())
        .unwrap_or(0.0);
    let cal = &clock.0;
    for mut text in &mut q {
        text.0 = format!(
            "Dwarf Kingdom — Phase 0\n\
             z-level {} / {}   cursor ({}, {})   {}\n\
             Year {}, {} {}   tick {}   {:.0} fps\n\
             arrows: cursor   [ ]: z   WASD: pan   -/=: zoom   F5/F9: save/load   Esc: quit",
            view_z.0,
            MAP_D - 1,
            cursor.x,
            cursor.y,
            under,
            cal.year(),
            cal.season().name(),
            cal.day_of_season(),
            cal.tick,
            fps,
        );
    }
}

/// Automated verification: DK_SCREENSHOT=1 captures a frame then exits.
fn screenshot_mode(
    mut state: ResMut<ShotState>,
    mut commands: Commands,
    mut exit: EventWriter<AppExit>,
) {
    if std::env::var_os("DK_SCREENSHOT").is_none() {
        return;
    }
    state.frames += 1;
    if state.frames == 90 && !state.taken {
        state.taken = true;
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(Path::new("phase0.png").to_path_buf()));
        info!("screenshot requested");
    }
    if state.frames >= 160 {
        exit.write(AppExit::Success);
    }
}
