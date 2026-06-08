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

use crate::element::Element::*;
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
    pub life: u8,   // generic countdown / health (fire lifetime, person HP, ...)
    pub charge: u8, // electrical charge level
    pub dir: u8,    // facing/heading: 1=up 2=down 3=left 4=right (projectiles, creatures)
    pub timer: u8,  // misc countdown / stored state (fuses, cooldowns, cloner material)
    pub moved: bool,
}

impl Cell {
    pub const EMPTY: Cell = Cell {
        el: Element::Empty,
        ra: 0,
        life: 0,
        charge: 0,
        dir: 0,
        timer: 0,
        moved: false,
    };
}

/// Decode a heading byte into a unit (dx, dy) step.
#[inline]
fn dir_delta(dir: u8) -> (i32, i32) {
    match dir {
        1 => (0, -1),
        2 => (0, 1),
        3 => (-1, 0),
        4 => (1, 0),
        _ => (0, -1),
    }
}

pub struct World {
    pub cells: Vec<Cell>,
    pub temp: Vec<f32>,
    scratch: Vec<f32>, // double buffer for heat diffusion
    pub frame: u64,
    /// Prevailing wind: -1 blows left, +1 right, 0 calm. Drifts gases & fire.
    pub wind: i32,
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
            wind: 0,
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
            dir: 0,
            timer: 0,
            moved: true,
        };
        if let Some(t) = natural_temp(el) {
            self.temp[i] = t;
        }
    }

    /// Seed sensible life / heading / fuse values for elements that need them,
    /// for cells the *user* paints directly.
    fn init_special(&mut self, i: usize) {
        let el = self.cells[i].el;
        let c = &mut self.cells[i];
        match el {
            Element::Person | Element::Zombie => {
                c.life = 100;
                c.dir = if gen_range(0, 2) == 0 { 3 } else { 4 };
            }
            Element::Fish => c.life = 90,
            Element::Grenade => c.timer = gen_range(70, 140) as u8,
            Element::Fireworks => {
                c.life = gen_range(45, 80) as u8;
                c.dir = 1; // launches upward
            }
            Element::Missile => {
                c.life = 255;
                c.dir = 1;
            }
            Element::Bullet => {
                c.life = 110;
                c.dir = 4;
            }
            Element::Gun => {
                c.dir = 4; // fires to the right
                c.timer = gen_range(20, 40) as u8;
            }
            Element::Laser => c.dir = 2, // beams downward
            _ => {}
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
                            dir: 0,
                            timer: 0,
                            moved: true,
                        };
                        if let Some(t) = natural_temp(el) {
                            self.temp[i] = t;
                        }
                        self.init_special(i);
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
            Empty | Wall | Stone | Brick | Glass | Metal => {}
            Fire => self.update_fire(x, y),
            Ember => self.update_ember(x, y),
            Smoke | Toxic => self.update_fading_gas(x, y),
            Steam => self.update_steam(x, y),
            Acid => self.update_acid(x, y),
            Napalm => self.update_napalm(x, y),
            Plant => self.update_plant(x, y),
            Seed => self.update_seed(x, y),
            Ant => self.update_ant(x, y),
            Person => self.update_person(x, y),
            Zombie => self.update_zombie(x, y),
            Fish => self.update_fish(x, y),
            Virus => self.update_virus(x, y),
            Salt => self.update_salt(x, y),
            Concrete => self.update_concrete(x, y),
            Bomb => self.update_bomb(x, y),
            Tnt => self.update_tnt(x, y),
            C4 => self.update_c4(x, y),
            Nuke => self.update_nuke(x, y),
            Mine => self.update_mine(x, y),
            Grenade => self.update_grenade(x, y),
            Fireworks => self.update_fireworks(x, y),
            Missile => self.update_missile(x, y),
            Bullet => self.update_bullet(x, y),
            Gun => self.update_gun(x, y),
            Laser => self.update_laser(x, y),
            Meteor => self.update_meteor(x, y),
            Volcano => self.update_volcano(x, y),
            WaterSource => self.update_source(x, y),
            Cloner => self.update_cloner(x, y),
            Void => self.update_void(x, y),
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
        // wind shoves gases & fire sideways before they get a chance to rise
        if self.wind != 0 && chance(2) && self.flow_side(x, y, self.wind) {
            return true;
        }
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

    /// A much larger blast: an incinerating core, a radioactive smoke/toxic
    /// fallout ring, and a huge thermal spike.
    fn nuke_blast(&mut self, cx: i32, cy: i32) {
        let r = 46;
        for dy in -r..=r {
            for dx in -r..=r {
                let d2 = dx * dx + dy * dy;
                if d2 > r * r {
                    continue;
                }
                let (x, y) = (cx + dx, cy + dy);
                if !in_bounds(x, y) {
                    continue;
                }
                let i = idx(x, y);
                let roll = gen_range(0, 100);
                if (d2 as f32) < (r as f32 * 0.45).powi(2) {
                    if roll < 70 {
                        self.spawn(x, y, Fire, gen_range(80, 180) as u8);
                    } else {
                        self.cells[i] = Cell::EMPTY;
                    }
                    self.temp[i] = 2200.0;
                } else if self.el_at(x, y) != Wall {
                    if roll < 25 {
                        self.spawn(x, y, Fire, gen_range(40, 120) as u8);
                    } else if roll < 50 {
                        self.spawn(x, y, Smoke, gen_range(120, 220) as u8);
                    } else if roll < 62 {
                        self.spawn(x, y, Toxic, gen_range(150, 255) as u8);
                    } else if roll < 75 {
                        self.cells[i] = Cell::EMPTY;
                    }
                    self.temp[i] = 950.0;
                }
            }
        }
    }

    // ---- movement helpers for creatures & projectiles ---------------------

    /// Swap two cells outright (used by creatures swimming/walking through
    /// fluids regardless of density).
    fn swap_cells(&mut self, x: i32, y: i32, nx: i32, ny: i32) {
        if !in_bounds(nx, ny) {
            return;
        }
        let i = idx(x, y);
        let j = idx(nx, ny);
        let mut a = self.cells[i];
        let mut b = self.cells[j];
        a.moved = true;
        b.moved = true;
        self.cells[j] = a;
        self.cells[i] = b;
        self.temp.swap(i, j);
    }

    /// Move a cell into an assumed-passable destination, clearing the origin.
    fn relocate(&mut self, x: i32, y: i32, nx: i32, ny: i32) {
        let i = idx(x, y);
        let j = idx(nx, ny);
        let mut c = self.cells[i];
        c.moved = true;
        self.cells[j] = c;
        self.cells[i] = Cell::EMPTY;
        self.temp.swap(i, j);
    }

    /// Cells a projectile flies through without stopping: air and gases.
    fn proj_passable(&self, x: i32, y: i32) -> bool {
        if !in_bounds(x, y) {
            return false;
        }
        let e = self.el_at(x, y);
        e == Empty || props(e).cat == Cat::Gas
    }

    // ---- burning liquids --------------------------------------------------

    fn update_napalm(&mut self, x: i32, y: i32) {
        let i = idx(x, y);
        let mut c = self.cells[i];
        if self.has_neighbor(x, y, &[Water, SaltWater]) && chance(2) {
            self.spawn(x, y, Smoke, gen_range(40, 90) as u8);
            return;
        }
        if c.life == 0 {
            if chance(2) {
                self.spawn(x, y, Ash, 0);
            } else {
                self.spawn(x, y, Smoke, gen_range(60, 120) as u8);
            }
            return;
        }
        c.life -= 1;
        self.cells[i] = c;
        // throw flames into the air around it
        for (dx, dy) in [(0, -1), (1, 0), (-1, 0)] {
            if self.el_at(x + dx, y + dy) == Empty && chance(3) {
                self.spawn(x + dx, y + dy, Fire, gen_range(20, 50) as u8);
            }
        }
        // sticky: only creeps occasionally
        if chance(2) {
            self.update_liquid(x, y);
        }
    }

    fn update_concrete(&mut self, x: i32, y: i32) {
        if self.update_powder(x, y) {
            return; // still settling
        }
        let i = idx(x, y);
        let mut c = self.cells[i];
        c.timer = c.timer.saturating_add(1);
        if c.timer > 50 {
            self.spawn(x, y, Stone, 0); // cured into rock
            return;
        }
        self.cells[i] = c;
    }

    // ---- people & creatures -----------------------------------------------

    fn update_person(&mut self, x: i32, y: i32) {
        let i = idx(x, y);
        let mut c = self.cells[i];
        let mut hp = c.life as i32;
        let mut flee = 0i32;
        for (dx, dy) in [(0, 1), (0, -1), (1, 0), (-1, 0)] {
            match self.el_at(x + dx, y + dy) {
                Fire | Ember => {
                    hp -= 8;
                    flee = -dx;
                }
                Napalm => {
                    hp -= 25;
                    flee = -dx;
                }
                Lava => hp -= 60,
                Acid => hp -= 20,
                Toxic => hp -= 2,
                Water | SaltWater => hp -= 3, // drowning
                _ => {}
            }
        }
        if hp <= 0 {
            self.spawn(x, y, Blood, 0);
            for (dx, dy) in [(1, 0), (-1, 0), (0, -1)] {
                if self.el_at(x + dx, y + dy) == Empty && chance(2) {
                    self.spawn(x + dx, y + dy, Blood, 0);
                }
            }
            return;
        }
        c.life = hp.min(100) as u8;
        // flee fire by turning away from it
        if flee > 0 {
            c.dir = 4;
        } else if flee < 0 {
            c.dir = 3;
        } else if chance(60) {
            c.dir = if c.dir == 3 { 4 } else { 3 };
        }
        self.cells[i] = c;

        // gravity
        if self.el_at(x, y + 1) == Empty {
            self.try_move(x, y, x, y + 1, true);
            return;
        }
        // sink in liquids
        if is_fluid(self.el_at(x, y + 1)) {
            self.try_move(x, y, x, y + 1, true);
            return;
        }
        // walk, climbing one-cell steps
        let step = if c.dir == 3 { -1 } else { 1 };
        let ahead = self.el_at(x + step, y);
        if ahead == Empty {
            self.try_move(x, y, x + step, y, true);
        } else if ahead != Wall
            && !is_fluid(ahead)
            && self.el_at(x + step, y - 1) == Empty
            && self.el_at(x, y - 1) == Empty
        {
            self.try_move(x, y, x + step, y - 1, true);
        } else {
            let mut nc = self.cells[i];
            nc.dir = if step < 0 { 4 } else { 3 };
            self.cells[i] = nc;
        }
    }

    fn update_zombie(&mut self, x: i32, y: i32) {
        let i = idx(x, y);
        let mut c = self.cells[i];
        let mut hp = c.life as i32;
        for (dx, dy) in [(0, 1), (0, -1), (1, 0), (-1, 0)] {
            match self.el_at(x + dx, y + dy) {
                Fire | Ember | Napalm => hp -= 12,
                Lava => hp -= 60,
                Acid => hp -= 25,
                // bite a person and turn them
                Person => {
                    self.spawn(x + dx, y + dy, Zombie, 100);
                }
                _ => {}
            }
        }
        if hp <= 0 {
            self.spawn(x, y, Blood, 0);
            return;
        }
        c.life = hp.min(100) as u8;
        // hunt: turn toward the nearest person on either side
        let mut target_dir = 0i32;
        for d in 1..7 {
            if self.el_at(x + d, y) == Person {
                target_dir = 1;
                break;
            }
            if self.el_at(x - d, y) == Person {
                target_dir = -1;
                break;
            }
        }
        if target_dir > 0 {
            c.dir = 4;
        } else if target_dir < 0 {
            c.dir = 3;
        } else if chance(40) {
            c.dir = if c.dir == 3 { 4 } else { 3 };
        }
        self.cells[i] = c;

        if self.el_at(x, y + 1) == Empty {
            self.try_move(x, y, x, y + 1, true);
            return;
        }
        let step = if c.dir == 3 { -1 } else { 1 };
        let ahead = self.el_at(x + step, y);
        if ahead == Empty {
            self.try_move(x, y, x + step, y, true);
        } else if ahead != Wall
            && !is_fluid(ahead)
            && self.el_at(x + step, y - 1) == Empty
        {
            self.try_move(x, y, x + step, y - 1, true);
        } else {
            let mut nc = self.cells[i];
            nc.dir = if step < 0 { 4 } else { 3 };
            self.cells[i] = nc;
        }
    }

    fn update_fish(&mut self, x: i32, y: i32) {
        let i = idx(x, y);
        let mut c = self.cells[i];
        let mut hp = c.life as i32;
        if self.has_neighbor(x, y, &[Fire, Lava, Acid]) {
            hp -= 40;
        }
        let watery = self.has_neighbor(x, y, &[Water, SaltWater]);
        if !watery {
            hp -= 6; // suffocating in open air
        }
        if hp <= 0 {
            self.spawn(x, y, Blood, 0);
            return;
        }
        c.life = hp.min(100) as u8;
        self.cells[i] = c;
        // swim toward water
        let order = [(0, 1), (0, -1), (1, 0), (-1, 0)];
        let start = gen_range(0, 4) as usize;
        for k in 0..4 {
            let (dx, dy) = order[(start + k) % 4];
            if matches!(self.el_at(x + dx, y + dy), Water | SaltWater) {
                self.swap_cells(x, y, x + dx, y + dy);
                return;
            }
        }
        // flop out of water
        if self.el_at(x, y + 1) == Empty {
            self.try_move(x, y, x, y + 1, true);
        }
    }

    fn update_virus(&mut self, x: i32, y: i32) {
        let i = idx(x, y);
        let mut c = self.cells[i];
        if self.has_neighbor(x, y, &[Fire, Ember, Lava, Acid]) {
            self.spawn(x, y, Ash, 0); // sterilised
            return;
        }
        if c.life == 0 {
            self.cells[i] = Cell::EMPTY; // burns itself out
            return;
        }
        c.life -= 1;
        self.cells[i] = c;
        let (dx, dy) = [(0, 1), (0, -1), (1, 0), (-1, 0)][gen_range(0, 4) as usize];
        let t = self.el_at(x + dx, y + dy);
        if matches!(t, Person | Zombie | Fish | Ant | Plant | Wood | Seed) && chance(3) {
            self.spawn(x + dx, y + dy, Virus, gen_range(180, 255) as u8);
        }
    }

    // ---- weapons ----------------------------------------------------------

    fn update_tnt(&mut self, x: i32, y: i32) {
        let i = idx(x, y);
        if self.cells[i].charge > 0
            || self.temp[i] > 180.0
            || self.has_neighbor(x, y, &[Fire, Ember, Lava])
        {
            self.explode(x, y, 24);
            return;
        }
        self.update_powder(x, y);
    }

    fn update_c4(&mut self, x: i32, y: i32) {
        let i = idx(x, y);
        // stable except under a detonator charge or extreme heat
        if self.cells[i].charge > 0 || self.temp[i] > 400.0 {
            self.explode(x, y, 20);
        }
    }

    fn update_nuke(&mut self, x: i32, y: i32) {
        let i = idx(x, y);
        if self.cells[i].charge > 0
            || self.temp[i] > 300.0
            || self.has_neighbor(x, y, &[Fire, Ember, Lava])
        {
            self.nuke_blast(x, y);
        }
    }

    fn update_mine(&mut self, x: i32, y: i32) {
        let i = idx(x, y);
        if self.cells[i].charge > 0 {
            self.explode(x, y, 12);
            return;
        }
        for (dx, dy) in [
            (0, 1), (0, -1), (1, 0), (-1, 0), (1, 1), (1, -1), (-1, 1), (-1, -1),
        ] {
            if is_creature(self.el_at(x + dx, y + dy)) {
                self.explode(x, y, 12);
                return;
            }
        }
    }

    fn update_grenade(&mut self, x: i32, y: i32) {
        let i = idx(x, y);
        let mut c = self.cells[i];
        if self.has_neighbor(x, y, &[Fire, Ember, Lava]) || c.timer <= 1 {
            self.explode(x, y, 13);
            return;
        }
        c.timer -= 1;
        self.cells[i] = c;
        self.update_powder(x, y);
    }

    fn update_fireworks(&mut self, x: i32, y: i32) {
        let i = idx(x, y);
        let mut c = self.cells[i];
        let (dx, dy) = dir_delta(c.dir);
        if c.life <= 1 || !self.proj_passable(x + dx, y + dy) {
            self.firework_burst(x, y);
            self.cells[idx(x, y)] = Cell::EMPTY;
            return;
        }
        c.life -= 1;
        self.cells[i] = c;
        self.relocate(x, y, x + dx, y + dy);
        if chance(2) {
            self.spawn(x, y, Fire, gen_range(6, 16) as u8); // spark trail
        }
    }

    fn firework_burst(&mut self, cx: i32, cy: i32) {
        let r = 7;
        for dy in -r..=r {
            for dx in -r..=r {
                if dx * dx + dy * dy > r * r {
                    continue;
                }
                let (x, y) = (cx + dx, cy + dy);
                if self.el_at(x, y) == Empty && chance(2) {
                    self.spawn(x, y, Fire, gen_range(20, 70) as u8);
                }
            }
        }
    }

    fn update_missile(&mut self, x: i32, y: i32) {
        let i = idx(x, y);
        let mut c = self.cells[i];
        let (dx, dy) = dir_delta(c.dir);
        if c.life <= 1 {
            self.explode(x, y, 11);
            return;
        }
        c.life -= 1;
        self.cells[i] = c;
        let (mut cx, mut cy) = (x, y);
        let mut hit = false;
        for _ in 0..2 {
            if self.proj_passable(cx + dx, cy + dy) {
                cx += dx;
                cy += dy;
            } else {
                hit = true;
                break;
            }
        }
        if (cx, cy) != (x, y) {
            self.relocate(x, y, cx, cy);
            self.spawn(x, y, if chance(2) { Smoke } else { Fire }, gen_range(20, 50) as u8);
            self.temp[idx(x, y)] = 400.0;
        }
        if hit {
            self.explode(cx, cy, 13);
        }
    }

    fn update_bullet(&mut self, x: i32, y: i32) {
        let i = idx(x, y);
        let mut c = self.cells[i];
        if c.life == 0 {
            self.cells[i] = Cell::EMPTY;
            return;
        }
        c.life = c.life.saturating_sub(4); // limited range
        self.cells[i] = c;
        let (dx, dy) = dir_delta(c.dir);
        let (mut cx, mut cy) = (x, y);
        let mut stop = false;
        for _ in 0..3 {
            let (nx, ny) = (cx + dx, cy + dy);
            if !in_bounds(nx, ny) {
                stop = true;
                break;
            }
            let t = self.el_at(nx, ny);
            if t == Empty || props(t).cat == Cat::Gas || props(t).cat == Cat::Liquid {
                cx = nx;
                cy = ny;
            } else if is_creature(t) {
                self.spawn(nx, ny, Blood, 0); // lethal hit
                stop = true;
                break;
            } else if matches!(t, Glass | Sand | Snow | Ash | Dirt | Plant | Seed) {
                self.cells[idx(nx, ny)] = Cell::EMPTY; // punches through soft stuff
                cx = nx;
                cy = ny;
            } else {
                stop = true; // stone/metal/wall stops it
                break;
            }
        }
        if (cx, cy) != (x, y) {
            self.relocate(x, y, cx, cy);
        }
        if stop {
            self.cells[idx(cx, cy)] = Cell::EMPTY;
        }
    }

    fn update_gun(&mut self, x: i32, y: i32) {
        let i = idx(x, y);
        let mut c = self.cells[i];
        if c.timer > 0 {
            c.timer -= 1;
            self.cells[i] = c;
            return;
        }
        c.timer = gen_range(18, 34) as u8;
        self.cells[i] = c;
        let (dx, dy) = dir_delta(c.dir);
        let (nx, ny) = (x + dx, y + dy);
        if self.el_at(nx, ny) == Empty {
            self.spawn(nx, ny, Bullet, 110);
            let j = idx(nx, ny);
            self.cells[j].dir = c.dir;
        }
    }

    fn update_laser(&mut self, x: i32, y: i32) {
        let dir = self.cells[idx(x, y)].dir;
        let (dx, dy) = dir_delta(dir);
        let (mut cx, mut cy) = (x, y);
        for _ in 0..250 {
            cx += dx;
            cy += dy;
            if !in_bounds(cx, cy) {
                break;
            }
            let t = self.el_at(cx, cy);
            let j = idx(cx, cy);
            if t == Empty {
                self.spawn(cx, cy, Fire, 1); // glowing beam
                self.temp[j] = 600.0;
            } else if props(t).cat == Cat::Gas {
                self.temp[j] = 600.0;
            } else {
                // burns the first solid/liquid/creature it meets, then stops
                self.temp[j] = self.temp[j].max(950.0);
                if is_creature(t) {
                    self.spawn(cx, cy, Blood, 0);
                }
                break;
            }
        }
    }

    // ---- tools ------------------------------------------------------------

    fn update_cloner(&mut self, x: i32, y: i32) {
        let i = idx(x, y);
        let mut c = self.cells[i];
        if c.timer == 0 {
            // learn the first interesting neighbour
            for (dx, dy) in [(0, 1), (0, -1), (1, 0), (-1, 0)] {
                let e = self.el_at(x + dx, y + dy);
                if !matches!(e, Empty | Wall | Cloner | Void | Bullet) {
                    c.timer = el_index(e).saturating_add(1);
                    self.cells[i] = c;
                    break;
                }
            }
        } else {
            let el = el_from_index(c.timer - 1);
            for (dx, dy) in [(0, 1), (0, -1), (1, 0), (-1, 0)] {
                let (nx, ny) = (x + dx, y + dy);
                if self.el_at(nx, ny) == Empty && chance(3) {
                    self.spawn(nx, ny, el, default_life(el));
                }
            }
        }
    }

    fn update_void(&mut self, x: i32, y: i32) {
        for (dx, dy) in [
            (0, 1), (0, -1), (1, 0), (-1, 0), (1, 1), (1, -1), (-1, 1), (-1, -1),
        ] {
            let (nx, ny) = (x + dx, y + dy);
            if in_bounds(nx, ny) {
                let e = self.el_at(nx, ny);
                if !matches!(e, Empty | Wall | Void) {
                    let j = idx(nx, ny);
                    self.cells[j] = Cell::EMPTY;
                    self.temp[j] = AMBIENT;
                }
            }
        }
    }

    // ---- disasters: airborne hazards --------------------------------------

    fn update_meteor(&mut self, x: i32, y: i32) {
        let i = idx(x, y);
        let mut c = self.cells[i];
        if c.life <= 1 {
            self.meteor_impact(x, y);
            return;
        }
        c.life -= 1;
        self.cells[i] = c;
        let (mut cx, mut cy) = (x, y);
        let mut hit = false;
        for _ in 0..3 {
            // meteors plunge down (with a touch of slant from the wind)
            let nx = cx + if chance(4) { self.wind } else { 0 };
            let ny = cy + 1;
            if self.proj_passable(nx, ny) {
                cx = nx;
                cy = ny;
            } else {
                hit = true;
                break;
            }
        }
        if (cx, cy) != (x, y) {
            self.relocate(x, y, cx, cy);
            // blazing tail
            self.spawn(x, y, if chance(2) { Smoke } else { Fire }, gen_range(20, 60) as u8);
            self.temp[idx(x, y)] = 500.0;
        }
        if hit {
            self.meteor_impact(cx, cy);
        }
    }

    fn meteor_impact(&mut self, x: i32, y: i32) {
        self.explode(x, y, 17);
        // leave molten splatter in the crater
        for _ in 0..40 {
            let (dx, dy) = (gen_range(-9, 10), gen_range(-9, 10));
            if dx * dx + dy * dy <= 81 && self.el_at(x + dx, y + dy) == Empty && chance(2) {
                self.spawn(x + dx, y + dy, Lava, 0);
            }
        }
    }

    fn update_volcano(&mut self, x: i32, y: i32) {
        let i = idx(x, y);
        let mut c = self.cells[i];
        // belch lava, fire and smoke upward out of the vent
        for dx in -2..=2 {
            let (nx, ny) = (x + dx, y - 1);
            if self.el_at(nx, ny) == Empty && chance(2) {
                let roll = gen_range(0, 100);
                if roll < 55 {
                    self.spawn(nx, ny, Lava, 0);
                } else if roll < 80 {
                    self.spawn(nx, ny, Fire, gen_range(40, 90) as u8);
                } else {
                    self.spawn(nx, ny, Smoke, gen_range(80, 160) as u8);
                }
            }
        }
        if c.timer <= 1 {
            self.spawn(x, y, Stone, 0); // vent crusts over
            return;
        }
        c.timer -= 1;
        self.cells[i] = c;
    }

    // ---- disasters & weather: public event API ----------------------------

    /// Rain a handful of fiery meteors down from the sky.
    pub fn meteor_shower(&mut self) {
        for _ in 0..gen_range(5, 10) {
            let x = gen_range(0, W as i32);
            let y = gen_range(0, 6);
            self.spawn(x, y, Meteor, gen_range(200, 255) as u8);
            self.cells[idx(x, y)].dir = 2;
        }
    }

    /// Strike a jagged lightning bolt down column `sx`: ignites, super-heats
    /// and charges any wiring it passes through.
    pub fn lightning(&mut self, sx: i32) {
        let mut x = sx.clamp(0, W as i32 - 1);
        for y in 0..H as i32 {
            if chance(2) {
                x = (x + gen_range(-1, 2)).clamp(0, W as i32 - 1);
            }
            // energise nearby conductors so bolts can power circuits
            for dx in -1..=1 {
                if conductive(self.el_at(x + dx, y)) {
                    self.cells[idx(x + dx, y)].charge = 18;
                }
            }
            let t = self.el_at(x, y);
            let j = idx(x, y);
            if t == Empty || props(t).cat == Cat::Gas {
                self.spawn(x, y, Fire, gen_range(4, 10) as u8);
                self.temp[j] = 1200.0;
            } else {
                self.temp[j] = self.temp[j].max(1200.0);
                self.explode(x, y, 5);
                break;
            }
        }
    }

    /// Erupt a volcano from the ground near the centre of the world.
    pub fn erupt_volcano(&mut self) {
        let x = gen_range(W as i32 / 4, 3 * W as i32 / 4);
        let y = H as i32 - 4;
        for dx in -1..=1 {
            self.spawn(x + dx, y, Volcano, 0);
            let i = idx(x + dx, y);
            self.cells[i].timer = gen_range(180, 255) as u8;
        }
    }

    /// Shake the world: crumble masonry and jostle loose powders so structures
    /// collapse.
    pub fn earthquake(&mut self) {
        let n = W * H / 30;
        for _ in 0..n {
            let x = gen_range(0, W as i32);
            let y = gen_range(0, H as i32);
            match self.el_at(x, y) {
                Stone if chance(3) => {
                    self.spawn(x, y, if chance(2) { Sand } else { Dirt }, 0);
                }
                Brick if chance(4) => self.spawn(x, y, Sand, 0),
                Sand | Dirt | Gunpowder | Salt | Coal | Ash | Snow => {
                    let d = if chance(2) { 1 } else { -1 };
                    if self.el_at(x + d, y) == Empty {
                        self.swap_cells(x, y, x + d, y);
                    }
                }
                _ => {}
            }
        }
    }

    /// A rising flood: fill the lower world with water.
    pub fn flood(&mut self) {
        for y in (H as i32 - 26)..(H as i32) {
            for x in 0..W as i32 {
                if self.el_at(x, y) == Empty {
                    self.spawn(x, y, Water, 0);
                }
            }
        }
    }

    /// Unleash a plague: infect people if any exist, else scatter virus.
    pub fn plague(&mut self) {
        let mut infected = false;
        for i in 0..self.cells.len() {
            if self.cells[i].el == Person && chance(14) {
                let (x, y) = ((i % W) as i32, (i / W) as i32);
                self.spawn(x, y, Virus, gen_range(180, 255) as u8);
                infected = true;
            }
        }
        if !infected {
            for _ in 0..40 {
                let x = gen_range(0, W as i32);
                let y = gen_range(0, H as i32);
                if self.el_at(x, y) == Empty {
                    self.spawn(x, y, Virus, gen_range(180, 255) as u8);
                }
            }
        }
    }

    /// Sprinkle a row of droplets from the sky (water or acid). Call per frame
    /// while the weather is on.
    pub fn rain(&mut self, el: Element) {
        for _ in 0..(W / 8) {
            let x = gen_range(0, W as i32);
            if self.el_at(x, 0) == Empty {
                self.spawn(x, 0, el, 0);
            }
        }
    }

    /// Count the living people (for the survival HUD).
    pub fn population(&self) -> usize {
        self.cells.iter().filter(|c| c.el == Person).count()
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
                Fire => self.temp[i] = self.temp[i].max(720.0),
                Ember => self.temp[i] = self.temp[i].max(520.0),
                Napalm => self.temp[i] = self.temp[i].max(520.0),
                Meteor => self.temp[i] = self.temp[i].max(700.0),
                Volcano => self.temp[i] = self.temp[i].max(900.0),
                Heater => self.temp[i] = self.temp[i].max(450.0),
                Cooler => self.temp[i] = self.temp[i].min(-30.0),
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
            Element::Gasoline if t >= 80.0 => self.spawn(x, y, Fire, gen_range(50, 100) as u8),
            Element::Coal if t >= 250.0 => self.spawn(x, y, Ember, gen_range(200, 255) as u8),
            Element::Methane if t >= 150.0 => self.spawn(x, y, Fire, gen_range(40, 90) as u8),
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
            Element::Napalm => {
                // flickering burning goo
                let f = (c.ra % 80) as u16;
                [255, (70 + f) as u8, 20, 255]
            }
            Element::Meteor => {
                // white-hot core with a flickering edge
                let f = (c.ra % 60) as u16;
                [255, (170 + f).min(255) as u8, (60 + f) as u8, 255]
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
        Element::Napalm => gen_range(140, 240) as u8,
        Element::Virus => gen_range(180, 255) as u8,
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

    #[test]
    fn person_dies_in_lava() {
        let mut w = World::new();
        for x in 0..20 {
            w.spawn(x, 30, Element::Wall, 0); // ground so they don't just fall
        }
        // trap a lava cell so it can't flow away from its victim
        w.spawn(4, 29, Element::Wall, 0);
        w.spawn(7, 29, Element::Wall, 0);
        w.spawn(6, 28, Element::Wall, 0);
        w.spawn(6, 29, Element::Lava, 0);
        w.spawn(5, 29, Element::Person, 100);
        for _ in 0..20 {
            w.step();
        }
        let mut alive = 0;
        for c in &w.cells {
            if c.el == Element::Person {
                alive += 1;
            }
        }
        assert_eq!(alive, 0, "person should have perished next to lava");
    }

    #[test]
    fn mine_detonates_on_creature() {
        let mut w = World::new();
        w.spawn(10, 10, Element::Mine, 0);
        w.spawn(11, 10, Element::Ant, 0);
        for _ in 0..4 {
            w.step();
        }
        assert_ne!(w.el_at(10, 10), Element::Mine, "mine should have triggered");
    }

    #[test]
    fn gun_fires_bullets() {
        let mut w = World::new();
        w.paint(5, 10, 0, Element::Gun); // radius 0 -> single turret, init sets it firing right
        let mut saw_bullet = false;
        for _ in 0..80 {
            w.step();
            if w.cells.iter().any(|c| c.el == Element::Bullet) {
                saw_bullet = true;
            }
        }
        assert!(saw_bullet, "turret should have spat out a bullet");
    }

    #[test]
    fn meteor_shower_causes_destruction() {
        let mut w = World::new();
        for x in 0..W as i32 {
            for y in (H as i32 - 3)..H as i32 {
                w.spawn(x, y, Element::Stone, 0);
            }
        }
        w.meteor_shower();
        let mut boom = false;
        for _ in 0..220 {
            w.step();
            if w.cells.iter().any(|c| matches!(c.el, Element::Fire | Element::Lava)) {
                boom = true;
            }
        }
        assert!(boom, "meteors should ignite fire/lava");
    }

    #[test]
    fn earthquake_crumbles_stone() {
        let mut w = World::new();
        for x in 0..40 {
            for y in 50..70 {
                w.spawn(x, y, Element::Stone, 0);
            }
        }
        let before = w.cells.iter().filter(|c| c.el == Element::Stone).count();
        for _ in 0..6 {
            w.earthquake();
            w.step();
        }
        let after = w.cells.iter().filter(|c| c.el == Element::Stone).count();
        assert!(after < before, "earthquake should crumble some stone");
    }
}
