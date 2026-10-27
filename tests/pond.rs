//! Behaviour of the pond, the renderer and the terminal diff, driven through the public API.

use std::time::Instant;

use koi_screensaver::render::{Frame, PondRenderer, DARK};
use koi_screensaver::term::{Terminal, DEFAULT_BG};
use koi_screensaver::world::{Variety, World, CELL_H, CELL_W};

const STEP: f64 = 1.0 / 60.0;

fn pond(cols: usize, rows: usize) -> (World, PondRenderer) {
    let mut renderer = PondRenderer::new();
    renderer.resize(cols, rows);
    let mut world = World::new(0xC0FFEE);
    world.resize(renderer.width(), renderer.height());
    (world, renderer)
}

#[test]
fn koi_seed_to_the_terminal_geometry() {
    let (world, renderer) = pond(190, 44);
    assert_eq!(renderer.width(), 190.0 * CELL_W);
    assert_eq!(renderer.height(), 44.0 * CELL_H);
    assert_eq!(world.width, renderer.width());
    assert_eq!(world.height, renderer.height());
    // 1900x880 pond: 6 adult koi plus 2 young, like the original's density rule.
    assert_eq!(world.koi.len(), 8);
    assert!(world
        .koi
        .iter()
        .all(|k| k.length > 60.0 && k.length < 430.0));
}

#[test]
fn canvas_preserves_the_original_tile_density_and_unflecked_fin_veil() {
    let mut renderer = PondRenderer::new();
    // Both clamps and the rounding boundary in the original width/125 density rule.
    for (width, cw) in [(640.0, 8.0), (1062.0, 8.0), (1063.0, 9.0), (2048.0, 12.0)] {
        renderer.resize_canvas(width, 1152.0);
        assert_eq!(renderer.cell_size(), (cw, cw + 2.0));
    }
    assert_eq!((renderer.frame.cols, renderer.frame.rows), (171, 83));

    let mut world = World::new(42);
    world.resize(2048.0, 1152.0);
    world.time = 2.5;
    world.koi.truncate(1);
    let k = &mut world.koi[0];
    k.id = 1;
    k.variety = Variety::Kohaku;
    k.length = 400.0;
    k.depth = 1.0;
    k.phase = 0.9;
    k.beat = 0.6;
    k.fin_phase = 0.7;
    k.turn = 0.2;
    for n in 0..20 {
        k.nodes[n * 2] = (650.0 - 0.25f64.cos() * 400.0 / 19.0 * n as f64) as f32 as f64;
        k.nodes[n * 2 + 1] = (350.0 - 0.25f64.sin() * 400.0 / 19.0 * n as f64) as f32 as f64;
    }
    renderer.render(&world, STEP);
    // Reference pose from portfolio's renderer, with the veil and red tile composited over black.
    // A fin cell without a dot must still carry its pale veil, preserving the silhouette.
    assert_eq!(renderer.frame.bg[0], 0x000000);
    let fin = 15 * 171 + 24;
    assert_eq!(renderer.frame.glyph[fin], b' ');
    assert_eq!(renderer.frame.bg[fin], 0x2a2826);
    assert_eq!(renderer.frame.bg[21 * 171 + 45], 0xb94419);
}

#[test]
fn koi_keep_swimming_inside_the_pond_for_two_minutes() {
    let (mut world, _) = pond(190, 44);
    let mut max_speed: f64 = 0.0;
    for _ in 0..(120 * 60) {
        world.step(STEP);
        for k in &world.koi {
            let head = (k.nodes[0], k.nodes[1]);
            assert!(head.0.is_finite() && head.1.is_finite(), "head went NaN");
            assert!(
                k.speed.is_finite() && k.speed >= 0.0,
                "speed went bad: {}",
                k.speed
            );
            max_speed = max_speed.max(k.speed);
            // A body may slip partly off-screen, but never its whole length away.
            assert!(head.0 > -k.length * 2.0 && head.0 < world.width + k.length * 2.0);
            assert!(head.1 > -k.length * 2.0 && head.1 < world.height + k.length * 2.0);
        }
    }
    // They burst and coast, so at some point they must have been swimming properly.
    assert!(
        max_speed > 20.0,
        "koi never picked up speed, max {max_speed}"
    );
}

#[test]
fn food_splashes_attracts_koi_and_gets_eaten() {
    let (mut world, _) = pond(190, 44);
    let (cx, cy) = (world.width / 2.0, world.height / 2.0);
    world.feed(cx, cy);
    assert!(world.pellets.len() >= 5, "a throw is a handful of pellets");
    assert_eq!(world.ripples.len(), 1, "the throw splashes");

    let mut bites = 0;
    let mut pellets_left = world.pellets.len();
    for _ in 0..(30 * 60) {
        world.step(STEP);
        bites += world.bites.len();
        assert!(world.pellets.len() <= pellets_left.max(24));
        pellets_left = world.pellets.len();
    }
    assert!(bites > 0, "no koi ever ate: the food behaviour is broken");
    assert_eq!(
        world.pellets.len(),
        0,
        "pellets should be eaten or sink away"
    );
}

#[test]
fn renderer_paints_koi_bodies_fins_and_water() {
    let (mut world, mut renderer) = pond(160, 44);
    for _ in 0..(4 * 60) {
        world.step(STEP);
    }
    renderer.render(&world, STEP);
    let frame = &renderer.frame;

    let inked = frame.glyph.iter().filter(|g| **g != b' ').count();
    let tiled = frame.bg.iter().filter(|c| **c != DEFAULT_BG).count();
    assert!(
        inked > 150,
        "expected a screen with koi on it, got {inked} inked cells"
    );
    assert!(tiled > 100, "expected coloured skin tiles, got {tiled}");

    // Koi skin glyphs and fin flecks are the vocabulary of the original's tiles.
    let text = frame.to_text();
    assert!(
        text.contains('%') || text.contains('S'),
        "no skin glyphs in the frame"
    );
    assert!(
        text.contains('o') || text.contains(':'),
        "no eyes or shadows in the frame"
    );

    // Nothing may be painted outside the grid, and every glyph is printable.
    assert_eq!(frame.glyph.len(), 160 * 44);
    assert!(frame.glyph.iter().all(|g| (32..127).contains(g)));

    // The skin tiles use the koi palette, not the background.
    let bg_packed = koi_screensaver::render::pack(DARK.bg);
    assert!(frame.bg.iter().any(|c| *c != bg_packed));
}

#[test]
fn terminal_repaints_only_what_changed() {
    let (world, mut renderer) = pond(80, 24);
    let mut terminal = Terminal::new(80, 24);
    renderer.render(&world, STEP);

    let (first, painted) = terminal.diff(&renderer.frame);
    assert_eq!(painted, 80 * 24, "the first frame must fill the screen");
    assert!(
        first.contains("\x1b[2J"),
        "the first frame clears the screen"
    );
    assert!(first.contains("\x1b[38;2;"), "colours are 24-bit SGR");
    assert!(
        first.contains("\x1b[48;2;"),
        "tiles are 24-bit background colours"
    );

    // Painting the same frame again sends nothing at all.
    let (again, painted) = terminal.diff(&renderer.frame);
    assert_eq!(painted, 0);
    assert!(
        again.is_empty(),
        "unchanged frame wrote {} bytes",
        again.len()
    );

    // One changed cell is one cursor jump and one character.
    let mut frame: Frame = renderer.frame.clone();
    let middle = 12 * 80 + 40;
    frame.glyph[middle] = b'X';
    frame.fg[middle] = 0xFF0000;
    let (delta, painted) = terminal.diff(&frame);
    assert_eq!(painted, 1);
    assert!(
        delta.contains("\x1b[13;41H"),
        "cursor should move to the changed cell: {delta:?}"
    );
    assert!(delta.contains('X'));
    assert!(
        delta.len() < 64,
        "one cell should cost a few dozen bytes, got {}",
        delta.len()
    );
}

#[test]
fn a_frame_fits_in_a_60fps_budget_at_2560x1440() {
    // 190x44 is what a 2560x1440 terminal at Omarchy's 18px screensaver font gives you.
    let (mut world, mut renderer) = pond(190, 44);
    for _ in 0..120 {
        world.step(STEP);
    }
    // Warm up, then time 120 frames.
    for _ in 0..10 {
        renderer.render(&world, STEP);
    }
    let start = Instant::now();
    for _ in 0..120 {
        world.step(STEP);
        renderer.render(&world, STEP);
    }
    let per_frame = start.elapsed().as_secs_f64() / 120.0;
    assert!(
        per_frame < 1.0 / 60.0,
        "render takes {:.2} ms per frame, over the 16.7 ms budget",
        per_frame * 1000.0
    );
    println!(
        "sim + render: {:.2} ms per frame at 190x44",
        per_frame * 1000.0
    );
}
