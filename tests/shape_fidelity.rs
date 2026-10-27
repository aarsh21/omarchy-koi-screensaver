//! The Rust port must classify koi outline points exactly like the TypeScript original.
//! `tests/fixtures/shape-reference.csv` was produced by running the portfolio's own `shape.ts`
//! (see scripts/gen-shape-reference.ts), so this compares against the real implementation rather
//! than a hand-written expectation.

use koi_screensaver::shape::{classify, ShapeHit};

#[test]
fn classifies_every_sample_like_the_original_typescript() {
    let csv = include_str!("fixtures/shape-reference.csv");
    let mut checked = 0;
    let mut mismatches = Vec::new();

    for line in csv.lines().skip(1) {
        let f: Vec<&str> = line.split(',').collect();
        let s: f64 = f[0].parse().unwrap();
        let a: f64 = f[1].parse().unwrap();
        let past: f64 = f[2].parse().unwrap();
        let flare: f64 = f[3].parse().unwrap();
        let flag: u8 = f[4].parse().unwrap();
        let across: f64 = f[5].parse().unwrap();

        let mut hit = ShapeHit::default();
        let inside = classify(s, a, past, flare, &mut hit, 0.0, 0.0);

        let across_ok =
            (hit.across.is_nan() && across.is_nan()) || (hit.across - across).abs() <= 1e-9;
        if inside != (flag != 0) || hit.flag != flag || !across_ok {
            mismatches.push(format!(
                "s={s} a={a} past={past} flare={flare}: rust flag={} across={} inside={inside}, ts flag={flag} across={across}",
                hit.flag, hit.across
            ));
        }
        checked += 1;
    }

    assert!(checked > 5000, "fixture should be large, got {checked}");
    assert!(
        mismatches.is_empty(),
        "{} of {checked} samples differ:\n{}",
        mismatches.len(),
        mismatches
            .iter()
            .take(10)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn hash_matches_javascript_math_imul() {
    // Printed by running the original world.ts under Bun.
    let cases = [
        (1.0, 2.0, 3.0, 0.6394013063982129),
        (20.0, 5.0, 41.0, 0.4331612712703645),
        (-7.0, 13.0, 0.0, 0.7774436273612082),
    ];
    for (x, y, z, expected) in cases {
        let got = koi_screensaver::world::hash3(x as i64, y as i64, z as i64);
        assert!(
            (got - expected).abs() < 1e-15,
            "hash3({x},{y},{z}) = {got}, want {expected}"
        );
    }
}
