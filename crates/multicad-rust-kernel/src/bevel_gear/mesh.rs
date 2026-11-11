//! Watertight indexed tessellation sharing every boundary vertex by identity.
use super::*;
use crate::display::{DisplayMesh, MeshError, Tessellation};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Key {
    Joint(u32, usize, End),
    Piece(u32, usize, usize, End),
    Row(u32, usize, usize, End),
    Inner(u32, usize, End),
    Bore(u32, usize, End),
    Apex(End),
}

#[derive(Clone, Copy)]
struct Node {
    key: Key,
    direction: Point,
    gamma: f64,
    phi: f64,
}

struct Builder<'a> {
    gear: &'a Bevel,
    keys: HashMap<Key, u32>,
    mesh: DisplayMesh,
    limit: usize,
}

impl Builder<'_> {
    fn vertex(&mut self, node: Node, end: End) -> u32 {
        let key = match node.key {
            Key::Joint(s, j, _) => Key::Joint(s, j, end),
            Key::Piece(s, p, i, _) => Key::Piece(s, p, i, end),
            Key::Row(s, r, i, _) => Key::Row(s, r, i, end),
            Key::Inner(s, i, _) => Key::Inner(s, i, end),
            Key::Bore(s, i, _) => Key::Bore(s, i, end),
            Key::Apex(_) => Key::Apex(end),
        };
        let count = self.mesh.vertices.len() / 3;
        *self.keys.entry(key).or_insert_with(|| {
            let point = self.gear.point(end, node.direction, node.gamma);
            self.mesh.vertices.extend(point);
            count as u32
        })
    }

    fn position(&self, index: u32) -> Point {
        let i = index as usize * 3;
        [
            self.mesh.vertices[i],
            self.mesh.vertices[i + 1],
            self.mesh.vertices[i + 2],
        ]
    }

    fn triangle(
        &mut self,
        [a, b, c]: [u32; 3],
        face: u32,
        outward: Point,
    ) -> Result<(), MeshError> {
        if self.mesh.face_indices.len() >= self.limit {
            return Err(MeshError::BudgetExceeded);
        }
        let [pa, pb, pc] = [a, b, c].map(|i| self.position(i));
        let normal = cross(sub(pb, pa), sub(pc, pa));
        let order = if dot(normal, outward) >= 0. {
            [a, b, c]
        } else {
            [a, c, b]
        };
        self.mesh.triangle(order.map(|i| i as usize), face);
        Ok(())
    }

    fn cap_outward(&self, [a, b, c]: [u32; 3], end: End) -> Point {
        let center: Point = std::array::from_fn(|i| {
            (self.position(a)[i] + self.position(b)[i] + self.position(c)[i]) / 3.
        });
        let (sp, cp) = self.gear.spec.pitch_cone_angle_rad.sin_cos();
        let phi = center[1].atan2(center[0]);
        let sign = if end == End::Heel { 1. } else { -1. };
        [sign * sp * phi.cos(), sign * sp * phi.sin(), sign * cp]
    }

    fn cap_triangle(&mut self, triangle: [u32; 3], face: u32, end: End) -> Result<(), MeshError> {
        let outward = self.cap_outward(triangle, end);
        self.triangle(triangle, face, outward)
    }

    /// Strip between two azimuth-ordered rows of one cap.
    fn zipper(&mut self, a: &[Node], b: &[Node], face: u32, end: End) -> Result<(), MeshError> {
        let a: Vec<_> = a.iter().map(|n| (self.vertex(*n, end), n.phi)).collect();
        let b: Vec<_> = b.iter().map(|n| (self.vertex(*n, end), n.phi)).collect();
        let (mut i, mut j) = (0, 0);
        while i + 1 < a.len() || j + 1 < b.len() {
            if j + 1 == b.len() || (i + 1 < a.len() && a[i + 1].1 <= b[j + 1].1) {
                self.cap_triangle([a[i].0, a[i + 1].0, b[j].0], face, end)?;
                i += 1;
            } else {
                self.cap_triangle([a[i].0, b[j + 1].0, b[j].0], face, end)?;
                j += 1;
            }
        }
        Ok(())
    }
}

fn row(
    gear: &Bevel,
    sector: u32,
    gamma: f64,
    [from, to]: [f64; 2],
    count: usize,
    key: impl Fn(usize) -> Key,
) -> Vec<Node> {
    (0..=count)
        .map(|i| {
            let phi = from + (to - from) * i as f64 / count as f64;
            Node {
                key: key(i),
                direction: gear.rotate(sector, polar(gamma, phi)),
                gamma,
                phi,
            }
        })
        .collect()
}

impl Bevel {
    /// Chord count for one boundary piece at the heel, the larger end.
    pub(crate) fn piece_segments(
        &self,
        piece: Piece,
        options: Tessellation,
    ) -> Result<usize, MeshError> {
        const FINE: usize = 64;
        let points: Vec<_> = (0..=FINE)
            .map(|i| {
                let sample = self.sample(piece, i as f64 / FINE as f64);
                self.point(End::Heel, sample.direction, sample.gamma)
            })
            .collect();
        let h = 1. / FINE as f64;
        let mut second = 0_f64;
        let mut turning = 0.;
        for w in points.windows(3) {
            let a = sub(w[1], w[0]);
            let b = sub(w[2], w[1]);
            second = second.max(norm(sub(b, a)) / (h * h));
            turning += norm(cross(a, b)).atan2(dot(a, b));
        }
        // Twice the sampled second derivative bounds the chord sagitta.
        let count = (2. * second / (8. * options.deflection))
            .sqrt()
            .ceil()
            .max((turning / options.angle).ceil())
            .max(1.);
        if count > options.triangle_limit as f64 {
            return Err(MeshError::BudgetExceeded);
        }
        Ok(count as usize)
    }

    /// Cap row interior point count on a heel-radius arc of `span` radians.
    pub(crate) fn row_segments(
        &self,
        gamma: f64,
        span: f64,
        options: Tessellation,
    ) -> Result<usize, MeshError> {
        let radius = self.end_radius(End::Heel, gamma);
        let count = (span / (options.maximum_arc_angle(radius) / 2.))
            .ceil()
            .max(1.);
        if count > options.triangle_limit as f64 {
            return Err(MeshError::BudgetExceeded);
        }
        Ok(count as usize)
    }

    pub fn mesh(&self, options: Tessellation) -> Result<DisplayMesh, MeshError> {
        options.check()?;
        let teeth = self.spec.teeth;
        let spacing = TAU / f64::from(teeth);
        let count = self.pieces.len();
        let mut segments = Vec::with_capacity(count);
        for (j, &piece) in self.pieces.iter().enumerate() {
            // Mirror counterparts share one count, keeping cap rows paired.
            segments.push(if piece.left() || piece == Piece::Tip {
                self.piece_segments(piece, options)?
            } else {
                segments[count - 1 - j]
            });
        }
        let topology = self.layout();
        let mut builder = Builder {
            gear: self,
            keys: HashMap::new(),
            mesh: DisplayMesh {
                vertices: vec![],
                indices: vec![],
                face_indices: vec![],
            },
            limit: options.triangle_limit,
        };
        let inner_count = self.row_segments(self.inner_gamma, spacing, options)?;
        let bore_count = self
            .bore_radius_mm()
            .map(|bore| options.arc_segments(bore, spacing))
            .transpose()?;
        let piece_index = |piece: Piece| self.pieces.iter().position(|p| *p == piece);
        for sector in 0..teeth {
            let next = (sector + 1) % teeth;
            let nodes: Vec<Vec<Node>> = self
                .pieces
                .iter()
                .enumerate()
                .map(|(j, &piece)| {
                    (0..=segments[j])
                        .map(|i| {
                            let sample = self.sample(piece, i as f64 / segments[j] as f64);
                            let key = if i == 0 {
                                Key::Joint(sector, j, End::Heel)
                            } else if i == segments[j] {
                                if j + 1 == count {
                                    Key::Joint(next, 0, End::Heel)
                                } else {
                                    Key::Joint(sector, j + 1, End::Heel)
                                }
                            } else {
                                Key::Piece(sector, j, i, End::Heel)
                            };
                            Node {
                                key,
                                direction: self.rotate(sector, sample.direction),
                                gamma: sample.gamma,
                                phi: sample.phi,
                            }
                        })
                        .collect()
                })
                .collect();
            for (j, piece_nodes) in nodes.iter().enumerate() {
                let face = topology.face(sector, FaceKind::Side(self.pieces[j]));
                for pair in piece_nodes.windows(2) {
                    let [t0, t1, h0, h1] = [
                        builder.vertex(pair[0], End::Toe),
                        builder.vertex(pair[1], End::Toe),
                        builder.vertex(pair[0], End::Heel),
                        builder.vertex(pair[1], End::Heel),
                    ];
                    let middle = scale(
                        std::array::from_fn(|i| pair[0].direction[i] + pair[1].direction[i]),
                        0.5,
                    );
                    let outward = cross(sub(pair[1].direction, pair[0].direction), middle);
                    builder.triangle([t0, t1, h1], face, outward)?;
                    builder.triangle([t0, h1, h0], face, outward)?;
                }
            }
            let chain = |pieces: &[Piece], reverse: bool| -> Vec<Node> {
                let mut out: Vec<Node> = vec![];
                for piece in pieces {
                    if let Some(j) = piece_index(*piece) {
                        let mut list = nodes[j].clone();
                        if reverse {
                            list.reverse();
                        }
                        if !out.is_empty() {
                            list.remove(0);
                        }
                        out.extend(list);
                    }
                }
                out
            };
            let left = chain(
                &[Piece::FilletLeft, Piece::MeridianLeft, Piece::InvoluteLeft],
                false,
            );
            let right = chain(
                &[
                    Piece::FilletRight,
                    Piece::MeridianRight,
                    Piece::InvoluteRight,
                ],
                true,
            );
            let tip = &nodes[piece_index(Piece::Tip).expect("tip piece")];
            let root_left = &nodes[0];
            let root_right = &nodes[count - 1];
            let mut rows = Vec::with_capacity(left.len());
            for (level, (l, r)) in left.iter().zip(&right).enumerate() {
                if level + 1 == left.len() {
                    rows.push(tip.clone());
                    break;
                }
                let segments = self.row_segments(l.gamma, r.phi - l.phi, options)?;
                let mut row = row(self, sector, l.gamma, [l.phi, r.phi], segments, |i| {
                    Key::Row(sector, level, i, End::Heel)
                });
                row[0] = *l;
                row[segments] = *r;
                rows.push(row);
            }
            let mut root_row = root_left.clone();
            root_row.extend_from_slice(&rows[0][1..]);
            root_row.extend_from_slice(&root_right[1..]);
            let half = spacing / 2.;
            for end in End::BOTH {
                let outer = topology.face(sector, FaceKind::Cap { end, core: false });
                for pair in rows.windows(2) {
                    builder.zipper(&pair[0], &pair[1], outer, end)?;
                }
                let bore_row = self.bore_gamma(end).zip(bore_count).map(|(gamma, n)| {
                    row(self, sector, gamma, [-half, half], n, |i| {
                        if i == n {
                            Key::Bore(next, 0, End::Heel)
                        } else {
                            Key::Bore(sector, i, End::Heel)
                        }
                    })
                });
                if self.has_core(end) {
                    let inner = row(
                        self,
                        sector,
                        self.inner_gamma,
                        [-half, half],
                        inner_count,
                        |i| {
                            if i == inner_count {
                                Key::Inner(next, 0, End::Heel)
                            } else {
                                Key::Inner(sector, i, End::Heel)
                            }
                        },
                    );
                    builder.zipper(&root_row, &inner, outer, end)?;
                    let core = topology.face(sector, FaceKind::Cap { end, core: true });
                    if let Some(bore_row) = &bore_row {
                        builder.zipper(&inner, bore_row, core, end)?;
                    } else {
                        let apex = builder.vertex(
                            Node {
                                key: Key::Apex(end),
                                direction: [0., 0., 1.],
                                gamma: 0.,
                                phi: 0.,
                            },
                            end,
                        );
                        for pair in inner.windows(2) {
                            let a = builder.vertex(pair[0], end);
                            let b = builder.vertex(pair[1], end);
                            builder.cap_triangle([apex, a, b], core, end)?;
                        }
                    }
                } else {
                    let bore_row = bore_row.as_ref().expect("a bore removes the core");
                    builder.zipper(&root_row, bore_row, outer, end)?;
                }
            }
            if let (Some(n), Some(bore)) = (bore_count, self.bore_radius_mm()) {
                let face = topology.face(sector, FaceKind::Bore);
                let rows = End::BOTH.map(|end| {
                    let gamma = self.bore_gamma(end).expect("bore");
                    row(self, sector, gamma, [-half, half], n, |i| {
                        if i == n {
                            Key::Bore(next, 0, End::Heel)
                        } else {
                            Key::Bore(sector, i, End::Heel)
                        }
                    })
                });
                for i in 0..n {
                    let [h0, h1] = [i, i + 1].map(|k| builder.vertex(rows[0][k], End::Heel));
                    let [t0, t1] = [i, i + 1].map(|k| builder.vertex(rows[1][k], End::Toe));
                    let middle = builder.position(h0);
                    let outward = [-middle[0] / bore, -middle[1] / bore, 0.];
                    builder.triangle([t0, t1, h1], face, outward)?;
                    builder.triangle([t0, h1, h0], face, outward)?;
                }
            }
        }
        Ok(builder.mesh)
    }
}
