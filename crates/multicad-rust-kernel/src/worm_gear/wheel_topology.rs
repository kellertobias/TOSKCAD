//! Native generated-wheel face roles and exact sector adjacency.
//!
//! Each sector carries nine faces in slot order: both end planes, the five
//! generated space pieces and the two tip-cylinder lands on either side of
//! the space. The optional bore follows all sectors.
use super::wheel::{PIECES, Piece, Wheel};
use super::wheel_mesh::CHAIN;
use super::*;
use crate::display::{MeshError, Tessellation};

const END_ROLES: [&str; 2] = ["end.start", "end.end"];

impl Wheel {
    fn place(&self, sector: u32, [x, y]: [f64; 2], v: f64) -> Point {
        let m = self.module_mm();
        rotate_z([x * m, y * m, v * m], self.sector_angle(sector))
    }

    pub fn faces(&self) -> Vec<Face> {
        let (sector, bore) = self.sector_faces();
        let m = self.module_mm();
        let mut faces = Vec::with_capacity(9 * self.teeth() as usize + 1);
        for k in 0..self.teeth() {
            for (slot, &(area_mm2, center)) in sector.iter().enumerate() {
                let name = match slot {
                    0 | 1 => END_ROLES[slot],
                    7 | 8 => "tip",
                    piece => PIECES[piece - 2].role(),
                };
                let plane = (slot < 2).then(|| {
                    (
                        [0., 0., self.end_v(slot) * m],
                        [0., 0., if slot == 0 { -1. } else { 1. }],
                    )
                });
                faces.push(Face {
                    role: format!("worm_wheel/sector/{k}/{name}"),
                    area_mm2,
                    center_mm: rotate_z(center, self.sector_angle(k)),
                    plane,
                });
            }
        }
        if let Some(area_mm2) = bore {
            faces.push(Face {
                role: "worm_wheel/bore".into(),
                area_mm2,
                center_mm: [0.; 3],
                plane: None,
            });
        }
        faces
    }

    pub fn topology(&self) -> Topology {
        let teeth = self.teeth();
        let m = self.module_mm();
        let span = self.span();
        let joint_id = |k: u32, e: usize, j: usize| ((k % teeth) as usize * 14 + e * 7 + j) as u32;
        let base = 14 * teeth;
        let bore = self.bore_radius_mm();
        let inner_id = |k: u32, e: usize| {
            if bore.is_some() {
                base + 2 * (k % teeth) + e as u32
            } else {
                base + e as u32
            }
        };
        let sections = [0, 1].map(|e| self.section(self.end_v(e)));
        let mut vertices_mm = Vec::with_capacity(16 * teeth as usize);
        for k in 0..teeth {
            for section in &sections {
                for j in 0..CHAIN {
                    vertices_mm.push(self.place(k, self.joint(section, j), section.v));
                }
            }
        }
        let inner_point = |k: u32, e: usize| {
            let z = self.end_v(e) * m;
            bore.map_or([0., 0., z], |b| {
                rotate_z([b, 0., z], self.sector_angle(k) - span)
            })
        };
        if bore.is_some() {
            for k in 0..teeth {
                for e in 0..2 {
                    vertices_mm.push(inner_point(k, e));
                }
            }
        } else {
            vertices_mm.extend([0, 1].map(|e| inner_point(0, e)));
        }
        let face = |k: u32, slot: u32| self.face_index(k % teeth, slot);
        let mut edges = vec![];
        for k in 0..teeth {
            let previous = (k + teeth - 1) % teeth;
            let angle = self.sector_angle(k);
            for (e, section) in sections.iter().enumerate() {
                let z = section.v * m;
                edges.push(Edge {
                    faces: [face(previous, e as u32), face(k, e as u32)],
                    vertices: [inner_id(k, e), joint_id(k, e, 0)],
                    shape: Shape::Line([
                        inner_point(k, e),
                        vertices_mm[joint_id(k, e, 0) as usize],
                    ]),
                });
                let [low, high] = section.exit;
                for p in 0..CHAIN {
                    let shape = match p {
                        0 => Shape::Arc {
                            z_mm: z,
                            radius_mm: self.blank() * m,
                            start_rad: angle - span,
                            sweep_rad: low + span,
                        },
                        6 => Shape::Arc {
                            z_mm: z,
                            radius_mm: self.blank() * m,
                            start_rad: angle + high,
                            sweep_rad: span - high,
                        },
                        3 => {
                            let [a, b] = [0., 1.].map(|u| {
                                let [x, y] = self.piece_point(section, Piece::Torus, u);
                                y.atan2(x)
                            });
                            Shape::Arc {
                                z_mm: z,
                                radius_mm: self.root_radius(section.v) * m,
                                start_rad: angle + a,
                                sweep_rad: b - a,
                            }
                        }
                        p => Shape::Sampled(Curve::WheelProfile {
                            sector: k,
                            end: e,
                            piece: p - 1,
                        }),
                    };
                    let next = if p + 1 == CHAIN {
                        joint_id(k + 1, e, 0)
                    } else {
                        joint_id(k, e, p + 1)
                    };
                    edges.push(Edge {
                        faces: [face(k, e as u32), face(k, Self::chain_slot(p))],
                        vertices: [joint_id(k, e, p), next],
                        shape,
                    });
                }
                if let Some(radius_mm) = bore {
                    edges.push(Edge {
                        faces: [face(k, e as u32), self.bore_face()],
                        vertices: [inner_id(k, e), inner_id(k + 1, e)],
                        shape: Shape::Arc {
                            z_mm: z,
                            radius_mm,
                            start_rad: angle - span,
                            sweep_rad: 2. * span,
                        },
                    });
                }
            }
            edges.push(Edge {
                faces: [face(previous, 8), face(k, 7)],
                vertices: [joint_id(k, 0, 0), joint_id(k, 1, 0)],
                shape: Shape::Line([0, 1].map(|e| vertices_mm[joint_id(k, e, 0) as usize])),
            });
            for j in 1..CHAIN {
                edges.push(Edge {
                    faces: [
                        face(k, Self::chain_slot(j - 1)),
                        face(k, Self::chain_slot(j)),
                    ],
                    vertices: [joint_id(k, 0, j), joint_id(k, 1, j)],
                    shape: Shape::Sampled(Curve::WheelJunction {
                        sector: k,
                        joint: j,
                    }),
                });
            }
        }
        Topology {
            faces: self.faces(),
            edges,
            vertices_mm,
        }
    }

    /// Local point at parameter `t` in `[0, 1]` along a sampled wheel curve.
    pub fn curve_point(&self, curve: Curve, t: f64) -> Option<Point> {
        match curve {
            Curve::WheelProfile { sector, end, piece }
                if sector < self.teeth() && end < 2 && piece < 5 =>
            {
                let section = self.section(self.end_v(end));
                let point = self.piece_point(&section, PIECES[piece], t);
                Some(self.place(sector, point, section.v))
            }
            Curve::WheelJunction { sector, joint }
                if sector < self.teeth() && (1..CHAIN).contains(&joint) =>
            {
                let h = self.half_width();
                let section = self.section(-h + 2. * h * t);
                Some(self.place(sector, self.joint(&section, joint), section.v))
            }
            _ => None,
        }
    }

    pub fn curve_segments(&self, curve: Curve, options: Tessellation) -> Result<usize, MeshError> {
        options.check()?;
        self.curve_point(curve, 0.)
            .ok_or(MeshError::UnresolvedBoundary)?;
        chord_count(
            |t| self.curve_point(curve, t).expect("checked wheel curve"),
            options.deflection,
            options.angle,
            options.triangle_limit,
        )
    }
}
