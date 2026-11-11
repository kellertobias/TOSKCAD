use super::*;
fn curve(curve: ReliefCurve, frame: Frame) -> Curve {
    let ReliefCurve::Profile { edge, z_mm } = curve else {
        return Curve::Line;
    };
    let Some(radius) = edge.radius_mm() else {
        return Curve::Line;
    };
    let (center, clockwise) = match edge {
        ProfileEdge::Circle {
            center, clockwise, ..
        }
        | ProfileEdge::Arc {
            center, clockwise, ..
        } => (center, clockwise),
        ProfileEdge::Line { .. } => unreachable!("radius implies circle"),
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
/// Compare independently decoded relief against a complete expected world
/// boundary. Entity/face ordering is immaterial; ambiguous matches refuse.
pub fn normal_relief_import_matches(
    imported: &ImportSolid,
    expected: &Relief,
    frame: [Point; 3],
    tolerance: f64,
) -> bool {
    let ImportProfile::NormalRelief { relief } = &imported.profile else {
        return false;
    };
    let result = (|| {
        let actual = Frame::new(imported.origin_mm, imported.u, imported.v)?;
        let expected_frame = Frame::new(frame[0], frame[1], frame[2])?;
        matches(relief, actual, expected, expected_frame, tolerance)
    })();
    result.is_ok()
}
