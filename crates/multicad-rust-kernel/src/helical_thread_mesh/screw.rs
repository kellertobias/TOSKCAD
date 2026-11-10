use super::{Builder, check_options, phase_strips, volume_error_bound};
use crate::{
    display::{DisplayMesh, MeshError, Tessellation},
    headed_screw::{Screw, ScrewFace, ScrewHead},
};
use std::f64::consts::{PI, TAU};

/// Mesh one connected screw shell using the same trimmed shaft rim at the
/// attachment. Angular columns include the exact corners of the head or socket.
pub fn tessellate_screw(screw: &Screw, options: Tessellation) -> Result<DisplayMesh, MeshError> {
    check_options(options)?;
    let thread = screw.thread();
    let spec = screw.spec();
    let radius = match spec.head {
        ScrewHead::SocketCap { radius_mm, .. } => radius_mm,
        ScrewHead::ExternalHex {
            across_flats_mm, ..
        } => across_flats_mm / 3_f64.sqrt(),
    };
    let max_angle = (options.angle * 0.5).min((2. * options.deflection / radius).sqrt());
    let angular_steps = (TAU / (12. * max_angle)).ceil() * 12.;
    if angular_steps * 2. > options.triangle_limit as f64 {
        return Err(MeshError::BudgetExceeded);
    }
    let steps = angular_steps as usize;
    let strips = phase_strips(thread, TAU / angular_steps, options)?;
    let mut builder = Builder::new(thread, steps, strips, options);
    builder.build_wall()?;
    let [mut shoulder, tip] = std::mem::take(&mut builder.caps);
    builder.add_cap(1, tip)?;
    Builder::sort_ring(&mut shoulder)?;
    let faces = screw.faces();
    let face = |kind| {
        faces
            .iter()
            .position(|&candidate| candidate == kind)
            .expect("screw face role") as u32
    };
    let [seat_z, top_z] = [screw.head_seat_z(), screw.head_top_z()];
    let mut shell = HeadMesh {
        builder: &mut builder,
        angles: shoulder.iter().map(|&(angle, _)| angle).collect(),
    };
    let shaft_rim: Vec<_> = shoulder.into_iter().map(|(_, id)| id).collect();
    let head_seat = shell.ring(seat_z, |angle| screw.head_radius(angle));
    if spec.shank_length_mm > 0. {
        let shank_top = shell.shank_join(&shaft_rim, spec.shank_radius_mm);
        let shank_bottom = shell.ring(seat_z, |_| spec.shank_radius_mm);
        shell.annulus(
            &shaft_rim,
            &shank_top,
            face(ScrewFace::ThreadShoulder),
            false,
        )?;
        let shank_wall = face(ScrewFace::ShankWall);
        shell.wall(&shank_bottom, &shank_top, |_| shank_wall, false)?;
        shell.annulus(&shank_bottom, &head_seat, face(ScrewFace::HeadSeat), false)?;
    } else {
        shell.annulus(
            &shaft_rim,
            &head_seat,
            face(ScrewFace::ThreadShoulder),
            false,
        )?;
    }
    let head_top = shell.ring(top_z, |angle| screw.head_radius(angle));
    let first_wall = face(ScrewFace::HeadWall(0));
    let head_side = match spec.head {
        ScrewHead::SocketCap { .. } => |_| 0,
        ScrewHead::ExternalHex { .. } => hex_side,
    };
    shell.wall(
        &head_top,
        &head_seat,
        |angle| first_wall + head_side(angle),
        false,
    )?;
    if let ScrewHead::SocketCap {
        socket_depth_mm, ..
    } = spec.head
    {
        let floor_z = top_z + socket_depth_mm;
        let socket_radius = |angle| screw.socket_radius(angle).expect("socket head");
        let mouth = shell.ring(top_z, socket_radius);
        let floor = shell.ring(floor_z, socket_radius);
        shell.annulus(&mouth, &head_top, face(ScrewFace::HeadTop), true)?;
        let first_wall = face(ScrewFace::SocketWall(0));
        shell.wall(&mouth, &floor, |angle| first_wall + hex_side(angle), true)?;
        shell.disk(&floor, floor_z, face(ScrewFace::SocketFloor))?;
    } else {
        shell.disk(&head_top, top_z, face(ScrewFace::HeadTop))?;
    }
    Ok(builder.mesh)
}

/// Combine the helical interpolation bound with the area lost by approximating
/// circular walls. The hex faces and recess are planar and use exact corners.
pub fn screw_volume_error_bound(screw: &Screw, options: Tessellation) -> f64 {
    let spec = screw.spec();
    let head_wall = match spec.head {
        ScrewHead::SocketCap {
            radius_mm,
            height_mm,
            ..
        } => radius_mm * height_mm,
        ScrewHead::ExternalHex { .. } => 0.,
    };
    volume_error_bound(screw.thread(), options)
        + TAU * (head_wall + spec.shank_radius_mm * spec.shank_length_mm) * options.deflection
}

fn hex_side(angle: f64) -> u32 {
    ((angle - PI / 6.).rem_euclid(TAU) / (TAU / 6.)).floor() as u32
}

struct HeadMesh<'a, 'thread> {
    builder: &'a mut Builder<'thread>,
    angles: Vec<f64>,
}

impl HeadMesh<'_, '_> {
    fn push(&mut self, point: [f64; 3]) -> u32 {
        let id = (self.builder.mesh.vertices.len() / 3) as u32;
        self.builder.mesh.vertices.extend(point);
        id
    }

    fn ring(&mut self, z: f64, radius: impl Fn(f64) -> f64) -> Vec<u32> {
        (0..self.angles.len())
            .map(|index| {
                let angle = self.angles[index];
                let (sin, cos) = angle.sin_cos();
                let radius = radius(angle);
                self.push([radius * cos, radius * sin, z])
            })
            .collect()
    }

    fn shank_join(&mut self, inner: &[u32], radius: f64) -> Vec<u32> {
        inner
            .iter()
            .enumerate()
            .map(|(index, &id)| {
                let [x, y, _] = self.builder.mesh.vertices.as_chunks::<3>().0[id as usize];
                if (radius - x.hypot(y)).abs() <= 64. * f64::EPSILON * radius {
                    // A full crest may coincide with a nominal-diameter shank.
                    // Give those stations one vertex identity on both walls.
                    id
                } else {
                    let (sin, cos) = self.angles[index].sin_cos();
                    self.push([radius * cos, radius * sin, 0.])
                }
            })
            .collect()
    }

    fn annulus(
        &mut self,
        inner: &[u32],
        outer: &[u32],
        face: u32,
        reverse: bool,
    ) -> Result<(), MeshError> {
        for i in 0..inner.len() {
            let next = (i + 1) % inner.len();
            let [a, b, c, d] = [inner[i], inner[next], outer[i], outer[next]];
            if a != c {
                self.triangle([a, c, d], face, reverse)?;
            }
            if b != d {
                self.triangle([a, d, b], face, reverse)?;
            }
        }
        Ok(())
    }

    fn wall(
        &mut self,
        low: &[u32],
        high: &[u32],
        face: impl Fn(f64) -> u32,
        reverse: bool,
    ) -> Result<(), MeshError> {
        for i in 0..low.len() {
            let next = (i + 1) % low.len();
            let lo = self.angles[i];
            let hi = if next == 0 { TAU } else { self.angles[next] };
            let face = face((lo + hi) * 0.5);
            let [a, b, c, d] = [low[i], low[next], high[i], high[next]];
            self.triangle([a, b, d], face, reverse)?;
            self.triangle([a, d, c], face, reverse)?;
        }
        Ok(())
    }

    fn disk(&mut self, rim: &[u32], z: f64, face: u32) -> Result<(), MeshError> {
        let center = self.push([0., 0., z]);
        for (&a, &b) in rim.iter().zip(rim.iter().cycle().skip(1)) {
            self.builder.triangle([center, b, a], face)?;
        }
        Ok(())
    }

    fn triangle(
        &mut self,
        mut triangle: [u32; 3],
        face: u32,
        reverse: bool,
    ) -> Result<(), MeshError> {
        if reverse {
            triangle.swap(1, 2);
        }
        self.builder.triangle(triangle, face)
    }
}
