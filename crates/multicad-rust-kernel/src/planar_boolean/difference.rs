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
        validate_tolerance(tolerance_mm)?;
        if stock.is_empty() || tools.is_empty() {
            return Err(Error::Invalid);
        }
        if stock.len().saturating_add(tools.len()) > MAX_PIECES {
            return Err(Error::Limit);
        }
        let stock_boundary = clipping::exterior(stock, tolerance_mm, MAX_WORK)?;
        let tool_boundary = clipping::exterior(tools, tolerance_mm, MAX_WORK)?;
        let mut cut = CutBoundary {
            boundary: shell::Boundary::new(tolerance_mm),
            tolerance: tolerance_mm,
            budget: MAX_WORK,
        };
        cut.stock_faces(stock_boundary, tools)?;
        cut.tool_faces(tool_boundary, stock)?;
        if cut.boundary.faces.is_empty() {
            return Ok(None);
        }
        cut.boundary.split_edges(&mut cut.budget)?;
        Self::from_boundary(cut.boundary).map(Some)
    }
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
                    stock.len() + face.source,
                    face.source_face,
                )?;
            }
        }
        Ok(())
    }

    fn stock_side(
        &mut self,
        mut fragments: Vec<Vec<Point>>,
        face_bounds: [Point; 2],
        side: Plane,
        stock: &[ConvexPiece],
    ) -> Result<Vec<Vec<Point>>, Error> {
        let mut covered = Vec::new();
        let mut covered_budget = MAX_INCIDENCES;
        for piece in stock {
            if fragments.is_empty() {
                break;
            }
            if !clipping::bounds_overlap(face_bounds, piece.bounds, self.tolerance)
                || piece
                    .planes
                    .iter()
                    .any(|&plane| side.same_direction(plane, self.tolerance))
            {
                continue;
            }
            let mut remaining = Vec::new();
            let mut fragment_budget = MAX_INCIDENCES;
            for ring in fragments {
                if let Some(inside) = clipping::partition(
                    ring,
                    &piece.planes,
                    self.tolerance,
                    &mut remaining,
                    &mut self.budget,
                    &mut fragment_budget,
                )? {
                    charge(&mut covered_budget, inside.len())?;
                    covered.push(inside);
                }
            }
            fragments = remaining;
        }
        Ok(covered)
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
        assert_eq!(cut.tool_faces(tool_exterior, &stock), Err(Error::Limit));
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
            cut.tool_faces(tool_exterior, &stock),
            Err(Error::Unresolved)
        );
    }
}
