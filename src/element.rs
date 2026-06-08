//! Element catalogue: every material the sandbox knows about, plus the static
//! data (colour, density, category, conductivity ...) that drives the
//! simulation. Behaviour lives in `world.rs`; this file is pure data.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Element {
    Empty,
    // --- solids / building ---------------------------------------------
    Wall,
    Stone,
    Brick,
    Wood,
    Metal,
    Glass,
    Ice,
    // --- powders -------------------------------------------------------
    Sand,
    Dirt,
    Salt,
    Coal,
    Gunpowder,
    Snow,
    Ash,
    Seed,
    Concrete,
    // --- liquids -------------------------------------------------------
    Water,
    SaltWater,
    Oil,
    Gasoline,
    Acid,
    Lava,
    Blood,
    Mercury,
    Napalm,
    // --- gases ---------------------------------------------------------
    Steam,
    Smoke,
    Toxic,
    Methane,
    Fire,
    // --- life ----------------------------------------------------------
    Plant,
    Ant,
    Person,
    Zombie,
    Fish,
    Virus,
    Ember, // burning solid (wood/coal/plant on fire)
    // --- weapons -------------------------------------------------------
    Bomb,
    Tnt,
    C4,
    Nuke,
    Mine,
    Grenade,
    Fireworks,
    Missile,
    Gun,
    Bullet,
    Laser,
    // --- tools / special ----------------------------------------------
    WaterSource,
    Cloner,
    Void,
    Heater,
    Cooler,
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

/// Which palette submenu an element appears under (Hidden = emergent only).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Menu {
    Solids,
    Powders,
    Liquids,
    Gases,
    Life,
    Weapons,
    Tools,
    Hidden,
}

/// Static properties of an element.
pub struct Props {
    pub name: &'static str,
    pub cat: Cat,
    pub menu: Menu,
    /// Higher sinks. Gases are negative so they rise. Used for displacement.
    pub density: i16,
    pub color: (u8, u8, u8),
    /// Per-cell colour jitter amplitude, for natural-looking texture.
    pub var: u8,
}

use Cat::{Gas, Life, Liquid, Powder, Solid};
use Element::*;
use Menu as M;

/// The big lookup table. Kept as a `match` so it compiles to a jump table and
/// stays allocation-free.
pub fn props(el: Element) -> Props {
    let p = |name, cat, menu, density, color, var| Props {
        name,
        cat,
        menu,
        density,
        color,
        var,
    };
    match el {
        Empty => p("Eraser", Cat::Empty, M::Tools, 0, (12, 12, 16), 0),

        // solids
        Wall => p("Wall", Solid, M::Solids, 9000, (80, 80, 92), 6),
        Stone => p("Stone", Solid, M::Solids, 9000, (110, 105, 100), 14),
        Brick => p("Brick", Solid, M::Solids, 9000, (150, 70, 55), 12),
        Wood => p("Wood", Solid, M::Solids, 9000, (120, 80, 40), 12),
        Metal => p("Metal", Solid, M::Solids, 9000, (150, 155, 165), 10),
        Glass => p("Glass", Solid, M::Solids, 9000, (170, 200, 210), 8),
        Ice => p("Ice", Solid, M::Solids, 9000, (160, 200, 235), 12),

        // powders
        Sand => p("Sand", Powder, M::Powders, 60, (200, 180, 90), 18),
        Dirt => p("Dirt", Powder, M::Powders, 62, (95, 65, 40), 16),
        Salt => p("Salt", Powder, M::Powders, 55, (230, 230, 235), 12),
        Coal => p("Coal", Powder, M::Powders, 52, (40, 40, 44), 10),
        Gunpowder => p("Gunpowder", Powder, M::Powders, 58, (60, 60, 66), 10),
        Snow => p("Snow", Powder, M::Powders, 20, (235, 240, 250), 8),
        Ash => p("Ash", Powder, M::Powders, 30, (90, 88, 86), 16),
        Seed => p("Seed", Powder, M::Powders, 40, (110, 150, 60), 14),
        Concrete => p("Concrete", Powder, M::Powders, 80, (130, 130, 135), 14),

        // liquids
        Water => p("Water", Liquid, M::Liquids, 10, (40, 110, 200), 16),
        SaltWater => p("Saltwater", Liquid, M::Hidden, 12, (60, 130, 190), 16),
        Oil => p("Oil", Liquid, M::Liquids, 6, (90, 70, 50), 12),
        Gasoline => p("Gasoline", Liquid, M::Liquids, 5, (200, 185, 80), 16),
        Acid => p("Acid", Liquid, M::Liquids, 11, (120, 220, 60), 20),
        Lava => p("Lava", Liquid, M::Liquids, 40, (210, 80, 20), 30),
        Blood => p("Blood", Liquid, M::Liquids, 11, (130, 20, 20), 14),
        Mercury => p("Mercury", Liquid, M::Liquids, 100, (190, 190, 200), 20),
        Napalm => p("Napalm", Liquid, M::Liquids, 9, (220, 120, 40), 30),

        // gases
        Steam => p("Steam", Gas, M::Gases, -8, (190, 190, 200), 16),
        Smoke => p("Smoke", Gas, M::Gases, -5, (60, 60, 66), 20),
        Toxic => p("Toxic Gas", Gas, M::Gases, -4, (130, 190, 90), 24),
        Methane => p("Methane", Gas, M::Gases, -7, (170, 200, 150), 18),
        Fire => p("Fire", Gas, M::Gases, -10, (255, 120, 30), 30),

        // life
        Plant => p("Plant", Life, M::Life, 9000, (60, 170, 70), 18),
        Ant => p("Ant", Life, M::Life, 65, (130, 40, 40), 10),
        Person => p("Person", Life, M::Life, 64, (225, 175, 135), 14),
        Zombie => p("Zombie", Life, M::Life, 64, (95, 145, 80), 16),
        Fish => p("Fish", Life, M::Life, 13, (110, 140, 205), 18),
        Virus => p("Virus", Life, M::Life, 50, (205, 60, 200), 26),
        Ember => p("Ember", Life, M::Hidden, 9000, (220, 90, 30), 30),

        // weapons
        Bomb => p("Bomb", Powder, M::Weapons, 70, (40, 30, 30), 6),
        Tnt => p("TNT", Powder, M::Weapons, 70, (185, 45, 45), 8),
        C4 => p("C4", Solid, M::Weapons, 9000, (210, 200, 150), 8),
        Nuke => p("Nuke", Solid, M::Weapons, 9000, (210, 200, 60), 10),
        Mine => p("Mine", Solid, M::Weapons, 9000, (70, 70, 80), 8),
        Grenade => p("Grenade", Powder, M::Weapons, 70, (70, 95, 60), 8),
        Fireworks => p("Fireworks", Life, M::Weapons, 0, (220, 90, 170), 20),
        Missile => p("Missile", Life, M::Weapons, 0, (185, 185, 195), 8),
        Gun => p("Turret Gun", Solid, M::Weapons, 9000, (90, 90, 105), 8),
        Bullet => p("Bullet", Life, M::Hidden, 0, (255, 240, 180), 6),
        Laser => p("Laser", Solid, M::Weapons, 9000, (200, 50, 50), 8),

        // tools / special
        WaterSource => p("Faucet", Solid, M::Tools, 9000, (60, 90, 150), 8),
        Cloner => p("Cloner", Solid, M::Tools, 9000, (180, 150, 220), 14),
        Void => p("Void", Solid, M::Tools, 9000, (24, 8, 32), 10),
        Heater => p("Heater", Solid, M::Tools, 9000, (200, 90, 60), 14),
        Cooler => p("Cooler", Solid, M::Tools, 9000, (90, 160, 210), 14),
    }
}

/// Every element, exactly once. Used for menu filtering and clone storage.
pub const ALL: &[Element] = &[
    Empty, Wall, Stone, Brick, Wood, Metal, Glass, Ice, Sand, Dirt, Salt, Coal,
    Gunpowder, Snow, Ash, Seed, Concrete, Water, SaltWater, Oil, Gasoline, Acid,
    Lava, Blood, Mercury, Napalm, Steam, Smoke, Toxic, Methane, Fire, Plant,
    Ant, Person, Zombie, Fish, Virus, Ember, Bomb, Tnt, C4, Nuke, Mine, Grenade,
    Fireworks, Missile, Gun, Bullet, Laser, WaterSource, Cloner, Void, Heater,
    Cooler,
];

/// The submenu tabs shown in the UI, in order.
pub const CATEGORIES: &[(Menu, &str)] = &[
    (M::Solids, "Solids"),
    (M::Powders, "Powders"),
    (M::Liquids, "Liquids"),
    (M::Gases, "Gases"),
    (M::Life, "Life"),
    (M::Weapons, "Weapons"),
    (M::Tools, "Tools"),
];

/// Elements shown under a given submenu tab.
pub fn menu_elements(m: Menu) -> Vec<Element> {
    ALL.iter().copied().filter(|&e| props(e).menu == m).collect()
}

/// Index of an element within `ALL` (for the Cloner to remember a material).
pub fn el_index(el: Element) -> u8 {
    ALL.iter().position(|&e| e == el).unwrap_or(0) as u8
}

pub fn el_from_index(i: u8) -> Element {
    ALL.get(i as usize).copied().unwrap_or(Empty)
}

#[inline]
pub fn is_fluid(el: Element) -> bool {
    matches!(props(el).cat, Cat::Liquid | Cat::Gas)
}

/// Conducts electrical charge.
#[inline]
pub fn conductive(el: Element) -> bool {
    matches!(el, Metal | Water | SaltWater | Mercury)
}

/// Can be eaten away by acid.
#[inline]
pub fn dissolvable(el: Element) -> bool {
    matches!(
        el,
        Stone | Brick | Wood | Metal | Sand | Dirt | Salt | Coal | Gunpowder
            | Snow | Ash | Seed | Concrete | Plant | Ice | Bomb
    )
}

/// Living things that bullets/mines/zombies care about.
#[inline]
pub fn is_creature(el: Element) -> bool {
    matches!(el, Person | Zombie | Fish | Ant)
}

/// The temperature a freshly-placed cell of this element starts at, if it has a
/// characteristic one. Returning `None` means "inherit the ambient/local temp".
#[inline]
pub fn natural_temp(el: Element) -> Option<f32> {
    match el {
        Lava => Some(1200.0),
        Fire => Some(720.0),
        Ember => Some(540.0),
        Napalm => Some(500.0),
        Ice => Some(-18.0),
        Snow => Some(-10.0),
        Steam => Some(105.0),
        _ => None,
    }
}
