//! ZA worm: the native single-start right-hand trapezoidal thread with an
//! optional through bore, centered on its threaded length.
use super::*;
use crate::{
    display::{DisplayMesh, MeshError, Tessellation, triangulate_region},
    helical_thread::{Spec, Thread},
    helical_thread_mesh,
};
use spade::Point2;
use std::f64::consts::{PI, TAU};

/// Axial pressure slope of the q10, 20-degree normal ZA recipe.
pub(crate) fn slope() -> f64 {
    20_f64.to_radians().tan() * 1.01_f64.sqrt()
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WormSpec {
    pub axial_module_mm: f64,
    pub threaded_length_mm: f64,
    pub bore_radius_mm: f64,
    pub axial_thinning_mm: f64,
}

/// Native thread face order: end caps, then the profile bands.
pub const ROLES: [&str; 7] = [
    "worm/end.start",
    "worm/end.end",
    "worm/flank.lower",
    "worm/crest",
    "worm/flank.upper",
    "worm/root",
    "worm/bore",
];

/// Rails in counterclockwise order on a cap and the band face that follows.
const RAIL_FACES: [[u32; 2]; 4] = [[2, 3], [5, 2], [4, 5], [3, 4]];
const RIM_FACES: [u32; 4] = [2, 5, 4, 3];

#[derive(Clone, Debug, PartialEq)]
pub struct Worm {
    spec: WormSpec,
    thread: Thread,
    phase: f64,
}

impl Worm {
    pub fn new(spec: WormSpec, minimum_mm: f64) -> Result<Self, Error> {
        let WormSpec {
            axial_module_mm: m,
            threaded_length_mm: length,
            bore_radius_mm: bore,
            axial_thinning_mm: thinning,
        } = spec;
        if ![m, length, bore, thinning, minimum_mm]
            .iter()
            .all(|v| v.is_finite())
            || !(0.5..=2.).contains(&m)
            || !(0. ..=0.1 * m).contains(&thinning)
            || bore < 0.
            || minimum_mm <= 0.
        {
            return Err(Error::InvalidDimensions);
        }
        if 3.75 * m - bore < minimum_mm {
            return Err(Error::InvalidProfile);
        }
        if !(16. * m..=32. * m).contains(&length) {
            return Err(Error::Limit);
        }
        let angle = slope().atan().to_degrees();
        let thread = Thread::new(Spec {
            major_radius_mm: 6. * m,
            pitch_mm: PI * m,
            length_mm: length,
            depth_mm: 2.25 * m,
            crest_width_mm: PI * m / 2. - thinning - 2. * m * slope(),
            lower_flank_deg: angle,
            upper_flank_deg: angle,
            starts: 1,
            left_hand: false,
            root_radius_mm: None,
        })
        .map_err(|_| Error::InvalidProfile)?;
        Ok(Self {
            spec,
            thread,
            phase: PI - length / m,
        })
    }

    pub fn spec(&self) -> WormSpec {
        self.spec
    }

    pub fn thread(&self) -> &Thread {
        &self.thread
    }

    /// Rotation of the thread frame about +Z before centering.
    pub fn phase_rad(&self) -> f64 {
        self.phase
    }

    pub fn bore_radius_mm(&self) -> Option<f64> {
        (self.spec.bore_radius_mm > 0.).then_some(self.spec.bore_radius_mm)
    }

    pub fn length_mm(&self) -> f64 {
        self.spec.threaded_length_mm
    }

    /// Local origin of the thread frame.
    pub fn thread_origin(&self) -> Point {
        [0., 0., -self.length_mm() / 2.]
    }

    pub(crate) fn to_thread(&self, local: Point) -> Point {
        let p = rotate_z(local, -self.phase);
        [p[0], p[1], p[2] + self.length_mm() / 2.]
    }

    pub(crate) fn to_local(&self, thread: Point) -> Point {
        let p = rotate_z(thread, self.phase);
        [p[0], p[1], p[2] - self.length_mm() / 2.]
    }

    /// Thread point at profile phase `s` and thread-frame height `z`.
    pub(crate) fn wall_point(&self, s: f64, z: f64) -> Point {
        let theta = (z - s) / self.thread.helical_rate();
        let r = self.thread.radius_at_phase(s);
        let (sin, cos) = theta.sin_cos();
        [r * cos, r * sin, z]
    }

    pub fn volume_mm3(&self) -> f64 {
        let bore = self.bore_radius_mm().unwrap_or(0.);
        self.thread.volume_mm3() - PI * bore * bore * self.length_mm()
    }

    /// Rails in counterclockwise order: lower|crest, root|lower, upper|root
    /// and crest|upper. Phases decrease counterclockwise at fixed height.
    pub(crate) fn rail_phases(&self) -> [f64; 5] {
        let [_, crest_end, upper_end, lower_end, crest_start, pitch] = self.thread.profile_breaks();
        [
            crest_start,
            lower_end,
            upper_end,
            crest_end,
            crest_start - pitch,
        ]
    }

    pub fn faces(&self) -> Vec<Face> {
        let thread = &self.thread;
        let spec = thread.spec();
        let height = spec.length_mm;
        let rate = thread.helical_rate();
        let breaks = thread.profile_breaks();
        let mut walls = [[0.; 3]; 4];
        let mut moment = [0.; 2];
        for (band, pair) in breaks.windows(2).enumerate() {
            let value = |i: usize| {
                move |phase: f64| {
                    let [r, slope] = thread.radius_and_slope_at_phase(phase);
                    let area = r.hypot(slope * r.hypot(rate));
                    let (sin, cos) = (-phase / rate).sin_cos();
                    [
                        area,
                        area * r * cos,
                        area * r * sin,
                        r.powi(3) * cos,
                        r.powi(3) * sin,
                    ][i]
                }
            };
            let slot = &mut walls[thread.face_index(0, band) as usize - 2];
            for (i, total) in slot.iter_mut().enumerate() {
                *total += crate::bevel_gear::integrate(value(i), pair[0], pair[1], 4);
            }
            for (i, total) in moment.iter_mut().enumerate() {
                *total += crate::bevel_gear::integrate(value(3 + i), pair[0], pair[1], 4);
            }
        }
        let bore = self.bore_radius_mm().unwrap_or(0.);
        let cap_area = thread.volume_mm3() / height - PI * bore * bore;
        let half_twist = height / (2. * rate);
        let spin = half_twist.sin() / half_twist;
        let (sin, cos) = half_twist.sin_cos();
        let mut faces = Vec::with_capacity(7);
        for (cap, role) in ROLES.iter().enumerate().take(2) {
            let z = cap as f64 * height;
            let [x, y] = moment.map(|m| m / (3. * rate.abs() * cap_area));
            let center = rotate_z([x, y, z], z / rate);
            let normal = [0., 0., if cap == 0 { -1. } else { 1. }];
            faces.push(Face {
                role: (*role).into(),
                area_mm2: cap_area,
                center_mm: self.to_local(center),
                plane: Some((self.to_local([0., 0., z]), normal)),
            });
        }
        for (index, [area, x, y]) in walls.into_iter().enumerate() {
            let center = [
                (x * cos - y * sin) * spin / area,
                (x * sin + y * cos) * spin / area,
                height / 2.,
            ];
            faces.push(Face {
                role: ROLES[2 + index].into(),
                area_mm2: area * height / rate.abs(),
                center_mm: self.to_local(center),
                plane: None,
            });
        }
        if bore > 0. {
            faces.push(Face {
                role: ROLES[6].into(),
                area_mm2: TAU * bore * height,
                center_mm: [0.; 3],
                plane: None,
            });
        }
        faces
    }

    /// Signed radial and axial material margin in the thread frame.
    pub(crate) fn margin(&self, local: Point) -> f64 {
        let [x, y, z] = self.to_thread(local);
        let length = self.length_mm();
        let radius = x.hypot(y);
        let mut margin = z
            .min(length - z)
            .min(self.thread.radius(y.atan2(x), z.clamp(0., length)) - radius);
        if let Some(bore) = self.bore_radius_mm() {
            margin = margin.min(radius - bore);
        }
        margin
    }

    pub fn classify(&self, local: Point, tolerance_mm: f64) -> Containment {
        let margin = self.margin(local);
        if margin > tolerance_mm {
            Containment::Inside
        } else if margin < -tolerance_mm {
            Containment::Outside
        } else {
            Containment::Boundary
        }
    }

    /// Maximum of `direction · p` over the local solid. The bore lies
    /// strictly inside the thread envelope and never carries the maximum.
    pub fn support(&self, direction: Point) -> f64 {
        let thread = rotate_z(direction, -self.phase);
        self.thread.support(thread) + dot(direction, self.thread_origin())
    }

    pub fn mesh(&self, options: Tessellation) -> Result<DisplayMesh, MeshError> {
        options.check()?;
        let mut mesh = match self.bore_radius_mm() {
            None => helical_thread_mesh::tessellate(&self.thread, options)?,
            Some(bore) => self.bored_mesh(bore, options)?,
        };
        for vertex in mesh.vertices.as_chunks_mut::<3>().0 {
            *vertex = self.to_local(*vertex);
        }
        Ok(mesh)
    }

    fn bored_mesh(&self, bore: f64, options: Tessellation) -> Result<DisplayMesh, MeshError> {
        let (mut mesh, rings) = helical_thread_mesh::open_wall(&self.thread, options)?;
        let count = options.arc_segments(bore, TAU)?.max(3);
        let first = mesh.vertices.len() / 3;
        for cap in 0..2 {
            let z = cap as f64 * self.length_mm();
            for i in 0..count {
                let (sin, cos) = (TAU * i as f64 / count as f64).sin_cos();
                mesh.vertices.extend([bore * cos, bore * sin, z]);
            }
        }
        let hole = |cap: usize, i: usize| (first + cap * count + i % count) as u32;
        let position = |mesh: &DisplayMesh, v: u32| {
            let i = v as usize * 3;
            [mesh.vertices[i], mesh.vertices[i + 1]]
        };
        for (cap, ring) in rings.iter().enumerate() {
            // Outer rim counterclockwise and the bore clockwise keep the
            // material on the left of every constraint.
            let ids: Vec<u32> = ring
                .iter()
                .copied()
                .chain((0..count).rev().map(|i| hole(cap, i)))
                .collect();
            let points = ids
                .iter()
                .map(|&v| {
                    let [x, y] = position(&mesh, v);
                    Point2::new(x, y)
                })
                .collect();
            let n = ring.len();
            let edges: Vec<[usize; 2]> = (0..n)
                .map(|i| [i, (i + 1) % n])
                .chain((0..count).map(|i| [n + i, n + (i + 1) % count]))
                .collect();
            let region = triangulate_region(points, &edges, 1)?;
            for [a, b, c] in region.triangles {
                let [pa, pb, pc] = [a, b, c].map(|i| position(&mesh, ids[i]));
                let area = (pb[0] - pa[0]) * (pc[1] - pa[1]) - (pb[1] - pa[1]) * (pc[0] - pa[0]);
                let upward = area > 0.;
                let order = if upward == (cap == 1) {
                    [a, b, c]
                } else {
                    [a, c, b]
                };
                mesh.triangle(order.map(|i| ids[i] as usize), cap as u32);
            }
        }
        for i in 0..count {
            let [a, b, c, d] = [hole(0, i), hole(0, i + 1), hole(1, i + 1), hole(1, i)];
            mesh.triangle([a, c, b].map(|v| v as usize), 6);
            mesh.triangle([a, d, c].map(|v| v as usize), 6);
        }
        (mesh.face_indices.len() <= options.triangle_limit)
            .then_some(mesh)
            .ok_or(MeshError::BudgetExceeded)
    }

    /// Local corner where `rail` meets cap `cap`.
    fn corner(&self, cap: usize, rail: usize) -> Point {
        let z = cap as f64 * self.length_mm();
        self.to_local(self.wall_point(self.rail_phases()[rail], z))
    }

    pub fn topology(&self) -> Topology {
        let faces = self.faces();
        let mut vertices_mm = vec![];
        for cap in 0..2 {
            for rail in 0..4 {
                vertices_mm.push(self.corner(cap, rail));
            }
        }
        let mut edges = vec![];
        for (rail, faces) in RAIL_FACES.into_iter().enumerate() {
            edges.push(Edge {
                faces,
                vertices: [rail as u32, 4 + rail as u32],
                shape: Shape::Sampled(Curve::WormRail { rail }),
            });
        }
        let rate = self.thread.helical_rate();
        let phases = self.rail_phases();
        for cap in 0..2 {
            let z = cap as f64 * self.length_mm();
            for rail in 0..4 {
                let face = RIM_FACES[rail];
                let shape = if face == 3 || face == 5 {
                    let start = (z - phases[rail]) / rate;
                    Shape::Arc {
                        z_mm: z - self.length_mm() / 2.,
                        radius_mm: self
                            .thread
                            .radius_at_phase(phases[rail + 1] / 2. + phases[rail] / 2.),
                        start_rad: start + self.phase,
                        sweep_rad: (phases[rail] - phases[rail + 1]) / rate,
                    }
                } else {
                    Shape::Sampled(Curve::WormRim { cap, rail })
                };
                edges.push(Edge {
                    faces: [cap as u32, face],
                    vertices: [(4 * cap + rail) as u32, (4 * cap + (rail + 1) % 4) as u32],
                    shape,
                });
            }
        }
        if let Some(bore) = self.bore_radius_mm() {
            for cap in 0..2 {
                let z = cap as f64 * self.length_mm() - self.length_mm() / 2.;
                let vertex = vertices_mm.len() as u32;
                vertices_mm.push(rotate_z([bore, 0., z], self.phase));
                edges.push(Edge {
                    faces: [cap as u32, 6],
                    vertices: [vertex; 2],
                    shape: Shape::Arc {
                        z_mm: z,
                        radius_mm: bore,
                        start_rad: self.phase,
                        sweep_rad: TAU,
                    },
                });
            }
        }
        Topology {
            faces,
            edges,
            vertices_mm,
        }
    }

    /// Local point at parameter `t` in `[0, 1]` along a sampled worm curve.
    pub fn curve_point(&self, curve: Curve, t: f64) -> Option<Point> {
        let phases = self.rail_phases();
        let thread = match curve {
            Curve::WormRail { rail } if rail < 4 => {
                self.wall_point(phases[rail], t * self.length_mm())
            }
            Curve::WormRim { cap, rail } if cap < 2 && rail < 4 => {
                let phase = phases[rail] + (phases[rail + 1] - phases[rail]) * t;
                self.wall_point(phase, cap as f64 * self.length_mm())
            }
            _ => return None,
        };
        Some(self.to_local(thread))
    }

    pub fn curve_segments(&self, curve: Curve, options: Tessellation) -> Result<usize, MeshError> {
        options.check()?;
        self.curve_point(curve, 0.)
            .ok_or(MeshError::UnresolvedBoundary)?;
        chord_count(
            |t| self.curve_point(curve, t).expect("checked worm curve"),
            options.deflection,
            options.angle,
            options.triangle_limit,
        )
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::manufacturing;

    pub(crate) fn spec(m: f64, length: f64, bore: f64, thinning: f64) -> WormSpec {
        WormSpec {
            axial_module_mm: m,
            threaded_length_mm: length * m,
            bore_radius_mm: bore,
            axial_thinning_mm: thinning,
        }
    }

    fn options(deflection: f64) -> Tessellation {
        Tessellation {
            deflection,
            angle: 0.5,
            triangle_limit: 2_000_000,
        }
    }

    #[test]
    fn worm_volume_faces_and_mesh_agree_with_the_thread() {
        for (m, length, bore) in [(1., 20., 0.), (2., 16., 3.), (0.5, 32., 0.8)] {
            let worm = Worm::new(spec(m, length, bore, 0.02 * m), 1e-3).unwrap();
            let faces = worm.faces();
            assert_eq!(faces.len(), if bore > 0. { 7 } else { 6 });
            let mesh = worm.mesh(options(0.002 * m)).unwrap();
            let proof = manufacturing::certify(&mesh).unwrap();
            let volume = worm.volume_mm3();
            assert!(
                (proof.signed_volume_mm3 - volume).abs() / volume < 2e-3,
                "{} {volume}",
                proof.signed_volume_mm3
            );
            let used: std::collections::BTreeSet<_> = mesh.face_indices.iter().copied().collect();
            assert_eq!(used.len(), faces.len());
            // The section area is constant: caps share V/L less the bore.
            let cap = volume / worm.length_mm();
            assert!((faces[0].area_mm2 - cap).abs() < 1e-9 * cap);
            assert!((faces[2].area_mm2 - faces[4].area_mm2).abs() < 1e-9 * faces[2].area_mm2);
            for face in &faces {
                assert!(face.area_mm2 > 0. && face.center_mm.iter().all(|v| v.is_finite()));
            }
            let topology = worm.topology();
            assert_eq!(topology.faces, faces);
            assert_eq!(topology.edges.len(), 12 + if bore > 0. { 2 } else { 0 });
            for edge in &topology.edges {
                let [a, b] = edge.vertices.map(|v| topology.vertices_mm[v as usize]);
                if let Shape::Sampled(curve) = edge.shape {
                    assert!(worm.curve_segments(curve, options(0.01)).unwrap() >= 1);
                }
                let ends = crate::worm_gear::tests::edge_ends(edge.shape, |c, t| {
                    worm.curve_point(c, t).unwrap()
                });
                assert!(norm(sub(ends[0], a)) < 1e-9 && norm(sub(ends[1], b)) < 1e-9);
            }
        }
    }

    #[test]
    fn worm_classification_support_and_placement_follow_the_native_recipe() {
        let worm = Worm::new(spec(1., 20., 2., 0.), 1e-3).unwrap();
        let rate = worm.thread().helical_rate();
        // The phase-zero crest faces -X at Z=0.
        assert_eq!(worm.classify([-5.9, 0., 0.], 1e-6), Containment::Inside);
        assert_eq!(worm.classify([-6.1, 0., 0.], 1e-6), Containment::Outside);
        assert_eq!(worm.classify([-6., 0., 0.], 1e-6), Containment::Boundary);
        let half_pitch = PI / 2.;
        assert_eq!(
            worm.classify([-5.9, 0., half_pitch], 1e-6),
            Containment::Outside
        );
        assert_eq!(worm.classify([0., 0., 0.], 1e-6), Containment::Outside);
        assert_eq!(worm.classify([2.5, 0., 0.], 1e-6), Containment::Inside);
        assert_eq!(worm.classify([2.5, 0., 10.1], 1e-6), Containment::Outside);
        assert!(rate > 0.);
        let z = worm.support([0., 0., 1.]);
        assert!((z - 10.).abs() < 1e-6);
        let x = worm.support([1., 0., 0.]);
        assert!((x - 6.).abs() < 1e-6, "{x}");
        assert_eq!(worm.bore_radius_mm(), Some(2.));
        assert!((worm.phase_rad() - (PI - 20.)).abs() < 1e-12);
        assert_eq!(worm.spec().bore_radius_mm, 2.);
        assert!(worm.curve_point(Curve::WormRail { rail: 4 }, 0.).is_none());
        assert_eq!(
            worm.curve_segments(
                Curve::WheelJunction {
                    sector: 0,
                    joint: 0
                },
                options(0.1)
            ),
            Err(MeshError::UnresolvedBoundary)
        );
        assert_eq!(
            worm.curve_segments(Curve::WormRail { rail: 0 }, options(0.)),
            Err(MeshError::InvalidOptions)
        );
        assert_eq!(worm.mesh(options(-1.)), Err(MeshError::InvalidOptions));
        let tight = Tessellation {
            deflection: 0.01,
            angle: 0.5,
            triangle_limit: 50,
        };
        assert_eq!(worm.mesh(tight), Err(MeshError::BudgetExceeded));
        // The wall fits but the bored caps do not.
        let wall = helical_thread_mesh::open_wall(worm.thread(), options(0.01))
            .unwrap()
            .0
            .face_indices
            .len();
        let caps = Tessellation {
            triangle_limit: wall + 1,
            ..options(0.01)
        };
        assert_eq!(worm.mesh(caps), Err(MeshError::BudgetExceeded));
    }

    #[test]
    fn worm_rejects_out_of_recipe_inputs() {
        for (spec, error) in [
            (spec(0.4, 20., 0., 0.), Error::InvalidDimensions),
            (spec(1., 20., 0., 0.2), Error::InvalidDimensions),
            (spec(1., 20., -1., 0.), Error::InvalidDimensions),
            (spec(1., f64::NAN, 0., 0.), Error::InvalidDimensions),
            (spec(1., 20., 3.75, 0.), Error::InvalidProfile),
            (spec(1., 15., 0., 0.), Error::Limit),
            (spec(1., 33., 0., 0.), Error::Limit),
        ] {
            assert_eq!(Worm::new(spec, 1e-3), Err(error), "{spec:?}");
        }
        assert_eq!(
            Worm::new(spec(1., 20., 0., 0.), 0.),
            Err(Error::InvalidDimensions)
        );
    }
}
