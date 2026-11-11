//! Watertight wheel tessellation. Every section station carries the same
//! number of nodes per boundary piece, so the side walls are quad strips.
//! Sector seams, piece joints and the bore share vertex identities.
use super::wheel::{PIECES, Section, Wheel};
use super::*;
use crate::display::{DisplayMesh, MeshError, Tessellation, triangulate_region};
use spade::Point2;
use std::collections::HashMap;

/// Chain pieces of one sector: the start rim, five space pieces, the end rim.
pub(crate) const CHAIN: usize = 7;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Key {
    Joint(u32, usize, usize),
    Node(u32, usize, usize, usize),
    Bore(u32, usize, usize),
    Center(usize),
}

impl Wheel {
    /// Topology slot of chain piece `p` within one sector.
    pub(crate) fn chain_slot(p: usize) -> u32 {
        match p {
            0 => 7,
            6 => 8,
            p => 1 + p as u32,
        }
    }

    pub(crate) fn face_index(&self, sector: u32, slot: u32) -> u32 {
        9 * sector + slot
    }

    pub(crate) fn bore_face(&self) -> u32 {
        9 * self.teeth()
    }

    /// Exact chain point of sector 0 in module units.
    pub(crate) fn chain_point(&self, section: &Section, p: usize, u: f64) -> [f64; 2] {
        let blank = self.blank();
        let span = self.span();
        let rim = |a: f64, b: f64| {
            let t = a + (b - a) * u;
            [blank * t.cos(), blank * t.sin()]
        };
        match p {
            0 => rim(-span, section.exit[0]),
            6 => rim(section.exit[1], span),
            p => self.piece_point(section, PIECES[p - 1], u),
        }
    }

    pub(crate) fn counts(
        &self,
        options: Tessellation,
    ) -> Result<([usize; CHAIN], usize), MeshError> {
        let m = self.module_mm();
        let h = self.half_width();
        let sections = [-h, 0., h].map(|v| self.section(v));
        let mut counts = [1; CHAIN];
        let mut rim_span = [0_f64; 2];
        for section in &sections {
            rim_span[0] = rim_span[0].max(section.exit[0] + self.span());
            rim_span[1] = rim_span[1].max(self.span() - section.exit[1]);
            for (p, count) in counts.iter_mut().enumerate().take(6).skip(1) {
                let n = chord_count(
                    |u| {
                        let [x, y] = self.chain_point(section, p, u);
                        [x * m, y * m, 0.]
                    },
                    options.deflection,
                    options.angle,
                    options.triangle_limit,
                )?;
                *count = (*count).max(n);
            }
        }
        counts[0] = options.arc_segments(self.blank() * m, rim_span[0])?.max(1);
        counts[6] = options.arc_segments(self.blank() * m, rim_span[1])?.max(1);
        // The width chord counts never exceed the budget the profile
        // counts already passed: the sections twist far less than they bend.
        let stations = (1..6)
            .flat_map(|p| [0., 0.5, 1.].map(|u| (p, u)))
            .map(|(p, u)| {
                chord_count(
                    |t| {
                        let v = -h + 2. * h * t;
                        let [x, y] = self.chain_point(&self.section(v), p, u);
                        [x * m, y * m, v * m]
                    },
                    options.deflection,
                    options.angle,
                    usize::MAX,
                )
                .unwrap_or(usize::MAX)
            })
            .fold(1, usize::max);
        Ok((counts, stations))
    }

    pub fn mesh(&self, options: Tessellation) -> Result<DisplayMesh, MeshError> {
        options.check()?;
        let m = self.module_mm();
        let h = self.half_width();
        let teeth = self.teeth();
        let (counts, stations) = self.counts(options)?;
        let per_sector: usize = counts.iter().sum();
        let bore = self.bore_radius_mm();
        let bore_count = bore
            .map(|b| options.arc_segments(b, 2. * self.span()).map(|n| n.max(1)))
            .transpose()?;
        let estimate = teeth as usize
            * (2 * per_sector * stations + 2 * per_sector + 4 * bore_count.unwrap_or(0) + 4);
        if estimate > options.triangle_limit {
            return Err(MeshError::BudgetExceeded);
        }
        let sections: Vec<_> = (0..=stations)
            .map(|j| self.section(-h + 2. * h * j as f64 / stations as f64))
            .collect();
        let chains: Vec<Vec<Vec<[f64; 2]>>> = sections
            .iter()
            .map(|section| {
                (0..CHAIN)
                    .map(|p| {
                        (0..=counts[p])
                            .map(|i| self.chain_point(section, p, i as f64 / counts[p] as f64))
                            .collect()
                    })
                    .collect()
            })
            .collect();
        let mut mesh = DisplayMesh {
            vertices: vec![],
            indices: vec![],
            face_indices: vec![],
        };
        let mut ids: HashMap<Key, u32> = HashMap::new();
        let mut vertex = |mesh: &mut DisplayMesh, key: Key, point: Point| -> u32 {
            *ids.entry(key).or_insert_with(|| {
                mesh.vertices.extend(point);
                (mesh.vertices.len() / 3 - 1) as u32
            })
        };
        let place = |sector: u32, [x, y]: [f64; 2], v: f64| {
            rotate_z([x * m, y * m, v * m], self.sector_angle(sector))
        };
        let node = |sector: u32, j: usize, p: usize, i: usize| -> Key {
            if i == 0 {
                Key::Joint(sector, j, p)
            } else if i == counts[p] {
                if p + 1 == CHAIN {
                    Key::Joint((sector + 1) % teeth, j, 0)
                } else {
                    Key::Joint(sector, j, p + 1)
                }
            } else {
                Key::Node(sector, j, p, i)
            }
        };
        let bore_point = |sector: u32, end: usize, i: usize| -> Point {
            let (n, radius) = (bore_count.unwrap_or(1), bore.unwrap_or(0.));
            let angle =
                self.sector_angle(sector) - self.span() + 2. * self.span() * i as f64 / n as f64;
            [
                radius * angle.cos(),
                radius * angle.sin(),
                self.end_v(end) * m,
            ]
        };
        let bore_key = |sector: u32, end: usize, i: usize| {
            if Some(i) == bore_count {
                Key::Bore((sector + 1) % teeth, end, 0)
            } else {
                Key::Bore(sector, end, i)
            }
        };
        for sector in 0..teeth {
            let grid: Vec<Vec<Vec<u32>>> = chains
                .iter()
                .enumerate()
                .map(|(j, chain)| {
                    chain
                        .iter()
                        .enumerate()
                        .map(|(p, points)| {
                            points
                                .iter()
                                .enumerate()
                                .map(|(i, point)| {
                                    vertex(
                                        &mut mesh,
                                        node(sector, j, p, i),
                                        place(sector, *point, sections[j].v),
                                    )
                                })
                                .collect()
                        })
                        .collect()
                })
                .collect();
            for p in 0..CHAIN {
                let face = self.face_index(sector, Self::chain_slot(p));
                for j in 0..stations {
                    for i in 0..counts[p] {
                        let [a, b, c, d] = [
                            grid[j][p][i],
                            grid[j][p][i + 1],
                            grid[j + 1][p][i + 1],
                            grid[j + 1][p][i],
                        ];
                        mesh.triangle([a, b, c].map(|v| v as usize), face);
                        mesh.triangle([a, c, d].map(|v| v as usize), face);
                    }
                }
            }
            for end in 0..2 {
                let j = if end == 0 { 0 } else { stations };
                let mut ring: Vec<u32> = vec![];
                for p in 0..CHAIN {
                    ring.extend(&grid[j][p][..counts[p]]);
                }
                ring.push(grid[j][CHAIN - 1][counts[CHAIN - 1]]);
                if let Some(n) = bore_count {
                    for i in (0..=n).rev() {
                        ring.push(vertex(
                            &mut mesh,
                            bore_key(sector, end, i),
                            bore_point(sector, end, i),
                        ));
                    }
                } else {
                    ring.push(vertex(
                        &mut mesh,
                        Key::Center(end),
                        [0., 0., self.end_v(end) * m],
                    ));
                }
                cap(&mut mesh, &ring, self.face_index(sector, end as u32), end)?;
            }
            if let Some(n) = bore_count {
                let face = self.bore_face();
                for i in 0..n {
                    let [a, b, c, d] = [(0, i), (0, i + 1), (1, i + 1), (1, i)].map(|(end, i)| {
                        vertex(
                            &mut mesh,
                            bore_key(sector, end, i),
                            bore_point(sector, end, i),
                        )
                    });
                    mesh.triangle([a, c, b].map(|v| v as usize), face);
                    mesh.triangle([a, d, c].map(|v| v as usize), face);
                }
            }
        }
        Ok(mesh)
    }
}

/// Constrained triangulation of one planar sector end with material on the
/// left of its counterclockwise ring.
fn cap(mesh: &mut DisplayMesh, ring: &[u32], face: u32, end: usize) -> Result<(), MeshError> {
    let position = |mesh: &DisplayMesh, v: u32| {
        let i = v as usize * 3;
        [mesh.vertices[i], mesh.vertices[i + 1]]
    };
    let points = ring
        .iter()
        .map(|&v| {
            let [x, y] = position(mesh, v);
            Point2::new(x, y)
        })
        .collect();
    let n = ring.len();
    let edges: Vec<[usize; 2]> = (0..n).map(|i| [i, (i + 1) % n]).collect();
    let region = triangulate_region(points, &edges, 0)?;
    for [a, b, c] in region.triangles {
        let [pa, pb, pc] = [a, b, c].map(|i| position(mesh, ring[i]));
        let area = (pb[0] - pa[0]) * (pc[1] - pa[1]) - (pb[1] - pa[1]) * (pc[0] - pa[0]);
        let order = if (area > 0.) == (end == 1) {
            [a, b, c]
        } else {
            [a, c, b]
        };
        mesh.triangle(order.map(|i| ring[i] as usize), face);
    }
    Ok(())
}
