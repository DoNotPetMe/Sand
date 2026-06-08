//! Sand — an advanced native falling-sand sandbox.
//!
//! Rendering: the simulation writes a W×H RGBA image every frame, which is
//! uploaded to a GPU texture and stretched over the sim viewport. That keeps
//! drawing ~120k cells effectively free, leaving the CPU budget for physics.

mod element;
mod physics;
mod ui;
mod world;

use element::Element;
use macroquad::prelude::*;
use physics::Ragdolls;
use ui::{Ev, Tool, Ui, PANEL_W, SCALE, SIM_W, STATUS_H};
use world::{World, H, W};

/// Apply a disaster/weather event. Toggles mutate the weather flags; one-shots
/// hit the world directly. `shake` drives the screen-shake effect.
fn apply_event(
    world: &mut World,
    ev: Ev,
    grid_x: i32,
    rain: &mut bool,
    acid: &mut bool,
    shake: &mut i32,
) {
    match ev {
        Ev::Meteor => {
            world.meteor_shower();
            *shake = (*shake).max(12);
        }
        Ev::Lightning => world.lightning(grid_x),
        Ev::Volcano => {
            world.erupt_volcano();
            *shake = (*shake).max(8);
        }
        Ev::Earthquake => {
            world.earthquake();
            *shake = (*shake).max(30);
        }
        Ev::Flood => world.flood(),
        Ev::Plague => world.plague(),
        Ev::ToggleRain => *rain = !*rain,
        Ev::ToggleAcidRain => *acid = !*acid,
        Ev::WindLeft => world.wind = if world.wind == -1 { 0 } else { -1 },
        Ev::WindRight => world.wind = if world.wind == 1 { 0 } else { 1 },
        Ev::Calm => {
            *rain = false;
            *acid = false;
            world.wind = 0;
        }
    }
}

/// Map disaster hotkeys to events.
fn event_hotkey() -> Option<Ev> {
    if is_key_pressed(KeyCode::M) {
        Some(Ev::Meteor)
    } else if is_key_pressed(KeyCode::L) {
        Some(Ev::Lightning)
    } else if is_key_pressed(KeyCode::V) {
        Some(Ev::Volcano)
    } else if is_key_pressed(KeyCode::E) {
        Some(Ev::Earthquake)
    } else if is_key_pressed(KeyCode::F) {
        Some(Ev::Flood)
    } else if is_key_pressed(KeyCode::P) {
        Some(Ev::Plague)
    } else if is_key_pressed(KeyCode::O) {
        Some(Ev::ToggleRain)
    } else if is_key_pressed(KeyCode::I) {
        Some(Ev::ToggleAcidRain)
    } else if is_key_pressed(KeyCode::Comma) {
        Some(Ev::WindLeft)
    } else if is_key_pressed(KeyCode::Period) {
        Some(Ev::WindRight)
    } else if is_key_pressed(KeyCode::Slash) {
        Some(Ev::Calm)
    } else {
        None
    }
}

fn window_conf() -> Conf {
    Conf {
        window_title: "Sand — Advanced Falling-Sand Sandbox".to_owned(),
        window_width: (W as f32 * SCALE + PANEL_W) as i32,
        window_height: (H as f32 * SCALE + STATUS_H) as i32,
        window_resizable: false,
        ..Default::default()
    }
}

/// Quick-pick number keys 1-9 select from the active category's element list.
fn quick_pick(list: &[Element]) -> Option<Element> {
    const KEYS: [KeyCode; 9] = [
        KeyCode::Key1, KeyCode::Key2, KeyCode::Key3, KeyCode::Key4, KeyCode::Key5,
        KeyCode::Key6, KeyCode::Key7, KeyCode::Key8, KeyCode::Key9,
    ];
    for (n, &k) in KEYS.iter().enumerate() {
        if is_key_pressed(k) && n < list.len() {
            return Some(list[n]);
        }
    }
    None
}

/// Seed a varied scene used by the headless screenshot mode (and handy as a
/// quick "interesting starting point"). Triggered by the `SAND_DEMO` env var.
fn setup_demo(world: &mut World) {
    let w = W as i32;
    let h = H as i32;
    // ground
    for x in 0..w {
        world.spawn(x, h - 1, Element::Wall, 0);
        world.spawn(x, h - 2, Element::Wall, 0);
    }
    // a water pool on the left, held in a stone basin
    for y in (h - 24)..(h - 2) {
        world.spawn(18, y, Element::Stone, 0);
        world.spawn(70, y, Element::Stone, 0);
    }
    for x in 19..70 {
        for y in (h - 20)..(h - 2) {
            world.spawn(x, y, Element::Water, 0);
        }
    }
    // a lava pit on the right
    for y in (h - 18)..(h - 2) {
        world.spawn(232, y, Element::Stone, 0);
        world.spawn(300, y, Element::Stone, 0);
    }
    for x in 233..300 {
        for y in (h - 14)..(h - 2) {
            world.spawn(x, y, Element::Lava, 0);
        }
    }
    // a turret on a metal post, firing right across the crowd at chest height
    for y in (h - 18)..(h - 2) {
        world.spawn(90, y, Element::Metal, 0);
    }
    world.paint(91, h - 16, 0, Element::Gun);
    // a campfire hazard on the floor
    world.paint(170, h - 4, 4, Element::Fire);
    for x in 165..176 {
        world.spawn(x, h - 3, Element::Wood, 0);
    }
}

#[macroquad::main(window_conf)]
async fn main() {
    rand::srand(macroquad::miniquad::date::now() as u64);

    let mut world = World::new();
    let mut ui = Ui::new();
    let mut ragdolls = Ragdolls::new();

    // Headless showcase: seed a scene, simulate, save a PNG, then exit.
    let demo = std::env::var("SAND_DEMO").is_ok();
    let shot_path = std::env::var("SAND_SCREENSHOT").ok();
    let shot_after: u64 = std::env::var("SAND_SHOT_FRAME")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(150);
    if demo {
        setup_demo(&mut world);
        ui.category = element::Menu::Life;
        ui.tool = Tool::Grab;
        // a line-up of ragdoll people between the turret and the lava pit
        for i in 0..6 {
            ragdolls.spawn_human(110.0 + i as f32 * 20.0, (H - 3) as f32);
        }
    }
    let mut frame_no: u64 = 0;

    let mut image = Image::gen_image_color(W as u16, H as u16, BLACK);
    let texture = Texture2D::from_image(&image);
    texture.set_filter(FilterMode::Nearest);

    let mut prev_mouse: Option<(f32, f32)> = None;
    let mut rain_on = false;
    let mut acid_on = false;
    let mut shake: i32 = 0;

    loop {
        // ---- input --------------------------------------------------------
        if is_key_pressed(KeyCode::Space) {
            ui.paused = !ui.paused;
        }
        if is_key_pressed(KeyCode::C) {
            world.clear();
            ragdolls.clear();
        }
        if is_key_pressed(KeyCode::R) {
            world = World::new();
            ragdolls.clear();
        }
        // tool hotkeys
        if is_key_pressed(KeyCode::G) {
            ui.tool = Tool::Grab;
        }
        if is_key_pressed(KeyCode::H) {
            ui.tool = Tool::Human;
        }
        if is_key_pressed(KeyCode::Z) {
            ui.tool = Tool::Spark;
        }
        if is_key_pressed(KeyCode::B) {
            ui.tool = Tool::Paint;
        }
        if let Some(el) = quick_pick(&ui.current_list()) {
            ui.selected = el;
            ui.tool = Tool::Paint;
        }

        let (_, wheel_y) = mouse_wheel();
        if wheel_y != 0.0 {
            ui.brush = (ui.brush + if wheel_y > 0.0 { 1 } else { -1 }).clamp(0, 40);
        }

        let (mx, my) = mouse_position();
        let in_sim = mx < SIM_W && my < H as f32 * SCALE;

        // panel clicks select tools / trigger events
        if is_mouse_button_pressed(MouseButton::Left) && mx >= SIM_W {
            ui.handle_panel_click(mx, my);
        }

        // disasters & weather, from panel buttons or hotkeys
        let grid_x = (mx / SCALE).clamp(0.0, (W - 1) as f32) as i32;
        if let Some(ev) = ui.event.take().or_else(event_hotkey) {
            apply_event(&mut world, ev, grid_x, &mut rain_on, &mut acid_on, &mut shake);
        }

        // ragdoll interaction (grab/throw, spawn) in grid coordinates
        let (fx, fy) = (mx / SCALE, my / SCALE);
        if in_sim && is_mouse_button_pressed(MouseButton::Left) {
            match ui.tool {
                Tool::Grab => ragdolls.grab(fx, fy),
                Tool::Human => ragdolls.spawn_human(fx, fy),
                _ => {}
            }
        }
        if ui.tool == Tool::Grab {
            if is_mouse_button_down(MouseButton::Left) {
                ragdolls.drag(fx, fy);
            } else {
                ragdolls.release();
            }
        }

        let painting_left =
            is_mouse_button_down(MouseButton::Left) && in_sim && matches!(ui.tool, Tool::Paint | Tool::Spark);
        let painting_right = is_mouse_button_down(MouseButton::Right) && in_sim;

        if painting_left || painting_right {
            let gx = (mx / SCALE) as i32;
            let gy = (my / SCALE) as i32;
            // interpolate between frames so fast drags draw a continuous stroke
            let (px, py) = prev_mouse
                .map(|(a, b)| ((a / SCALE) as i32, (b / SCALE) as i32))
                .unwrap_or((gx, gy));
            let steps = (gx - px).abs().max((gy - py).abs()).max(1);
            for s in 0..=steps {
                let x = px + (gx - px) * s / steps;
                let y = py + (gy - py) * s / steps;
                if painting_right {
                    world.paint(x, y, ui.brush, Element::Empty);
                } else if ui.tool == Tool::Spark {
                    world.zap(x, y, ui.brush.max(1));
                } else {
                    world.paint(x, y, ui.brush, ui.selected);
                }
            }
            prev_mouse = Some((mx, my));
        } else {
            prev_mouse = None;
        }

        // ---- simulate -----------------------------------------------------
        if !ui.paused {
            if rain_on {
                world.rain(Element::Water);
            }
            if acid_on {
                world.rain(Element::Acid);
            }
            world.step();
            ragdolls.step(&mut world);
        }

        // ---- render -------------------------------------------------------
        let pixels = image.get_image_data_mut();
        let mut particles = 0usize;
        for i in 0..world.cells.len() {
            if world.cells[i].el != Element::Empty {
                particles += 1;
            }
            pixels[i] = world.color_at(i);
        }
        texture.update(&image);

        // screen shake offset for earthquakes / big impacts
        let (ox, oy) = if shake > 0 {
            shake -= 1;
            (rand::gen_range(-4.0, 4.0), rand::gen_range(-4.0, 4.0))
        } else {
            (0.0, 0.0)
        };

        clear_background(Color::from_rgba(12, 12, 16, 255));
        draw_texture_ex(
            &texture,
            ox,
            oy,
            WHITE,
            DrawTextureParams {
                dest_size: Some(vec2(W as f32 * SCALE, H as f32 * SCALE)),
                ..Default::default()
            },
        );

        // ragdoll humans drawn on top of the sand
        ragdolls.draw(SCALE, ox, oy);

        // cursor: brush ring for painting, reticle for grab
        if in_sim {
            match ui.tool {
                Tool::Paint | Tool::Spark => {
                    draw_circle_lines(mx, my, ui.brush as f32 * SCALE, 1.0, Color::from_rgba(255, 255, 255, 120));
                }
                _ => {
                    draw_circle_lines(mx, my, 7.0 * SCALE, 1.0, Color::from_rgba(120, 220, 220, 140));
                }
            }
        }

        ui.draw(get_fps(), particles, ragdolls.count(), rain_on, acid_on, world.wind);

        // headless screenshot: capture the framebuffer and quit
        if let Some(path) = &shot_path {
            if frame_no >= shot_after {
                let screen = get_screen_data();
                screen.export_png(path);
                println!("saved screenshot to {path} after {frame_no} frames");
                break;
            }
        }
        frame_no += 1;

        next_frame().await;
    }
}
