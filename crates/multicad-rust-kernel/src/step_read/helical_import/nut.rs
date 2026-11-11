//! Independent hex-nut dimensions plus complete thread and finite exterior proof.
use super::*;
use crate::hex_nut::Nut;
pub(super) fn read(
    shell: &mut Shell<'_, '_>,
    cap: u32,
    thread: &Thread,
    frame: [Point; 3],
) -> Result<Nut> {
    let (_, forward, bounds) = face(shell, cap)?;
    require(forward && bounds.len() == 2)?;
    let chart = headed::Chart::new(frame)?;
    let edges = helical::ring(shell, bounds[0], true)?;
    require(edges.len() == 6)?;
    let mut half_width = 0_f64;
    for (edge, _) in edges {
        for vertex in shell.edge(edge)? {
            half_width = half_width.max(chart.local(shell.vertex(vertex)?)[0].abs());
        }
    }
    Nut::new(*thread, 2. * half_width).map_err(|e| match e {
        crate::helical_thread::ThreadError::Invalid => ReadError::Invalid,
        crate::helical_thread::ThreadError::Limit => ReadError::Limit,
    })
}
pub fn helical_nut_import_matches(
    imported: &ImportSolid,
    nut: &Nut,
    frame: [Point; 3],
    tolerance: f64,
) -> bool {
    let ImportProfile::HelicalNut {
        spec,
        across_flats_mm,
    } = imported.profile
    else {
        return false;
    };
    let thread = ImportSolid {
        profile: ImportProfile::HelicalThread {
            spec,
            stock_radius_mm: None,
        },
        ..imported.clone()
    };
    if !helical_import_matches(&thread, nut.thread(), None, frame, tolerance) {
        return false;
    }
    let world = |[o, u, v]: [Point; 3], p: Point2, z: f64| {
        let n = cross(u, v);
        std::array::from_fn(|i| o[i] + u[i] * p[0] + v[i] * p[1] + n[i] * z)
    };
    for (a, b) in crate::headed_screw::hex_vertices(nut.across_flats_mm())
        .into_iter()
        .zip(crate::headed_screw::hex_vertices(across_flats_mm))
    {
        for (za, zb) in [(0., 0.), (nut.thread().spec().length_mm, spec.length_mm)] {
            let error = length(sub(
                world(frame, a, za),
                world([imported.origin_mm, imported.u, imported.v], b, zb),
            ));
            if !error.is_finite() || error > tolerance / 16. {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nut_candidate_and_material_matcher_refuse_unproved_exteriors() {
        let thread = Thread::new(Spec {
            major_radius_mm: 3.,
            pitch_mm: 1.,
            length_mm: 3.7,
            depth_mm: 5. * 3_f64.sqrt() / 16.,
            crest_width_mm: 0.125,
            lower_flank_deg: 30.,
            upper_flank_deg: 30.,
            starts: 1,
            left_hand: false,
            root_radius_mm: None,
        })
        .unwrap();
        let nut = Nut::new(thread, 10.).unwrap();
        let frame = [[0.; 3], [1., 0., 0.], [0., 1., 0.]];
        let text =
            crate::step::write_helical_nut(&nut, frame[0], frame[1], frame[2], 1e-7).unwrap();
        let (_, imported) = crate::step_read::read_import_solid(&text, 1e-7).unwrap();
        let mut wrong = imported.clone();
        wrong.profile = ImportProfile::HelicalThread {
            spec: thread.spec(),
            stock_radius_mm: None,
        };
        assert!(!helical_nut_import_matches(&wrong, &nut, frame, 1e-7));
        wrong = imported.clone();
        wrong.u = [0., 1., 0.];
        assert!(!helical_nut_import_matches(&wrong, &nut, frame, 1e-7));
        wrong = imported.clone();
        wrong.profile = ImportProfile::HelicalNut {
            spec: thread.spec(),
            across_flats_mm: 11.,
        };
        assert!(!helical_nut_import_matches(&wrong, &nut, frame, 1e-7));
        let data = parse_entities(&text, 1e-7).unwrap();
        let mut shell = Shell::new(&data, 1e-7, true);
        let solid = shell.represented_solids(true).unwrap()[0];
        let faces = shell.member_faces(solid).unwrap();
        let (_, _, bounds) = face(&shell, faces[4]).unwrap();
        let edges = helical::ring(&shell, bounds[0], true).unwrap();
        let vertices = edges
            .into_iter()
            .flat_map(|(e, _)| shell.edge(e).unwrap())
            .collect::<std::collections::BTreeSet<_>>();
        for width in [1., 4e7] {
            let mut changed = text.clone();
            for &vertex in &vertices {
                let point = reference(&shell.args(vertex, "VERTEX_POINT").unwrap()[1]).unwrap();
                let p = shell.vertex(vertex).unwrap();
                changed = super::super::rejection_tests::edit(&changed, point, |e| {
                    e.args[1] = Value::List(
                        p.into_iter()
                            .map(|v| Value::Number(v * width / 10.))
                            .collect(),
                    )
                });
            }
            let parsed = parse_entities(&changed, 1e-7).unwrap();
            let mut reader = Shell::new(&parsed, 1e-7, true);
            assert_eq!(
                read(&mut reader, faces[4], &thread, frame).unwrap_err(),
                if width == 1. {
                    ReadError::Invalid
                } else {
                    ReadError::Limit
                }
            );
            assert!(crate::step_read::read_import_solid(&changed, 1e-7).is_err());
        }
        let (_, _, upper_bounds) = face(&shell, faces[5]).unwrap();
        let upper_edges = helical::ring(&shell, upper_bounds[0], true).unwrap();
        let upper_vertex = shell.edge(upper_edges[0].0).unwrap()[0];
        let point = reference(&shell.args(upper_vertex, "VERTEX_POINT").unwrap()[1]).unwrap();
        let mut position = shell.vertex(upper_vertex).unwrap();
        position[0] += 0.1;
        let changed = super::super::rejection_tests::edit(&text, point, |e| {
            e.args[1] = Value::List(position.into_iter().map(Value::Number).collect())
        });
        assert!(
            crate::step_read::verify_helical_nut(
                &changed, &nut, frame[0], frame[1], frame[2], 1e-7
            )
            .is_err()
        );
        // Independently invalidate an upper exterior trim and a side plane.
        for target in [faces[5], faces[6]] {
            let changed = super::super::rejection_tests::edit(&text, target, |e| {
                e.args[3] = Value::Enum("F")
            });
            assert!(
                crate::step_read::verify_helical_nut(
                    &changed, &nut, frame[0], frame[1], frame[2], 1e-7
                )
                .is_err()
            );
        }
    }
}
