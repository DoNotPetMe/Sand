//! Element catalogue: every material the sandbox knows about, plus the static
//! data (colour, density, category, conductivity ...) that drives the
//! simulation. Behaviour lives in `world.rs`; this file is pure data.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Element {
    Empty,
    // --- static solids -------------------------------------------------
    Wall,   // indestructible boundary material
    Stone,
    Wood,
    Metal,
    Glass,
    Ice,
    // --- powders -------------------------------------------------------
    Sand,
    Salt,
    Gunpowder,
    Snow,
    Ash,
    Seed,
    Bomb,
    // --- liquids -------------------------------------------------------
    Water,
    SaltWater,
    Oil,
    Acid,
    Lava,
    // --- gases ---------------------------------------------------------
    Steam,
    Smoke,
    Toxic,
    Fire,
    // --- life / special ------------------------------------------------
    Plant,
    Ant,
    Ember,        // burning solid (wood/plant on fire)
    WaterSource,  // faucet that endlessly emits water
}

/// Broad movement class for an element.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cat {
    Empty,
    Solid,  // does not fall on its own
    Powder, // falls and piles
    Liquid, // falls and spreads
    Gas,    // rises and disperses
    Life,   // bespoke behaviour
}

/// Static properties of an element.
pub struct Props {
    pub name: &'static str,
    pub cat: Cat,
    /// Higher sinks. Gases are negative so they rise. Used for displacement.
    pub density: i16,
    pub color: (u8, u8, u8),
    /// Per-cell colour jitter amplitude, for natural-looking texture.
    pub var: u8,
}

use Element::*;

/// The big lookup table. Kept as a `match` so it compiles to a jump table and
/// stays allocation-free.
pub fn props(el: Element) -> Props {
    let p = |name, cat, density, color, var| Props { name, cat, density, color, var };
    match el {
        Empty => p("Eraser", Cat::Empty, 0, (12, 12, 16), 0),

        Wall => p("Wall", Cat::Solid, 9000, (80, 80, 92), 6),
        Stone => p("Stone", Cat::Solid, 9000, (110, 105, 100), 14),
        Wood => p("Wood", Cat::Solid, 9000, (120, 80, 40), 12),
        Metal => p("Metal", Cat::Solid, 9000, (150, 155, 165), 10),
        Glass => p("Glass", Cat::Solid, 9000, (170, 200, 210), 8),
        Ice => p("Ice", Cat::Solid, 9000, (160, 200, 235), 12),

        Sand => p("Sand", Cat::Powder, 60, (200, 180, 90), 18),
        Salt => p("Salt", Cat::Powder, 55, (230, 230, 235), 12),
        Gunpowder => p("Gunpowder", Cat::Powder, 58, (60, 60, 66), 10),
        Snow => p("Snow", Cat::Powder, 20, (235, 240, 250), 8),
        Ash => p("Ash", Cat::Powder, 30, (90, 88, 86), 16),
        Seed => p("Seed", Cat::Powder, 40, (110, 150, 60), 14),
        Bomb => p("Bomb", Cat::Powder, 70, (40, 30, 30), 6),

        Water => p("Water", Cat::Liquid, 10, (40, 110, 200), 16),
        SaltWater => p("Saltwater", Cat::Liquid, 12, (60, 130, 190), 16),
        Oil => p("Oil", Cat::Liquid, 6, (90, 70, 50), 12),
        Acid => p("Acid", Cat::Liquid, 11, (120, 220, 60), 20),
        Lava => p("Lava", Cat::Liquid, 40, (210, 80, 20), 30),

        Steam => p("Steam", Cat::Gas, -8, (190, 190, 200), 16),
        Smoke => p("Smoke", Cat::Gas, -5, (60, 60, 66), 20),
        Toxic => p("Toxic Gas", Cat::Gas, -4, (130, 190, 90), 24),
        Fire => p("Fire", Cat::Gas, -10, (255, 120, 30), 30),

        Plant => p("Plant", Cat::Life, 9000, (60, 170, 70), 18),
        Ant => p("Ant", Cat::Life, 65, (130, 40, 40), 10),
        Ember => p("Ember", Cat::Life, 9000, (220, 90, 30), 30),
        WaterSource => p("Faucet", Cat::Life, 9000, (60, 90, 150), 8),
    }
}

#[inline]
pub fn is_fluid(el: Element) -> bool {
    matches!(props(el).cat, Cat::Liquid | Cat::Gas)
}

/// Conducts electrical charge.
#[inline]
pub fn conductive(el: Element) -> bool {
    matches!(el, Element::Metal | Element::Water | Element::SaltWater)
}

/// Can be eaten away by acid.
#[inline]
pub fn dissolvable(el: Element) -> bool {
    matches!(
        el,
        Element::Stone
            | Element::Wood
            | Element::Metal
            | Element::Sand
            | Element::Salt
            | Element::Gunpowder
            | Element::Snow
            | Element::Ash
            | Element::Seed
            | Element::Plant
            | Element::Ice
            | Element::Bomb
    )
}

/// The temperature a freshly-placed cell of this element starts at, if it has a
/// characteristic one. Returning `None` means "inherit the ambient/local temp".
#[inline]
pub fn natural_temp(el: Element) -> Option<f32> {
    match el {
        Element::Lava => Some(1200.0),
        Element::Fire => Some(720.0),
        Element::Ember => Some(540.0),
        Element::Ice => Some(-18.0),
        Element::Snow => Some(-10.0),
        Element::Steam => Some(105.0),
        _ => None,
    }
}

/// Elements offered in the side palette, in display order.
pub const PALETTE: &[Element] = &[
    Element::Empty,
    Element::Wall,
    Element::Sand,
    Element::Water,
    Element::Oil,
    Element::Salt,
    Element::Stone,
    Element::Wood,
    Element::Metal,
    Element::Glass,
    Element::Ice,
    Element::Snow,
    Element::Lava,
    Element::Acid,
    Element::Fire,
    Element::Gunpowder,
    Element::Bomb,
    Element::Plant,
    Element::Seed,
    Element::Ant,
    Element::WaterSource,
];
