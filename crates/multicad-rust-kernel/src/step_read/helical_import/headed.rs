//! Infer candidate head dimensions from physical finite face boundaries.
use super::*;

pub(super) fn hex(
    shell: &mut Shell<'_, '_>,
    faces: &[u32; 7],
    thread: &Thread,
    [origin, u, v]: [Point; 3],
) -> Result<Screw> {
    let cap = faces[0];
    let (surface, forward, bounds) = face(shell, cap)?;
    require(forward && bounds.len() == 1)?;
    let [Value::String(_), placement] = shell.args(surface, "PLANE")? else {
        return Err(ReadError::Invalid);
    };
    let cap = shell.placement(reference(placement)?)?;
    let n = cross(u, v);
    let rows = [cross(v, n), cross(n, u), cross(u, v)];
    let determinant = dot(u, rows[0]);
    require(determinant.is_finite() && determinant > 0.)?;
    let local = |point| rows.map(|row| dot(row, sub(point, origin)) / determinant);
    let height = -local(cap.origin)[2];
    let edges = helical::ring(shell, bounds[0], true)?;
    require(edges.len() == 6)?;
    let mut half_width = 0_f64;
    for (edge, _) in edges {
        for vertex in shell.edge(edge)? {
            half_width = half_width.max(local(shell.vertex(vertex)?)[0].abs());
        }
    }
    Screw::new(
        *thread,
        ScrewSpec {
            head: ScrewHead::ExternalHex {
                across_flats_mm: 2. * half_width,
                height_mm: height,
            },
            shank_length_mm: 0.,
            shank_radius_mm: thread.spec().major_radius_mm,
        },
    )
    .map_err(|error| match error {
        crate::helical_thread::ThreadError::Invalid => ReadError::Invalid,
        crate::helical_thread::ThreadError::Limit => ReadError::Limit,
    })
}

pub fn helical_screw_import_matches(
    imported: &ImportSolid,
    source: &Screw,
    [origin, u, v]: [Point; 3],
    tolerance: f64,
) -> bool {
    let ImportProfile::HelicalScrew { spec, screw } = imported.profile else {
        return false;
    };
    let (
        ScrewHead::ExternalHex {
            across_flats_mm: a,
            height_mm: ha,
        },
        ScrewHead::ExternalHex {
            across_flats_mm: b,
            height_mm: hb,
        },
    ) = (source.spec().head, screw.head)
    else {
        return false;
    };
    if source.spec().shank_length_mm != 0. || screw.shank_length_mm != 0. {
        return false;
    }
    let thread_profile = ImportSolid {
        profile: ImportProfile::HelicalThread {
            spec,
            stock_radius_mm: None,
        },
        ..imported.clone()
    };
    if !helical_import_matches(
        &thread_profile,
        source.thread(),
        None,
        [origin, u, v],
        tolerance,
    ) {
        return false;
    }
    let world = |origin: Point, u: Point, v: Point, p: Point2, z: f64| {
        let n = cross(u, v);
        std::array::from_fn(|i| origin[i] + u[i] * p[0] + v[i] * p[1] + n[i] * z)
    };
    for (a, b) in crate::headed_screw::hex_vertices(a)
        .into_iter()
        .zip(crate::headed_screw::hex_vertices(b))
    {
        for (za, zb) in [(0., 0.), (-ha, -hb)] {
            let error = length(sub(
                world(origin, u, v, a, za),
                world(imported.origin_mm, imported.u, imported.v, b, zb),
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
    fn independently_decodes_hex_heads_from_full_physical_boundaries() {
        for left_hand in [false, true] {
            let thread = Thread::new(Spec {
                major_radius_mm: 3.,
                pitch_mm: 1.,
                length_mm: 3.7,
                depth_mm: 5. * 3_f64.sqrt() / 16.,
                crest_width_mm: 0.125,
                lower_flank_deg: 30.,
                upper_flank_deg: 30.,
                starts: 1,
                left_hand,
                root_radius_mm: None,
            })
            .unwrap();
            let screw = Screw::new(
                thread,
                ScrewSpec {
                    head: ScrewHead::ExternalHex {
                        across_flats_mm: 10.,
                        height_mm: 4.,
                    },
                    shank_length_mm: 0.,
                    shank_radius_mm: 3.,
                },
            )
            .unwrap();
            for [origin, u, v] in [
                [[0.; 3], [1., 0., 0.], [0., 1., 0.]],
                [[2., -3., 4.], [0.8, 0.6, 0.], [-0.36, 0.48, 0.8]],
            ] {
                let source = crate::step::write_helical_screw(&screw, origin, u, v, 1e-7).unwrap();
                let (_, read) = crate::step_read::read_import_solid(&source, 1e-7)
                    .expect("independent hex-headed screw");
                assert!(matches!(
                    read.profile,
                    ImportProfile::HelicalScrew {
                        screw: ScrewSpec {
                            head: ScrewHead::ExternalHex { .. },
                            ..
                        },
                        ..
                    }
                ));
                assert!(helical_screw_import_matches(
                    &read,
                    &screw,
                    [origin, u, v],
                    1e-7
                ));
                let mut wrong = read.clone();
                wrong.profile = ImportProfile::HelicalThread {
                    spec: thread.spec(),
                    stock_radius_mm: None,
                };
                assert!(!helical_screw_import_matches(
                    &wrong,
                    &screw,
                    [origin, u, v],
                    1e-7
                ));
                for head in [
                    ScrewHead::SocketCap {
                        radius_mm: 6.,
                        height_mm: 4.,
                        socket_across_flats_mm: 4.,
                        socket_depth_mm: 2.,
                    },
                    ScrewHead::ExternalHex {
                        across_flats_mm: 11.,
                        height_mm: 4.,
                    },
                    ScrewHead::ExternalHex {
                        across_flats_mm: 10.,
                        height_mm: 5.,
                    },
                ] {
                    wrong.profile = ImportProfile::HelicalScrew {
                        spec: thread.spec(),
                        screw: ScrewSpec {
                            head,
                            ..screw.spec()
                        },
                    };
                    assert!(!helical_screw_import_matches(
                        &wrong,
                        &screw,
                        [origin, u, v],
                        1e-7
                    ));
                }
                wrong.profile = ImportProfile::HelicalScrew {
                    spec: thread.spec(),
                    screw: ScrewSpec {
                        shank_length_mm: 1.,
                        ..screw.spec()
                    },
                };
                assert!(!helical_screw_import_matches(
                    &wrong,
                    &screw,
                    [origin, u, v],
                    1e-7
                ));
                wrong = read.clone();
                wrong.origin_mm[0] += 1.;
                assert!(!helical_screw_import_matches(
                    &wrong,
                    &screw,
                    [origin, u, v],
                    1e-7
                ));
                if !left_hand && origin == [0.; 3] {
                    let large = Screw::new(
                        thread,
                        ScrewSpec {
                            head: ScrewHead::ExternalHex {
                                across_flats_mm: 40_000.,
                                height_mm: 4.,
                            },
                            ..screw.spec()
                        },
                    )
                    .unwrap();
                    let mut large_import = read.clone();
                    large_import.profile = ImportProfile::HelicalScrew {
                        spec: thread.spec(),
                        screw: large.spec(),
                    };
                    assert!(helical_screw_import_matches(
                        &large_import,
                        &large,
                        [origin, u, v],
                        1e-7
                    ));
                    large_import.u[0] = (1. + 8e-11_f64).sqrt();
                    let only_thread = ImportSolid {
                        profile: ImportProfile::HelicalThread {
                            spec: thread.spec(),
                            stock_radius_mm: None,
                        },
                        ..large_import.clone()
                    };
                    assert!(helical_import_matches(
                        &only_thread,
                        &thread,
                        None,
                        [origin, u, v],
                        1e-7
                    ));
                    assert!(!helical_screw_import_matches(
                        &large_import,
                        &large,
                        [origin, u, v],
                        1e-7
                    ));
                    let entities = parse_entities(&source, 1e-7).unwrap();
                    let reader = Shell::new(&entities, 1e-7, true);
                    let solid = reader.represented_solids(true).unwrap()[0];
                    let face_ids = reader.member_faces(solid).unwrap();
                    let (surface, _, _) = face(&reader, face_ids[6]).unwrap();
                    let placement = reference(&reader.args(surface, "PLANE").unwrap()[1]).unwrap();
                    let center =
                        reference(&reader.args(placement, "AXIS2_PLACEMENT_3D").unwrap()[1])
                            .unwrap();
                    for (z, expected) in [(4., ReadError::Invalid), (-2e7, ReadError::Limit)] {
                        let malformed = super::super::rejection_tests::edit(&source, center, |e| {
                            e.args[1] = Value::List(vec![
                                Value::Number(0.),
                                Value::Number(0.),
                                Value::Number(z),
                            ]);
                        });
                        assert_eq!(
                            crate::step_read::read_import_solid(&malformed, 1e-7).unwrap_err(),
                            expected
                        );
                    }
                }
                let generic = crate::step_read::read_analytic_solid(&source, 1e-7).unwrap();
                assert!((generic.volume_mm3 - screw.volume_mm3()).abs() < 1e-6);
            }
        }
    }
}
