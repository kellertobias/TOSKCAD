//! Descriptor equivalence uses all world Bernstein controls, never mass or
//! rounded world endpoints alone. The imported descriptor must first have
//! passed the independent full file proof.
use super::super::helical_import::cylindrical_closure::expression_bound;
use super::*;

pub fn rounded_thread_import_matches(
    imported: &ImportedRoundedThread,
    expected: &Thread,
    frame: [Point; 3],
    tolerance: f64,
) -> bool {
    matches(imported, expected, frame, tolerance).is_ok()
}
fn matches(
    imported: &ImportedRoundedThread,
    expected: &Thread,
    frame: [Point; 3],
    tolerance: f64,
) -> Result<()> {
    require(
        tolerance.is_finite()
            && tolerance > 0.
            && imported.thread.spec().left_hand == expected.spec().left_hand,
    )?;
    let frames = [[imported.origin_mm, imported.u, imported.v], frame];
    for [o, u, v] in frames {
        require(
            o.into_iter().chain(u).chain(v).all(f64::is_finite)
                && (dot(u, u) - 1.).abs() <= 1e-10
                && (dot(v, v) - 1.).abs() <= 1e-10
                && dot(u, v).abs() <= 1e-10,
        )?;
    }
    let a = prepare(&imported.thread, frames[0][0], tolerance).map_err(helical::step_error)?;
    let b = prepare(expected, frames[1][0], tolerance).map_err(helical::step_error)?;
    require(a.axial.poles.len() == b.axial.poles.len())?;
    let ns = frames.map(|[_, u, v]| cross(u, v));
    let budget = tolerance.min(1e-7) / 16.;
    for (ra, rb) in a.bands.iter().zip(&b.bands) {
        require(ra.poles.len() == rb.poles.len())?;
        for (&pa, &pb) in ra.poles.iter().zip(&rb.poles) {
            for (j, (&aa, &ab)) in a.axial.poles.iter().zip(&b.axial.poles).enumerate() {
                let xy = [product(pa, aa), product(pb, ab)];
                // Each complex product is explicitly rounded before placement.
                // Its full coefficient error is charged in addition to exact-real
                // FMA/TwoSum placement differences, including large-origin cancel.
                let scale = pa.into_iter().chain(pb).map(f64::abs).sum::<f64>();
                let mut bound = 64. * f64::EPSILON * scale;
                for i in 0..3 {
                    let terms = [
                        (frames[0][0][i], 1.),
                        (frames[1][0][i], -1.),
                        (frames[0][1][i], xy[0][0]),
                        (-frames[1][1][i], xy[1][0]),
                        (frames[0][2][i], xy[0][1]),
                        (-frames[1][2][i], xy[1][1]),
                        (ns[0][i], a.heights[j]),
                        (-ns[1][i], b.heights[j]),
                    ];
                    bound = (bound + expression_bound(terms)).next_up();
                }
                require(bound.is_finite() && bound <= budget)?;
            }
        }
    }
    Ok(())
}
