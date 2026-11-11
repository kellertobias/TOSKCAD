//! Verify the exact two-arc tangent transition and cylindrical shank.
use super::*;

pub(super) struct Junction {
    pub(super) crest: u32,
    pub(super) complement: u32,
    pub(super) corners: [u32; 2],
}

pub(super) fn verify(
    proof: Proof,
    shell: &mut Shell<'_, '_>,
    [wall, seat]: [u32; 2],
    joint: Junction,
    thread: &Thread,
    length_mm: f64,
    [origin, u, v]: [Point; 3],
) -> Result<(u32, Point, f64)> {
    let axis = cross(u, v);
    let spec = thread.spec();
    let [_, start, _, _, crest, _] = thread.profile_breaks();
    let rate = thread.helical_rate();
    let radius = spec.major_radius_mm;
    let (sin, cos) = (-start / rate).sin_cos();
    let reference = std::array::from_fn(|i| cos * u[i] + sin * v[i]);
    let arc_axis = axis.map(|n| if spec.left_hand { n } else { -n });
    let [first, last] = joint.corners;
    arc(
        shell,
        joint.crest,
        [last, first],
        origin,
        arc_axis,
        (proof, radius, (start + spec.pitch_mm - crest) / rate.abs()),
    )?;
    arc(
        shell,
        joint.complement,
        [first, last],
        origin,
        arc_axis,
        (proof, radius, (crest - start) / rate.abs()),
    )?;

    let (surface, forward, bounds) = face(shell, seat)?;
    let [head_rim, shank_rim] = bounds[..] else {
        return Err(ReadError::Invalid);
    };
    require(forward)?;
    let center = std::array::from_fn(|i| origin[i] + axis[i] * -length_mm);
    let plane = proof.axis(shell, hex_head::plane(shell, surface)?, center, axis, u)?;
    let lower = socket_head::circle_rim(
        shell,
        shank_rim,
        false,
        false,
        plane.origin,
        axis,
        (proof, radius),
    )?;

    let (surface, forward, bounds) = face(shell, wall)?;
    let [bound] = bounds[..] else {
        return Err(ReadError::Invalid);
    };
    require(forward)?;
    let [Value::String(_), placement, Value::Number(measured_radius)] =
        shell.args(surface, "CYLINDRICAL_SURFACE")?
    else {
        return Err(ReadError::Invalid);
    };
    let measured_radius = *measured_radius;
    require(measured_radius == radius)?;
    let cylinder = proof.axis(shell, placement, plane.origin, axis, reference)?;
    let edges = ring(shell, bound, true)?;
    let [(bottom, true), (seam, true), top_a, top_b, (other, false)] = edges[..] else {
        return Err(ReadError::Invalid);
    };
    let top = if spec.left_hand {
        [(joint.crest, false), (joint.complement, false)]
    } else {
        [(joint.complement, true), (joint.crest, true)]
    };
    require(bottom == lower && seam == other && [top_a, top_b] == top)?;
    let seam_vertices = shell.edge(seam)?;
    require(
        matches!(shell.edges[&seam].curve, Curve::Line)
            && seam_vertices == [shell.edges[&lower].vertices[0], first],
    )?;
    let height = dot(sub(origin, plane.origin), cylinder.normal);
    let volume = std::f64::consts::PI * measured_radius * measured_radius * height;
    require(volume.is_finite() && volume > 0.)?;
    Ok((head_rim, plane.origin, volume))
}

pub(super) fn arc(
    shell: &mut Shell<'_, '_>,
    edge: u32,
    vertices: [u32; 2],
    center: Point,
    normal: Point,
    (proof, radius, sweep): (Proof, f64, f64),
) -> Result<()> {
    let [Value::String(_), from, to, curve, Value::Enum("T")] = shell.args(edge, "EDGE_CURVE")?
    else {
        return Err(ReadError::Invalid);
    };
    require([reference(from)?, reference(to)?] == vertices)?;
    let [Value::String(_), placement, Value::Number(measured)] =
        shell.args(reference(curve)?, "CIRCLE")?
    else {
        return Err(ReadError::Invalid);
    };
    require(*measured == radius)?;
    let axis = shell.placement(reference(placement)?)?;
    require(
        proof.equal(axis.origin, center) && length(sub(axis.normal, normal)) < proof.angular(),
    )?;
    let (sin, cos) = sweep.sin_cos();
    let expected = [
        std::array::from_fn(|i| center[i] + radius * axis.u[i]),
        std::array::from_fn(|i| center[i] + radius * (cos * axis.u[i] + sin * axis.v[i])),
    ];
    for (vertex, expected) in vertices.into_iter().zip(expected) {
        let actual = shell.vertex(vertex)?;
        require(
            proof
                .error
                .map_or(length(sub(actual, expected)) <= shell.tolerance, |_| {
                    proof.equal(actual, expected)
                }),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        headed_screw::{ScrewHead, ScrewSpec},
        helical_thread::Spec,
        step::write_helical_screw,
    };

    fn screw(left_hand: bool, shank_length_mm: f64, shank_radius_mm: f64) -> Screw {
        let thread = Thread::new(Spec {
            major_radius_mm: 3.,
            pitch_mm: 1.,
            length_mm: 4.2,
            depth_mm: 5. * 3_f64.sqrt() / 16.,
            crest_width_mm: 0.125,
            lower_flank_deg: 30.,
            upper_flank_deg: 30.,
            starts: 1,
            left_hand,
            root_radius_mm: None,
        })
        .unwrap();
        Screw::new(
            thread,
            ScrewSpec {
                head: ScrewHead::SocketCap {
                    radius_mm: 5.,
                    height_mm: 6.,
                    socket_across_flats_mm: 5.,
                    socket_depth_mm: 3.,
                },
                shank_length_mm,
                shank_radius_mm,
            },
        )
        .unwrap()
    }

    #[test]
    fn shank_is_measured_with_the_exact_crest_and_complement_arcs() {
        let [origin, u, v] = [[0.; 3], [1., 0., 0.], [0., 1., 0.]];
        let write = |body: &Screw| write_helical_screw(body, origin, u, v, 1e-7);
        for hand in [false, true] {
            let body = screw(hand, 2., 3.);
            let step = write(&body).unwrap();
            assert_eq!(step.matches("ADVANCED_FACE").count(), 17);
            assert_eq!(step.matches("CYLINDRICAL_SURFACE").count(), 2);
            let measured = verify_helical_screw(&step, &body, origin, u, v, 1e-7).unwrap();
            assert!((measured - body.volume_mm3()).abs() < 1e-6);
        }
        let clear = screw(false, 2., 3.01);
        assert_eq!(write(&clear), Err(StepError::Unsupported));
        let too_short = screw(false, 1e-8, 3.);
        assert_eq!(write(&too_short), Err(StepError::Limit));
    }
    #[test]
    fn rejects_malformed_shank_transition_and_head_seat() {
        let [origin, u, v] = [[0.; 3], [1., 0., 0.], [0., 1., 0.]];
        let body = screw(false, 2., 3.);
        let encoded = write_helical_screw(&body, origin, u, v, 1e-7).unwrap();
        let entities = parse_entities(&encoded, 1e-7).unwrap();
        let reader = Shell::new(&entities, 1e-7, true);
        let solid = reader.represented_solids(true).unwrap()[0];
        let faces = reader.member_faces(solid).unwrap();
        let wire = |bound| reader.bound(bound, false).unwrap().1;
        let uses = |wire| refs(&reader.args(wire, "EDGE_LOOP").unwrap()[1]).unwrap();

        let (_, _, shoulder_bounds) = face(&reader, faces[4]).unwrap();
        let complement_use = *uses(wire(shoulder_bounds[0])).last().unwrap();
        let complement = ring(&reader, shoulder_bounds[0], true)
            .unwrap()
            .last()
            .unwrap()
            .0;
        let (wall, _, wall_bounds) = face(&reader, faces[6]).unwrap();
        let wall_wire = wire(wall_bounds[0]);
        let wall_edges = ring(&reader, wall_bounds[0], true).unwrap();
        let seam = wall_edges[1].0;
        let crest = wall_edges[3].0;
        let edge_geometry = |id| {
            let args = reader.args(id, "EDGE_CURVE").unwrap();
            [1, 2, 3].map(|index| reference(&args[index]).unwrap())
        };
        let [crest_start, crest_end, crest_circle] = edge_geometry(crest);
        let [complement_start, complement_end, complement_circle] = edge_geometry(complement);
        let [seam_start, seam_end, seam_line] = edge_geometry(seam);
        let (seat, _, seat_bounds) = face(&reader, faces[7]).unwrap();

        let lower_edge = ring(&reader, seat_bounds[1], false).unwrap()[0].0;
        let lower_circle = edge_geometry(lower_edge)[2];
        let crest_axis = reference(&reader.args(crest_circle, "CIRCLE").unwrap()[1]).unwrap();
        let crest_placement = reader.args(crest_axis, "AXIS2_PLACEMENT_3D").unwrap();
        let normal = reference(&crest_placement[2]).unwrap();
        let reference_direction = reference(&crest_placement[3]).unwrap();
        for (id, record) in [
            (
                complement_use,
                format!("ORIENTED_EDGE('',*,*,#{complement},.T.);"),
            ),
            (
                crest,
                format!("EDGE_CURVE('',#{crest_start},#{crest_end},#{crest_circle},.F.);"),
            ),
            (
                complement,
                format!(
                    "EDGE_CURVE('',#{complement_start},#{complement_end},#{complement_circle},.F.);"
                ),
            ),
            (normal, "DIRECTION('',(0.,0.,1.));".into()),
            (reference_direction, "DIRECTION('',(0.,1.,0.));".into()),
            (lower_circle, "CIRCLE('');".into()),
            (crest_circle, "CIRCLE('');".into()),
            (faces[6], format!("ADVANCED_FACE('',(),#{wall},.T.);")),
            (wall, "CYLINDRICAL_SURFACE('');".into()),
            (wall_wire, "EDGE_LOOP('',());".into()),
            (
                seam,
                format!("EDGE_CURVE('',#{seam_end},#{seam_start},#{seam_line},.F.);"),
            ),
            (faces[7], format!("ADVANCED_FACE('',(),#{seat},.T.);")),
            (wire(seat_bounds[0]), "EDGE_LOOP('',());".into()),
        ] {
            let key = format!("#{id}=");
            let original = encoded.lines().find(|line| line.starts_with(&key)).unwrap();
            let changed = encoded.replacen(original, &format!("{key}{record}"), 1);
            assert_eq!(
                verify_helical_screw(&changed, &body, origin, u, v, 1e-7),
                Err(ReadError::Invalid),
                "altered record {id}: {record}"
            );
            assert!(crate::step_read::read_import_solid(&changed, 1e-7).is_err());
        }
    }
}
