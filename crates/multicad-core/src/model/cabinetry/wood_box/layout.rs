use super::super::{PanelBox, read_panel_stock, stock_thickness};
use super::*;
use crate::{
    document::{Document, DocumentError},
    model::common::number,
    parameters::{EvaluatedParameters, Unit},
};

#[derive(Clone, Debug, PartialEq)]
pub struct WoodBoxPanel {
    pub face: WoodBoxFace,
    pub panel: PanelBox,
}
impl WoodBoxPanel {
    pub fn corners(&self) -> [[f64; 3]; 8] {
        let b = self.panel;
        let n = b.normal();
        std::array::from_fn(|index| {
            std::array::from_fn(|axis| {
                b.origin[axis]
                    + b.u[axis] * if index & 1 != 0 { b.length_mm } else { 0. }
                    + b.v[axis] * if index & 2 != 0 { b.width_mm } else { 0. }
                    + n[axis] * if index & 4 != 0 { b.thickness_mm } else { 0. }
            })
        })
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedWoodBox {
    pub outside_mm: [f64; 3],
    pub clear_mm: [f64; 3],
    pub clear_origin_mm: [f64; 3],
    pub thickness_mm: f64,
    pub panels: Vec<WoodBoxPanel>,
    pub envelope_corners: [[f64; 3]; 8],
    pub actual_bounds: [[f64; 3]; 2],
}
pub fn resolve_wood_box(
    document: &Document,
    parameters: &EvaluatedParameters,
    source: &WoodBoxDefinition,
) -> Result<ResolvedWoodBox, DocumentError> {
    use WoodBoxFace::*;
    source.validate()?;
    let stock = read_panel_stock(document, source.stock)?;
    let t = stock_thickness(&stock);
    let w = number(&source.width, Unit::Millimeter, parameters)?;
    let d = number(&source.depth, Unit::Millimeter, parameters)?;
    let h = number(&source.height, Unit::Millimeter, parameters)?;
    if !t.is_finite() || t <= 0. || [w, d, h].iter().any(|value| *value <= 0.) {
        return Err(super::super::invalid(
            "Wood Box dimensions and stock thickness must be positive",
        ));
    }
    let present = |face| if source.faces.contains(&face) { t } else { 0. };
    let min = [present(Left), present(Front), present(Bottom)];
    let max = [w - present(Right), d - present(Back), h - present(Top)];
    let clear = std::array::from_fn(|axis| max[axis] - min[axis]);
    if clear.iter().any(|value| *value <= 1e-6) {
        return Err(super::super::invalid(
            "Stock thickness and chosen faces leave no positive inner usable space",
        ));
    }
    let angle = number(&source.rotation, Unit::Degree, parameters)?.to_radians();
    let (s, c) = angle.sin_cos();
    let origin = [
        number(&source.origin[0], Unit::Millimeter, parameters)?,
        number(&source.origin[1], Unit::Millimeter, parameters)?,
        number(&source.origin[2], Unit::Millimeter, parameters)?,
    ];
    let rotate = |p: [f64; 3]| [c * p[0] - s * p[1], s * p[0] + c * p[1], p[2]];
    let point = |p| {
        let p = rotate(p);
        std::array::from_fn(|axis| p[axis] + origin[axis])
    };
    let mut panels = vec![];
    for face in WoodBoxFace::ALL
        .into_iter()
        .filter(|face| source.faces.contains(face))
    {
        let (position, u, v, length_mm, width_mm) = match face {
            Left => ([0., 0., 0.], [0., 1., 0.], [0., 0., 1.], d, h),
            Right => ([w - t, 0., 0.], [0., 1., 0.], [0., 0., 1.], d, h),
            Front => ([min[0], t, 0.], [1., 0., 0.], [0., 0., 1.], clear[0], h),
            Back => ([min[0], d, 0.], [1., 0., 0.], [0., 0., 1.], clear[0], h),
            Bottom => (
                [min[0], min[1], 0.],
                [1., 0., 0.],
                [0., 1., 0.],
                clear[0],
                clear[1],
            ),
            Top => (
                [min[0], min[1], h - t],
                [1., 0., 0.],
                [0., 1., 0.],
                clear[0],
                clear[1],
            ),
        };
        panels.push(WoodBoxPanel {
            face,
            panel: PanelBox {
                origin: point(position),
                u: rotate(u),
                v: rotate(v),
                length_mm,
                width_mm,
                thickness_mm: t,
            },
        });
    }
    let mut actual_bounds = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
    for corner in panels.iter().flat_map(|panel| panel.corners()) {
        for (axis, value) in corner.into_iter().enumerate() {
            actual_bounds[0][axis] = actual_bounds[0][axis].min(value);
            actual_bounds[1][axis] = actual_bounds[1][axis].max(value);
        }
    }
    let envelope_corners = std::array::from_fn(|index| {
        point([
            if index & 1 != 0 { w } else { 0. },
            if index & 2 != 0 { d } else { 0. },
            if index & 4 != 0 { h } else { 0. },
        ])
    });
    Ok(ResolvedWoodBox {
        outside_mm: [w, d, h],
        clear_mm: clear,
        clear_origin_mm: point(min),
        thickness_mm: t,
        panels,
        envelope_corners,
        actual_bounds,
    })
}
