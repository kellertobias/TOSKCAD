use super::*;

impl Opening {
    pub(super) fn append(self, shell: &mut Shell, cut: &AxialCut) -> Result<(), Error> {
        let [start, end] = [self.entry_level, self.tube.length].map(|level| self.tube.point(level));
        let axis = self.tube.axes[2];
        let depth = self.tube.length - self.entry_level;
        let radius = self.segment.radius;
        // Even at coarse tessellation, the chord and curved floor need
        // at least three distinct vertices.
        let arc_count = ((self.segment.sweep_rad / FRAC_PI_2).ceil() as usize).max(2);
        let sweep = self.segment.sweep_rad / arc_count as f64;
        let [rim, bottom] =
            [start, end].map(|p| append_arc(shell, p, &self.segment, axis, arc_count, sweep));
        let [rim, bottom] = [rim?, bottom?];
        let rails: Vec<_> = rim
            .vertices
            .iter()
            .zip(&bottom.vertices)
            .map(|(&a, &b)| append_edge(shell, [a, b], Curve::Line))
            .collect();
        let floor_chord = append_edge(
            shell,
            [bottom.vertices[arc_count], bottom.vertices[0]],
            Curve::Line,
        );
        let [old_start, old_end] = self.entry_ends;
        // The old edge has exactly the two proved incidences. Its first
        // piece keeps the original index; its middle now belongs to the void.
        shell.edges[self.common_edge].vertices = [old_start, rim.vertices[0]];
        let tail = append_edge(shell, [rim.vertices[arc_count], old_end], Curve::Line);
        replace_edge(
            &mut shell.faces[self.entry],
            self.common_edge,
            std::iter::once(oriented(self.common_edge, true))
                .chain(rim.edges.iter().map(|&i| oriented(i, true)))
                .chain(std::iter::once(oriented(tail, true))),
        );
        replace_edge(
            &mut shell.faces[self.roof],
            self.common_edge,
            [
                oriented(tail, false),
                oriented(rails[arc_count], true),
                oriented(floor_chord, true),
                oriented(rails[0], false),
                oriented(self.common_edge, false),
            ]
            .into_iter(),
        );

        let area = self.segment.area();
        shell.faces[self.entry].area_mm2 -= area;
        shell.faces[self.roof].area_mm2 -= 2. * self.segment.half_chord * depth;
        // The old shared edge endpoints lie strictly outside both notches.
        shell.faces[self.entry].point_mm = shell.vertices[old_start];
        shell.faces[self.roof].point_mm = shell.vertices[old_end];
        let support = Support::Cylinder {
            axis_origin_mm: start,
            axis_direction: axis,
            radius_mm: radius,
            outward: false,
        };
        let mid = self.tube.point(self.entry_level.midpoint(self.tube.length));
        for i in 0..arc_count {
            let angle = self.segment.start_rad + (i as f64 + 0.5) * sweep;
            let (sin, cos) = angle.sin_cos();
            let point_mm = std::array::from_fn(|j| {
                mid[j] + radius * (cos * self.segment.normal[j] + sin * self.segment.tangent[j])
            });
            shell.faces.push(Face {
                boundary: vec![
                    oriented(rim.edges[i], false),
                    oriented(rails[i], true),
                    oriented(bottom.edges[i], true),
                    oriented(rails[i + 1], false),
                ],
                holes: Vec::new(),
                support,
                point_mm,
                area_mm2: sweep * radius * depth,
                source: cut.source,
                source_face: cut.wall_source_face,
            });
        }
        let radial = self.segment.offset.midpoint(-radius);
        shell.faces.push(Face {
            boundary: std::iter::once(oriented(floor_chord, false))
                .chain(bottom.edges.iter().rev().map(|&i| oriented(i, false)))
                .collect(),
            holes: Vec::new(),
            support: Support::Plane {
                normal: axis.map(|x| -x),
            },
            point_mm: std::array::from_fn(|i| end[i] + radial * self.segment.normal[i]),
            area_mm2: area,
            source: cut.source,
            source_face: self.floor_source_face,
        });
        shell.volume_mm3 = positive_volume(shell.volume_mm3 - area * depth)?;
        Ok(())
    }
}

struct ArcRim {
    vertices: Vec<usize>,
    edges: Vec<usize>,
}

fn append_arc(
    shell: &mut Shell,
    center: Point,
    segment: &Segment,
    axis: Point,
    count: usize,
    sweep: f64,
) -> Result<ArcRim, Error> {
    let ellipse = Ellipse::new(center, axis, segment.normal, [segment.radius; 2])
        .map_err(|_| Error::Invalid)?;
    let [first, last] = segment.chord(center);
    let offset = shell.vertices.len();
    shell.vertices.push(first);
    shell
        .vertices
        .extend((1..count).map(|i| ellipse.point_at(segment.start_rad + sweep * i as f64)));
    shell.vertices.push(last);
    let vertices = (offset..offset + count + 1).collect::<Vec<_>>();
    let mut edges = Vec::with_capacity(count);
    for i in 0..count {
        let arc = EllipseArc::new(ellipse, segment.start_rad + sweep * i as f64, sweep)
            .map_err(|_| Error::Invalid)?;
        edges.push(append_edge(
            shell,
            [vertices[i], vertices[i + 1]],
            Curve::Ellipse(arc),
        ));
    }
    Ok(ArcRim { vertices, edges })
}

fn replace_edge(face: &mut Face, old: usize, replacement: impl Iterator<Item = OrientedEdge>) {
    let at = face
        .boundary
        .iter()
        .position(|edge| edge.edge == old)
        .expect("proved boundary incidence");
    face.boundary.splice(at..=at, replacement);
}
