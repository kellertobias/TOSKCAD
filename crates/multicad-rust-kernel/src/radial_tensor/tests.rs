use super::*;
use std::sync::OnceLock;

pub(crate) fn fixture() -> BoundaryOwnership {
    static SOURCE: OnceLock<BoundaryOwnership> = OnceLock::new();
    SOURCE
        .get_or_init(|| {
            let spec = crate::helical_thread::Spec {
                major_radius_mm: 4.,
                pitch_mm: 1.25,
                length_mm: 8.,
                depth_mm: 0.5,
                crest_width_mm: 0.25,
                lower_flank_deg: 12.,
                upper_flank_deg: 18.,
                starts: 1,
                left_hand: false,
                root_radius_mm: None,
            };
            let thread = crate::tapered_thread::Thread::new(spec, 0.03125, 1e-7, 1e-6).unwrap();
            let text = crate::step::write_tapered_thread(
                &thread,
                [0.; 3],
                [1., 0., 0.],
                [0., 1., 0.],
                1e-7,
            )
            .unwrap();
            crate::step_read::read_radial_boundary(&text, 1e-7).unwrap()
        })
        .clone()
}
fn axial() -> AxialBoundary {
    fixture()
        .bind_surfaces(1e-7, None)
        .unwrap()
        .certify_axial(1e-7, 1e-6, None)
        .unwrap()
}
pub(crate) fn material() -> RadialBoundary {
    axial()
        .certify_caps(None)
        .unwrap()
        .certify_ordering(None)
        .unwrap()
        .certify_material(1e-7, None)
        .unwrap()
}

#[test]
fn cancellation_and_invalid_requests_leave_retained_geometry_reusable() {
    let source = fixture();
    let original = source.input().clone();
    assert_eq!(
        source.clone().bind_surfaces(1e-7, Some(&|| true)).err(),
        Some(Error::Cancelled)
    );
    let trimmed = source.clone().bind_surfaces(1e-7, None).unwrap();
    assert_eq!(
        trimmed
            .clone()
            .certify_axial(1e-7, 1e-6, Some(&|| true))
            .err(),
        Some(Error::Cancelled)
    );
    for invalid in [0., -1., f64::NAN] {
        assert_eq!(
            BoundaryOwnership::new(original.clone(), invalid, None).err(),
            Some(Error::Invalid)
        );
        assert_eq!(
            trimmed.clone().certify_axial(invalid, 1e-6, None).err(),
            Some(Error::Invalid)
        );
        assert_eq!(
            trimmed.clone().certify_axial(1e-7, invalid, None).err(),
            Some(Error::Invalid)
        );
    }
    let axial = trimmed.certify_axial(1e-7, 1e-6, None).unwrap();
    assert_eq!(
        axial.clone().certify_caps(Some(&|| true)).err(),
        Some(Error::Cancelled)
    );
    let caps = axial.certify_caps(None).unwrap();
    assert_eq!(
        caps.clone().certify_ordering(Some(&|| true)).err(),
        Some(Error::Cancelled)
    );
    let ordered = caps.certify_ordering(None).unwrap();
    assert_eq!(
        ordered.clone().certify_material(1e-7, Some(&|| true)).err(),
        Some(Error::Cancelled)
    );
    for invalid in [0., -1., f64::NAN] {
        assert_eq!(
            ordered.clone().certify_material(invalid, None).err(),
            Some(Error::Invalid)
        );
    }
    let material = ordered.certify_material(1e-7, None).unwrap();
    assert_eq!(material.input(), &original);
    for point in [[f64::NAN, 0., 0.], [1e8, 0., 0.]] {
        assert_eq!(material.classify(point, None), Err(Error::Invalid));
    }
    for accuracy in [0., -1., f64::NAN] {
        assert_eq!(
            material.support_bounds([1., 0., 0.], accuracy, None),
            Err(Error::Invalid)
        );
    }
    assert_eq!(
        material.support_bounds([f64::INFINITY, 0., 0.], 1e-6, None),
        Err(Error::Invalid)
    );
    assert_eq!(
        material.support_bounds([0.; 3], 1e-6, None).unwrap(),
        [0.; 2]
    );
    assert_eq!(
        material.support_bounds([1., 0., 0.], 1e-30, None),
        Err(Error::Unresolved)
    );
    assert_eq!(
        material.support_bounds([1., 0., 0.], 1e-6, Some(&|| true)),
        Err(Error::Cancelled)
    );
    assert!(material.cap_rims(2).is_none());
    assert_eq!(material.side_faces().len(), 4);
    assert_eq!(
        material.classify([0., 0., 4.], None).unwrap(),
        Location::Inside
    );
    assert_eq!(
        material.classify([0., 0., 8.1], None).unwrap(),
        Location::Outside
    );
    let bytes = crate::step::write_radial_boundary(&material, 1e-7).unwrap();
    assert_eq!(
        crate::step::write_radial_boundary(&material, 0.),
        Err(crate::step::StepError::InvalidGeometry)
    );
    assert_eq!(
        crate::step::write_radial_boundary(&material, 1e-7).unwrap(),
        bytes
    );
}

#[test]
fn a_buried_ridge_is_refused_by_actual_tensor_coefficients() {
    let mut input = fixture().input().clone();
    let mut changed = std::collections::BTreeMap::new();
    // The second tensor is the actual crest in the source writer. Change only
    // interior U rows; all edge incidence, long rails, cap endpoints and
    // analytic core supports remain the same physical source data.
    let surface = input
        .faces
        .iter()
        .filter(|f| matches!(input.surfaces[f.surface], Surface::Tensor(_)))
        .nth(1)
        .unwrap()
        .surface;
    let Surface::Tensor(t) = &input.surfaces[surface] else {
        panic!("crest tensor")
    };
    let [w, h] = t.dimensions();
    let spans = t.spans();
    let mut poles = t.poles().to_vec();
    for u in 1..w - 1 {
        for p in &mut poles[u * h..(u + 1) * h] {
            let old = *p;
            p[0] *= 0.65;
            p[1] *= 0.65;
            changed.insert(old.map(f64::to_bits), *p);
        }
    }
    input.surfaces[surface] = Surface::Tensor(RetainedTensor::new(spans, poles).unwrap());
    for curve in &mut input.curves {
        if let Curve::Spline { poles, .. } = curve {
            for p in poles {
                if let Some(&new) = changed.get(&p.map(f64::to_bits)) {
                    *p = new;
                }
            }
        }
    }
    let source = BoundaryOwnership::new(input.clone(), 1e-7, None).unwrap();
    let axial = source
        .bind_surfaces(1e-7, None)
        .unwrap()
        .certify_axial(1e-7, 1e-6, None)
        .unwrap();
    let ordered = axial
        .certify_caps(None)
        .unwrap()
        .certify_ordering(None)
        .unwrap();
    assert_eq!(
        ordered.certify_material(1e-7, None).err(),
        Some(Error::Unresolved)
    );
    assert_eq!(
        material().classify([0., 0., 4.], None).unwrap(),
        Location::Inside
    );
}

#[test]
fn finite_source_tables_and_supports_refuse_malformed_data_before_publication() {
    let original = fixture().input().clone();
    let reject = |change: &dyn Fn(&mut BoundaryInput), expected| {
        let mut input = original.clone();
        change(&mut input);
        assert_eq!(
            BoundaryOwnership::new(input, 1e-7, None).err(),
            Some(expected)
        );
    };
    reject(&|i| i.vertices.resize(81, [0.; 3]), Error::Limit);
    reject(&|i| i.faces[0].outer.resize(81, (0, true)), Error::Limit);
    reject(
        &|i| {
            i.curves = vec![
                Curve::Spline {
                    spans: 1,
                    poles: vec![[0.; 3]; 400_000]
                };
                3
            ]
        },
        Error::Limit,
    );
    reject(&|i| i.vertices[0][0] = f64::NAN, Error::Invalid);
    reject(
        &|i| {
            if let Surface::Plane(a) = &mut i.surfaces[0] {
                a.normal[0] = f64::NAN;
            }
        },
        Error::Invalid,
    );
    let cone = original
        .surfaces
        .iter()
        .position(|s| matches!(s, Surface::Cone { .. }))
        .unwrap();
    reject(
        &|i| {
            if let Surface::Cone { radius_mm, .. } = &mut i.surfaces[cone] {
                *radius_mm = 0.;
            }
        },
        Error::Invalid,
    );
    reject(
        &|i| {
            if let Surface::Cone { semi_angle_rad, .. } = &mut i.surfaces[cone] {
                *semi_angle_rad = std::f64::consts::FRAC_PI_2;
            }
        },
        Error::Invalid,
    );
    let spline = original
        .curves
        .iter()
        .position(|c| matches!(c, Curve::Spline { .. }))
        .unwrap();
    reject(
        &|i| {
            if let Curve::Spline { spans, .. } = &mut i.curves[spline] {
                *spans = 0;
            }
        },
        Error::Invalid,
    );
    reject(
        &|i| {
            if let Curve::Spline { spans, .. } = &mut i.curves[spline] {
                *spans = usize::MAX;
            }
        },
        Error::Limit,
    );
    reject(
        &|i| {
            if let Curve::Spline { poles, .. } = &mut i.curves[spline] {
                poles.pop();
            }
        },
        Error::Invalid,
    );
    reject(
        &|i| {
            if let Curve::Spline { poles, .. } = &mut i.curves[spline] {
                poles[0][0] = f64::INFINITY;
            }
        },
        Error::Invalid,
    );
    reject(
        &|i| {
            if let Curve::Spline { poles, .. } = &mut i.curves[spline] {
                poles[0][0] += 0.01;
            }
        },
        Error::Invalid,
    );
    let circle = original
        .curves
        .iter()
        .position(|c| matches!(c, Curve::Circle { .. }))
        .unwrap();
    reject(
        &|i| {
            if let Curve::Circle { radius_mm, .. } = &mut i.curves[circle] {
                *radius_mm = -1.;
            }
        },
        Error::Invalid,
    );
    reject(
        &|i| {
            if let Curve::Circle { axis, .. } = &mut i.curves[circle] {
                axis.origin[0] += 0.01;
            }
        },
        Error::Unresolved,
    );
    reject(&|i| i.edges[0].vertices = [0, 0], Error::Invalid);
    reject(&|i| i.edges[0].vertices[0] = usize::MAX, Error::Invalid);
    reject(&|i| i.edges[0].curve = usize::MAX, Error::Invalid);
    reject(&|i| i.faces[0].surface = usize::MAX, Error::Invalid);
    let source = fixture();
    assert_eq!(source.edge_faces().len(), 14);
    assert_eq!(source.vertex_edges().len(), 10);
    assert!(axial().start_radius_bounds_mm()[0] > 3.4);
    assert!(material().boundary_error_mm() < 1e-7 / 8.);
    assert_eq!(source.input(), &original);
}

#[test]
fn axial_admission_uses_retained_supports_and_rejects_unproved_parameterizations() {
    let original = fixture().input().clone();
    let reject = |change: &dyn Fn(&mut BoundaryInput), minimum, expected| {
        let mut input = original.clone();
        change(&mut input);
        let trimmed = BoundaryOwnership::new(input, 1e-7, None)
            .unwrap()
            .bind_surfaces(1e-7, None)
            .unwrap();
        assert_eq!(
            trimmed.certify_axial(1e-7, minimum, None).err(),
            Some(expected)
        );
    };
    reject(&|_| {}, 9., Error::Unresolved);
    reject(&|_| {}, 4., Error::Unresolved);
    let plane = original
        .faces
        .iter()
        .position(|f| matches!(original.surfaces[f.surface], Surface::Plane(_)))
        .unwrap();
    reject(
        &|i| {
            i.faces[plane].forward = !i.faces[plane].forward;
        },
        1e-6,
        Error::Invalid,
    );
    reject(
        &|i| {
            if let Surface::Plane(a) = &mut i.surfaces[original.faces[plane].surface] {
                a.normal = a.normal.map(|v| 2. * v);
            }
        },
        1e-6,
        Error::Unsupported,
    );
    let cone = original
        .faces
        .iter()
        .position(|f| matches!(original.surfaces[f.surface], Surface::Cone { .. }))
        .unwrap();
    reject(&|i| i.faces[cone].forward = false, 1e-6, Error::Unsupported);
    let tensor = original
        .faces
        .iter()
        .find(|f| matches!(original.surfaces[f.surface], Surface::Tensor(_)))
        .unwrap()
        .surface;
    for (delta, expected) in [(0.001, Error::Unresolved), (-100., Error::Unresolved)] {
        reject(
            &|i| {
                let Surface::Tensor(t) = &i.surfaces[tensor] else {
                    unreachable!()
                };
                let spans = t.spans();
                let h = t.dimensions()[1];
                let mut poles = t.poles().to_vec();
                poles[h + 1][2] += delta;
                i.surfaces[tensor] = Surface::Tensor(RetainedTensor::new(spans, poles).unwrap());
            },
            1e-6,
            expected,
        );
    }
    let trimmed = fixture().bind_surfaces(1e-7, None).unwrap();
    assert_eq!(
        trimmed.certify_axial(1e-30, 1e-6, None).err(),
        Some(Error::Unresolved)
    );
    assert!(axial().height_bounds_mm()[0] > 7.99);
}

#[test]
fn equivalent_face_edge_curve_and_tensor_parameterizations_keep_physical_material() {
    let original = fixture().input().clone();
    for variant in 0..4 {
        let mut input = original.clone();
        input.faces.reverse();
        match variant {
            0 => {
                for edge in &mut input.edges {
                    edge.vertices.swap(0, 1);
                    edge.forward = !edge.forward;
                }
                for face in &mut input.faces {
                    for (_, forward) in &mut face.outer {
                        *forward = !*forward;
                    }
                }
            }
            1 => {
                for curve in &mut input.curves {
                    if let Curve::Spline { poles, .. } = curve {
                        poles.reverse();
                    }
                }
                for edge in &mut input.edges {
                    if matches!(input.curves[edge.curve], Curve::Spline { .. }) {
                        edge.forward = !edge.forward;
                    }
                }
            }
            2 | 3 => {
                for surface in &mut input.surfaces {
                    if let Surface::Tensor(t) = surface {
                        let spans = t.spans();
                        let h = t.dimensions()[1];
                        let mut rows = t
                            .poles()
                            .chunks_exact(h)
                            .map(|r| r.to_vec())
                            .collect::<Vec<_>>();
                        if variant == 2 {
                            rows.reverse();
                        } else {
                            for row in &mut rows {
                                row.reverse();
                            }
                        }
                        *surface = Surface::Tensor(
                            RetainedTensor::new(spans, rows.into_iter().flatten().collect())
                                .unwrap(),
                        );
                    }
                }
                for face in &mut input.faces {
                    if matches!(input.surfaces[face.surface], Surface::Tensor(_)) {
                        face.forward = !face.forward;
                    }
                }
            }
            _ => unreachable!(),
        }
        let body = BoundaryOwnership::new(input, 1e-7, None)
            .unwrap()
            .bind_surfaces(1e-7, None)
            .unwrap()
            .certify_axial(1e-7, 1e-6, None)
            .unwrap()
            .certify_caps(None)
            .unwrap()
            .certify_ordering(None)
            .unwrap()
            .certify_material(1e-7, None)
            .unwrap();
        assert_eq!(body.classify([0., 0., 4.], None).unwrap(), Location::Inside);
        assert_eq!(
            body.classify([10., 0., 4.], None).unwrap(),
            Location::Outside
        );
        let bytes = crate::step::write_radial_boundary(&body, 1e-7).unwrap();
        let decoded = crate::step_read::read_radial_boundary(&bytes, 1e-7).unwrap();
        assert_eq!(decoded.input().faces.len(), original.faces.len());
        assert!(
            decoded
                .input()
                .vertices
                .iter()
                .all(|p| original.vertices.contains(p))
        );
    }
}
#[test]
fn an_ordinary_closed_planar_cube_is_bound_but_not_misclassified_as_radial_material() {
    let vertices = vec![
        [0., 0., 0.],
        [1., 0., 0.],
        [1., 1., 0.],
        [0., 1., 0.],
        [0., 0., 1.],
        [1., 0., 1.],
        [1., 1., 1.],
        [0., 1., 1.],
    ];
    let pairs = [
        [0, 1],
        [1, 2],
        [2, 3],
        [3, 0],
        [4, 5],
        [5, 6],
        [6, 7],
        [7, 4],
        [0, 4],
        [1, 5],
        [2, 6],
        [3, 7],
    ];
    let curves = pairs
        .iter()
        .map(|&[a, b]| Curve::Spline {
            spans: 1,
            poles: (0..=9)
                .map(|i| {
                    std::array::from_fn(|k| {
                        vertices[a][k] + (vertices[b][k] - vertices[a][k]) * i as f64 / 9.
                    })
                })
                .collect(),
        })
        .collect();
    let edges = pairs
        .iter()
        .enumerate()
        .map(|(curve, &vertices)| Edge {
            vertices,
            curve,
            forward: true,
        })
        .collect();
    let wires = vec![
        vec![(3, false), (2, false), (1, false), (0, false)],
        vec![(4, true), (5, true), (6, true), (7, true)],
        vec![(0, true), (9, true), (4, false), (8, false)],
        vec![(1, true), (10, true), (5, false), (9, false)],
        vec![(2, true), (11, true), (6, false), (10, false)],
        vec![(3, true), (8, true), (7, false), (11, false)],
    ];
    let mut surfaces = Vec::new();
    let mut faces = Vec::new();
    for wire in wires {
        let points = wire
            .iter()
            .map(|&(edge, d)| vertices[pairs[edge][usize::from(!d)]])
            .collect::<Vec<_>>();
        let a: Point = std::array::from_fn(|i| points[1][i] - points[0][i]);
        let b: Point = std::array::from_fn(|i| points[2][i] - points[0][i]);
        let normal = [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ];
        let surface = surfaces.len();
        surfaces.push(Surface::Plane(Axis {
            origin: points[0],
            normal,
            reference: a,
        }));
        faces.push(Face {
            surface,
            outer: wire,
            forward: true,
        });
    }
    let input = BoundaryInput {
        vertices,
        curves,
        edges,
        surfaces,
        faces,
    };
    let bound = BoundaryOwnership::new(input, 1e-7, None)
        .unwrap()
        .bind_surfaces(1e-7, None)
        .unwrap();
    assert_eq!(
        bound.certify_axial(1e-7, 1e-6, None).err(),
        Some(Error::Unsupported)
    );
}
