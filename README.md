# Sand — an advanced native falling-sand sandbox

A desktop falling-sand toy in the spirit of the classic web "powder games", but
built as a native application in **Rust** with a focus on *emergent* physics
rather than a long list of hard-coded special cases.

![showcase](showcase.png)

The screenshot above is a single simulated scene: oil floating on a water basin
(left), a faucet pouring a water stream (centre), a wooden block mid-combustion
throwing a plume of fire and smoke (right), and a lava pool cooling into a stone
crust on a shelf (far right).

## What makes it "more advanced"

Most web sand games script every interaction as an explicit `A + B -> C` rule.
Sand instead runs a couple of small physical *fields* and lets the interesting
behaviour fall out of them:

- **A real temperature field with heat diffusion.** Every cell has a
  temperature; heat spreads each frame. Materials change *phase* by temperature,
  so you get behaviour for free: lava radiates heat, ignites nearby wood/oil and
  slowly **cools into stone from the edges inward** (a crust); water touching
  lava **boils to steam** which **condenses back to water** as it cools; ice
  melts near flame; wood and plants combust when they get hot enough.
- **Density-based movement.** Powders, liquids and gases all displace each other
  by density, so **oil floats on water**, **sand sinks through both**, smoke and
  steam rise, and lava sinks below water.
- **Electrical conduction.** Charge propagates one cell per frame through metal
  and water (the *Spark* tool), detonating gunpowder/bombs and igniting oil —
  build wires and circuits.
- **Life & chemistry.** Ants wander, eat plant/wood/sand and multiply; seeds
  sprout into plants that climb where there's water; acid dissolves most solids
  and vents toxic gas; salt dissolves into brine; gunpowder and bombs explode.

## Elements

Eraser · Wall · Sand · Water · Oil · Salt · Stone · Wood · Metal · Glass · Ice ·
Snow · Lava · Acid · Fire · Gunpowder · Bomb · Plant · Seed · Ant · Faucet, plus
emergent materials you don't paint directly (Steam, Smoke, Toxic Gas, Ember,
Saltwater, Ash).

## Build & run

Requires a Rust toolchain (`rustup`) and a desktop OpenGL context.

```sh
cargo run --release
```

### Controls

| Input | Action |
|-------|--------|
| Left click / drag | Paint the selected material |
| Right click / drag | Erase |
| Mouse wheel | Brush size |
| Click palette (right) | Select material |
| `1`–`9` | Quick-pick first nine materials |
| `Z` / palette *Spark* | Spark tool — inject charge into wires |
| `Space` | Pause / resume |
| `C` | Clear the world |
| `R` | Reset everything |

### Headless showcase

The binary can seed a demo scene, simulate it, and export a PNG without any
interaction — useful for CI or generating the image above:

```sh
SAND_DEMO=1 SAND_SCREENSHOT=showcase.png SAND_SHOT_FRAME=90 cargo run --release
```

## Architecture

```
src/
  element.rs   element catalogue + static properties (colour, density, ...)
  world.rs     the simulation: cell grid, temperature field, movement,
               chemistry, electricity, heat diffusion, phase changes
  ui.rs        side-panel palette + status bar (plain macroquad drawing)
  main.rs      window, input, and the GPU-texture render path
```

Each `World::step()` runs four passes: clear move-flags → propagate charge →
movement + chemistry (scanned bottom-up, alternating x direction to avoid drift
bias) → heat (emit from sources, diffuse, apply phase changes). The whole grid
is uploaded to a single GPU texture each frame, so rendering ~120k cells is
effectively free and the CPU budget goes to physics.

Run the physics tests (no display needed) with:

```sh
cargo test
```
