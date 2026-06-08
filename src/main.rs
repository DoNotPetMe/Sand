//! Sand — an advanced native falling-sand sandbox.
//!
//! Rendering: the simulation writes a W×H RGBA image every frame, which is
//! uploaded to a GPU texture and stretched over the sim viewport. That keeps
//! drawing ~120k cells effectively free, leaving the CPU budget for physics.

mod element;
mod ui;
mod world;

use element::{Element, PALETTE};
use macroquad::prelude::*;
use ui::{Ui, PANEL_W, SCALE, SIM_W, STATUS_H};
use world::{World, H, W};

fn window_conf() -> Conf {
    Conf {
        window_title: "Sand — Advanced Falling-Sand Sandbox".to_owned(),
        window_width: (W as f32 * SCALE + PANEL_W) as i32,
        window_height: (H as f32 * SCALE + STATUS_H) as i32,
        window_resizable: false,
        ..Default::default()
    }
}

/// Quick-pick number keys 1-9 map to the first nine palette entries.
fn quick_pick() -> Option<Element> {
    const KEYS: [KeyCode; 9] = [
        KeyCode::Key1, KeyCode::Key2, KeyCode::Key3, KeyCode::Key4, KeyCode::Key5,
        KeyCode::Key6, KeyCode::Key7, KeyCode::Key8, KeyCode::Key9,
    ];
    for (n, &k) in KEYS.iter().enumerate() {
        if is_key_pressed(k) && n < PALETTE.len() {
            return Some(PALETTE[n]);
        }
    }
    None
}

/// Seed a varied scene used by the headless screenshot mode (and handy as a
/// quick "interesting starting point"). Triggered by the `SAND_DEMO` env var.
fn setup_demo(world: &mut World) {
    let w = W as i32;
    let h = H as i32;
    // floor and side walls
    for x in 0..w {
        world.spawn(x, h - 1, Element::Wall, 0);
        world.spawn(x, h - 2, Element::Wall, 0);
    }
    // a stone basin on the left holding water with oil floating on top
    for y in (h - 60)..(h - 2) {
        world.spawn(40, y, Element::Stone, 0);
        world.spawn(110, y, Element::Stone, 0);
    }
    for x in 41..110 {
        for y in (h - 40)..(h - 2) {
            world.spawn(x, y, Element::Water, 0);
        }
        for y in (h - 46)..(h - 40) {
            world.spawn(x, y, Element::Oil, 0);
        }
    }
    // a sand dune pouring from above
    world.paint(75, 30, 16, Element::Sand);
    // a solid wooden block set alight on the right (fire seeded *inside* the
    // wood so it actually catches and spreads, throwing smoke)
    for x in 215..=265 {
        for y in (h - 50)..(h - 2) {
            world.spawn(x, y, Element::Wood, 0);
        }
    }
    world.paint(240, h - 48, 7, Element::Fire);
    world.paint(225, h - 40, 4, Element::Fire);
    // a glowing lava pool held on a stone shelf
    for x in 285..=315 {
        world.spawn(x, h - 30, Element::Stone, 0);
    }
    for x in 286..=314 {
        for y in (h - 44)..(h - 30) {
            world.spawn(x, y, Element::Lava, 0);
        }
    }
    // a salted plant garden fed by a faucet
    world.spawn(150, 40, Element::WaterSource, 0);
    for x in 145..156 {
        world.spawn(x, h - 3, Element::Plant, 0);
    }
    // a pile of gunpowder with a bomb, near the flames
    world.paint(200, h - 10, 6, Element::Gunpowder);
    // some ants on the sand
    for i in 0..12 {
        world.spawn(70 + i, 50, Element::Ant, 0);
    }
}

#[macroquad::main(window_conf)]
async fn main() {
    rand::srand(macroquad::miniquad::date::now() as u64);

    let mut world = World::new();
    let mut ui = Ui::new();

    // Headless showcase: seed a scene, simulate, save a PNG, then exit.
    let demo = std::env::var("SAND_DEMO").is_ok();
    let shot_path = std::env::var("SAND_SCREENSHOT").ok();
    let shot_after: u64 = std::env::var("SAND_SHOT_FRAME")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(150);
    if demo {
        setup_demo(&mut world);
    }
    let mut frame_no: u64 = 0;

    let mut image = Image::gen_image_color(W as u16, H as u16, BLACK);
    let texture = Texture2D::from_image(&image);
    texture.set_filter(FilterMode::Nearest);

    let mut prev_mouse: Option<(f32, f32)> = None;

    loop {
        // ---- input --------------------------------------------------------
        if is_key_pressed(KeyCode::Space) {
            ui.paused = !ui.paused;
        }
        if is_key_pressed(KeyCode::C) {
            world.clear();
        }
        if is_key_pressed(KeyCode::R) {
            world = World::new();
        }
        if is_key_pressed(KeyCode::Z) {
            ui.spark_tool = !ui.spark_tool;
        }
        if let Some(el) = quick_pick() {
            ui.selected = el;
            ui.spark_tool = false;
        }

        let (_, wheel_y) = mouse_wheel();
        if wheel_y != 0.0 {
            ui.brush = (ui.brush + if wheel_y > 0.0 { 1 } else { -1 }).clamp(0, 40);
        }

        let (mx, my) = mouse_position();
        let in_sim = mx < SIM_W && my < H as f32 * SCALE;

        // panel clicks select tools
        if is_mouse_button_pressed(MouseButton::Left) && mx >= SIM_W {
            ui.handle_panel_click(mx, my);
        }

        let painting_left = is_mouse_button_down(MouseButton::Left) && in_sim;
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
                } else if ui.spark_tool {
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
            world.step();
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

        clear_background(Color::from_rgba(12, 12, 16, 255));
        draw_texture_ex(
            &texture,
            0.0,
            0.0,
            WHITE,
            DrawTextureParams {
                dest_size: Some(vec2(W as f32 * SCALE, H as f32 * SCALE)),
                ..Default::default()
            },
        );

        // brush outline cursor
        if in_sim {
            draw_circle_lines(mx, my, ui.brush as f32 * SCALE, 1.0, Color::from_rgba(255, 255, 255, 120));
        }

        ui.draw(get_fps(), particles);

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
