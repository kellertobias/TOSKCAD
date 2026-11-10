//! Smooth, centered fastener envelopes with analytic circular and hexagonal rings.
use crate::{
    display::{DisplayMesh, MeshError, Tessellation},
    headed_screw::{ScrewHead, hex_area, hex_radius, hex_support},
};
use std::f64::consts::{FRAC_PI_6, PI, TAU};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Section {
    Pole,
    Circle(f64),
    Hexagon(f64),
}

impl Section {
    pub fn area(self) -> f64 {
        match self {
            Self::Pole => 0.,
            Self::Circle(radius) => PI * radius.powi(2),
            Self::Hexagon(across_flats) => hex_area(across_flats),
        }
    }

    pub fn outer_radius(self) -> f64 {
        match self {
            Self::Pole => 0.,
            Self::Circle(radius) => radius,
            Self::Hexagon(across_flats) => across_flats / 3_f64.sqrt(),
        }
    }

    fn radius(self, angle: f64) -> f64 {
        match self {
            Self::Hexagon(across_flats) => hex_radius(across_flats, angle),
            other => other.outer_radius(),
        }
    }

    fn support(self, xy: [f64; 2]) -> f64 {
        match self {
            Self::Hexagon(across_flats) => hex_support(across_flats, xy),
            other => other.outer_radius() * xy[0].hypot(xy[1]),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ring {
    pub section: Section,
    pub z: f64,
}

#[derive(Clone, Debug)]
pub struct Surface {
    pub rings: [Ring; 2],
    pub role: &'static str,
    pub first_face: usize,
    pub face_count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Countersunk {
    pub head_radius_mm: f64,
    pub head_height_mm: f64,
    pub included_angle_deg: f64,
    pub edge_chamfer_mm: f64,
    pub shaft_radius_mm: f64,
    pub shaft_length_mm: f64,
    pub socket_across_flats_mm: f64,
    pub socket_depth_mm: f64,
}

#[derive(Clone, Debug)]
pub struct Fastener {
    path: Vec<(Ring, &'static str)>,
    volume: f64,
}

impl Fastener {
    pub fn hex_nut(across_flats: f64, bore: f64, height: f64, minimum: f64) -> Option<Self> {
        if !dimensions([across_flats, bore, height], minimum) || across_flats / 2. - bore < minimum
        {
            return None;
        }
        let outer = Section::Hexagon(across_flats);
        let inner = Section::Circle(bore);
        Some(Self {
            path: vec![
                (
                    Ring {
                        section: inner,
                        z: 0.,
                    },
                    "cap.start",
                ),
                (
                    Ring {
                        section: outer,
                        z: 0.,
                    },
                    "hardware.outer",
                ),
                (
                    Ring {
                        section: outer,
                        z: height,
                    },
                    "cap.end",
                ),
                (
                    Ring {
                        section: inner,
                        z: height,
                    },
                    "thread.cosmetic.envelope",
                ),
            ],
            volume: height * (outer.area() - inner.area()),
        })
    }

    pub fn headed_screw(head: ScrewHead, radius: f64, length: f64, minimum: f64) -> Option<Self> {
        let height = head.height_mm();
        let (outer, inset, side) = match head {
            ScrewHead::SocketCap { radius_mm, .. } => {
                (Section::Circle(radius_mm), radius_mm, "screw.head/edge/0")
            }
            ScrewHead::ExternalHex {
                across_flats_mm, ..
            } => (
                Section::Hexagon(across_flats_mm),
                across_flats_mm / 2.,
                "screw.head/edge",
            ),
        };
        if !dimensions([height, radius, length, inset], minimum) || inset - radius < minimum {
            return None;
        }
        let (opening, depth) = match head.socket() {
            Some((width, depth)) => {
                let socket = Section::Hexagon(width);
                if !dimensions([width, depth], minimum)
                    || height - depth < minimum
                    || inset - socket.outer_radius() < minimum
                {
                    return None;
                }
                (socket, depth)
            }
            None => (Section::Pole, 0.),
        };
        let shaft = Section::Circle(radius);
        let ring = |section, z| Ring { section, z };
        let mut path = vec![
            (ring(opening, -height), "screw.head/start"),
            (ring(outer, -height), side),
            (ring(outer, 0.), "screw.head/end"),
            (ring(shaft, 0.), "thread.cosmetic.envelope"),
            (ring(shaft, length), "screw.thread/end"),
            (ring(Section::Pole, length), ""),
        ];
        if depth > 0. {
            let floor = -height + depth;
            path.extend([
                (ring(Section::Pole, floor), "screw.socket/end"),
                (ring(opening, floor), "screw.socket/edge"),
            ]);
        }
        Some(Self {
            path,
            volume: outer.area() * height + shaft.area() * length - opening.area() * depth,
        })
    }

    /// The countersunk head, axial shaft and blind hex socket share one boundary.
    /// Refuse any rim, wall or floor that cannot be proved above the minimum size.
    pub fn countersunk(spec: Countersunk, minimum: f64) -> Option<Self> {
        let Countersunk {
            head_radius_mm: head,
            head_height_mm: height,
            included_angle_deg: angle,
            edge_chamfer_mm: chamfer,
            shaft_radius_mm: shaft,
            shaft_length_mm: length,
            socket_across_flats_mm: socket,
            socket_depth_mm: depth,
        } = spec;
        if !dimensions([head, height, shaft, length, socket, depth], minimum)
            || !(0. < angle && angle < 180.)
            || !chamfer.is_finite()
            || chamfer < 0.
            || head - shaft < minimum
        {
            return None;
        }
        let cone = (head - shaft) / (angle / 2.).to_radians().tan();
        let rim = height - cone;
        let top = -height;
        let land = head - chamfer;
        let floor = top + depth;
        let socket_ring = Section::Hexagon(socket);
        let shaft_at_floor = if depth < rim {
            head - (chamfer - depth).max(0.)
        } else {
            head - ((depth - rim) / cone).min(1.) * (head - shaft)
        };
        if cone < minimum
            || rim < chamfer
            || ![chamfer, rim - chamfer]
                .into_iter()
                .all(|size| size == 0. || size >= minimum)
            || land.min(shaft_at_floor) - socket_ring.outer_radius() < minimum
            || length - floor < minimum
        {
            return None;
        }
        let ring = |section, z| Ring { section, z };
        let circle = |radius, z| ring(Section::Circle(radius), z);
        let mut path = vec![
            (ring(socket_ring, top), "screw.head/top.land"),
            (circle(land, top), "screw.head/top.chamfer"),
        ];
        if chamfer > 0. {
            path.push((circle(head, top + chamfer), "screw.head/rim"));
        } else {
            path[1].1 = "screw.head/rim";
        }
        if rim > chamfer {
            path.push((circle(head, top + rim), "screw.head/cone"));
        } else {
            path.last_mut().expect("top rim").1 = "screw.head/cone";
        }
        path.extend([
            (circle(shaft, 0.), "thread.cosmetic.envelope"),
            (circle(shaft, length), "screw.thread/end"),
            (ring(Section::Pole, length), ""),
            (ring(Section::Pole, floor), "screw.socket/end"),
            (ring(socket_ring, floor), "screw.socket/edge"),
        ]);
        let frustum = |a: f64, b: f64, height| PI / 3. * height * (a * a + a * b + b * b);
        let volume = frustum(land, head, chamfer)
            + PI * head.powi(2) * (rim - chamfer)
            + frustum(head, shaft, cone)
            + PI * shaft.powi(2) * length
            - socket_ring.area() * depth;
        Some(Self { path, volume })
    }

    pub fn volume_mm3(&self) -> f64 {
        self.volume
    }

    pub fn support(&self, [x, y, z]: [f64; 3]) -> f64 {
        self.path
            .iter()
            .map(|(ring, _)| ring.section.support([x, y]) + z * ring.z)
            .fold(f64::NEG_INFINITY, f64::max)
    }

    pub fn surfaces(&self) -> impl Iterator<Item = Surface> + '_ {
        self.path.iter().zip(self.path.iter().cycle().skip(1)).scan(
            0,
            |first, (&(a, role), &(b, _))| {
                let face_count = if a.section == Section::Pole && b.section == Section::Pole {
                    0
                } else if a.z != b.z && matches!(a.section, Section::Hexagon(_)) {
                    6
                } else {
                    1
                };
                let surface = Surface {
                    rings: [a, b],
                    role,
                    first_face: *first,
                    face_count,
                };
                *first += face_count;
                Some(surface)
            },
        )
    }

    pub fn mesh(&self, options: Tessellation) -> Result<DisplayMesh, MeshError> {
        options.check()?;
        let radius = self.support([1., 0., 0.]).max(self.support([0., 1., 0.]));
        let per_side = options.arc_segments(radius, TAU / 6.)?;
        let stations = 6 * per_side;
        let per_station: usize = self
            .surfaces()
            .map(|face| {
                face.rings
                    .iter()
                    .filter(|ring| ring.section != Section::Pole)
                    .count()
            })
            .sum();
        if stations * per_station > options.triangle_limit {
            return Err(MeshError::BudgetExceeded);
        }
        let angle = |i| FRAC_PI_6 + TAU * i as f64 / stations as f64;
        let mut mesh = DisplayMesh {
            vertices: vec![],
            indices: vec![],
            face_indices: vec![],
        };
        let bases: Vec<_> = self
            .path
            .iter()
            .map(|(ring, _)| {
                let base = mesh.vertices.len() / 3;
                if ring.section == Section::Pole {
                    mesh.vertices.extend([0., 0., ring.z]);
                } else {
                    for i in 0..stations {
                        let theta = angle(i);
                        let (sin, cos) = theta.sin_cos();
                        let r = ring.section.radius(theta);
                        mesh.vertices.extend([r * cos, r * sin, ring.z]);
                    }
                }
                base
            })
            .collect();
        for (i, surface) in self.surfaces().enumerate() {
            if surface.face_count == 0 {
                continue;
            }
            let [a, b] = surface.rings;
            let next_ring = (i + 1) % self.path.len();
            let point = |j, station, ring: Ring| {
                bases[j]
                    + if ring.section == Section::Pole {
                        0
                    } else {
                        station
                    }
            };
            for station in 0..stations {
                let next = (station + 1) % stations;
                let [a0, a1] = [station, next].map(|s| point(i, s, a));
                let [b0, b1] = [station, next].map(|s| point(next_ring, s, b));
                let facet = if surface.face_count == 6 {
                    station / per_side
                } else {
                    0
                };
                let face = (surface.first_face + facet) as u32;
                if a0 != a1 {
                    mesh.triangle([a0, a1, b1], face);
                }
                if b0 != b1 {
                    mesh.triangle([a0, b1, b0], face);
                }
            }
        }
        Ok(mesh)
    }
}

fn dimensions<const N: usize>(values: [f64; N], minimum: f64) -> bool {
    minimum.is_finite()
        && minimum > 0.
        && values
            .into_iter()
            .all(|value| (minimum..=1e7).contains(&value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manufacturing::certify;

    const SPEC: Countersunk = Countersunk {
        head_radius_mm: 4.2,
        head_height_mm: 2.48,
        included_angle_deg: 90.,
        edge_chamfer_mm: 0.03125,
        shaft_radius_mm: 2.,
        shaft_length_mm: 9.52,
        socket_across_flats_mm: 2.55,
        socket_depth_mm: 1.5,
    };
    const MINIMUM: f64 = 1e-5;
    const TESSELLATION: Tessellation = Tessellation {
        deflection: 0.001,
        angle: 0.1,
        triangle_limit: 25_000,
    };

    #[test]
    fn both_envelopes_have_one_oriented_mesh_boundary() {
        assert_eq!(Section::Pole.area(), 0.);
        let nut = Fastener::hex_nut(7., 1.65, 3.2, MINIMUM).unwrap();
        let screw = Fastener::countersunk(SPEC, MINIMUM).unwrap();
        let expected_nut = 3.2 * (3_f64.sqrt() * 49. / 2. - PI * 1.65_f64.powi(2));
        assert!((nut.volume_mm3() - expected_nut).abs() < 1e-10);
        assert!((nut.support([1., 0., 0.]) - 3.5).abs() < 1e-10);
        assert!((screw.support([0., 0., -1.]) - SPEC.head_height_mm).abs() < 1e-10);
        assert!((screw.support([0., 0., 1.]) - SPEC.shaft_length_mm).abs() < 1e-10);
        assert!(screw.surfaces().any(|face| face.role == "screw.head/cone"));
        for fastener in [nut, screw] {
            let mesh = fastener.mesh(TESSELLATION).unwrap();
            let evidence = certify(&mesh).unwrap();
            assert!((evidence.signed_volume_mm3 - fastener.volume_mm3()).abs() < 0.12);
            assert_eq!(
                mesh.face_indices.into_iter().max().unwrap() as usize + 1,
                fastener.surfaces().map(|surface| surface.face_count).sum()
            );
        }
    }

    #[test]
    fn chamfer_and_rim_can_disappear_without_degenerate_surfaces() {
        let no_chamfer = Countersunk {
            edge_chamfer_mm: 0.,
            socket_depth_mm: 0.1,
            ..SPEC
        };
        let mut no_rim = SPEC;
        no_rim.head_height_mm = (SPEC.head_radius_mm - SPEC.shaft_radius_mm)
            / (SPEC.included_angle_deg / 2.).to_radians().tan()
            + SPEC.edge_chamfer_mm;
        no_rim.edge_chamfer_mm = no_rim.head_height_mm
            - (SPEC.head_radius_mm - SPEC.shaft_radius_mm)
                / (SPEC.included_angle_deg / 2.).to_radians().tan();
        for spec in [no_chamfer, no_rim] {
            let fastener = Fastener::countersunk(spec, MINIMUM).unwrap();
            assert!(
                fastener
                    .surfaces()
                    .all(|surface| surface.rings[0] != surface.rings[1])
            );
            certify(&fastener.mesh(TESSELLATION).unwrap()).unwrap();
        }
    }

    #[test]
    fn socket_cap_and_external_hex_have_exact_shoulders() {
        let socket = ScrewHead::SocketCap {
            radius_mm: 3.5,
            height_mm: 2.,
            socket_across_flats_mm: 2.,
            socket_depth_mm: 1.,
        };
        let hex = ScrewHead::ExternalHex {
            across_flats_mm: 7.,
            height_mm: 2.,
        };
        let shaft_volume = 6. * PI * 4.;
        for head in [socket, hex] {
            let fastener = Fastener::headed_screw(head, 2., 6., MINIMUM).unwrap();
            let removed = head.socket().map_or(0., |(af, depth)| hex_area(af) * depth);
            let volume = shaft_volume + head.cross_section_area_mm2() * 2. - removed;
            assert!((fastener.volume_mm3() - volume).abs() < 1e-10);
            assert!(
                fastener
                    .surfaces()
                    .any(|surface| surface.role == "screw.head/end")
            );
            certify(&fastener.mesh(TESSELLATION).unwrap()).unwrap();
        }
        assert!(Fastener::headed_screw(hex, 3.5, 6., MINIMUM).is_none());
        assert!(
            Fastener::headed_screw(
                ScrewHead::SocketCap {
                    radius_mm: 3.5,
                    height_mm: 2.,
                    socket_across_flats_mm: 2.,
                    socket_depth_mm: 2.,
                },
                2.,
                6.,
                MINIMUM,
            )
            .is_none()
        );
    }

    #[test]
    fn rejects_unresolved_wall_clearance_and_tessellation_limits() {
        assert!(Fastener::hex_nut(4., 2., 1., MINIMUM).is_none());
        assert!(Fastener::hex_nut(4., 1., 1., 0.).is_none());
        assert!(
            Fastener::countersunk(
                Countersunk {
                    socket_across_flats_mm: 20.,
                    ..SPEC
                },
                MINIMUM
            )
            .is_none()
        );
        assert!(
            Fastener::countersunk(
                Countersunk {
                    included_angle_deg: 180.,
                    ..SPEC
                },
                MINIMUM
            )
            .is_none()
        );
        let fastener = Fastener::countersunk(SPEC, MINIMUM).unwrap();
        assert_eq!(
            fastener.mesh(Tessellation {
                triangle_limit: 15,
                ..TESSELLATION
            }),
            Err(MeshError::BudgetExceeded)
        );
        assert_eq!(
            fastener.mesh(Tessellation {
                triangle_limit: 100,
                ..TESSELLATION
            }),
            Err(MeshError::BudgetExceeded)
        );
        assert_eq!(
            fastener.mesh(Tessellation {
                angle: 0.,
                ..TESSELLATION
            }),
            Err(MeshError::InvalidOptions)
        );
    }
}
