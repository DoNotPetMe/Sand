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
const ITERS: usize = 4;
const MAXV: f32 = 6.0; // velocity clamp; swept collision keeps fast bodies from tunnelling

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
            Part::Head => &[0],
            Part::Torso => &[1, 2],
            Part::ArmL => &[3, 4, 5],
            Part::ArmR => &[6, 7, 8],
            Part::LegL => &[9, 10],
            Part::LegR => &[11, 12],
        }
    }
    fn max_hp(self) -> f32 {
        match self {
            Part::Head => 45.0,
            Part::Torso => 110.0,
            Part::ArmL | Part::ArmR => 50.0,
            Part::LegL | Part::LegR => 60.0,
        }
    }
}

/// Which region a particle index belongs to (for damage routing & colour).
fn particle_part(i: usize) -> Part {
    match i {
        0 => Part::Head,
        1 | 2 => Part::Torso,
        3 | 4 | 5 => Part::ArmL,
        6 | 7 | 8 => Part::ArmR,
        9 | 10 => Part::LegL,
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
    pub dead: bool,
}

impl Body {
    /// Build a standing humanoid centred on (cx, cy) at the feet.
    fn human(cx: f32, cy: f32) -> Self {
        // skeleton layout (head at top, feet at cy)
        let pts = [
            (0.0, -26.0), // 0 head
            (0.0, -21.0), // 1 neck
            (0.0, -12.0), // 2 pelvis
            (-3.0, -20.0), // 3 L shoulder
            (-5.0, -15.0), // 4 L elbow
            (-5.0, -10.0), // 5 L hand
            (3.0, -20.0),  // 6 R shoulder
            (5.0, -15.0),  // 7 R elbow
            (5.0, -10.0),  // 8 R hand
            (-2.0, -6.0),  // 9 L knee
            (-2.0, -1.0),  // 10 L foot
            (2.0, -6.0),   // 11 R knee
            (2.0, -1.0),   // 12 R foot
        ];
        let particles: Vec<Particle> =
            pts.iter().map(|&(dx, dy)| Particle::new(cx + dx, cy + dy)).collect();

        let links = [
            (0, 1), // head-neck
            (1, 2), // spine
            (1, 3), (3, 4), (4, 5), // left arm
            (1, 6), (6, 7), (7, 8), // right arm
            (2, 9), (9, 10),        // left leg
            (2, 11), (11, 12),      // right leg
            (3, 6), (3, 2), (6, 2), // torso bracing
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
            dead: false,
        }
    }

    /// A living person actively holds itself upright: while at least one foot
    /// is supported, gently steer the pelvis above the feet and the head above
    /// the pelvis. Weak enough to be shoved over (and flung), and it switches
    /// off at death so corpses go limp.
    fn balance(&mut self, world: &World) {
        if self.dead || self.severed[Part::LegL.idx()] && self.severed[Part::LegR.idx()] {
            return;
        }
        let foot_l = self.particles[10];
        let foot_r = self.particles[12];
        let supported = world.solid_at(foot_l.x, foot_l.y + 1.3)
            || world.solid_at(foot_r.x, foot_r.y + 1.3);
        if !supported {
            return;
        }
        let fx = (foot_l.x + foot_r.x) * 0.5;
        let fy = foot_l.y.min(foot_r.y);
        let k = 0.35;
        // Pull the torso + legs toward an upright pose relative to the planted
        // feet. Arms are left free to dangle. The correction moves a point and
        // its Verlet history together, so it injects no velocity and stays
        // grab/throw-friendly (and it's gated on foot support, so a body flung
        // into the air just ragdolls).
        const POSE: &[(usize, f32, f32)] = &[
            (0, 0.0, -25.0),  // head
            (1, 0.0, -20.0),  // neck
            (2, 0.0, -11.0),  // pelvis
            (3, -3.0, -19.0), // L shoulder
            (6, 3.0, -19.0),  // R shoulder
            (9, -1.5, -6.0),  // L knee
            (11, 1.5, -6.0),  // R knee
        ];
        for &(i, dx, dy) in POSE {
            let p = &mut self.particles[i];
            if p.pinned {
                continue;
            }
            let cx = (fx + dx - p.x) * k;
            let cy = (fy + dy - p.y) * k;
            p.x += cx;
            p.y += cy;
            p.px += cx;
            p.py += cy;
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
        for _ in 0..14 {
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
        let mut best = 7.0f32 * 7.0;
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
            // limbs
            for s in &body.sticks {
                if s.broken {
                    continue;
                }
                let a = &body.particles[s.a];
                let b = &body.particles[s.b];
                let part = pick_part(s.a, s.b);
                let col = body_color(body, part);
                let thick = match part {
                    Part::Torso | Part::LegL | Part::LegR => scale * 2.1,
                    _ => scale * 1.6,
                };
                draw_line(
                    a.x * scale + ox,
                    a.y * scale + oy,
                    b.x * scale + ox,
                    b.y * scale + oy,
                    thick,
                    col,
                );
            }
            // head
            if !body.severed[Part::Head.idx()] || true {
                let h = &body.particles[0];
                let col = body_color(body, Part::Head);
                draw_circle(h.x * scale + ox, h.y * scale + oy, scale * 2.4, col);
            }
        }
    }
}

/// Choose which region colours a link (prefer the limb end over the torso).
fn pick_part(a: usize, b: usize) -> Part {
    let pa = particle_part(a);
    let pb = particle_part(b);
    if matches!(pa, Part::Torso) {
        pb
    } else {
        pa
    }
}

fn body_color(body: &Body, part: Part) -> Color {
    let i = part.idx();
    let frac = (body.hp[i] / part.max_hp()).clamp(0.0, 1.0);
    // healthy flesh -> wounded dark red
    let flesh = (235.0, 180.0, 150.0);
    let wound = (135.0, 35.0, 35.0);
    let mut r = wound.0 + (flesh.0 - wound.0) * frac;
    let mut g = wound.1 + (flesh.1 - wound.1) * frac;
    let mut b = wound.2 + (flesh.2 - wound.2) * frac;
    // blacken with char
    let c = body.char[i].clamp(0.0, 1.0);
    r += (35.0 - r) * c;
    g += (28.0 - g) * c;
    b += (26.0 - b) * c;
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
            body.particles[i].shove(
                gen_range(-2.0, 2.0),
                gen_range(-2.5, 1.5),
            );
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
            // shouldn't sink past the stone floor
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
        let head_y = b.particles[0].y;
        let foot_y = b.particles[10].y.max(b.particles[12].y);
        assert!(
            foot_y - head_y > 12.0,
            "a living person should stand (head above feet), got {}",
            foot_y - head_y
        );
    }

    #[test]
    fn explosion_flings_body() {
        let mut w = World::new();
        let mut r = Ragdolls::new();
        r.spawn_human(160.0, 100.0);
        let before = r.bodies[0].particles[2].x;
        // a blast to the left should shove the body rightward
        w.blasts.push(Blast { x: 150.0, y: 100.0, r: 40.0, power: 30.0 });
        r.step(&mut w);
        w.blasts.clear();
        for _ in 0..6 {
            r.step(&mut w);
        }
        let after = r.bodies[0].particles[2].x;
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
