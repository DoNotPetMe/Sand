//! Articulated ragdoll humans — a Verlet point-mass physics layer that lives on
//! top of the sand grid. Bodies fall, collide with terrain, can be grabbed and
//! flung with the mouse, and react to the simulation: fire ignites and chars
//! limbs, bullets punch through them, explosions send them flying, lava and
//! acid kill, water makes them float, and electricity makes them spasm. Limbs
//! sever at zero health and everything bleeds onto the grid.

use crate::element::Element;
use crate::world::{World, H, W};
use macroquad::prelude::*;
use macroquad::rand::gen_range;

const GRAV: f32 = 0.42;
const DAMP: f32 = 0.985;
const ITERS: usize = 5;
const MAXV: f32 = 6.0; // velocity clamp; swept collision keeps fast bodies from tunnelling

// Particle indices for the humanoid skeleton (feet on the ground, head on top).
const HEAD: usize = 0;
const NECK: usize = 1;
const CHEST: usize = 2;
const PELVIS: usize = 3;
const L_SHOULDER: usize = 4;
const L_ELBOW: usize = 5;
const L_HAND: usize = 6;
const R_SHOULDER: usize = 7;
const R_ELBOW: usize = 8;
const R_HAND: usize = 9;
const L_KNEE: usize = 10;
const L_FOOT: usize = 11;
const R_KNEE: usize = 12;
const R_FOOT: usize = 13;
const N: usize = 14;

/// Standing rest pose: each joint's offset from the feet-centre, in grid cells.
const POSE: [(f32, f32); N] = [
    (0.0, -33.0),  // head
    (0.0, -27.0),  // neck
    (0.0, -23.0),  // chest
    (0.0, -14.0),  // pelvis
    (-4.0, -24.0), // L shoulder
    (-6.5, -18.0), // L elbow
    (-7.5, -12.0), // L hand
    (4.0, -24.0),  // R shoulder
    (6.5, -18.0),  // R elbow
    (7.5, -12.0),  // R hand
    (-3.0, -7.0),  // L knee
    (-3.0, 0.0),   // L foot
    (3.0, -7.0),   // R knee
    (3.0, 0.0),    // R foot
];

/// The six damageable regions of a body.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Part {
    Head,
    Torso,
    ArmL,
    ArmR,
    LegL,
    LegR,
}
const PARTS: [Part; 6] = [
    Part::Head,
    Part::Torso,
    Part::ArmL,
    Part::ArmR,
    Part::LegL,
    Part::LegR,
];

impl Part {
    fn idx(self) -> usize {
        self as usize
    }
    /// Which particles make up this region (severing cuts links that cross out).
    fn particles(self) -> &'static [usize] {
        match self {
            Part::Head => &[HEAD],
            Part::Torso => &[NECK, CHEST, PELVIS],
            Part::ArmL => &[L_SHOULDER, L_ELBOW, L_HAND],
            Part::ArmR => &[R_SHOULDER, R_ELBOW, R_HAND],
            Part::LegL => &[L_KNEE, L_FOOT],
            Part::LegR => &[R_KNEE, R_FOOT],
        }
    }
    fn max_hp(self) -> f32 {
        match self {
            Part::Head => 45.0,
            Part::Torso => 120.0,
            Part::ArmL | Part::ArmR => 50.0,
            Part::LegL | Part::LegR => 60.0,
        }
    }
}

/// Which region a particle index belongs to (for damage routing & colour).
fn particle_part(i: usize) -> Part {
    match i {
        HEAD => Part::Head,
        NECK | CHEST | PELVIS => Part::Torso,
        L_SHOULDER | L_ELBOW | L_HAND => Part::ArmL,
        R_SHOULDER | R_ELBOW | R_HAND => Part::ArmR,
        L_KNEE | L_FOOT => Part::LegL,
        _ => Part::LegR,
    }
}

#[derive(Clone, Copy)]
struct Particle {
    x: f32,
    y: f32,
    px: f32,
    py: f32,
    pinned: bool,
}

impl Particle {
    fn new(x: f32, y: f32) -> Self {
        Particle { x, y, px: x, py: y, pinned: false }
    }
    /// Apply an impulse by displacing the Verlet history.
    fn shove(&mut self, fx: f32, fy: f32) {
        self.px -= fx;
        self.py -= fy;
    }
}

#[derive(Clone, Copy)]
struct Stick {
    a: usize,
    b: usize,
    len: f32,
    broken: bool,
}

pub struct Body {
    particles: Vec<Particle>,
    sticks: Vec<Stick>,
    hp: [f32; 6],
    burn: [f32; 6], // remaining burn time per part
    char: [f32; 6], // 0..1 blackening for rendering
    severed: [bool; 6],
    facing: f32,          // +1 faces right, -1 faces left (cosmetic)
    anchor: Option<f32>,  // x the feet are planted at, for drift-free standing
    pub dead: bool,
}

impl Body {
    /// Build a standing humanoid whose feet rest at (cx, cy).
    fn human(cx: f32, cy: f32) -> Self {
        let particles: Vec<Particle> = POSE
            .iter()
            .map(|&(dx, dy)| Particle::new(cx + dx, cy + dy))
            .collect();

        // limbs first, then internal bracing so the torso keeps its shape
        let links = [
            (HEAD, NECK),
            (NECK, CHEST),
            (CHEST, PELVIS),
            (CHEST, L_SHOULDER),
            (L_SHOULDER, L_ELBOW),
            (L_ELBOW, L_HAND),
            (CHEST, R_SHOULDER),
            (R_SHOULDER, R_ELBOW),
            (R_ELBOW, R_HAND),
            (PELVIS, L_KNEE),
            (L_KNEE, L_FOOT),
            (PELVIS, R_KNEE),
            (R_KNEE, R_FOOT),
            // bracing
            (L_SHOULDER, R_SHOULDER),
            (L_SHOULDER, PELVIS),
            (R_SHOULDER, PELVIS),
            (HEAD, CHEST),
            (L_KNEE, R_KNEE),
        ];
        let sticks = links
            .iter()
            .map(|&(a, b)| {
                let dx = particles[b].x - particles[a].x;
                let dy = particles[b].y - particles[a].y;
                Stick { a, b, len: (dx * dx + dy * dy).sqrt(), broken: false }
            })
            .collect();

        let mut hp = [0.0; 6];
        for p in PARTS {
            hp[p.idx()] = p.max_hp();
        }
        Body {
            particles,
            sticks,
            hp,
            burn: [0.0; 6],
            char: [0.0; 6],
            severed: [false; 6],
            facing: if gen_range(0, 2) == 0 { 1.0 } else { -1.0 },
            anchor: None,
            dead: false,
        }
    }

    fn part_pos(&self, part: Part) -> (f32, f32) {
        let ps = part.particles();
        let (mut sx, mut sy) = (0.0, 0.0);
        for &i in ps {
            sx += self.particles[i].x;
            sy += self.particles[i].y;
        }
        (sx / ps.len() as f32, sy / ps.len() as f32)
    }

    fn hurt(&mut self, part: Part, dmg: f32, world: &mut World) {
        if self.severed[part.idx()] {
            return;
        }
        self.hp[part.idx()] -= dmg;
        // spurt a little blood at the wound
        let (x, y) = self.part_pos(part);
        if gen_range(0, 2) == 0 {
            world.drip(x as i32, y as i32, Element::Blood);
        }
        if self.hp[part.idx()] <= 0.0 {
            self.sever(part, world);
        }
    }

    fn sever(&mut self, part: Part, world: &mut World) {
        if self.severed[part.idx()] {
            return;
        }
        self.severed[part.idx()] = true;
        let set = part.particles();
        // cut every link that crosses out of this region
        for s in &mut self.sticks {
            let a_in = set.contains(&s.a);
            let b_in = set.contains(&s.b);
            if a_in != b_in {
                s.broken = true;
            }
        }
        // gush of blood at the stump
        let (x, y) = self.part_pos(part);
        for _ in 0..16 {
            let bx = x as i32 + gen_range(-2, 3);
            let by = y as i32 + gen_range(-2, 3);
            world.drip(bx, by, Element::Blood);
        }
        if matches!(part, Part::Head | Part::Torso) {
            self.dead = true;
        }
    }

    fn integrate(&mut self, world: &World) {
        for p in &mut self.particles {
            if p.pinned {
                continue;
            }
            let mut vx = (p.x - p.px) * DAMP;
            let mut vy = (p.y - p.py) * DAMP + GRAV;
            // clamp speed so swept stepping stays cheap and stable
            let sp = (vx * vx + vy * vy).sqrt();
            if sp > MAXV {
                vx *= MAXV / sp;
                vy *= MAXV / sp;
            }
            p.px = p.x;
            p.py = p.y;
            // swept, per-axis: advance in <=1px steps, stopping each axis on a wall
            let n = sp.ceil().max(1.0) as i32;
            let (sx, sy) = (vx / n as f32, vy / n as f32);
            for _ in 0..n {
                if world.solid_at(p.x + sx, p.y) {
                    p.px = p.x; // kill horizontal velocity at the wall
                } else {
                    p.x += sx;
                }
                if world.solid_at(p.x, p.y + sy) {
                    p.py = p.y; // kill vertical velocity at floor/ceiling
                } else {
                    p.y += sy;
                }
            }
            // standing friction: a point resting on the ground shouldn't glide
            if world.solid_at(p.x, p.y + 1.0) {
                let vx = p.x - p.px;
                p.px = p.x - vx * 0.25; // keep 25% of horizontal speed -> quick stop
            }
        }
    }

    fn solve(&mut self, world: &World) {
        for _ in 0..ITERS {
            for s in &self.sticks {
                if s.broken {
                    continue;
                }
                let (ax, ay) = (self.particles[s.a].x, self.particles[s.a].y);
                let (bx, by) = (self.particles[s.b].x, self.particles[s.b].y);
                let dx = bx - ax;
                let dy = by - ay;
                let d = (dx * dx + dy * dy).sqrt().max(0.0001);
                let diff = (s.len - d) / d * 0.5;
                let ox = dx * diff;
                let oy = dy * diff;
                let (pa, pb) = (self.particles[s.a].pinned, self.particles[s.b].pinned);
                if !pa {
                    self.particles[s.a].x -= ox;
                    self.particles[s.a].y -= oy;
                }
                if !pb {
                    self.particles[s.b].x += ox;
                    self.particles[s.b].y += oy;
                }
            }
            for p in &mut self.particles {
                collide(p, world);
            }
        }
    }

    /// A living person actively holds itself upright: while at least one foot is
    /// supported, steer the torso and legs toward the standing rest pose (arms
    /// and forearms stay free to dangle). Weak enough to be shoved over and
    /// flung; switches off at death so corpses go limp.
    fn balance(&mut self, world: &World) {
        let legless = self.severed[Part::LegL.idx()] && self.severed[Part::LegR.idx()];
        if self.dead || legless {
            return;
        }
        let foot_l = self.particles[L_FOOT];
        let foot_r = self.particles[R_FOOT];
        let supported = world.solid_at(foot_l.x, foot_l.y + 1.3)
            || world.solid_at(foot_r.x, foot_r.y + 1.3);
        if !supported {
            self.anchor = None; // lifted off the ground -> re-plant on landing
            return;
        }
        let center = (foot_l.x + foot_r.x) * 0.5;
        // Anchor the stance to an absolute x so tiny solver asymmetries can't
        // make a standing person glide. Within a small deadband the anchor holds
        // firm (zero drift); shove them past it and the stance re-plants where
        // they stumble to.
        let a = self.anchor.get_or_insert(center);
        if (center - *a).abs() > 5.0 {
            *a = center;
        }
        let fx = *a;
        let fy = foot_l.y.min(foot_r.y);
        let k = 0.4;
        // Positional correction that moves a point *and* its Verlet history
        // together, so it injects no velocity (stays grab/throw-friendly).
        let correct = |p: &mut Particle, tx: f32, ty: f32| {
            if p.pinned {
                return;
            }
            let cx = (tx - p.x) * k;
            let cy = (ty - p.y) * k;
            p.x += cx;
            p.y += cy;
            p.px += cx;
            p.py += cy;
        };
        // upright spine + braced shoulders + knees over the feet
        for &i in &[HEAD, NECK, CHEST, PELVIS, L_SHOULDER, R_SHOULDER, L_KNEE, R_KNEE] {
            let (dx, dy) = POSE[i];
            // keep knees over their own foot (POSE x is relative to centre)
            correct(&mut self.particles[i], fx + dx, fy + dy);
        }
        // hold a planted stance so the legs don't scissor together
        let cx = (foot_l.x + foot_r.x) * 0.5;
        let (lfy, rfy) = (self.particles[L_FOOT].y, self.particles[R_FOOT].y);
        correct(&mut self.particles[L_FOOT], cx - 3.0, lfy);
        correct(&mut self.particles[R_FOOT], cx + 3.0, rfy);
    }
}

/// Resolve a particle against solid terrain by pushing it out of the nearest
/// open face, then killing its velocity along that normal.
fn collide(p: &mut Particle, world: &World) {
    if p.pinned {
        // still keep grabbed limbs inside the world
        p.x = p.x.clamp(0.5, W as f32 - 0.5);
        p.y = p.y.clamp(0.5, H as f32 - 0.5);
        return;
    }
    if !world.solid_at(p.x, p.y) {
        return;
    }
    let cx = p.x.floor();
    let cy = p.y.floor();
    let dl = p.x - cx; // toward left face
    let dr = (cx + 1.0) - p.x;
    let dt = p.y - cy; // toward top face
    let db = (cy + 1.0) - p.y;
    let eps = 0.02;
    let mut best = f32::MAX;
    let mut push = (0.0f32, 0.0f32);
    if !world.solid_at(p.x - 1.0, p.y) && dl < best {
        best = dl;
        push = (-(dl + eps), 0.0);
    }
    if !world.solid_at(p.x + 1.0, p.y) && dr < best {
        best = dr;
        push = (dr + eps, 0.0);
    }
    if !world.solid_at(p.x, p.y - 1.0) && dt < best {
        best = dt;
        push = (0.0, -(dt + eps));
    }
    if !world.solid_at(p.x, p.y + 1.0) && db < best {
        best = db;
        push = (0.0, db + eps);
    }
    // Fully embedded (e.g. the middle of a thick floor): no open face borders
    // this cell. Climb upward one cell per iteration toward open sky.
    if best == f32::MAX {
        push = (0.0, -1.0);
    }
    p.x += push.0;
    p.y += push.1;
    // friction / stop along the contact normal
    if push.0 != 0.0 {
        p.px = p.x;
    }
    if push.1 != 0.0 {
        p.py = p.y;
        // ground friction: damp the tangential (horizontal) velocity
        p.px = p.x - (p.x - p.px) * 0.5;
    }
    // never let a particle leave the world
    p.x = p.x.clamp(0.5, W as f32 - 0.5);
    p.y = p.y.clamp(0.5, H as f32 - 0.5);
}

/// Manager for all bodies plus the mouse-grab state.
pub struct Ragdolls {
    pub bodies: Vec<Body>,
    grab: Option<(usize, usize)>, // (body, particle)
    last_mouse: (f32, f32),
}

impl Ragdolls {
    pub fn new() -> Self {
        Ragdolls { bodies: Vec::new(), grab: None, last_mouse: (0.0, 0.0) }
    }

    pub fn spawn_human(&mut self, x: f32, y: f32) {
        self.bodies.push(Body::human(x, y));
    }

    pub fn count(&self) -> usize {
        self.bodies.len()
    }

    pub fn clear(&mut self) {
        self.bodies.clear();
        self.grab = None;
    }

    /// Grab the nearest particle within reach (grid coords).
    pub fn grab(&mut self, mx: f32, my: f32) {
        let mut best = 9.0f32 * 9.0;
        let mut found = None;
        for (bi, body) in self.bodies.iter().enumerate() {
            for (pi, p) in body.particles.iter().enumerate() {
                let d = (p.x - mx).powi(2) + (p.y - my).powi(2);
                if d < best {
                    best = d;
                    found = Some((bi, pi));
                }
            }
        }
        if let Some((bi, pi)) = found {
            self.grab = Some((bi, pi));
            self.bodies[bi].particles[pi].pinned = true;
            self.last_mouse = (mx, my);
        }
    }

    /// Drag the held particle to follow the mouse, recording velocity to throw.
    pub fn drag(&mut self, mx: f32, my: f32) {
        if let Some((bi, pi)) = self.grab {
            let p = &mut self.bodies[bi].particles[pi];
            p.px = self.last_mouse.0;
            p.py = self.last_mouse.1;
            p.x = mx;
            p.y = my;
        }
        self.last_mouse = (mx, my);
    }

    pub fn release(&mut self) {
        if let Some((bi, pi)) = self.grab.take() {
            self.bodies[bi].particles[pi].pinned = false;
        }
    }

    /// One physics step coupled to the sand world.
    pub fn step(&mut self, world: &mut World) {
        for body in &mut self.bodies {
            body.integrate(world);
            body.solve(world);
            environment(body, world);
            body.balance(world); // last, so the standing pose isn't undone
        }
        // blast knockback from explosions recorded this frame
        let blasts = world.blasts.clone();
        for blast in &blasts {
            for body in &mut self.bodies {
                for i in 0..body.particles.len() {
                    let p = &mut body.particles[i];
                    let dx = p.x - blast.x;
                    let dy = p.y - blast.y;
                    let d = (dx * dx + dy * dy).sqrt();
                    if d < blast.r && d > 0.01 {
                        let f = blast.power * (1.0 - d / blast.r);
                        // (dx,dy) points from the blast to the particle: shove away
                        p.shove(dx / d * f, dy / d * f);
                        let part = particle_part(i);
                        if gen_range(0, 3) == 0 {
                            body.hurt(part, f * 1.2, world);
                        }
                    }
                }
            }
        }
        // tidy up bodies that have entirely fallen apart
        self.bodies
            .retain(|b| b.severed.iter().filter(|&&s| s).count() < 5);
    }

    pub fn draw(&self, scale: f32, ox: f32, oy: f32) {
        for body in &self.bodies {
            let px = |i: usize| body.particles[i].x * scale + ox;
            let py = |i: usize| body.particles[i].y * scale + oy;

            // fleshy limbs: each bone is a thick capsule (line + rounded joints).
            // proximal bones vanish when their limb is severed.
            for &(a, b, th, part, proximal) in BONES {
                if proximal && body.severed[part.idx()] {
                    continue;
                }
                let col = body_color(body, part);
                draw_line(px(a), py(a), px(b), py(b), th * scale, col);
            }
            // round the joints so the capsules read as one continuous body
            for i in 1..body.particles.len() {
                let part = particle_part(i);
                draw_circle(px(i), py(i), JOINT_R[i] * scale, body_color(body, part));
            }
            // head + a little face dot for orientation
            let hcol = body_color(body, Part::Head);
            draw_circle(px(HEAD), py(HEAD), 3.4 * scale, hcol);
            if !body.severed[Part::Head.idx()] && body.char[Part::Head.idx()] < 0.6 {
                let fx = px(HEAD) + body.facing * 1.4 * scale;
                draw_circle(fx, py(HEAD) - 0.3 * scale, 0.6 * scale, Color::from_rgba(30, 25, 25, 255));
            }
        }
    }
}

// Visible bones: (a, b, thickness, colour-part, proximal?). Proximal bones are
// the ones that attach a limb to the trunk, and disappear when it's severed.
const BONES: &[(usize, usize, f32, Part, bool)] = &[
    (NECK, CHEST, 3.0, Part::Torso, false),
    (CHEST, PELVIS, 4.2, Part::Torso, false),
    (HEAD, NECK, 1.9, Part::Torso, false),
    (CHEST, L_SHOULDER, 2.6, Part::ArmL, true),
    (L_SHOULDER, L_ELBOW, 2.3, Part::ArmL, false),
    (L_ELBOW, L_HAND, 2.0, Part::ArmL, false),
    (CHEST, R_SHOULDER, 2.6, Part::ArmR, true),
    (R_SHOULDER, R_ELBOW, 2.3, Part::ArmR, false),
    (R_ELBOW, R_HAND, 2.0, Part::ArmR, false),
    (PELVIS, L_KNEE, 3.0, Part::LegL, true),
    (L_KNEE, L_FOOT, 2.6, Part::LegL, false),
    (PELVIS, R_KNEE, 3.0, Part::LegR, true),
    (R_KNEE, R_FOOT, 2.6, Part::LegR, false),
];

// Joint disc radius per particle (index 0/head drawn separately).
const JOINT_R: [f32; N] = [
    0.0, 1.0, 2.1, 2.1, 1.3, 1.1, 1.2, 1.3, 1.1, 1.2, 1.4, 1.4, 1.4, 1.4,
];

fn body_color(body: &Body, part: Part) -> Color {
    let i = part.idx();
    let frac = (body.hp[i] / part.max_hp()).clamp(0.0, 1.0);
    // healthy flesh -> wounded dark red
    let flesh = (236.0, 188.0, 158.0);
    let wound = (140.0, 38.0, 38.0);
    let mut r = wound.0 + (flesh.0 - wound.0) * frac;
    let mut g = wound.1 + (flesh.1 - wound.1) * frac;
    let mut b = wound.2 + (flesh.2 - wound.2) * frac;
    // blacken with char
    let c = body.char[i].clamp(0.0, 1.0);
    r += (32.0 - r) * c;
    g += (26.0 - g) * c;
    b += (24.0 - b) * c;
    Color::from_rgba(r as u8, g as u8, b as u8, 255)
}

/// Per-particle interaction with the surrounding sand world.
fn environment(body: &mut Body, world: &mut World) {
    for i in 0..body.particles.len() {
        let (x, y) = (body.particles[i].x, body.particles[i].y);
        let (gx, gy) = (x as i32, y as i32);
        let part = particle_part(i);
        let el = world.el_at(gx, gy);
        let temp = world.temp_at(gx, gy);

        // bullets punch through flesh
        if let Some((dx, dy)) = world.take_bullet(gx, gy) {
            body.particles[i].shove(dx as f32 * 2.5, dy as f32 * 2.5);
            body.hurt(part, 28.0, world);
        }

        // heat & fire: ignite and char
        if matches!(el, Element::Fire | Element::Ember | Element::Napalm) || temp > 130.0 {
            body.burn[part.idx()] = body.burn[part.idx()].max(160.0);
        }
        match el {
            Element::Lava => body.hurt(part, 6.0, world),
            Element::Acid => body.hurt(part, 3.0, world),
            Element::Water | Element::SaltWater => {
                // buoyancy + drag, and douse flames
                body.particles[i].shove(0.0, -GRAV * 1.35);
                let p = &mut body.particles[i];
                p.px += (p.x - p.px) * 0.12;
                p.py += (p.y - p.py) * 0.12;
                body.burn[part.idx()] = 0.0;
            }
            _ => {}
        }

        // electricity: violent spasms + damage
        if world.charge_at(gx, gy) > 0 {
            body.particles[i].shove(gen_range(-2.0, 2.0), gen_range(-2.5, 1.5));
            if gen_range(0, 4) == 0 {
                body.hurt(part, 2.0, world);
            }
        }
    }

    // resolve ongoing burns
    for p in PARTS {
        let i = p.idx();
        if body.burn[i] > 0.0 {
            body.burn[i] -= 1.0;
            body.char[i] = (body.char[i] + 0.01).min(1.0);
            let (x, y) = body.part_pos(p);
            if gen_range(0, 3) == 0 {
                world.drip(
                    x as i32 + gen_range(-1, 2),
                    y as i32 - 1,
                    if gen_range(0, 3) == 0 { Element::Smoke } else { Element::Fire },
                );
            }
            body.hurt(p, 0.35, world);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::Blast;

    fn floor(w: &mut World) {
        for x in 0..W as i32 {
            for y in (H as i32 - 3)..H as i32 {
                w.spawn(x, y, Element::Stone, 0);
            }
        }
    }

    #[test]
    fn body_rests_on_ground() {
        let mut w = World::new();
        floor(&mut w);
        let mut r = Ragdolls::new();
        r.spawn_human(60.0, (H - 4) as f32);
        for _ in 0..300 {
            w.step();
            r.step(&mut w);
        }
        assert_eq!(r.count(), 1, "intact body should survive a quiet drop");
        for p in &r.bodies[0].particles {
            assert!(p.x.is_finite() && p.y.is_finite(), "no NaNs in the sim");
            assert!(p.y <= (H - 2) as f32 + 1.0, "body fell through the floor");
        }
    }

    #[test]
    fn living_person_stands() {
        let mut w = World::new();
        floor(&mut w);
        let mut r = Ragdolls::new();
        r.spawn_human(60.0, (H - 4) as f32);
        for _ in 0..200 {
            w.step();
            r.step(&mut w);
        }
        let b = &r.bodies[0];
        let head_y = b.particles[HEAD].y;
        let foot_y = b.particles[L_FOOT].y.max(b.particles[R_FOOT].y);
        assert!(
            foot_y - head_y > 20.0,
            "a living person should stand tall (head above feet), got {}",
            foot_y - head_y
        );
    }

    #[test]
    fn body_does_not_slide() {
        let mut w = World::new();
        floor(&mut w);
        let mut r = Ragdolls::new();
        r.spawn_human(60.0, (H - 4) as f32);
        // let it settle, then check it stays put over many frames
        for _ in 0..120 {
            w.step();
            r.step(&mut w);
        }
        let x0 = r.bodies[0].particles[PELVIS].x;
        for _ in 0..200 {
            w.step();
            r.step(&mut w);
        }
        let x1 = r.bodies[0].particles[PELVIS].x;
        assert!((x1 - x0).abs() < 4.0, "a standing person should not glide, drifted {}", x1 - x0);
    }

    #[test]
    fn explosion_flings_body() {
        let mut w = World::new();
        let mut r = Ragdolls::new();
        r.spawn_human(160.0, 100.0);
        let before = r.bodies[0].particles[CHEST].x;
        // a blast to the left should shove the body rightward
        w.blasts.push(Blast { x: 150.0, y: 100.0, r: 40.0, power: 30.0 });
        r.step(&mut w);
        w.blasts.clear();
        for _ in 0..6 {
            r.step(&mut w);
        }
        let after = r.bodies[0].particles[CHEST].x;
        assert!(after > before + 1.0, "blast should fling the body away");
    }

    #[test]
    fn fire_kills_body() {
        let mut w = World::new();
        for x in 45..75 {
            for y in 90..120 {
                w.spawn(x, y, Element::Fire, 250);
            }
        }
        let mut r = Ragdolls::new();
        r.spawn_human(60.0, 100.0);
        for _ in 0..400 {
            r.step(&mut w);
        }
        assert!(
            r.count() == 0 || r.bodies[0].dead,
            "burning should kill the ragdoll"
        );
    }
}
