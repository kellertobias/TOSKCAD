//! Recognize through cuts of concentric cylinders as one analytic annulus.

use super::*;

impl Solid {
    pub(in crate::geometry) fn subtract_coaxial_cylinder(
        &self,
        tool: &Self,
        policy: GeometryPolicy,
    ) -> Option<Result<Option<Self>, GeometryError>> {
        let (outer, frame) = self.axial_cylinder_frame()?;
        let (inner, tool_frame) = tool.axial_cylinder_frame()?;
        let height = outer.height().as_f64();
        if let (Some(target), Some(cutter)) = (self.axial_cylinder(), tool.axial_cylinder()) {
            if target.circle().center() != cutter.circle().center()
                || cutter.bottom() > target.bottom()
                || cutter.top() < target.top()
            {
                return None;
            }
        } else {
            let axis = frame.normal();
            let cutter_axis = tool_frame.normal();
            let direction = if cutter_axis == axis {
                1.
            } else if cutter_axis == axis.map(|c| -c) {
                -1.
            } else {
                return None;
            };
            let delta = std::array::from_fn(|i| tool_frame.origin[i] - frame.origin[i]);
            // Do not round a nearby off-axis cut to a concentric one.
            if cross(delta, axis) != [0.; 3] {
                return None;
            }
            let start = dot(delta, axis);
            let finish = start + direction * inner.height().as_f64();
            if start.min(finish) > 0. || start.max(finish) < height {
                return None;
            }
        }

        let [inner_radius, outer_radius] = [inner, outer].map(|c| c.circle().radius());
        if inner_radius >= outer_radius {
            return Some(Ok(None));
        }
        let origin = |solid: &Solid, feature| {
            solid.cylinder_sketch().map_or_else(
                || Origin {
                    source: EntityId::from_uuid_bits(feature),
                    role: "side.cylinder".into(),
                },
                |sketch| sketch.origin.clone(),
            )
        };
        let sketch = self.cylinder_sketch();
        let signed_height = if sketch.is_some_and(|sketch| sketch.reverse) {
            -height
        } else {
            height
        };
        Some(
            Self::build_rust_annulus(
                [inner_radius.as_f64(), outer_radius.as_f64()],
                signed_height,
                EntityId::from_uuid_bits(outer.feature()),
                frame,
                sketch.map_or([0.; 2], |sketch| sketch.center_uv),
                [origin(tool, inner.feature()), origin(self, outer.feature())],
                policy,
            )
            .map(Some),
        )
    }
}
