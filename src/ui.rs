//! Side-panel palette and interaction state. Pure macroquad drawing plus simple
//! rectangle hit-testing — no immediate-mode GUI dependency needed.

use crate::element::*;
use crate::world::W;
use macroquad::prelude::*;

pub const SCALE: f32 = 4.0;
pub const PANEL_W: f32 = 230.0;
pub const STATUS_H: f32 = 34.0;

pub const SIM_W: f32 = W as f32 * SCALE;

pub struct Ui {
    pub selected: Element,
    pub brush: i32,
    pub paused: bool,
    /// "spark" mode: left-click injects charge instead of painting.
    pub spark_tool: bool,
}

impl Ui {
    pub fn new() -> Self {
        Ui {
            selected: Element::Sand,
            brush: 4,
            paused: false,
            spark_tool: false,
        }
    }

    /// Layout of one palette button for element index `n`.
    fn button_rect(n: usize) -> Rect {
        let pad = 8.0;
        let x = SIM_W + pad;
        let y = 64.0 + n as f32 * 30.0;
        Rect::new(x, y, PANEL_W - pad * 2.0, 26.0)
    }

    fn spark_rect() -> Rect {
        let pad = 8.0;
        let y = 64.0 + PALETTE.len() as f32 * 30.0 + 8.0;
        Rect::new(SIM_W + pad, y, PANEL_W - pad * 2.0, 26.0)
    }

    /// Handle a click in the panel. Returns true if it was consumed by the UI.
    pub fn handle_panel_click(&mut self, mx: f32, my: f32) -> bool {
        if mx < SIM_W {
            return false;
        }
        for (n, &el) in PALETTE.iter().enumerate() {
            if Self::button_rect(n).contains(vec2(mx, my)) {
                self.selected = el;
                self.spark_tool = false;
                return true;
            }
        }
        if Self::spark_rect().contains(vec2(mx, my)) {
            self.spark_tool = true;
            return true;
        }
        true // swallow clicks anywhere in the panel
    }

    pub fn draw(&self, fps: i32, particles: usize) {
        // panel background
        draw_rectangle(SIM_W, 0.0, PANEL_W, screen_height(), Color::from_rgba(24, 24, 30, 255));
        draw_line(SIM_W, 0.0, SIM_W, screen_height(), 2.0, Color::from_rgba(50, 50, 60, 255));

        draw_text("SAND", SIM_W + 12.0, 30.0, 34.0, Color::from_rgba(220, 200, 140, 255));
        draw_text("sandbox", SIM_W + 110.0, 30.0, 20.0, Color::from_rgba(120, 120, 130, 255));
        draw_text("pick a material:", SIM_W + 12.0, 56.0, 18.0, GRAY);

        for (n, &el) in PALETTE.iter().enumerate() {
            let r = Self::button_rect(n);
            let p = props(el);
            let selected = !self.spark_tool && el == self.selected;
            let bg = if selected {
                Color::from_rgba(70, 70, 90, 255)
            } else {
                Color::from_rgba(40, 40, 50, 255)
            };
            draw_rectangle(r.x, r.y, r.w, r.h, bg);
            if selected {
                draw_rectangle_lines(r.x, r.y, r.w, r.h, 2.0, YELLOW);
            }
            // colour swatch
            let (cr, cg, cb) = p.color;
            draw_rectangle(r.x + 4.0, r.y + 4.0, 18.0, 18.0, Color::from_rgba(cr, cg, cb, 255));
            draw_rectangle_lines(r.x + 4.0, r.y + 4.0, 18.0, 18.0, 1.0, Color::from_rgba(0, 0, 0, 120));
            draw_text(p.name, r.x + 30.0, r.y + 18.0, 18.0, WHITE);
        }

        // spark tool button
        let sr = Self::spark_rect();
        let bg = if self.spark_tool {
            Color::from_rgba(70, 70, 90, 255)
        } else {
            Color::from_rgba(40, 40, 50, 255)
        };
        draw_rectangle(sr.x, sr.y, sr.w, sr.h, bg);
        if self.spark_tool {
            draw_rectangle_lines(sr.x, sr.y, sr.w, sr.h, 2.0, YELLOW);
        }
        draw_rectangle(sr.x + 4.0, sr.y + 4.0, 18.0, 18.0, Color::from_rgba(120, 200, 255, 255));
        draw_text("Spark (wire metal)", sr.x + 30.0, sr.y + 18.0, 18.0, WHITE);

        // help text at the bottom of the panel
        let help = [
            "L-click: paint   R-click: erase",
            "wheel: brush size",
            "Space: pause   C: clear   R: reset",
            "1-9: quick-pick   Z: spark tool",
        ];
        let mut hy = screen_height() - 96.0;
        for line in help {
            draw_text(line, SIM_W + 12.0, hy, 16.0, Color::from_rgba(150, 150, 160, 255));
            hy += 20.0;
        }

        // status bar across the bottom of the sim view
        let by = screen_height() - STATUS_H;
        draw_rectangle(0.0, by, SIM_W, STATUS_H, Color::from_rgba(18, 18, 24, 230));
        let tool = if self.spark_tool {
            "Spark"
        } else {
            props(self.selected).name
        };
        let status = format!(
            "{}   |   brush {}   |   {} particles   |   {} fps{}",
            tool,
            self.brush,
            particles,
            fps,
            if self.paused { "   |   PAUSED" } else { "" }
        );
        draw_text(&status, 10.0, by + 22.0, 20.0, Color::from_rgba(200, 200, 210, 255));
    }
}
