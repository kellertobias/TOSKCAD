//! STEP proof of a cylindrical screw head and its exact hex socket.
use super::*;
use crate::{headed_screw::hex_vertices, step::helical::SocketHead};

pub(super) fn verify(
    shell: &mut Shell<'_, '_>,
    faces: &[u32],
    shoulder: u32,
    shoulder_center: Point,
    [reference, axis]: [Point; 2],
    head: SocketHead,
    world: impl Fn(Point2, f64) -> Point,
) -> Result<f64> {
    let lower = -head.shank_length_mm - head.height_mm;
    let upper = lower + head.depth_mm;
    let down = axis.map(|n| -n);
    let (end, forward, bounds) = face(shell, faces[0])?;
    let [outer, mouth] = bounds[..] else {
        return Err(ReadError::Invalid);
    };
    require(forward)?;
    let bottom = check_axis(
        shell,
        hex_head::plane(shell, end)?,
        world([0.; 2], lower),
        down,
        reference,
    )?;
    let lower_rim = circle_rim(
        shell,
        outer,
        true,
        false,
        bottom.origin,
        axis,
        head.radius_mm,
    )?;
    let upper_rim = circle_rim(
        shell,
        shoulder,
        true,
        true,
        shoulder_center,
        axis,
        head.radius_mm,
    )?;
    let (radius, cylinder_axis) = check_cylinder(
        shell,
        faces[1],
        [lower_rim, upper_rim],
        (head.radius_mm, true),
        [bottom.origin, axis, reference],
    )?;

    let (floor, forward, bounds) = face(shell, faces[2])?;
    let [socket_floor] = bounds[..] else {
        return Err(ReadError::Invalid);
    };
    require(forward)?;
    let floor = check_axis(
        shell,
        hex_head::plane(shell, floor)?,
        world([0.; 2], upper),
        down,
        reference,
    )?;
    let hex = hex_vertices(head.across_flats_mm);
    let lower_rim = hex_head::hex_rim(
        Proof {
            error: None,
            extent: 1.,
        },
        shell,
        mouth,
        false,
        true,
        hex.map(|p| world(p, lower)),
    )?;
    let upper_rim = hex_head::hex_rim(
        Proof {
            error: None,
            extent: 1.,
        },
        shell,
        socket_floor,
        true,
        false,
        hex.map(|p| world(p, upper)),
    )?;
    let area = hex_head::walls_area(
        Proof {
            error: None,
            extent: 1.,
        },
        shell,
        &faces[3..],
        [&lower_rim, &upper_rim],
        bottom.origin,
        axis,
        true,
    )?;
    let height = dot(sub(shoulder_center, bottom.origin), cylinder_axis);
    let depth = dot(sub(floor.origin, bottom.origin), cylinder_axis);
    let volume = std::f64::consts::PI * radius * radius * height - area * depth;
    require(volume.is_finite() && volume > 0.)?;
    Ok(volume)
}

pub(super) fn circle_rim(
    shell: &mut Shell<'_, '_>,
    bound: u32,
    exterior: bool,
    forward: bool,
    center: Point,
    normal: Point,
    radius: f64,
) -> Result<u32> {
    let edges = ring(shell, bound, exterior)?;
    let [(edge, sense)] = edges[..] else {
        return Err(ReadError::Invalid);
    };
    require(sense == forward)?;
    shell.edge(edge)?;
    let Curve::Circle(circle) = shell.edges[&edge].curve else {
        return Err(ReadError::Invalid);
    };
    require(
        circle.center_mm == center
            && length(sub(circle.normal, normal)) < 1e-10
            && circle.radius_mm == radius,
    )?;
    Ok(edge)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        headed_screw::{ScrewHead, ScrewSpec},
        helical_thread::Spec,
        step::write_helical_screw,
    };

    fn screw(head: ScrewHead, left_hand: bool) -> Screw {
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
        Screw::new(
            thread,
            ScrewSpec {
                head,
                shank_length_mm: 0.,
                shank_radius_mm: 3.,
            },
        )
        .unwrap()
    }

    const HEAD: ScrewHead = ScrewHead::SocketCap {
        radius_mm: 5.,
        height_mm: 6.,
        socket_across_flats_mm: 5.,
        socket_depth_mm: 3.,
    };

    #[test]
    fn cylindrical_head_and_hex_socket_are_both_measured() {
        let [origin, u, v] = [[0.; 3], [1., 0., 0.], [0., 1., 0.]];
        let write = |screw: &Screw| write_helical_screw(screw, origin, u, v, 1e-7);
        for hand in [false, true] {
            let screw = screw(HEAD, hand);
            let step = write(&screw).unwrap();
            let measured = verify_helical_screw(&step, &screw, origin, u, v, 1e-7).unwrap();
            assert!((measured - screw.volume_mm3()).abs() < 1e-6);
            assert_eq!(step.matches("ADVANCED_FACE").count(), 15);
        }
        let thin = screw(
            ScrewHead::SocketCap {
                radius_mm: 5.,
                height_mm: 6.,
                socket_across_flats_mm: 5.,
                socket_depth_mm: 6. - 1e-8,
            },
            false,
        );
        assert_eq!(write(&thin), Err(StepError::Limit));
    }

    #[test]
    fn refuses_a_missing_socket_wire_or_altered_circular_rim() {
        let [origin, u, v] = [[0.; 3], [1., 0., 0.], [0., 1., 0.]];
        let screw = screw(HEAD, true);
        let encoded = write_helical_screw(&screw, origin, u, v, 1e-7).unwrap();
        let data = parse_entities(&encoded, 1e-7).unwrap();
        let reader = Shell::new(&data, 1e-7, true);
        let root = reader.represented_solids(true).unwrap()[0];
        let faces = reader.member_faces(root).unwrap();
        let (end, _, bounds) = face(&reader, faces[6]).unwrap();
        let (floor, _, floor_bounds) = face(&reader, faces[8]).unwrap();
        let (_, floor_wire, _) = reader.bound(floor_bounds[0], false).unwrap();
        let (side, _, _) = face(&reader, faces[9]).unwrap();
        let invalid_plane = |surface| {
            let axis = reference(hex_head::plane(&reader, surface).unwrap()).unwrap();
            let placement = reader.args(axis, "AXIS2_PLACEMENT_3D").unwrap();
            let [p, n, u] = [1, 2, 3].map(|i| reference(&placement[i]).unwrap());
            (axis, format!("AXIS2_PLACEMENT_3D('',#{p},#{u},#{n});"))
        };
        let (_, outer_wire, _) = reader.bound(bounds[0], false).unwrap();
        let lower = ring(&reader, bounds[0], true).unwrap()[0].0;
        let (wall_surface, _, wall_bounds) = face(&reader, faces[7]).unwrap();
        let wall_axis =
            reference(&reader.args(wall_surface, "CYLINDRICAL_SURFACE").unwrap()[1]).unwrap();
        let seam = ring(&reader, wall_bounds[0], true).unwrap()[1].0;
        let edge_uses = reader.args(outer_wire, "EDGE_LOOP").unwrap();
        let oriented = refs(&edge_uses[1]).unwrap()[0];
        let circle = reference(&reader.args(lower, "EDGE_CURVE").unwrap()[3]).unwrap();
        let axis = reference(&reader.args(circle, "CIRCLE").unwrap()[1]).unwrap();
        let direction = reference(&reader.args(axis, "AXIS2_PLACEMENT_3D").unwrap()[2]).unwrap();
        let mouth_edge = ring(&reader, bounds[1], false).unwrap()[0].0;
        let mouth_vertex = reference(&reader.args(mouth_edge, "EDGE_CURVE").unwrap()[1]).unwrap();
        let mouth_point =
            reference(&reader.args(mouth_vertex, "VERTEX_POINT").unwrap()[1]).unwrap();
        for (id, record) in [
            (mouth_point, "CARTESIAN_POINT('',(123.,0.,0.));".into()),
            (faces[6], format!("ADVANCED_FACE('',(),#{end},.T.);")),
            (faces[8], format!("ADVANCED_FACE('',(),#{floor},.T.);")),
            (faces[9], format!("ADVANCED_FACE('',(),#{side},.T.);")),
            (outer_wire, "EDGE_LOOP('',());".into()),
            (floor_wire, "EDGE_LOOP('',());".into()),
            (
                wall_surface,
                format!("CYLINDRICAL_SURFACE('',#{wall_axis},4.);"),
            ),
            invalid_plane(end),
            invalid_plane(floor),
            (oriented, format!("ORIENTED_EDGE('',*,*,#{seam},.F.);")),
            (direction, "DIRECTION('',(0.,0.,-1.));".into()),
        ] {
            let key = format!("#{id}=");
            let original = encoded.lines().find(|line| line.starts_with(&key)).unwrap();
            let changed = encoded.replacen(original, &format!("{key}{record}"), 1);
            assert!(verify_helical_screw(&changed, &screw, origin, u, v, 1e-7).is_err());
        }
    }
}
