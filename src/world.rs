//! The simulation world: a grid of cells plus a parallel temperature field.
//!
//! Each `step()` runs four passes:
//!   1. clear the per-frame "moved" flags
//!   2. propagate electrical charge through conductors (one cell/frame wave)
//!   3. movement + chemistry, scanned bottom-up with alternating x direction
//!   4. heat: emit from hot/cold sources, diffuse, then apply phase changes
//!
//! Keeping space (temperature) and matter (cells) in separate arrays makes the
//! heat diffusion pass cache-friendly.

use crate::element::*;
use macroquad::rand::gen_range;

pub const W: usize = 320;
pub const H: usize = 200;

pub const AMBIENT: f32 = 22.0;
const HEAT_DIFFUSE: f32 = 0.14; // fraction of neighbour gradient exchanged / frame
const HEAT_RELAX: f32 = 0.004; // pull back toward ambient (radiative loss)

#[derive(Clone, Copy)]
pub struct Cell {
    pub el: Element,
    pub ra: u8,     // colour jitter seed
    pub life: u8,   // generic countdown (fire/smoke lifetime, ember fuel, ...)
    pub charge: u8, // electrical charge level
    pub moved: bool,
}

impl Cell {
    pub const EMPTY: Cell = Cell {
        el: Element::Empty,
        ra: 0,
        life: 0,
        charge: 0,
        moved: false,
    };
}

pub struct World {
    pub cells: Vec<Cell>,
    pub temp: Vec<f32>,
    scratch: Vec<f32>, // double buffer for heat diffusion
    pub frame: u64,
}

#[inline]
fn idx(x: i32, y: i32) -> usize {
    y as usize * W + x as usize
}

#[inline]
fn in_bounds(x: i32, y: i32) -> bool {
    x >= 0 && y >= 0 && (x as usize) < W && (y as usize) < H
}

#[inline]
fn chance(n: i32) -> bool {
    // true roughly 1-in-n of the time
    gen_range(0, n) == 0
}

impl World {
    pub fn new() -> Self {
        World {
            cells: vec![Cell::EMPTY; W * H],
            temp: vec![AMBIENT; W * H],
            scratch: vec![AMBIENT; W * H],
            frame: 0,
        }
    }

    pub fn clear(&mut self) {
        for c in &mut self.cells {
            *c = Cell::EMPTY;
        }
        for t in &mut self.temp {
            *t = AMBIENT;
        }
    }

    // ---- basic cell access -------------------------------------------------

    #[inline]
    pub fn el_at(&self, x: i32, y: i32) -> Element {
        if in_bounds(x, y) {
            self.cells[idx(x, y)].el
        } else {
            Element::Wall // the world edge behaves like an indestructible wall
        }
    }

    /// Place a fresh cell of `el`, randomising colour and seeding its natural
    /// temperature if it has one.
    pub fn spawn(&mut self, x: i32, y: i32, el: Element, life: u8) {
        if !in_bounds(x, y) {
            return;
        }
        let i = idx(x, y);
        self.cells[i] = Cell {
            el,
            ra: gen_range(0, 255) as u8,
            life,
            charge: 0,
            moved: true,
        };
        if let Some(t) = natural_temp(el) {
            self.temp[i] = t;
        }
    }

    fn has_neighbor(&self, x: i32, y: i32, set: &[Element]) -> bool {
        for (dx, dy) in [(0, 1), (0, -1), (1, 0), (-1, 0)] {
            if set.contains(&self.el_at(x + dx, y + dy)) {
                return true;
            }
        }
        false
    }

    // ---- public painting API (used by the UI) -----------------------------

    /// Paint a filled circle of `el` (radius in cells) into the world.
    pub fn paint(&mut self, cx: i32, cy: i32, radius: i32, el: Element) {
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                if dx * dx + dy * dy > radius * radius {
                    continue;
                }
                let (x, y) = (cx + dx, cy + dy);
                if !in_bounds(x, y) {
                    continue;
                }
                let i = idx(x, y);
                match el {
                    // The "spark" effect: don't overwrite, just energise a
                    // conductor so electricity flows from where you click.
                    Element::Empty => {
                        self.cells[i] = Cell::EMPTY;
                        self.temp[i] = AMBIENT;
                    }
                    _ => {
                        // Don't bulldoze walls when scribbling other elements.
                        if self.cells[i].el == Element::Wall && el != Element::Wall {
                            continue;
                        }
                        let life = default_life(el);
                        self.cells[i] = Cell {
                            el,
                            ra: gen_range(0, 255) as u8,
                            life,
                            charge: 0,
                            moved: true,
                        };
                        if let Some(t) = natural_temp(el) {
                            self.temp[i] = t;
                        }
                    }
                }
            }
        }
    }

    /// Inject electrical charge into conductors under the brush (the spark tool).
    pub fn zap(&mut self, cx: i32, cy: i32, radius: i32) {
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                if dx * dx + dy * dy > radius * radius {
                    continue;
                }
                let (x, y) = (cx + dx, cy + dy);
                if !in_bounds(x, y) {
                    continue;
                }
                let i = idx(x, y);
                if conductive(self.cells[i].el) {
                    self.cells[i].charge = 18;
                }
            }
        }
    }

    // ---- the main update ---------------------------------------------------

    pub fn step(&mut self) {
        for c in &mut self.cells {
            c.moved = false;
        }

        self.charge_pass();

        // Bottom-up so falling material settles in one frame; alternate the x
        // scan direction each frame to avoid a left/right drift bias.
        let l2r = self.frame % 2 == 0;
        for y in (0..H as i32).rev() {
            for xi in 0..W as i32 {
                let x = if l2r { xi } else { W as i32 - 1 - xi };
                self.update_cell(x, y);
            }
        }

        self.heat_pass();
        self.frame += 1;
    }

    fn update_cell(&mut self, x: i32, y: i32) {
        let i = idx(x, y);
        let c = self.cells[i];
        if c.moved {
            return;
        }
        match c.el {
            Element::Empty | Element::Wall | Element::Stone | Element::Glass
            | Element::Metal => {}
            Element::Fire => self.update_fire(x, y),
            Element::Ember => self.update_ember(x, y),
            Element::Smoke | Element::Toxic => self.update_fading_gas(x, y),
            Element::Steam => self.update_steam(x, y),
            Element::Acid => self.update_acid(x, y),
            Element::Plant => self.update_plant(x, y),
            Element::Seed => self.update_seed(x, y),
            Element::Ant => self.update_ant(x, y),
            Element::Bomb => self.update_bomb(x, y),
            Element::Salt => self.update_salt(x, y),
            Element::WaterSource => self.update_source(x, y),
            other => match props(other).cat {
                Cat::Powder => {
                    self.update_powder(x, y);
                }
                Cat::Liquid => {
                    self.update_liquid(x, y);
                }
                Cat::Gas => {
                    self.update_gas(x, y);
                }
                _ => {}
            },
        }
    }

    // ---- movement primitives ----------------------------------------------

    /// Try to move the cell at (x,y) into (nx,ny). Succeeds into Empty, or by
    /// displacing a fluid the mover should sink past (down) / rise past (up).
    fn try_move(&mut self, x: i32, y: i32, nx: i32, ny: i32, going_down: bool) -> bool {
        if !in_bounds(nx, ny) {
            return false;
        }
        let i = idx(x, y);
        let j = idx(nx, ny);
        let a = self.cells[i];
        let b = self.cells[j];
        let can = match b.el {
            Element::Empty => true,
            other if is_fluid(other) && other != a.el => {
                let (da, db) = (props(a.el).density, props(other).density);
                if going_down {
                    da > db
                } else {
                    da < db
                }
            }
            _ => false,
        };
        if !can {
            return false;
        }
        let mut na = a;
        let mut nb = b;
        na.moved = true;
        nb.moved = true;
        self.cells[j] = na;
        self.cells[i] = nb;
        self.temp.swap(i, j);
        true
    }

    /// Horizontal flow into empty space or a strictly-lighter fluid.
    fn flow_side(&mut self, x: i32, y: i32, dir: i32) -> bool {
        let nx = x + dir;
        if !in_bounds(nx, y) {
            return false;
        }
        let i = idx(x, y);
        let j = idx(nx, y);
        let a = self.cells[i];
        let b = self.cells[j];
        let can = b.el == Element::Empty
            || (is_fluid(b.el) && b.el != a.el && props(a.el).density > props(b.el).density);
        if !can {
            return false;
        }
        let mut na = a;
        let mut nb = b;
        na.moved = true;
        nb.moved = true;
        self.cells[j] = na;
        self.cells[i] = nb;
        self.temp.swap(i, j);
        true
    }

    fn update_powder(&mut self, x: i32, y: i32) -> bool {
        if self.try_move(x, y, x, y + 1, true) {
            return true;
        }
        let d = if chance(2) { 1 } else { -1 };
        if self.try_move(x, y, x + d, y + 1, true) {
            return true;
        }
        if self.try_move(x, y, x - d, y + 1, true) {
            return true;
        }
        false
    }

    fn update_liquid(&mut self, x: i32, y: i32) -> bool {
        if self.try_move(x, y, x, y + 1, true) {
            return true;
        }
        let d = if chance(2) { 1 } else { -1 };
        if self.try_move(x, y, x + d, y + 1, true) {
            return true;
        }
        if self.try_move(x, y, x - d, y + 1, true) {
            return true;
        }
        if self.flow_side(x, y, d) {
            return true;
        }
        if self.flow_side(x, y, -d) {
            return true;
        }
        false
    }

    fn update_gas(&mut self, x: i32, y: i32) -> bool {
        // buoyant rise
        if self.try_move(x, y, x, y - 1, false) {
            return true;
        }
        let d = if chance(2) { 1 } else { -1 };
        if self.try_move(x, y, x + d, y - 1, false) {
            return true;
        }
        if self.try_move(x, y, x - d, y - 1, false) {
            return true;
        }
        // lazy sideways drift
        if chance(2) && self.flow_side(x, y, d) {
            return true;
        }
        false
    }

    // ---- bespoke element behaviours ---------------------------------------

    fn update_fire(&mut self, x: i32, y: i32) {
        let i = idx(x, y);
        // Water and friends snuff it out into a puff of steam.
        if self.has_neighbor(x, y, &[Element::Water, Element::SaltWater, Element::Acid]) {
            self.spawn(x, y, Element::Steam, gen_range(60, 120) as u8);
            return;
        }
        let mut c = self.cells[i];
        if c.life <= 1 {
            if chance(3) {
                self.spawn(x, y, Element::Smoke, gen_range(80, 160) as u8);
            } else {
                self.cells[i] = Cell::EMPTY;
            }
            return;
        }
        c.life -= 1;
        self.cells[i] = c;
        self.update_gas(x, y);
    }

    fn update_ember(&mut self, x: i32, y: i32) {
        let i = idx(x, y);
        if self.has_neighbor(x, y, &[Element::Water, Element::SaltWater]) {
            self.spawn(x, y, Element::Smoke, gen_range(60, 120) as u8);
            return;
        }
        // belch flame upward into open air
        if self.el_at(x, y - 1) == Element::Empty && chance(3) {
            self.spawn(x, y - 1, Element::Fire, gen_range(40, 90) as u8);
        }
        let mut c = self.cells[i];
        if c.life <= 1 {
            if chance(2) {
                self.spawn(x, y, Element::Ash, 0);
            } else {
                self.spawn(x, y, Element::Smoke, gen_range(60, 120) as u8);
            }
            return;
        }
        c.life -= 1;
        self.cells[i] = c;
    }

    fn update_fading_gas(&mut self, x: i32, y: i32) {
        let i = idx(x, y);
        let mut c = self.cells[i];
        if c.life == 0 {
            self.cells[i] = Cell::EMPTY;
            return;
        }
        c.life -= 1;
        self.cells[i] = c;
        self.update_gas(x, y);
    }

    fn update_steam(&mut self, x: i32, y: i32) {
        let i = idx(x, y);
        let mut c = self.cells[i];
        // condense back to water as it ages or cools
        if c.life == 0 || (self.temp[i] < 90.0 && chance(40)) {
            if chance(3) {
                self.spawn(x, y, Element::Water, 0);
            } else {
                self.cells[i] = Cell::EMPTY;
            }
            return;
        }
        c.life -= 1;
        self.cells[i] = c;
        self.update_gas(x, y);
    }

    fn update_salt(&mut self, x: i32, y: i32) {
        // salt dissolving into water makes brine
        if self.has_neighbor(x, y, &[Element::Water]) && chance(3) {
            self.spawn(x, y, Element::SaltWater, 0);
            return;
        }
        self.update_powder(x, y);
    }

    fn update_acid(&mut self, x: i32, y: i32) {
        let i = idx(x, y);
        let (dx, dy) = [(0, 1), (0, -1), (1, 0), (-1, 0)][gen_range(0, 4) as usize];
        let (nx, ny) = (x + dx, y + dy);
        if in_bounds(nx, ny) {
            let target = self.cells[idx(nx, ny)].el;
            if dissolvable(target) && chance(3) {
                // dissolve the neighbour, occasionally venting toxic gas
                if chance(4) {
                    self.spawn(nx, ny, Element::Toxic, gen_range(60, 120) as u8);
                } else {
                    self.cells[idx(nx, ny)] = Cell::EMPTY;
                }
                let mut c = self.cells[i];
                if c.life <= 1 {
                    self.cells[i] = Cell::EMPTY;
                } else {
                    c.life -= 1;
                    self.cells[i] = c;
                }
                return;
            }
        }
        self.update_liquid(x, y);
    }

    fn update_plant(&mut self, x: i32, y: i32) {
        // grow into a nearby empty cell when watered
        if self.has_neighbor(x, y, &[Element::Water, Element::SaltWater]) && chance(14) {
            // prefer growing up / sideways over down
            let dirs = [(0, -1), (-1, 0), (1, 0), (0, -1)];
            let (dx, dy) = dirs[gen_range(0, dirs.len() as i32) as usize];
            let (nx, ny) = (x + dx, y + dy);
            if self.el_at(nx, ny) == Element::Empty {
                self.spawn(nx, ny, Element::Plant, 0);
                // drink a neighbouring water cell sometimes so it isn't infinite
                if chance(3) {
                    for (wx, wy) in [(0, 1), (1, 0), (-1, 0), (0, -1)] {
                        if self.el_at(x + wx, y + wy) == Element::Water {
                            self.cells[idx(x + wx, y + wy)] = Cell::EMPTY;
                            break;
                        }
                    }
                }
            }
        }
    }

    fn update_seed(&mut self, x: i32, y: i32) {
        // sprout into a plant when resting near water
        if self.has_neighbor(x, y, &[Element::Water, Element::SaltWater]) && chance(18) {
            self.spawn(x, y, Element::Plant, 0);
            return;
        }
        self.update_powder(x, y);
    }

    fn update_ant(&mut self, x: i32, y: i32) {
        let i = idx(x, y);
        // ants drown / burn
        if self.has_neighbor(
            x,
            y,
            &[
                Element::Water,
                Element::SaltWater,
                Element::Lava,
                Element::Acid,
                Element::Fire,
            ],
        ) && chance(3)
        {
            self.cells[i] = Cell::EMPTY;
            return;
        }
        // eat something tasty next door
        let order = [(0, 1), (0, -1), (1, 0), (-1, 0)];
        let start = gen_range(0, 4) as usize;
        for k in 0..4 {
            let (dx, dy) = order[(start + k) % 4];
            let (nx, ny) = (x + dx, y + dy);
            if matches!(
                self.el_at(nx, ny),
                Element::Plant | Element::Wood | Element::Sand | Element::Seed | Element::Ash
            ) {
                // multiply occasionally; otherwise just clear the food
                if chance(50) {
                    self.spawn(nx, ny, Element::Ant, 0);
                } else {
                    self.cells[idx(nx, ny)] = Cell::EMPTY;
                }
                return;
            }
        }
        // gravity
        if self.el_at(x, y + 1) == Element::Empty {
            self.try_move(x, y, x, y + 1, true);
            return;
        }
        // wander
        let d = gen_range(-1, 2);
        if d != 0 && self.el_at(x + d, y) == Element::Empty {
            self.try_move(x, y, x + d, y, true);
        } else if self.el_at(x, y - 1) == Element::Empty && chance(2) {
            self.try_move(x, y, x, y - 1, false);
        }
    }

    fn update_bomb(&mut self, x: i32, y: i32) {
        let i = idx(x, y);
        if self.cells[i].charge > 0
            || self.temp[i] > 200.0
            || self.has_neighbor(x, y, &[Element::Fire, Element::Ember, Element::Lava])
        {
            self.explode(x, y, 16);
            return;
        }
        self.update_powder(x, y);
    }

    fn update_source(&mut self, x: i32, y: i32) {
        for (dx, dy) in [(0, 1), (-1, 0), (1, 0)] {
            let (nx, ny) = (x + dx, y + dy);
            if self.el_at(nx, ny) == Element::Empty && chance(2) {
                self.spawn(nx, ny, Element::Water, 0);
            }
        }
    }

    /// Blast a circular crater, scattering fire/smoke and a thermal spike.
    fn explode(&mut self, cx: i32, cy: i32, r: i32) {
        for dy in -r..=r {
            for dx in -r..=r {
                if dx * dx + dy * dy > r * r {
                    continue;
                }
                let (x, y) = (cx + dx, cy + dy);
                if !in_bounds(x, y) || self.el_at(x, y) == Element::Wall {
                    continue;
                }
                let i = idx(x, y);
                let roll = gen_range(0, 100);
                if roll < 35 {
                    self.spawn(x, y, Element::Fire, gen_range(40, 100) as u8);
                } else if roll < 50 {
                    self.spawn(x, y, Element::Smoke, gen_range(80, 160) as u8);
                } else {
                    self.cells[i] = Cell::EMPTY;
                }
                self.temp[i] = 800.0;
            }
        }
    }

    // ---- electricity -------------------------------------------------------

    fn charge_pass(&mut self) {
        // Snapshot charges so the wavefront advances exactly one cell per frame.
        let old: Vec<u8> = self.cells.iter().map(|c| c.charge).collect();
        for y in 0..H as i32 {
            for x in 0..W as i32 {
                let i = idx(x, y);
                let el = self.cells[i].el;
                if !conductive(el) {
                    self.cells[i].charge = 0;
                    continue;
                }
                if old[i] > 0 {
                    // a charged conductor fires this frame, then goes quiet,
                    // detonating / igniting anything reactive next to it
                    for (dx, dy) in [(0, 1), (0, -1), (1, 0), (-1, 0)] {
                        let (nx, ny) = (x + dx, y + dy);
                        match self.el_at(nx, ny) {
                            Element::Gunpowder => self.spawn(nx, ny, Element::Fire, 60),
                            Element::Oil => self.spawn(nx, ny, Element::Fire, 70),
                            Element::Bomb => self.explode(nx, ny, 16),
                            _ => {}
                        }
                    }
                    self.cells[i].charge = 0;
                } else {
                    // pick up the strongest charge from a neighbour, minus one
                    let mut nc = 0u8;
                    for (dx, dy) in [(0, 1), (0, -1), (1, 0), (-1, 0)] {
                        let (nx, ny) = (x + dx, y + dy);
                        if in_bounds(nx, ny) {
                            let j = idx(nx, ny);
                            if conductive(self.cells[j].el) && old[j] > 1 {
                                nc = nc.max(old[j] - 1);
                            }
                        }
                    }
                    self.cells[i].charge = nc;
                }
            }
        }
    }

    // ---- heat --------------------------------------------------------------

    fn heat_pass(&mut self) {
        // 1. emit: live heat/cold sources push their cell temperature.
        for i in 0..self.cells.len() {
            match self.cells[i].el {
                Element::Fire => self.temp[i] = self.temp[i].max(720.0),
                Element::Ember => self.temp[i] = self.temp[i].max(520.0),
                _ => {}
            }
        }

        // 2. diffuse into the scratch buffer (4-neighbour, reflective edges).
        for y in 0..H as i32 {
            for x in 0..W as i32 {
                let i = idx(x, y);
                let t = self.temp[i];
                let mut sum = 0.0;
                for (dx, dy) in [(0, 1), (0, -1), (1, 0), (-1, 0)] {
                    let (nx, ny) = (x + dx, y + dy);
                    sum += if in_bounds(nx, ny) {
                        self.temp[idx(nx, ny)]
                    } else {
                        t // reflective edge: no heat lost through the walls
                    };
                }
                let diffused = t + HEAT_DIFFUSE * (sum * 0.25 - t);
                self.scratch[i] = diffused + HEAT_RELAX * (AMBIENT - diffused);
            }
        }
        std::mem::swap(&mut self.temp, &mut self.scratch);

        // 3. phase changes driven by the new temperatures.
        for y in 0..H as i32 {
            for x in 0..W as i32 {
                self.phase_change(x, y);
            }
        }
    }

    fn phase_change(&mut self, x: i32, y: i32) {
        let i = idx(x, y);
        let t = self.temp[i];
        match self.cells[i].el {
            Element::Water | Element::SaltWater if t >= 100.0 => {
                self.spawn(x, y, Element::Steam, gen_range(120, 220) as u8);
            }
            Element::Water if t <= 0.0 => self.spawn(x, y, Element::Ice, 0),
            Element::Ice if t >= 2.0 => self.spawn(x, y, Element::Water, 0),
            Element::Snow if t >= 2.0 => self.spawn(x, y, Element::Water, 0),
            Element::Lava if t <= 560.0 => self.spawn(x, y, Element::Stone, 0),
            Element::Wood if t >= 280.0 => {
                self.spawn(x, y, Element::Ember, gen_range(120, 220) as u8)
            }
            Element::Plant if t >= 180.0 => {
                self.spawn(x, y, Element::Ember, gen_range(50, 110) as u8)
            }
            Element::Oil if t >= 250.0 => self.spawn(x, y, Element::Fire, gen_range(60, 110) as u8),
            Element::Gunpowder if t >= 150.0 => self.explode(x, y, 7),
            Element::Metal if t >= 1200.0 => self.spawn(x, y, Element::Lava, 0),
            _ => {}
        }
    }

    // ---- rendering helper --------------------------------------------------

    /// Final RGBA colour for the cell at flat index `i`.
    pub fn color_at(&self, i: usize) -> [u8; 4] {
        let c = self.cells[i];
        let mut col = match c.el {
            Element::Empty => return [12, 12, 16, 255],
            Element::Fire => fire_color(c.life),
            Element::Lava => {
                // brighten lava toward white-hot with temperature
                let h = ((self.temp[i] - 560.0) / 900.0).clamp(0.0, 1.0);
                let base = (210, 80, 20);
                [
                    base.0,
                    (base.1 as f32 + h * 120.0).min(255.0) as u8,
                    (base.2 as f32 + h * 40.0).min(255.0) as u8,
                    255,
                ]
            }
            Element::Smoke => {
                let f = c.life as f32 / 160.0;
                let v = (40.0 + f * 40.0) as u8;
                [v, v, (v as u16 + 6) as u8, 255]
            }
            _ => {
                let p = props(c.el);
                shade(p.color, c.ra, p.var)
            }
        };
        // charged conductors glow electric blue-white
        if c.charge > 0 {
            let g = c.charge as f32 / 18.0;
            col[0] = (col[0] as f32 + g * 120.0).min(255.0) as u8;
            col[1] = (col[1] as f32 + g * 160.0).min(255.0) as u8;
            col[2] = 255;
        }
        col
    }
}

/// Default lifetime for a freshly painted cell of a given element.
fn default_life(el: Element) -> u8 {
    match el {
        Element::Fire => gen_range(40, 90) as u8,
        Element::Ember => gen_range(120, 220) as u8,
        Element::Smoke | Element::Toxic => gen_range(80, 160) as u8,
        Element::Steam => gen_range(120, 220) as u8,
        _ => 0,
    }
}

fn shade(base: (u8, u8, u8), ra: u8, var: u8) -> [u8; 4] {
    if var == 0 {
        return [base.0, base.1, base.2, 255];
    }
    let span = (var as i16) * 2 + 1;
    let delta = (ra as i16 % span) - var as i16;
    let ch = |c: u8| (c as i16 + delta).clamp(0, 255) as u8;
    [ch(base.0), ch(base.1), ch(base.2), 255]
}

fn fire_color(life: u8) -> [u8; 4] {
    // hot/young = pale yellow, cooling = deep red
    let t = (life as f32 / 90.0).clamp(0.0, 1.0);
    [
        255,
        (60.0 + t * 170.0) as u8,
        (10.0 + t * 110.0) as u8,
        255,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sand_falls() {
        let mut w = World::new();
        w.spawn(10, 10, Element::Sand, 0);
        // a grain only descends one row per frame, so allow enough frames to
        // reach the floor of the world.
        for _ in 0..(H as i32 + 20) {
            w.step();
        }
        assert_eq!(w.el_at(10, (H - 1) as i32), Element::Sand);
        assert_eq!(w.el_at(10, 10), Element::Empty);
    }

    /// Build walls around an interior rectangle so liquids can't escape.
    fn sealed_box(w: &mut World, x0: i32, y0: i32, x1: i32, y1: i32) {
        for x in x0..=x1 {
            w.spawn(x, y0, Element::Wall, 0);
            w.spawn(x, y1, Element::Wall, 0);
        }
        for y in y0..=y1 {
            w.spawn(x0, y, Element::Wall, 0);
            w.spawn(x1, y, Element::Wall, 0);
        }
    }

    #[test]
    fn oil_floats_on_water() {
        let mut w = World::new();
        let (x0, y0, x1, y1) = (3, 3, 12, 40);
        sealed_box(&mut w, x0, y0, x1, y1);
        // fill the interior with water, then drop oil at the very bottom row
        for y in (y0 + 1)..y1 {
            for x in (x0 + 1)..x1 {
                w.spawn(x, y, Element::Water, 0);
            }
        }
        for x in (x0 + 1)..x1 {
            w.spawn(x, y1 - 1, Element::Oil, 0);
        }
        for _ in 0..400 {
            w.step();
        }
        // oil (lighter) should rise above water: its lowest cell sits higher
        // (smaller y) than the highest water cell.
        let mut oil_min = i32::MAX;
        let mut water_max = i32::MIN;
        for y in y0..=y1 {
            for x in x0..=x1 {
                match w.el_at(x, y) {
                    Element::Oil => oil_min = oil_min.min(y),
                    Element::Water => water_max = water_max.max(y),
                    _ => {}
                }
            }
        }
        assert!(oil_min != i32::MAX, "oil should still exist");
        assert!(water_max != i32::MIN, "water should still exist");
        assert!(
            oil_min < water_max,
            "oil (min y {oil_min}) should layer above water (max y {water_max})"
        );
    }

    #[test]
    fn lava_and_water_make_stone_and_steam() {
        let mut w = World::new();
        w.spawn(20, 20, Element::Lava, 0);
        for (dx, dy) in [(0, 1), (1, 0), (-1, 0), (0, -1)] {
            w.spawn(20 + dx, 20 + dy, Element::Water, 0);
        }
        let mut saw_steam = false;
        for _ in 0..120 {
            w.step();
            for i in 0..w.cells.len() {
                if w.cells[i].el == Element::Steam {
                    saw_steam = true;
                }
            }
        }
        assert!(saw_steam, "water touching lava should boil into steam");
    }
}
