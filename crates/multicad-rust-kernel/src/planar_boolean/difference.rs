use super::*;

impl Union {
    /// Remove the regularized union of all tools from the regularized stock.
    /// Source indices follow stock pieces then tools. Tool walls are reversed
    /// to point out of the remaining material, including around enclosed voids.
    pub fn subtract(
        stock: &[ConvexPiece],
        tools: &[ConvexPiece],
        tolerance_mm: f64,
    ) -> Result<Option<Self>, Error> {
        validate_cut(stock, tools, tolerance_mm)?;
        let stock_boundary = clipping::exterior(stock, tolerance_mm, MAX_WORK)?;
        subtract(stock_boundary, stock, tools, stock.len(), tolerance_mm)
    }
}

impl CutMaterial {
    /// Keep the existing exposed face indices as source 0. Cutter cell
    /// sources follow at index 1. Interior partition caps have no ancestry.
    pub fn subtract(
        &self,
        tools: &[ConvexPiece],
        tolerance_mm: f64,
    ) -> Result<Option<Union>, Error> {
        validate_cut(&self.pieces, tools, tolerance_mm)?;
        let mut boundary = shell::Boundary::new(tolerance_mm);
        for (i, face) in self.boundary.faces.iter().enumerate() {
            boundary.add(&polygon(&self.boundary.vertices, face), face.normal, 0, i)?;
        }
        subtract(boundary, &self.pieces, tools, 1, tolerance_mm)
    }
}

fn validate_cut(
    stock: &[ConvexPiece],
    tools: &[ConvexPiece],
    tolerance_mm: f64,
) -> Result<(), Error> {
    validate_tolerance(tolerance_mm)?;
    if stock.is_empty() || tools.is_empty() {
        return Err(Error::Invalid);
    }
    if stock.len().saturating_add(tools.len()) > MAX_PIECES {
        return Err(Error::Limit);
    }
    Ok(())
}

fn subtract(
    stock_boundary: shell::Boundary,
    stock: &[ConvexPiece],
    tools: &[ConvexPiece],
    tool_offset: usize,
    tolerance: f64,
) -> Result<Option<Union>, Error> {
    let tool_boundary = clipping::exterior(tools, tolerance, MAX_WORK)?;
    let mut cut = CutBoundary {
        boundary: shell::Boundary::new(tolerance),
        tolerance,
        budget: MAX_WORK,
    };
    cut.stock_faces(stock_boundary, tools)?;
    cut.tool_faces(tool_boundary, stock, tool_offset)?;
    if cut.boundary.faces.is_empty() {
        return Ok(None);
    }
    cut.boundary.split_edges(&mut cut.budget)?;
    Union::from_boundary(cut.boundary).map(Some)
}

struct CutBoundary {
    boundary: shell::Boundary,
    tolerance: f64,
    budget: usize,
}

fn polygon(vertices: &[Point], face: &Face) -> Vec<Point> {
    face.vertices.iter().map(|&index| vertices[index]).collect()
}

impl CutBoundary {
    fn stock_faces(
        &mut self,
        exterior: shell::Boundary,
        tools: &[ConvexPiece],
    ) -> Result<(), Error> {
        for face in &exterior.faces {
            let ring = polygon(&exterior.vertices, face);
            let face_bounds = bounds(ring.iter().copied());
            let opposite = Plane {
                normal: scale(face.normal, -1.),
                point: ring[0],
            };
            let mut fragments = vec![ring];
            for tool in tools.iter().filter(|tool| {
                clipping::bounds_overlap(face_bounds, tool.bounds, self.tolerance)
                    && !tool
                        .planes
                        .iter()
                        .any(|&plane| opposite.same_direction(plane, self.tolerance))
            }) {
                let mut remaining = Vec::new();
                let mut fragment_budget = MAX_INCIDENCES;
                for ring in fragments {
                    clipping::partition(
                        ring,
                        &tool.planes,
                        self.tolerance,
                        &mut remaining,
                        &mut self.budget,
                        &mut fragment_budget,
                    )?;
                }
                fragments = remaining;
                if fragments.is_empty() {
                    break;
                }
            }
            for fragment in fragments {
                self.boundary
                    .add(&fragment, face.normal, face.source, face.source_face)?;
            }
        }
        Ok(())
    }

    fn tool_faces(
        &mut self,
        exterior: shell::Boundary,
        stock: &[ConvexPiece],
        tool_offset: usize,
    ) -> Result<(), Error> {
        for face in &exterior.faces {
            let ring = polygon(&exterior.vertices, face);
            let face_bounds = bounds(ring.iter().copied());
            let support = Plane {
                normal: face.normal,
                point: ring[0],
            };
            let opposite = Plane {
                normal: scale(face.normal, -1.),
                ..support
            };
            // Cutter walls are new only where stock originally occupied both
            // sides. One-sided coplanar contact keeps the existing stock face.
            // Different stock pieces may contribute either side of a seam.
            let exterior_side = self.stock_side(vec![ring], face_bounds, support, stock)?;
            let interior_side = self.stock_side(exterior_side, face_bounds, opposite, stock)?;
            for mut ring in interior_side {
                ring.reverse();
                self.boundary.add(
                    &ring,
                    opposite.normal,
                    tool_offset + face.source,
                    face.source_face,
                )?;
            }
        }
        Ok(())
    }

    fn stock_side(
        &mut self,
        fragments: Vec<Vec<Point>>,
        face_bounds: [Point; 2],
        side: Plane,
        stock: &[ConvexPiece],
    ) -> Result<Vec<Vec<Point>>, Error> {
        clipping::covered(
            fragments,
            stock,
            face_bounds,
            self.tolerance,
            &mut self.budget,
            |piece| {
                !piece
                    .planes
                    .iter()
                    .any(|&plane| side.same_direction(plane, self.tolerance))
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn difference_stops_at_the_clipping_budget() {
        let tolerance = 1e-7;
        let stock = [ConvexPiece::from_box([0.; 3], [4.; 3], tolerance).unwrap()];
        let tools = [ConvexPiece::from_box([1.; 3], [5.; 3], tolerance).unwrap()];
        let mut cut = CutBoundary {
            boundary: shell::Boundary::new(tolerance),
            tolerance,
            budget: 0,
        };
        let stock_exterior = clipping::exterior(&stock, tolerance, MAX_WORK).unwrap();
        assert_eq!(cut.stock_faces(stock_exterior, &tools), Err(Error::Limit));
        let tool_exterior = clipping::exterior(&tools, tolerance, MAX_WORK).unwrap();
        assert_eq!(
            cut.tool_faces(tool_exterior, &stock, stock.len()),
            Err(Error::Limit)
        );
    }

    #[test]
    fn difference_refuses_ambiguous_boundary_vertices() {
        let tolerance = 1e-7;
        let stock = [ConvexPiece::from_box([0.; 3], [4.; 3], tolerance).unwrap()];
        let tools = [ConvexPiece::from_box([1.; 3], [2.; 3], tolerance).unwrap()];
        let mut cut = CutBoundary {
            boundary: shell::Boundary::new(tolerance),
            tolerance,
            budget: MAX_WORK,
        };
        cut.boundary
            .add(
                &[[1., 1., 1. + tolerance / 2.], [1., 0., 0.], [1., 0., 1.]],
                [1., 0., 0.],
                0,
                0,
            )
            .unwrap();
        let tool_exterior = clipping::exterior(&tools, tolerance, MAX_WORK).unwrap();
        assert_eq!(
            cut.tool_faces(tool_exterior, &stock, stock.len()),
            Err(Error::Unresolved)
        );
    }
}
