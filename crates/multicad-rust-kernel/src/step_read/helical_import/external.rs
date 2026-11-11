//! Complete exterior material identity; role ordinals alone are never authority.
use super::*;
use crate::threaded_shaft::Shaft;

fn up_sum(a: f64, b: f64) -> f64 {
    (a + b).next_up()
}
fn up_product(a: f64, b: f64) -> f64 {
    (a * b).next_up()
}
fn difference(a: f64, b: f64) -> [f64; 2] {
    let s = a - b;
    let part = s - a;
    [s, (a - (s - part)) + (-b - part)]
}
/// Bound the exact real sum of products of the supplied finite f64 values.
/// Subnormal product residuals get one outward ulp even when FMA rounds zero.
fn expression_bound(terms: impl IntoIterator<Item = (f64, f64)>) -> f64 {
    let mut value = 0.;
    let mut error = 0.;
    for (a, b) in terms {
        let product = a * b;
        let sum = value + product;
        if !sum.is_finite() {
            return f64::INFINITY;
        }
        let part = sum - value;
        let residual = (value - (sum - part)) + (product - part);
        error = up_sum(
            error,
            up_sum(a.mul_add(b, -product).abs().next_up(), residual.abs()),
        );
        value = sum;
    }
    up_sum(value.abs(), error)
}
fn vector_difference(a: Point, b: Point) -> f64 {
    (0..3).fold(0., |bound, i| {
        up_sum(bound, expression_bound([(a[i], 1.), (b[i], -1.)]))
    })
}
fn endpoint_error(a: [Point; 2], b: [Point; 2], la: f64, lb: f64) -> f64 {
    let [length, low_length] = difference(la, lb);
    (0..3).fold(0., |bound, i| {
        let [origin, low_origin] = difference(a[0][i], b[0][i]);
        let [axis, low_axis] = difference(a[1][i], b[1][i]);
        up_sum(
            bound,
            expression_bound([
                (origin, 1.),
                (low_origin, 1.),
                (axis, la),
                (low_axis, la),
                (b[1][i], length),
                (b[1][i], low_length),
            ]),
        )
    })
}

pub fn threaded_shaft_import_matches(
    imported: &ImportSolid,
    shaft: &Shaft,
    frame: [Point; 3],
    tolerance: f64,
) -> bool {
    let ImportProfile::ThreadedShaft {
        spec,
        stock_radius_mm,
        stock_length_mm,
    } = imported.profile
    else {
        return false;
    };
    if imported.height_mm != stock_length_mm {
        return false;
    }
    let Ok(candidate) = Thread::new(spec) else {
        return false;
    };
    if Shaft::new(candidate, stock_radius_mm, stock_length_mm).is_err() {
        return false;
    }
    let thread = ImportSolid {
        height_mm: spec.length_mm,
        profile: ImportProfile::HelicalThread {
            spec,
            stock_radius_mm: None,
        },
        ..imported.clone()
    };
    if !helical_import_matches(&thread, shaft.thread(), None, frame, tolerance) {
        return false;
    }
    let [origin, u, v] = frame;
    let na = cross(u, v);
    let nb = cross(imported.u, imported.v);
    let ends = [shaft.thread().spec().length_mm, shaft.stock_length_mm()];
    let candidate_ends = [spec.length_mm, stock_length_mm];
    let center_error = ends
        .into_iter()
        .zip(candidate_ends)
        .map(|(a, b)| endpoint_error([origin, na], [imported.origin_mm, nb], a, b))
        .fold(0., f64::max);
    let radius = shaft.radius_mm();
    let frame_error = up_product(
        radius.max(stock_radius_mm),
        up_sum(
            vector_difference(u, imported.u),
            vector_difference(v, imported.v),
        ),
    );
    let radius_error = expression_bound([(radius, 1.), (stock_radius_mm, -1.)]);
    let radius_scale = up_sum(
        vector_difference(imported.u, [0.; 3]),
        vector_difference(imported.v, [0.; 3]),
    );
    let error = up_sum(
        center_error,
        up_sum(frame_error, up_product(radius_error, radius_scale)),
    );
    error.is_finite() && error <= tolerance / 8.
}

#[cfg(test)]
mod tests {
    use super::*;
    fn body() -> Shaft {
        Shaft::new(
            Thread::new(Spec {
                major_radius_mm: 3.,
                pitch_mm: 1.,
                length_mm: 2.37,
                depth_mm: 5. * 3_f64.sqrt() / 16.,
                crest_width_mm: 0.125,
                lower_flank_deg: 30.,
                upper_flank_deg: 30.,
                starts: 2,
                left_hand: false,
                root_radius_mm: None,
            })
            .unwrap(),
            3.,
            5.,
        )
        .unwrap()
    }
    #[test]
    fn finite_exterior_identity_covers_plain_tail_and_thread_nets() {
        let body = body();
        let frame = [[0.; 3], [1., 0., 0.], [0., 1., 0.]];
        let text =
            crate::step::write_threaded_shaft(&body, frame[0], frame[1], frame[2], 1e-7).unwrap();
        let (_, source) = read_import_solid(&text, 1e-7).unwrap();
        let matches =
            |candidate: &ImportSolid| threaded_shaft_import_matches(candidate, &body, frame, 1e-7);
        assert!(matches(&source));
        let mut changed = source.clone();
        changed.height_mm += 1.;
        assert!(!matches(&changed));
        changed = source.clone();
        changed.profile = ImportProfile::HelicalThread {
            spec: body.thread().spec(),
            stock_radius_mm: None,
        };
        assert!(!matches(&changed));
        for (radius, height) in [(4., 5.), (3., 6.), (3., 1.)] {
            changed = source.clone();
            changed.height_mm = height;
            changed.profile = ImportProfile::ThreadedShaft {
                spec: body.thread().spec(),
                stock_radius_mm: radius,
                stock_length_mm: height,
            };
            assert!(!matches(&changed));
        }
        let mut spec = body.thread().spec();
        spec.pitch_mm = f64::NAN;
        changed = source.clone();
        changed.profile = ImportProfile::ThreadedShaft {
            spec,
            stock_radius_mm: 3.,
            stock_length_mm: 5.,
        };
        assert!(!matches(&changed));
        changed = source.clone();
        changed.u = [0., 1., 0.];
        assert!(!matches(&changed));
        let large = Shaft::new(*body.thread(), 1e6, 5.).unwrap();
        changed = source.clone();
        changed.profile = ImportProfile::ThreadedShaft {
            spec: body.thread().spec(),
            stock_radius_mm: 1e6,
            stock_length_mm: 5.,
        };
        assert!(threaded_shaft_import_matches(&changed, &large, frame, 1e-7));
        changed.u[0] += 4e-14;
        assert!(!threaded_shaft_import_matches(
            &changed, &large, frame, 1e-7
        ));
    }

    #[test]
    fn factored_tail_identity_cannot_hide_a_changed_length_behind_world_rounding() {
        let source = body();
        let height: f64 = 1e7;
        let lower = height.next_down();
        let origin = [0., 0., (height - lower) / 2.];
        assert_eq!(origin[2] + height, origin[2] + lower);
        let body = Shaft::new(*source.thread(), 3., height).unwrap();
        let frame = [origin, [1., 0., 0.], [0., 1., 0.]];
        let mut imported = ImportSolid {
            origin_mm: origin,
            u: frame[1],
            v: frame[2],
            height_mm: height,
            profile: ImportProfile::ThreadedShaft {
                spec: body.thread().spec(),
                stock_radius_mm: 3.,
                stock_length_mm: height,
            },
        };
        assert!(threaded_shaft_import_matches(
            &imported, &body, frame, 1e-10
        ));
        imported.height_mm = lower;
        imported.profile = ImportProfile::ThreadedShaft {
            spec: body.thread().spec(),
            stock_radius_mm: 3.,
            stock_length_mm: lower,
        };
        assert!(!threaded_shaft_import_matches(
            &imported, &body, frame, 1e-10
        ));
        assert!(expression_bound([(1e16, 1.), (1., 1.), (-1e16, 1.)]) >= 1.);
        assert!(expression_bound([(f64::from_bits(1), 0.5)]) > 0.);
        assert_eq!(expression_bound([(f64::MAX, 2.)]), f64::INFINITY);
    }

    #[test]
    fn tampered_tail_shoulders_and_shared_circle_wires_are_refused() {
        for tangent in [false, true] {
            let source = body();
            let body = Shaft::new(*source.thread(), if tangent { 3. } else { 4. }, 5.).unwrap();
            let frame = [[0.; 3], [1., 0., 0.], [0., 1., 0.]];
            let text = crate::step::write_threaded_shaft(&body, frame[0], frame[1], frame[2], 1e-7)
                .unwrap();
            let data = parse_entities(&text, 1e-7).unwrap();
            let shell = Shell::new(&data, 1e-7, true);
            let solid = shell.represented_solids(true).unwrap()[0];
            let faces = shell.member_faces(solid).unwrap();
            let count = 4 * body.thread().spec().starts as usize;
            let reject = |changed: String| {
                assert!(
                    crate::step_read::verify_threaded_shaft(
                        &changed, &body, frame[0], frame[1], frame[2], 1e-7
                    )
                    .is_err()
                );
                assert!(read_import_solid(&changed, 1e-7).is_err());
            };
            for &id in &faces[count..] {
                reject(super::super::rejection_tests::edit(&text, id, |e| {
                    e.args[3] = Value::Enum("F")
                }));
                reject(super::super::rejection_tests::edit(&text, id, |e| {
                    e.args[1] = Value::List(vec![])
                }));
            }
            let wall = faces[count + body.shoulder_count() as usize + 1];
            let (surface, _, bounds) = face(&shell, wall).unwrap();
            reject(super::super::rejection_tests::edit(&text, surface, |e| {
                e.args.truncate(1)
            }));
            reject(super::super::rejection_tests::edit(&text, surface, |e| {
                e.args[2] = Value::Number(8.)
            }));
            let loop_id =
                reference(&shell.args(bounds[0], "FACE_OUTER_BOUND").unwrap()[1]).unwrap();
            let oriented = refs(&shell.args(loop_id, "EDGE_LOOP").unwrap()[1]).unwrap();
            for &id in &oriented {
                reject(super::super::rejection_tests::edit(&text, id, |e| {
                    e.args[4] = if e.args[4] == Value::Enum("T") {
                        Value::Enum("F")
                    } else {
                        Value::Enum("T")
                    };
                }));
            }
            let far = *faces.last().unwrap();
            let (_, _, bounds) = face(&shell, far).unwrap();
            let (far_surface, _, _) = face(&shell, far).unwrap();
            let plane_axis = reference(&shell.args(far_surface, "PLANE").unwrap()[1]).unwrap();
            let center =
                reference(&shell.args(plane_axis, "AXIS2_PLACEMENT_3D").unwrap()[1]).unwrap();
            for height in [2., 2e7] {
                reject(super::super::rejection_tests::edit(&text, center, |e| {
                    e.args[1] = Value::List(vec![
                        Value::Number(0.),
                        Value::Number(0.),
                        Value::Number(height),
                    ])
                }));
            }
            let (edge, _) = helical::ring(&shell, bounds[0], true).unwrap()[0];
            let curve = reference(&shell.args(edge, "EDGE_CURVE").unwrap()[3]).unwrap();
            reject(super::super::rejection_tests::edit(&text, curve, |e| {
                e.args[2] = Value::Number(8.)
            }));
            if !tangent {
                let (_, _, bounds) = face(&shell, faces[count + 1]).unwrap();
                let (lower, _) = helical::ring(&shell, bounds[0], true).unwrap()[0];
                let curve = reference(&shell.args(lower, "EDGE_CURVE").unwrap()[3]).unwrap();
                reject(super::super::rejection_tests::edit(&text, curve, |e| {
                    e.args[2] = Value::Number(8.)
                }));
            }
            if tangent {
                let (_, _, crest_bounds) = face(&shell, faces[3]).unwrap();
                let crest_edges = helical::orient(
                    &helical::ring(&shell, crest_bounds[0], true).unwrap(),
                    false,
                );
                let crest = crest_edges[2].0;
                let curve = reference(&shell.args(crest, "EDGE_CURVE").unwrap()[3]).unwrap();
                reject(super::super::rejection_tests::edit(&text, curve, |e| {
                    e.args[2] = Value::Number(8.)
                }));
                let (_, _, wall_bounds) = face(&shell, wall).unwrap();
                let uses = helical::ring(&shell, wall_bounds[0], true).unwrap();
                let seam = uses[uses.len() - 3].0;
                let args = shell.args(edge, "EDGE_CURVE").unwrap();
                let [a, b, curve] = [1, 2, 3].map(|i| reference(&args[i]).unwrap());
                reject(super::super::rejection_tests::edit(&text, seam, |e| {
                    e.args = vec![
                        Value::String(""),
                        Value::Reference(a),
                        Value::Reference(b),
                        Value::Reference(curve),
                        Value::Enum("T"),
                    ];
                }));
                let (_, _, bounds) = face(&shell, faces[count + 1]).unwrap();
                let loop_id =
                    reference(&shell.args(bounds[0], "FACE_OUTER_BOUND").unwrap()[1]).unwrap();
                reject(super::super::rejection_tests::edit(&text, loop_id, |e| {
                    if let Value::List(ref mut v) = e.args[1] {
                        v.pop();
                    }
                }));
                let uses = helical::ring(&shell, bounds[0], true).unwrap();
                for (edge, _) in uses.into_iter().filter(|(e, _)| {
                    shell
                        .simple(reference(&shell.args(*e, "EDGE_CURVE").unwrap()[3]).unwrap())
                        .unwrap()
                        .name
                        == "CIRCLE"
                }) {
                    let curve = reference(&shell.args(edge, "EDGE_CURVE").unwrap()[3]).unwrap();
                    reject(super::super::rejection_tests::edit(&text, curve, |e| {
                        e.args[2] = Value::Number(8.)
                    }));
                }
            }
        }
    }
}
