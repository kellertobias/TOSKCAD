use super::*;
use crate::normal_sweep::Sweep;
use std::collections::BTreeMap;

struct Rim {
    edges: Vec<OrientedEdge>,
    vertices: Vec<usize>,
}

#[derive(Default)]
struct Builder {
    vertices: Vec<Point>,
    edges: Vec<Edge>,
    faces: Vec<Face>,
    points: BTreeMap<[u64; 3], usize>,
    connections: BTreeMap<[usize; 3], usize>,
}

fn lift([x, y]: Point2, z: f64) -> Point {
    [x, y, z]
}

fn backward(edges: &[OrientedEdge]) -> impl Iterator<Item = OrientedEdge> {
    edges.iter().rev().map(|edge| OrientedEdge {
        edge: edge.edge,
        forward: !edge.forward,
    })
}

impl Builder {
    fn vertex(&mut self, point: Point) -> usize {
        let key = point.map(|x| if x == 0. { 0 } else { x.to_bits() });
        *self.points.entry(key).or_insert_with(|| {
            self.vertices.push(point);
            self.vertices.len() - 1
        })
    }

    fn edge(&mut self, vertices: [usize; 2], curve: Curve, ordinal: usize) -> OrientedEdge {
        let [a, b] = vertices;
        let key = [a.min(b), a.max(b), ordinal];
        let index = *self.connections.entry(key).or_insert_with(|| {
            self.edges.push(Edge { vertices, curve });
            self.edges.len() - 1
        });
        OrientedEdge {
            edge: index,
            forward: self.edges[index].vertices == vertices,
        }
    }

    fn rim(&mut self, section: &Rim2, level: f64) -> Rim {
        let vertices: Vec<_> = section
            .edges
            .iter()
            .map(|edge| self.vertex(lift(edge.start(), level)))
            .collect();
        let edges = section
            .edges
            .iter()
            .enumerate()
            .map(|(i, edge)| {
                let (curve, ordinal) = if let Some(radius_mm) = section.radius {
                    (
                        Curve::Arc {
                            center_mm: lift(section.center, level),
                            radius_mm,
                            sweep_rad: edge.signed_sweep_rad().expect("round profile"),
                        },
                        i + 1,
                    )
                } else {
                    (Curve::Line, 0)
                };
                self.edge(
                    [vertices[i], vertices[(i + 1) % vertices.len()]],
                    curve,
                    ordinal,
                )
            })
            .collect();
        Rim { edges, vertices }
    }

    fn walls(&mut self, section: &Rim2, [low, high]: [&Rim; 2], slab: &Slab) {
        let [lo, hi] = slab.span;
        let middle = lo.midpoint(hi);
        for group in &section.groups {
            let first = group.start;
            let next = group.end % section.edges.len();
            let rail = |builder: &mut Self, index: usize| {
                builder.edge([low.vertices[index], high.vertices[index]], Curve::Line, 0)
            };
            let end = rail(self, next);
            let begin = rail(self, first);
            let mut boundary = low.edges[group.clone()].to_vec();
            boundary.push(end);
            boundary.extend(backward(&high.edges[group.clone()]));
            boundary.push(OrientedEdge {
                edge: begin.edge,
                forward: !begin.forward,
            });
            let edge = section.edges[first];
            let (support, center, length) = if let Some(radius_mm) = section.radius {
                (
                    Support::Cylinder {
                        center_mm: section.center,
                        radius_mm,
                    },
                    edge.centroid(),
                    radius_mm * edge.signed_sweep_rad().expect("round wall"),
                )
            } else {
                let a = edge.start();
                let b = section.edges[group.end - 1].end();
                let [x, y] = std::array::from_fn(|i| b[i] - a[i]);
                let length = x.hypot(y);
                (
                    Support::Plane {
                        normal: [y / length, -x / length, 0.],
                    },
                    std::array::from_fn(|i| a[i].midpoint(b[i])),
                    length,
                )
            };
            self.faces.push(Face {
                support,
                boundary,
                center_mm: lift(center, middle),
                area_mm2: length * (hi - lo),
                sources: slab.sources.clone(),
            });
        }
    }

    fn cap(
        &mut self,
        boundary: &[OrientedEdge],
        center: Point,
        area: f64,
        outward_up: bool,
        sources: &[usize],
    ) {
        self.faces.push(Face {
            support: Support::Plane {
                normal: [0., 0., if outward_up { 1. } else { -1. }],
            },
            boundary: if outward_up {
                boundary.to_vec()
            } else {
                backward(boundary).collect()
            },
            center_mm: center,
            area_mm2: area,
            sources: sources.to_vec(),
        });
    }

    fn transition(
        &mut self,
        profiles: &Profiles,
        pair: [&Slab; 2],
        tolerance: f64,
        minimum: f64,
    ) -> Result<(), Error> {
        let [a, b] = pair;
        let level = a.span[1];
        let circle = profiles.rim(Section::Circle);
        let rectangle = profiles.rim(Section::Rectangle);
        let curved = self.rim(circle, level);
        let straight = self.rim(rectangle, level);
        let outward_up = a.section == Section::Rectangle;
        let mut sources = a.sources.clone();
        merge_sources(&mut sources, &b.sources);
        let count = rectangle.edges.len();
        for (i, &start) in profiles.contacts.iter().enumerate() {
            let finish = profiles.contacts[(i + 1) % profiles.contacts.len()];
            let length = (finish + count - start) % count;
            let indices = || (0..length).map(|offset| (start + offset) % count);
            let mut boundary: Vec<_> = indices().map(|j| straight.edges[j]).collect();
            boundary.extend(backward(&curved.edges[i..i + 1]));
            let mut ring: Vec<_> = indices().map(|j| rectangle.edges[j]).collect();
            ring.push(circle.edges[i].reversed());
            let patch = Sweep::new(vec![ring], minimum.max(1.), tolerance, minimum)
                .map_err(|_| Error::Unsupported)?;
            self.cap(
                &boundary,
                lift(patch.centroid(), level),
                patch.area_mm2(),
                outward_up,
                &sources,
            );
        }
        Ok(())
    }
}

pub(super) fn build(
    profiles: &Profiles,
    slabs: &[Slab],
    tolerance: f64,
    minimum: f64,
) -> Result<ProfileStack, Error> {
    let mut builder = Builder::default();
    let mut volume = 0.;
    for (i, slab) in slabs.iter().enumerate() {
        let section = profiles.rim(slab.section);
        let [lo, hi] = slab.span;
        let [low, high] = [lo, hi].map(|level| builder.rim(section, level));
        builder.walls(section, [&low, &high], slab);
        if i == 0 {
            builder.cap(
                &low.edges,
                lift(section.center, lo),
                section.area,
                false,
                &slab.sources,
            );
        }
        if let Some(next) = slabs.get(i + 1) {
            builder.transition(profiles, [slab, next], tolerance, minimum)?;
        } else {
            builder.cap(
                &high.edges,
                lift(section.center, hi),
                section.area,
                true,
                &slab.sources,
            );
        }
        volume += section.area * (hi - lo);
    }
    let mut bounds = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
    for point in &builder.vertices {
        for (i, value) in point.iter().copied().enumerate() {
            bounds[0][i] = bounds[0][i].min(value);
            bounds[1][i] = bounds[1][i].max(value);
        }
    }
    Ok(ProfileStack {
        bounds,
        volume,
        vertices: builder.vertices,
        edges: builder.edges,
        faces: builder.faces,
    })
}
