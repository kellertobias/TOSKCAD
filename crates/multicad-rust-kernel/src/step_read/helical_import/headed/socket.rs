//! Candidate dimensions from a cylindrical socket head's actual finite trims.
use super::*;
pub(in super::super) fn socket(
    shell: &mut Shell<'_, '_>,
    faces: &[u32; 9],
    thread: &Thread,
    frame: [Point; 3],
    shank_length_mm: f64,
) -> Result<Screw> {
    let chart = Chart::new(frame)?;
    let (bottom, bounds) = plane(shell, faces[0])?;
    require(bounds.len() == 2)?;
    let (floor, _) = plane(shell, faces[2])?;
    let below = -chart.local(bottom.origin)[2];
    let height = below - shank_length_mm;
    let depth = chart.local(floor.origin)[2] + below;
    let (cylinder, forward, _) = face(shell, faces[1])?;
    require(forward)?;
    let [Value::String(_), _, Value::Number(radius)] =
        shell.args(cylinder, "CYLINDRICAL_SURFACE")?
    else {
        return Err(ReadError::Invalid);
    };
    let radius = *radius;
    let edges = helical::ring(shell, bounds[1], false)?;
    require(edges.len() == 6)?;
    let mut half_width = 0_f64;
    for (edge, _) in edges {
        for vertex in shell.edge(edge)? {
            half_width = half_width.max(chart.local(shell.vertex(vertex)?)[0].abs());
        }
    }
    Screw::new(
        *thread,
        ScrewSpec {
            head: ScrewHead::SocketCap {
                radius_mm: radius,
                height_mm: height,
                socket_across_flats_mm: 2. * half_width,
                socket_depth_mm: depth,
            },
            shank_length_mm,
            shank_radius_mm: thread.spec().major_radius_mm,
        },
    )
    .map_err(|error| match error {
        crate::helical_thread::ThreadError::Invalid => ReadError::Invalid,
        crate::helical_thread::ThreadError::Limit => ReadError::Limit,
    })
}

fn plane(shell: &Shell<'_, '_>, id: u32) -> Result<(Plane, Vec<u32>)> {
    let (surface, forward, bounds) = face(shell, id)?;
    require(forward)?;
    let [Value::String(_), placement] = shell.args(surface, "PLANE")? else {
        return Err(ReadError::Invalid);
    };
    Ok((shell.placement(reference(placement)?)?, bounds))
}
pub(in super::super) fn shank_length(
    shell: &Shell<'_, '_>,
    seat: u32,
    frame: [Point; 3],
) -> Result<f64> {
    let (seat, bounds) = plane(shell, seat)?;
    require(bounds.len() == 2)?;
    let length = -Chart::new(frame)?.local(seat.origin)[2];
    require(length.is_finite() && length > 0.)?;
    Ok(length)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn screw(left_hand: bool, rounded: bool) -> Screw {
        screw_with_shank(left_hand, rounded, 0.)
    }
    fn screw_with_shank(left_hand: bool, rounded: bool, shank_length_mm: f64) -> Screw {
        let root = rounded.then_some(1. / (4. * 3_f64.sqrt()));
        let thread = Thread::new(Spec {
            major_radius_mm: 3.,
            pitch_mm: 1.,
            length_mm: 3.7,
            depth_mm: root.map_or(5. * 3_f64.sqrt() / 16., |f| 0.875 * 3_f64.sqrt() / 2. - f),
            crest_width_mm: 0.125,
            lower_flank_deg: 30.,
            upper_flank_deg: 30.,
            starts: 1,
            left_hand,
            root_radius_mm: root,
        })
        .unwrap();
        Screw::new(
            thread,
            ScrewSpec {
                head: ScrewHead::SocketCap {
                    radius_mm: 5.5,
                    height_mm: 4.,
                    socket_across_flats_mm: 4.,
                    socket_depth_mm: 2.,
                },
                shank_length_mm,
                shank_radius_mm: 3.,
            },
        )
        .unwrap()
    }
    #[test]
    fn independent_socket_heads_keep_both_hands_roots_and_full_finite_closure() {
        for left_hand in [false, true] {
            for rounded in [false, true] {
                let screw = screw(left_hand, rounded);
                for frame in [
                    [[0.; 3], [1., 0., 0.], [0., 1., 0.]],
                    [[2., -3., 4.], [0.8, 0.6, 0.], [-0.36, 0.48, 0.8]],
                ] {
                    let [o, u, v] = frame;
                    let text = crate::step::write_helical_screw(&screw, o, u, v, 1e-7).unwrap();
                    let (metrics, read) = crate::step_read::read_import_solid(&text, 1e-7).unwrap();
                    assert_eq!(metrics.face_count, 15);
                    assert!((metrics.volume_mm3 - screw.volume_mm3()).abs() < 1e-6);
                    assert!(helical_screw_import_matches(&read, &screw, frame, 1e-7));
                    if !left_hand && !rounded && o == [0.; 3] {
                        let data = parse_entities(&text, 1e-7).unwrap();
                        let reader = Shell::new(&data, 1e-7, true);
                        let solid = reader.represented_solids(true).unwrap()[0];
                        let faces = reader.member_faces(solid).unwrap();
                        let (surface, _, _) = face(&reader, faces[8]).unwrap();
                        let placement =
                            reference(&reader.args(surface, "PLANE").unwrap()[1]).unwrap();
                        let center =
                            reference(&reader.args(placement, "AXIS2_PLACEMENT_3D").unwrap()[1])
                                .unwrap();
                        let changed =
                            super::super::super::rejection_tests::edit(&text, center, |e| {
                                e.args[1] = Value::List(vec![
                                    Value::Number(0.),
                                    Value::Number(0.),
                                    Value::Number(2.),
                                ]);
                            });
                        assert_eq!(
                            crate::step_read::read_import_solid(&changed, 1e-7).unwrap_err(),
                            ReadError::Invalid
                        );
                        let (cylinder, _, _) = face(&reader, faces[7]).unwrap();
                        for (surface, count) in [(surface, 1), (cylinder, 2)] {
                            let malformed =
                                super::super::super::rejection_tests::edit(&text, surface, |e| {
                                    e.args.truncate(count)
                                });
                            assert_eq!(
                                crate::step_read::read_import_solid(&malformed, 1e-7).unwrap_err(),
                                ReadError::Invalid
                            );
                        }
                        let changed =
                            super::super::super::rejection_tests::edit(&text, cylinder, |e| {
                                e.args[2] = Value::Number(2e7);
                            });
                        assert_eq!(
                            crate::step_read::read_import_solid(&changed, 1e-7).unwrap_err(),
                            ReadError::Limit
                        );
                        let large_head = ScrewHead::SocketCap {
                            radius_mm: 40_000.,
                            height_mm: 4.,
                            socket_across_flats_mm: 4.,
                            socket_depth_mm: 2.,
                        };
                        let large = Screw::new(
                            *screw.thread(),
                            ScrewSpec {
                                head: large_head,
                                ..screw.spec()
                            },
                        )
                        .unwrap();
                        let mut noisy = read.clone();
                        noisy.profile = ImportProfile::HelicalScrew {
                            spec: screw.thread().spec(),
                            screw: large.spec(),
                        };
                        assert!(helical_screw_import_matches(&noisy, &large, frame, 1e-7));
                        noisy.u[0] = (1. + 8e-11_f64).sqrt();
                        let only_thread = ImportSolid {
                            profile: ImportProfile::HelicalThread {
                                spec: screw.thread().spec(),
                                stock_radius_mm: None,
                            },
                            ..noisy.clone()
                        };
                        assert!(helical_import_matches(
                            &only_thread,
                            screw.thread(),
                            None,
                            frame,
                            1e-7
                        ));
                        assert!(!helical_screw_import_matches(&noisy, &large, frame, 1e-7));
                    }
                    for head in [
                        ScrewHead::SocketCap {
                            radius_mm: 6.5,
                            height_mm: 4.,
                            socket_across_flats_mm: 4.,
                            socket_depth_mm: 2.,
                        },
                        ScrewHead::SocketCap {
                            radius_mm: 5.5,
                            height_mm: 5.,
                            socket_across_flats_mm: 4.,
                            socket_depth_mm: 2.,
                        },
                        ScrewHead::SocketCap {
                            radius_mm: 5.5,
                            height_mm: 4.,
                            socket_across_flats_mm: 5.,
                            socket_depth_mm: 2.,
                        },
                        ScrewHead::SocketCap {
                            radius_mm: 5.5,
                            height_mm: 4.,
                            socket_across_flats_mm: 4.,
                            socket_depth_mm: 2.5,
                        },
                    ] {
                        let wrong = ImportSolid {
                            profile: ImportProfile::HelicalScrew {
                                spec: screw.thread().spec(),
                                screw: ScrewSpec {
                                    head,
                                    ..screw.spec()
                                },
                            },
                            ..read.clone()
                        };
                        assert!(!helical_screw_import_matches(&wrong, &screw, frame, 1e-7));
                    }
                }
            }
        }
    }
    #[test]
    fn independent_partial_shanks_keep_all_tangent_arcs_roots_hands_and_retained_charts() {
        for left in [false, true] {
            for rounded in [false, true] {
                for length in [0.5, 2., 10.] {
                    let screw = screw_with_shank(left, rounded, length);
                    for frame in [
                        [[0.; 3], [1., 0., 0.], [0., 1., 0.]],
                        [[3.3, -7.1, 11.2], [0.8, 0.6, 0.], [-0.36, 0.48, 0.8]],
                        [[1e6, -5e5, 3e5], [0.8, 0.6, 0.], [-0.36, 0.48, 0.8]],
                    ] {
                        let [o, u, v] = frame;
                        let text = crate::step::write_helical_screw(&screw, o, u, v, 1e-7).unwrap();
                        eprintln!(
                            "shank left={left} rounded={rounded} length={length} origin={o:?}"
                        );
                        let (metrics, read) =
                            crate::step_read::read_import_solid(&text, 1e-7).unwrap();
                        if !left && !rounded && length == 0.5 && o == [0.; 3] {
                            let data = parse_entities(&text, 1e-7).unwrap();
                            let reader = Shell::new(&data, 1e-7, true);
                            let solid = reader.represented_solids(true).unwrap()[0];
                            let faces = reader.member_faces(solid).unwrap();
                            let (seat, _, _) = face(&reader, faces[7]).unwrap();
                            let placement =
                                reference(&reader.args(seat, "PLANE").unwrap()[1]).unwrap();
                            let center = reference(
                                &reader.args(placement, "AXIS2_PLACEMENT_3D").unwrap()[1],
                            )
                            .unwrap();
                            let malformed =
                                super::super::super::rejection_tests::edit(&text, center, |e| {
                                    e.args[1] = Value::List(vec![
                                        Value::Number(0.),
                                        Value::Number(0.),
                                        Value::Number(0.25),
                                    ])
                                });
                            assert!(crate::step_read::read_import_solid(&malformed, 1e-7).is_err());
                            for actual in [
                                ScrewSpec {
                                    shank_radius_mm: 3.25,
                                    ..screw.spec()
                                },
                                ScrewSpec {
                                    shank_length_mm: 0.75,
                                    ..screw.spec()
                                },
                                ScrewSpec {
                                    shank_length_mm: -1.,
                                    ..screw.spec()
                                },
                            ] {
                                let wrong = ImportSolid {
                                    profile: ImportProfile::HelicalScrew {
                                        spec: screw.thread().spec(),
                                        screw: actual,
                                    },
                                    ..read.clone()
                                };
                                assert!(!helical_screw_import_matches(&wrong, &screw, frame, 1e-7));
                            }
                            let hex = Screw::new(
                                *screw.thread(),
                                ScrewSpec {
                                    head: ScrewHead::ExternalHex {
                                        across_flats_mm: 10.,
                                        height_mm: 4.,
                                    },
                                    ..screw.spec()
                                },
                            )
                            .unwrap();
                            let wrong = ImportSolid {
                                profile: ImportProfile::HelicalScrew {
                                    spec: hex.thread().spec(),
                                    screw: hex.spec(),
                                },
                                ..read.clone()
                            };
                            assert!(!helical_screw_import_matches(&wrong, &hex, frame, 1e-7));
                            let long = Screw::new(
                                *screw.thread(),
                                ScrewSpec {
                                    shank_length_mm: 40_000.,
                                    ..screw.spec()
                                },
                            )
                            .unwrap();
                            let mut noisy = ImportSolid {
                                profile: ImportProfile::HelicalScrew {
                                    spec: long.thread().spec(),
                                    screw: long.spec(),
                                },
                                ..read.clone()
                            };
                            assert!(helical_screw_import_matches(&noisy, &long, frame, 1e-7));
                            noisy.u[0] = (1. + 8e-11_f64).sqrt();
                            let only_thread = ImportSolid {
                                profile: ImportProfile::HelicalThread {
                                    spec: long.thread().spec(),
                                    stock_radius_mm: None,
                                },
                                ..noisy.clone()
                            };
                            assert!(helical_import_matches(
                                &only_thread,
                                long.thread(),
                                None,
                                frame,
                                1e-7
                            ));
                            assert!(!helical_screw_import_matches(&noisy, &long, frame, 1e-7));
                        }
                        assert_eq!(metrics.face_count, 17);
                        assert!((metrics.volume_mm3 - screw.volume_mm3()).abs() < 1e-6);
                        assert!(helical_screw_import_matches(&read, &screw, frame, 1e-7));
                        assert!(
                            matches!(read.profile, ImportProfile::HelicalScrew { screw: actual, .. } if (actual.shank_length_mm - length).abs() < 1e-8)
                        );
                    }
                }
            }
        }
    }
}
