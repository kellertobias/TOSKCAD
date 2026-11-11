//! Full boundary proof for an externally threaded prefix and cylindrical tail.
use super::*;

pub(super) fn verify(
    proof: Proof,
    shell: &mut Shell<'_, '_>,
    faces: &[u32],
    thread: &Thread,
    [radius, stock_length]: [f64; 2],
    rings: &[Vec<u32>; 2],
    corners: &[[[u32; 2]; 2]],
    [origin, u, v]: [Point; 3],
) -> Result<f64> {
    let spec = thread.spec();
    let axis = cross(u, v);
    let world = |z| std::array::from_fn(|i| origin[i] + axis[i] * z);
    let tangent = radius == spec.major_radius_mm;
    let shoulders = if tangent { spec.starts as usize } else { 1 };
    require(faces.len() == shoulders + 3)?;
    let cap_plane = |shell: &Shell<'_, '_>, id, z, positive| -> Result<Vec<u32>> {
        let (surface, forward, bounds) = face(shell, id)?;
        require(forward)?;
        let normal = axis.map(|n| if positive { n } else { -n });
        proof.axis(shell, hex_head::plane(shell, surface)?, world(z), normal, u)?;
        Ok(bounds)
    };
    let entry = cap_plane(shell, faces[0], 0., false)?;
    require(entry.len() == 1)?;
    let expected: Vec<_> = rings[0].iter().map(|&e| (e, true)).collect();
    require(orient(&ring(shell, entry[0], true)?, !spec.left_hand) == expected)?;
    let far = cap_plane(shell, faces[shoulders + 2], stock_length, true)?;
    let [far_bound] = far[..] else {
        return Err(ReadError::Invalid);
    };
    let far_edge = socket_head::circle_rim(
        shell,
        far_bound,
        true,
        true,
        world(stock_length),
        axis,
        (proof, radius),
    )?;
    if !tangent {
        let shoulder = cap_plane(shell, faces[1], spec.length_mm, false)?;
        let [outer, inner] = shoulder[..] else {
            return Err(ReadError::Invalid);
        };
        let expected: Vec<_> = rings[1].iter().map(|&e| (e, true)).collect();
        require(orient(&ring(shell, inner, false)?, spec.left_hand) == expected)?;
        let low = socket_head::circle_rim(
            shell,
            outer,
            true,
            false,
            world(spec.length_mm),
            axis,
            (proof, radius),
        )?;
        check_cylinder_with_proof(
            proof,
            shell,
            faces[2],
            [low, far_edge],
            (radius, true),
            [world(spec.length_mm), axis, u],
        )?;
    } else {
        let [_, start, _, _, crest, _] = thread.profile_breaks();
        let rate = thread.helical_rate();
        let normal = axis.map(|n| if spec.left_hand { n } else { -n });
        let mut complements = Vec::with_capacity(shoulders);
        for k in 0..shoulders {
            let bounds = cap_plane(shell, faces[k + 1], spec.length_mm, false)?;
            let [bound] = bounds[..] else {
                return Err(ReadError::Invalid);
            };
            let actual = orient(&ring(shell, bound, true)?, spec.left_hand);
            let [(a, true), (b, true), (c, true), (complement, false)] = actual[..] else {
                return Err(ReadError::Invalid);
            };
            require([a, b, c] == rings[1][4 * k..4 * k + 3])?;
            let first = corners[4 * k][0][1];
            let last = corners[4 * k + 3][0][1];
            let next = corners[(4 * k + 4) % corners.len()][0][1];
            tangent_shank::arc(
                shell,
                rings[1][4 * k + 3],
                [last, next],
                world(spec.length_mm),
                normal,
                (proof, radius, (start + spec.pitch_mm - crest) / rate.abs()),
            )?;
            tangent_shank::arc(
                shell,
                complement,
                [first, last],
                world(spec.length_mm),
                normal,
                (proof, radius, (crest - start) / rate.abs()),
            )?;
            complements.push(complement);
        }
        let (surface, forward, bounds) = face(shell, faces[shoulders + 1])?;
        require(forward && bounds.len() == 1)?;
        let [Value::String(_), placement, Value::Number(measured)] =
            shell.args(surface, "CYLINDRICAL_SURFACE")?
        else {
            return Err(ReadError::Invalid);
        };
        require(*measured == radius)?;
        let (sin, cos) = ((spec.length_mm - start) / rate).sin_cos();
        let reference = std::array::from_fn(|i| cos * u[i] + sin * v[i]);
        proof.axis(shell, placement, world(spec.length_mm), axis, reference)?;
        let uses = ring(shell, bounds[0], true)?;
        require(uses.len() == 2 * shoulders + 3)?;
        let mut expected: Vec<_> = complements
            .iter()
            .enumerate()
            .flat_map(|(k, &e)| [(e, true), (rings[1][4 * k + 3], true)])
            .collect();
        expected = orient(&expected, spec.left_hand);
        require(uses[..2 * shoulders] == expected)?;
        let [(seam, true), (top, false), (other, false)] = uses[2 * shoulders..] else {
            return Err(ReadError::Invalid);
        };
        require(top == far_edge && seam == other)?;
        let endpoints = shell.edge(seam)?;
        require(
            matches!(shell.edges[&seam].curve, Curve::Line)
                && endpoints == [corners[0][0][1], shell.edges[&far_edge].vertices[0]],
        )?;
    }
    Ok(std::f64::consts::PI * radius * radius * (stock_length - spec.length_mm))
}
