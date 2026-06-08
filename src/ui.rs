//! Side-panel palette with category tabs (submenus) and the status bar. Pure
//! macroquad drawing plus simple rectangle hit-testing.

use crate::element::*;
use crate::world::W;
use macroquad::prelude::*;

pub const SCALE: f32 = 4.0;
pub const PANEL_W: f32 = 250.0;
pub const STATUS_H: f32 = 34.0;

pub const SIM_W: f32 = W as f32 * SCALE;

const TOOL_TOP: f32 = 34.0;
const TAB_TOP: f32 = 62.0;
const TAB_H: f32 = 22.0;
const LIST_TOP: f32 = 178.0; // below the tool row + 4 rows of category tabs
const ROW_H: f32 = 28.0;

/// What a left-click does in the sim viewport.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Grab,
    Paint,
    Human,
    Spark,
}

const TOOLS: &[(Tool, &str)] = &[
    (Tool::Grab, "Grab"),
    (Tool::Paint, "Paint"),
    (Tool::Human, "Human"),
    (Tool::Spark, "Spark"),
];

/// A one-shot or toggle world event triggered from the Events tab.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Ev {
    Meteor,
    Lightning,
    Volcano,
    Earthquake,
    Flood,
    Plague,
    ToggleRain,
    ToggleAcidRain,
    WindLeft,
    WindRight,
    Calm,
}

const EVENTS: &[(Ev, &str, (u8, u8, u8))] = &[
    (Ev::Meteor, "Meteor Shower  M", (255, 150, 50)),
    (Ev::Lightning, "Lightning  L", (180, 220, 255)),
    (Ev::Volcano, "Volcano  V", (200, 80, 30)),
    (Ev::Earthquake, "Earthquake  E", (150, 115, 70)),
    (Ev::Flood, "Flood  F", (50, 120, 210)),
    (Ev::Plague, "Plague  P", (205, 60, 200)),
    (Ev::ToggleRain, "Rain  O", (80, 150, 220)),
    (Ev::ToggleAcidRain, "Acid Rain  I", (120, 220, 60)),
    (Ev::WindLeft, "Wind <  ,", (190, 190, 200)),
    (Ev::WindRight, "Wind >  .", (190, 190, 200)),
    (Ev::Calm, "Calm / Clear  /", (120, 120, 130)),
];

pub struct Ui {
    pub category: Menu,
    pub selected: Element,
    pub brush: i32,
    pub paused: bool,
    /// The active left-click tool.
    pub tool: Tool,
    /// An event the user clicked this frame, consumed by the main loop.
    pub event: Option<Ev>,
}

impl Ui {
    pub fn new() -> Self {
        Ui {
            category: Menu::Powders,
            selected: Element::Sand,
            brush: 4,
            paused: false,
            tool: Tool::Grab,
            event: None,
        }
    }

    /// Elements currently listed (depends on the active tab).
    pub fn current_list(&self) -> Vec<Element> {
        menu_elements(self.category)
    }

    fn tool_rect(n: usize) -> Rect {
        let pad = 6.0;
        let w = (PANEL_W - pad * 5.0) / 4.0;
        let x = SIM_W + pad + n as f32 * (w + pad);
        Rect::new(x, TOOL_TOP, w, 22.0)
    }

    fn tab_rect(n: usize) -> Rect {
        // two columns of category tabs
        let pad = 6.0;
        let col = (n % 2) as f32;
        let row = (n / 2) as f32;
        let w = (PANEL_W - pad * 3.0) / 2.0;
        let x = SIM_W + pad + col * (w + pad);
        let y = TAB_TOP + row * (TAB_H + 4.0);
        Rect::new(x, y, w, TAB_H)
    }

    fn button_rect(n: usize) -> Rect {
        let pad = 8.0;
        let x = SIM_W + pad;
        let y = LIST_TOP + n as f32 * ROW_H;
        Rect::new(x, y, PANEL_W - pad * 2.0, ROW_H - 4.0)
    }

    /// Handle a click in the panel. Returns true if it was consumed.
    pub fn handle_panel_click(&mut self, mx: f32, my: f32) -> bool {
        if mx < SIM_W {
            return false;
        }
        for (n, &(tool, _)) in TOOLS.iter().enumerate() {
            if Self::tool_rect(n).contains(vec2(mx, my)) {
                self.tool = tool;
                return true;
            }
        }
        for (n, &(menu, _)) in CATEGORIES.iter().enumerate() {
            if Self::tab_rect(n).contains(vec2(mx, my)) {
                self.category = menu;
                return true;
            }
        }
        if self.category == Menu::Events {
            for (n, &(ev, _, _)) in EVENTS.iter().enumerate() {
                if Self::button_rect(n).contains(vec2(mx, my)) {
                    self.event = Some(ev);
                    return true;
                }
            }
            return true;
        }
        let list = self.current_list();
        for (n, &el) in list.iter().enumerate() {
            if Self::button_rect(n).contains(vec2(mx, my)) {
                self.selected = el;
                self.tool = Tool::Paint; // picking a material implies painting
                return true;
            }
        }
        true // swallow other clicks in the panel
    }

    pub fn draw(
        &self,
        fps: i32,
        particles: usize,
        population: usize,
        rain_on: bool,
        acid_on: bool,
        wind: i32,
    ) {
        draw_rectangle(SIM_W, 0.0, PANEL_W, screen_height(), Color::from_rgba(24, 24, 30, 255));
        draw_line(SIM_W, 0.0, SIM_W, screen_height(), 2.0, Color::from_rgba(50, 50, 60, 255));

        draw_text("SAND", SIM_W + 12.0, 26.0, 28.0, Color::from_rgba(220, 200, 140, 255));
        draw_text("sandbox", SIM_W + 108.0, 26.0, 16.0, Color::from_rgba(120, 120, 130, 255));

        // tool selector row
        for (n, &(tool, label)) in TOOLS.iter().enumerate() {
            let r = Self::tool_rect(n);
            let active = tool == self.tool;
            let bg = if active {
                Color::from_rgba(60, 95, 95, 255)
            } else {
                Color::from_rgba(40, 40, 50, 255)
            };
            draw_rectangle(r.x, r.y, r.w, r.h, bg);
            if active {
                draw_rectangle_lines(r.x, r.y, r.w, r.h, 2.0, SKYBLUE);
            }
            draw_text(label, r.x + 6.0, r.y + 16.0, 16.0, WHITE);
        }

        // category tabs
        for (n, &(menu, label)) in CATEGORIES.iter().enumerate() {
            let r = Self::tab_rect(n);
            let active = menu == self.category;
            let bg = if active {
                Color::from_rgba(90, 80, 60, 255)
            } else {
                Color::from_rgba(40, 40, 50, 255)
            };
            draw_rectangle(r.x, r.y, r.w, r.h, bg);
            if active {
                draw_rectangle_lines(r.x, r.y, r.w, r.h, 2.0, YELLOW);
            }
            draw_text(label, r.x + 8.0, r.y + 16.0, 17.0, WHITE);
        }

        // the Events tab lists disasters/weather instead of materials
        if self.category == Menu::Events {
            for (n, &(ev, label, color)) in EVENTS.iter().enumerate() {
                let r = Self::button_rect(n);
                let active = matches!(
                    (ev, rain_on, acid_on, wind),
                    (Ev::ToggleRain, true, _, _)
                        | (Ev::ToggleAcidRain, _, true, _)
                        | (Ev::WindLeft, _, _, -1)
                        | (Ev::WindRight, _, _, 1)
                );
                let bg = if active {
                    Color::from_rgba(80, 70, 95, 255)
                } else {
                    Color::from_rgba(40, 40, 50, 255)
                };
                draw_rectangle(r.x, r.y, r.w, r.h, bg);
                if active {
                    draw_rectangle_lines(r.x, r.y, r.w, r.h, 2.0, YELLOW);
                }
                let (cr, cg, cb) = color;
                draw_rectangle(r.x + 4.0, r.y + 3.0, 18.0, 18.0, Color::from_rgba(cr, cg, cb, 255));
                draw_text(label, r.x + 30.0, r.y + 17.0, 18.0, WHITE);
            }
            self.draw_status(fps, particles, population, rain_on, acid_on, wind);
            return;
        }

        // element buttons for the active category
        let list = self.current_list();
        for (n, &el) in list.iter().enumerate() {
            let r = Self::button_rect(n);
            let p = props(el);
            let selected = self.tool == Tool::Paint && el == self.selected;
            let bg = if selected {
                Color::from_rgba(70, 70, 90, 255)
            } else {
                Color::from_rgba(40, 40, 50, 255)
            };
            draw_rectangle(r.x, r.y, r.w, r.h, bg);
            if selected {
                draw_rectangle_lines(r.x, r.y, r.w, r.h, 2.0, YELLOW);
            }
            let (cr, cg, cb) = p.color;
            draw_rectangle(r.x + 4.0, r.y + 3.0, 18.0, 18.0, Color::from_rgba(cr, cg, cb, 255));
            draw_rectangle_lines(r.x + 4.0, r.y + 3.0, 18.0, 18.0, 1.0, Color::from_rgba(0, 0, 0, 120));
            draw_text(p.name, r.x + 30.0, r.y + 17.0, 18.0, WHITE);
        }

        self.draw_status(fps, particles, population, rain_on, acid_on, wind);
    }

    fn draw_status(
        &self,
        fps: i32,
        particles: usize,
        population: usize,
        rain_on: bool,
        acid_on: bool,
        wind: i32,
    ) {
        // help text at the bottom of the panel
        let help = [
            "Grab: drag/fling ragdolls",
            "Human(H): drop a person",
            "R-click erase  wheel: brush",
            "Space pause  C clear  R reset",
        ];
        let mut hy = screen_height() - 92.0;
        for line in help {
            draw_text(line, SIM_W + 12.0, hy, 15.0, Color::from_rgba(150, 150, 160, 255));
            hy += 19.0;
        }

        // status bar across the bottom of the sim view
        let by = screen_height() - STATUS_H;
        draw_rectangle(0.0, by, SIM_W, STATUS_H, Color::from_rgba(18, 18, 24, 230));
        let tool = match self.tool {
            Tool::Grab => "Grab",
            Tool::Human => "Human",
            Tool::Spark => "Spark",
            Tool::Paint => props(self.selected).name,
        };
        let mut weather = String::new();
        if rain_on {
            weather.push_str("  rain");
        }
        if acid_on {
            weather.push_str("  acid-rain");
        }
        if wind < 0 {
            weather.push_str("  wind<");
        } else if wind > 0 {
            weather.push_str("  wind>");
        }
        let status = format!(
            "{}  |  brush {}  |  pop {}  |  {} particles  |  {} fps{}{}",
            tool,
            self.brush,
            population,
            particles,
            fps,
            if self.paused { "  |  PAUSED" } else { "" },
            weather,
        );
        draw_text(&status, 10.0, by + 22.0, 20.0, Color::from_rgba(200, 200, 210, 255));
    }
}
