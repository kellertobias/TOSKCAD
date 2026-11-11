use super::*;
fn curve(curve: ReliefCurve, frame: Frame) -> Curve {
    let ReliefCurve::Profile { edge, z_mm } = curve else {
        return Curve::Line;
    };
    let (center, clockwise, radius) = match edge {
        ProfileEdge::Line { .. } => return Curve::Line,
        ProfileEdge::Circle {
            center,
            clockwise,
            radius_mm,
        } => (center, clockwise, radius_mm),
        ProfileEdge::Arc {
            center, clockwise, ..
        } => (center, clockwise, edge.radius_mm().expect("arc radius")),
    };
    let center_mm = frame.world([center[0], center[1], z_mm]);
    let normal = unit(frame.axes[2])
        .expect("checked chart")
        .map(|v| if clockwise { -v } else { v });
    if matches!(edge, ProfileEdge::Circle { .. }) {
        return Curve::Circle(CircleEdge {
            center_mm,
            normal,
            radius_mm: radius,
        });
    }
    let radial = |p: Point2| {
        unit(frame.vector([(p[0] - center[0]) / radius, (p[1] - center[1]) / radius, 0.]))
            .expect("certified radial edge")
    };
    Curve::Arc(ArcEdge {
        center_mm,
        normal,
        radius_mm: radius,
        radial_start: radial(edge.start()),
        radial_end: radial(edge.end()),
        sweep_rad: edge.signed_sweep_rad().expect("circular sweep").abs(),
    })
}
fn support(a: Surface, af: Frame, b: Surface, bf: Frame, epsilon: f64, extent: f64) -> Result<()> {
    match (a, b) {
        (
            Surface::Plane {
                origin_mm: ao,
                u: au,
                v: av,
            },
            Surface::Plane {
                origin_mm: bo,
                u: bu,
                v: bv,
            },
        ) => {
            let an = unit(af.vector(cross(au, av)))?;
            let bn = unit(bf.vector(cross(bu, bv)))?;
            require(
                dot(an, bn) > 0.
                    && length(cross(an, bn)) * extent.max(1.) <= epsilon
                    && dot(sub(af.world(ao), bf.world(bo)), bn).abs() <= epsilon,
            )
        }
        (
            Surface::Cylinder {
                origin_mm: ao,
                axis: aa,
                radius_mm: ar,
                outward: asense,
                ..
            },
            Surface::Cylinder {
                origin_mm: bo,
                axis: ba,
                radius_mm: br,
                outward: bsense,
                ..
            },
        ) => {
            let an = unit(af.vector(aa))?;
            let bn = unit(bf.vector(ba))?;
            require(
                asense == bsense
                    && (ar - br).abs() <= epsilon
                    && length(cross(an, bn)) * extent.max(ar).max(br) <= epsilon
                    && length(cross(sub(af.world(ao), bf.world(bo)), bn)) <= epsilon,
            )
        }
        _ => Err(ReadError::Invalid),
    }
}
fn matches(actual: &Relief, af: Frame, expected: &Relief, ef: Frame, tolerance: f64) -> Result<()> {
    require(tolerance.is_finite() && tolerance >= 0.)?;
    let graph = actual.boundary();
    let other = expected.boundary();
    require(graph.faces.len() == other.faces.len())?;
    let entities = BTreeMap::new();
    let mut shell = Shell::new(&entities, tolerance, true);
    let mut work = Work(MAX_WORK);
    for (id, &p) in graph.vertices.iter().enumerate() {
        work.charge(1)?;
        shell.points.insert(
            u32::try_from(id).map_err(|_| ReadError::Limit)?,
            af.world(p),
        );
    }
    for (id, edge) in graph.edges.iter().enumerate() {
        work.charge(1)?;
        shell.edges.insert(
            u32::try_from(id).map_err(|_| ReadError::Limit)?,
            Edge {
                vertices: edge
                    .vertices
                    .map(|v| u32::try_from(v).expect("bounded boundary vertex")),
                curve: curve(edge.curve, af),
                uses: Vec::new(),
            },
        );
    }
    let epsilon = tolerance / 16.;
    let vertices = proof::vertices(&shell, other, ef, epsilon, &mut work)?;
    let edges = proof::edges(&shell, other, &vertices, ef, epsilon, &mut work)?;
    let mut faces = BTreeMap::new();
    for (i, face) in graph.faces.iter().enumerate() {
        work.charge(face.loops.iter().map(Vec::len).sum())?;
        let mut wires = face
            .loops
            .iter()
            .map(|wire| {
                cyclic(
                    wire.iter()
                        .map(|e| (u32::try_from(e.edge).expect("bounded edge"), e.forward))
                        .collect(),
                )
            })
            .collect::<Vec<_>>();
        wires.sort_unstable();
        require(faces.insert(wires, i).is_none())?;
    }
    let bounds = expected.bounds();
    let extent = length(sub(bounds[1], bounds[0]));
    for face in &other.faces {
        work.charge(face.loops.iter().map(Vec::len).sum())?;
        let mut wires = face
            .loops
            .iter()
            .map(|wire| {
                cyclic(
                    wire.iter()
                        .map(|e| {
                            let (id, forward) = edges[e.edge];
                            (id, e.forward == forward)
                        })
                        .collect(),
                )
            })
            .collect::<Vec<_>>();
        wires.sort_unstable();
        let i = faces.remove(&wires).ok_or(ReadError::Invalid)?;
        support(
            graph.faces[i].surface,
            af,
            face.surface,
            ef,
            epsilon,
            extent,
        )?;
    }
    require(faces.is_empty())
}
fn pocketed(imported: &ImportSolid, frame: Frame, tolerance: f64) -> Result<(Relief, Frame)> {
    use crate::normal_sweep::Sweep;
    let ImportProfile::PocketedPolygon {
        rings,
        circular_holes,
        pockets,
    } = &imported.profile
    else {
        return Err(ReadError::Unsupported);
    };
    require(
        !pockets.is_empty()
            && pockets.len() <= crate::normal_relief::MAX_MOTIFS
            && tolerance.is_finite()
            && tolerance > 0.,
    )?;
    let end = pockets[0].end;
    require(pockets.iter().all(|p| p.end == end))?;
    let mut count = circular_holes.len();
    for ring in rings {
        count = count.checked_add(ring.len()).ok_or(ReadError::Limit)?;
    }
    require(count <= crate::polygon::MAX_VERTICES)?;
    let point = |[x, y]: Point2| [x, if end { y } else { -y }];
    let mut profile = rings
        .iter()
        .map(|ring| {
            let mut points = ring.iter().copied().map(point).collect::<Vec<_>>();
            if !end {
                points.reverse();
            }
            points
                .iter()
                .copied()
                .zip(points.iter().copied().cycle().skip(1))
                .map(|(a, b)| ProfileEdge::Line { a, b })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    profile.extend(circular_holes.iter().map(|&(center, radius_mm)| {
        vec![ProfileEdge::Circle {
            center: point(center),
            radius_mm,
            clockwise: true,
        }]
    }));
    let minimum = tolerance.next_up();
    let base = Sweep::new(profile, imported.height_mm, tolerance, minimum)
        .map_err(|_| ReadError::Invalid)?;
    let motifs = pockets
        .iter()
        .map(|p| {
            Sweep::new(
                vec![vec![ProfileEdge::Circle {
                    center: point(p.center_mm),
                    radius_mm: p.radius_mm,
                    clockwise: false,
                }]],
                p.depth_mm,
                tolerance,
                minimum,
            )
            .map_err(|_| ReadError::Invalid)
        })
        .collect::<Result<Vec<_>>>()?;
    let relief = Relief::new(
        base,
        motifs,
        crate::normal_relief::Kind::Recessed,
        tolerance,
        minimum,
    )
    .map_err(|_| ReadError::Invalid)?;
    let frame = if end {
        frame
    } else {
        // A half-turn of this already checked finite chart preserves its
        // Gram matrix exactly; the validated stock height bounds translation.
        Frame {
            origin: frame.world([0., 0., imported.height_mm]),
            axes: [
                frame.axes[0],
                frame.axes[1].map(|v| -v),
                frame.axes[2].map(|v| -v),
            ],
        }
    };
    Ok((relief, frame))
}
/// Compare independently decoded relief against a complete expected world
/// boundary. Entity/face ordering is immaterial; ambiguous matches refuse.
pub fn normal_relief_import_matches(
    imported: &ImportSolid,
    expected: &Relief,
    frame: [Point; 3],
    tolerance: f64,
) -> bool {
    let result = (|| {
        let actual = Frame::new(imported.origin_mm, imported.u, imported.v)?;
        let expected_frame = Frame::new(frame[0], frame[1], frame[2])?;
        match &imported.profile {
            ImportProfile::NormalRelief { relief } => {
                matches(relief, actual, expected, expected_frame, tolerance)
            }
            ImportProfile::PocketedPolygon { .. } => {
                let (relief, actual) = pocketed(imported, actual, tolerance)?;
                matches(&relief, actual, expected, expected_frame, tolerance)
            }
            _ => Err(ReadError::Unsupported),
        }
    })();
    result.is_ok()
}
#[cfg(test)]
mod tests;
