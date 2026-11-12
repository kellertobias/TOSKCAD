//! Opaque finite faces retain complete curve, extent, material-side and chart identities.
use super::*;
use multicad_core::project_file::ContentHash;

fn scalar(bytes: &mut Vec<u8>, value: f64) {
    bytes.extend(if value == 0. {
        0_f64.to_le_bytes()
    } else {
        value.to_le_bytes()
    });
}
fn point(bytes: &mut Vec<u8>, value: [f64; 3]) {
    for coordinate in value {
        scalar(bytes, coordinate);
    }
}
fn record(bytes: &mut Vec<u8>, value: &[u8]) {
    bytes.extend((value.len() as u64).to_le_bytes());
    bytes.extend(value);
}
fn origin(feature: EntityId, kind: &str, bytes: &[u8]) -> Origin {
    Origin {
        source: feature,
        role: format!("spline-{kind}.v1/{}", ContentHash::of(bytes)),
    }
}
fn curve(edge: &Edge, frame: PlaneFrame, z: f64) -> Vec<u8> {
    let place = |p: [f64; 2]| position(frame, [p[0], p[1], z]);
    let mut bytes = Vec::new();
    match edge {
        Edge::Cubic(cubic) => {
            bytes.push(3);
            let poles: Vec<_> = cubic.poles().iter().copied().map(place).collect();
            let local: Vec<_> = cubic.poles().iter().map(|&[x, y]| [x, y, 0.]).collect();
            let mut literal_knots = Vec::new();
            for &knot in cubic.knots() {
                scalar(&mut literal_knots, knot);
            }
            record(&mut bytes, &literal_knots);
            for word in super::super::step_import::spline_control_key(&local, cubic.knots()) {
                bytes.extend(word.to_le_bytes());
            }
            for word in super::super::step_import::spline_control_key(&poles, cubic.knots()) {
                bytes.extend(word.to_le_bytes());
            }
        }
        Edge::Analytic(edge) => {
            bytes.push(match edge {
                AnalyticEdge::Line { .. } => 0,
                AnalyticEdge::Arc { .. } => 1,
                AnalyticEdge::Circle { .. } => 2,
            });
            // Retain the complete local source law as well as its world
            // image: rounded world sums must not erase a changed coefficient.
            let mut local = [*edge, edge.reversed()].map(|edge| {
                let mut key = Vec::new();
                let local = |[x, y]: [f64; 2]| [x, y, 0.];
                match edge {
                    AnalyticEdge::Line { a, b } => {
                        point(&mut key, local(a));
                        point(&mut key, local(b));
                    }
                    AnalyticEdge::Arc {
                        center,
                        a,
                        b,
                        clockwise,
                    } => {
                        point(&mut key, local(center));
                        point(&mut key, local(a));
                        point(&mut key, local(b));
                        key.push(u8::from(clockwise));
                    }
                    AnalyticEdge::Circle {
                        center, radius_mm, ..
                    } => {
                        point(&mut key, local(center));
                        scalar(&mut key, radius_mm);
                    }
                }
                key
            });
            local.sort();
            record(&mut bytes, &local[0]);
            let [a, b] = [place(edge.point(0.)), place(edge.point(1.))];
            let ends = if a < b { [a, b] } else { [b, a] };
            for endpoint in ends {
                point(&mut bytes, endpoint);
            }
            point(&mut bytes, place(edge.point(0.5)));
            if let AnalyticEdge::Arc { center, .. } | AnalyticEdge::Circle { center, .. } = edge {
                point(&mut bytes, place(*center));
                scalar(&mut bytes, edge.radius_mm().expect("round edge"));
                scalar(
                    &mut bytes,
                    edge.signed_sweep_rad().expect("round edge").abs(),
                );
            }
        }
    }
    bytes
}

pub(super) fn caps(feature: EntityId, sweep: &Sweep, frame: PlaneFrame) -> [Origin; 2] {
    [false, true].map(|top| {
        let z = if top { sweep.height_mm() } else { 0. };
        let mut bytes = Vec::new();
        scalar(&mut bytes, z);
        // A changed opaque UV chart requires repair rather than moving a datum.
        point(&mut bytes, position(frame, [0., 0., z]));
        point(&mut bytes, frame.u);
        point(&mut bytes, frame.v);
        point(&mut bytes, frame.normal().map(|v| if top { v } else { -v }));
        let mut rings: Vec<_> = sweep
            .rings()
            .map(|ring| {
                let mut edges: Vec<_> = ring.iter().map(|edge| curve(edge, frame, z)).collect();
                edges.sort();
                let mut bytes = Vec::new();
                for edge in edges {
                    record(&mut bytes, &edge);
                }
                bytes
            })
            .collect();
        record(&mut bytes, &rings.remove(0));
        rings.sort();
        for ring in rings {
            record(&mut bytes, &ring);
        }
        origin(feature, "cap", &bytes)
    })
}

pub(super) fn walls(feature: EntityId, sweep: &Sweep, frame: PlaneFrame) -> Vec<Vec<Origin>> {
    sweep
        .rings()
        .enumerate()
        .flat_map(|(ring_index, ring)| {
            ring.iter().map(move |edge| {
                let mut bytes = vec![u8::from(ring_index == 0)];
                scalar(&mut bytes, sweep.height_mm());
                point(&mut bytes, frame.origin);
                point(&mut bytes, frame.u);
                point(&mut bytes, frame.v);
                if let Some(anchor) = SplineSweepBody::side_plane(frame, edge) {
                    point(&mut bytes, anchor.origin_mm);
                    point(&mut bytes, anchor.u);
                    point(&mut bytes, anchor.v);
                }

                record(&mut bytes, &curve(edge, frame, 0.));
                record(&mut bytes, &curve(edge, frame, sweep.height_mm()));
                // Full finite axial reach and directed material side. A reversal of
                // the represented opaque chart may require repair, never a side guess.
                point(&mut bytes, frame.normal());
                point(&mut bytes, frame.normal().map(|v| v * sweep.height_mm()));
                for fraction in [0., 0.25, 1.] {
                    let [x, y] = edge.point(fraction);
                    point(&mut bytes, [x, y, 0.]);
                    point(&mut bytes, position(frame, [x, y, 0.]));
                }
                vec![origin(feature, "wall", &bytes)]
            })
        })
        .collect()
}
