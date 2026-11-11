//! Complete prefix, upward annular shoulder and terminal neck incidence proof.
use super::*;
pub(super) fn verify(
    proof: Proof,
    shell: &mut Shell<'_, '_>,
    faces: &[u32],
    thread: &Thread,
    [radius, full_length]: [f64; 2],
    rings: &[Vec<u32>; 2],
    [origin, u, v]: [Point; 3],
) -> Result<f64> {
    require(faces.len() == 4)?;
    let axis = cross(u, v);
    let world = |z| std::array::from_fn(|i| origin[i] + axis[i] * z);
    let plane = |shell: &Shell<'_, '_>, id, z, positive| -> Result<Vec<u32>> {
        let (surface, forward, bounds) = face(shell, id)?;
        require(forward)?;
        proof.axis(
            shell,
            hex_head::plane(shell, surface)?,
            world(z),
            axis.map(|n| if positive { n } else { -n }),
            u,
        )?;
        Ok(bounds)
    };
    let entry = plane(shell, faces[0], 0., false)?;
    let [entry_bound] = entry[..] else {
        return Err(ReadError::Invalid);
    };
    let lower: Vec<_> = rings[0].iter().map(|&e| (e, true)).collect();
    require(orient(&ring(shell, entry_bound, true)?, !thread.spec().left_hand) == lower)?;
    let shoulder = plane(shell, faces[1], thread.spec().length_mm, true)?;
    let [hole, outer] = shoulder[..] else {
        return Err(ReadError::Invalid);
    };
    let upper: Vec<_> = rings[1].iter().map(|&e| (e, true)).collect();
    require(orient(&ring(shell, outer, true)?, thread.spec().left_hand) == upper)?;
    let neck_start = socket_head::circle_rim(
        shell,
        hole,
        false,
        false,
        world(thread.spec().length_mm),
        axis,
        (proof, radius),
    )?;
    let far = plane(shell, faces[3], full_length, true)?;
    let [far_bound] = far[..] else {
        return Err(ReadError::Invalid);
    };
    let neck_end = socket_head::circle_rim(
        shell,
        far_bound,
        true,
        true,
        world(full_length),
        axis,
        (proof, radius),
    )?;
    check_cylinder_with_proof(
        proof,
        shell,
        faces[2],
        [neck_start, neck_end],
        (radius, true),
        [world(thread.spec().length_mm), axis, u],
    )?;
    Ok(std::f64::consts::PI * radius * radius * (full_length - thread.spec().length_mm))
}
