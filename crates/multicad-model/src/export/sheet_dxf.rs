//! Developed sheet-metal DXF: the unfolded blank of one selected sheet Part
//! at 1:1 in explicit units. Cut contours keep the planar DXF semantics;
//! bend lines, drill points and their labels go on separate non-cut layers
//! and name the bend or cutout they came from. This module is separate from
//! the planar panel DXF route.
use super::{
    DevelopedContourProvider, Exporter, checkpoint,
    dxf::{MAX_DXF_BYTES, Writer, unit_scale, vertices},
    failure,
};
use crate::{DevelopedPattern, GeometryBackend, Model};
use multicad_core::{
    document::{Document, EntityId},
    exchange::{ErrorKind, ExportError, ExportStamp, LengthUnit, StampedPlanarContours},
};

/// Layer names of a developed sheet DXF. Openings use `<cut>_INNER`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SheetDxfLayers {
    pub cut: String,
    pub bend_up: String,
    pub bend_down: String,
    pub drill: String,
    pub label: String,
}
impl Default for SheetDxfLayers {
    fn default() -> Self {
        Self {
            cut: "CUT".into(),
            bend_up: "BEND_UP".into(),
            bend_down: "BEND_DOWN".into(),
            drill: "DRILL".into(),
            label: "LABEL".into(),
        }
    }
}
impl SheetDxfLayers {
    fn names(&self) -> Result<[String; 6], String> {
        let names = [
            self.cut.clone(),
            format!("{}_INNER", self.cut),
            self.bend_up.clone(),
            self.bend_down.clone(),
            self.drill.clone(),
            self.label.clone(),
        ];
        let distinct = names
            .iter()
            .map(|s| s.to_uppercase())
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            == names.len();
        if !distinct
            || names.iter().any(|s| {
                s.trim().is_empty()
                    || s == "0"
                    || s.len() > 255
                    || s.chars()
                        .any(|c| c.is_control() || "<>/\\\":;?*|=,".contains(c))
            })
        {
            return Err("Sheet DXF layers must have distinct valid names".into());
        }
        Ok(names)
    }
}

/// A developed sheet DXF and the pattern and source stamp it was made from.
#[derive(Clone, Debug, PartialEq)]
pub struct SheetDxf {
    pub stamp: ExportStamp,
    pub pattern: DevelopedPattern,
    pub bytes: Vec<u8>,
}
impl SheetDxf {
    /// Refuse to publish bytes made from an older or different source.
    pub fn check_current(&self, document: &Document, revision: u64) -> Result<(), ExportError> {
        self.stamp.check(document, revision)
    }
}

fn label_text(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}
/// Encode a developed pattern. Coordinates are base-frame millimeters
/// scaled to `units`; nothing is rotated, nested or offset.
pub fn encode_developed(
    pattern: &DevelopedPattern,
    units: LengthUnit,
    layers: &SheetDxfLayers,
    tolerance: f64,
) -> Result<Vec<u8>, String> {
    pattern.cut.validate(tolerance)?;
    if pattern.units != "mm" {
        return Err("Developed patterns must be in millimeters".into());
    }
    let names = layers.names()?;
    let (scale, unit) = unit_scale(units);
    let mut w = Writer {
        text: String::new(),
        handle: 0x100,
    };
    w.pair(0, "SECTION");
    w.pair(2, "HEADER");
    w.pair(9, "$ACADVER");
    w.pair(1, "AC1021");
    w.pair(9, "$INSUNITS");
    w.pair(70, unit);
    w.pair(9, "$MEASUREMENT");
    w.pair(70, if units == LengthUnit::Inch { 0 } else { 1 });
    w.pair(9, "$LUNITS");
    w.pair(70, 2);
    w.pair(9, "$LUPREC");
    w.pair(70, 9);
    w.pair(0, "ENDSEC");
    w.pair(0, "SECTION");
    w.pair(2, "TABLES");
    w.pair(0, "TABLE");
    w.pair(2, "LTYPE");
    w.pair(70, 1);
    w.pair(0, "LTYPE");
    w.pair(2, "CONTINUOUS");
    w.pair(70, 0);
    w.pair(3, "Solid line");
    w.pair(72, 65);
    w.pair(73, 0);
    w.pair(40, 0);
    w.pair(0, "ENDTAB");
    w.pair(0, "TABLE");
    w.pair(2, "LAYER");
    w.pair(70, 7);
    for (i, name) in std::iter::once("0")
        .chain(names.iter().map(String::as_str))
        .enumerate()
    {
        w.pair(0, "LAYER");
        w.pair(100, "AcDbSymbolTableRecord");
        w.pair(100, "AcDbLayerTableRecord");
        w.pair(2, name);
        w.pair(70, 0);
        w.pair(62, [7, 1, 3, 4, 6, 5, 7][i]);
        w.pair(6, "CONTINUOUS");
    }
    w.pair(0, "ENDTAB");
    w.pair(0, "ENDSEC");
    w.pair(0, "SECTION");
    w.pair(2, "BLOCKS");
    w.pair(0, "ENDSEC");
    w.pair(0, "SECTION");
    w.pair(2, "ENTITIES");
    for wire in &pattern.cut.loops {
        let vertices = vertices(wire);
        w.entity(
            "LWPOLYLINE",
            &names[usize::from(!wire.outer)],
            "AcDbPolyline",
        );
        w.pair(90, vertices.len());
        w.pair(70, 1);
        for v in vertices {
            w.pair(10, v.p[0] * scale);
            w.pair(20, v.p[1] * scale);
            w.pair(42, v.bulge);
        }
    }
    let text = |w: &mut Writer, at: [f64; 2], value: &str| {
        w.entity("TEXT", &names[5], "AcDbText");
        w.pair(10, at[0] * scale);
        w.pair(20, at[1] * scale);
        w.pair(30, 0);
        w.pair(40, 2.5 * scale);
        w.pair(1, label_text(value));
        w.pair(50, 0);
        w.pair(7, "STANDARD");
    };
    for b in &pattern.bends {
        let r = &b.record;
        let layer = if r.direction == "up" {
            &names[2]
        } else {
            &names[3]
        };
        w.entity("LINE", layer, "AcDbLine");
        w.pair(10, r.line[0][0] * scale);
        w.pair(20, r.line[0][1] * scale);
        w.pair(30, 0);
        w.pair(11, r.line[1][0] * scale);
        w.pair(21, r.line[1][1] * scale);
        w.pair(31, 0);
        let mid = [
            (r.line[0][0] + r.line[1][0]) / 2.,
            (r.line[0][1] + r.line[1][1]) / 2.,
        ];
        text(
            &mut w,
            mid,
            &format!(
                "{} {} {:.3} R{:.3} #{}",
                b.id,
                r.direction.to_uppercase(),
                r.angle_deg,
                r.inside_radius_mm,
                b.order + 1
            ),
        );
    }
    for d in &pattern.drills {
        w.entity("POINT", &names[4], "AcDbPoint");
        w.pair(10, d.center[0] * scale);
        w.pair(20, d.center[1] * scale);
        w.pair(30, 0);
        text(
            &mut w,
            [d.center[0] + d.diameter_mm / 2., d.center[1]],
            &format!("{} D{:.3}", d.id, d.diameter_mm),
        );
    }
    let [min, _] = pattern
        .cut
        .bounds_mm()
        .ok_or("Missing finite contour bounds")?;
    text(
        &mut w,
        [min[0], min[1] - 4.],
        &format!(
            "{} t{} K{} {}",
            pattern.name, pattern.thickness_mm, pattern.k_factor, pattern.feature
        ),
    );
    w.pair(0, "ENDSEC");
    w.pair(0, "EOF");
    if w.text.len() > MAX_DXF_BYTES {
        return Err("DXF output exceeds its bounded byte budget".into());
    }
    Ok(w.text.into_bytes())
}

/// The developed-contour provider for sheet parts: the cut loops of the
/// feature's Part, stamped with the evaluated source.
pub struct SheetFlatProvider;
impl DevelopedContourProvider for SheetFlatProvider {
    fn contours(
        &self,
        document: &Document,
        model: &Model,
        feature: EntityId,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<StampedPlanarContours, ExportError> {
        checkpoint(interrupt)?;
        let sheet = multicad_core::model::read_sheet(document, feature)
            .map_err(|e| failure(ErrorKind::Unsupported, Some(feature), e))?;
        let pattern = model
            .unfold(document, sheet.output.body)
            .map_err(|e| failure(ErrorKind::Invalid, Some(feature), e.message))?;
        Ok(StampedPlanarContours {
            stamp: ExportStamp::capture(document, model.snapshot().stamp.revision)?,
            source: feature,
            contours: pattern.cut,
        })
    }
}

/// Exact backend and presentation settings for a developed sheet DXF.
pub struct SheetDxfOptions<'a> {
    pub backend: GeometryBackend,
    pub units: LengthUnit,
    pub layers: &'a SheetDxfLayers,
}

impl Exporter {
    /// Unfold one sheet Part from a fresh evaluation of `document` at
    /// `revision` and encode its DXF. A failed evaluation or unfolding
    /// produces no file.
    pub fn developed_sheet_dxf(
        &self,
        document: &Document,
        revision: u64,
        part: EntityId,
        units: LengthUnit,
        layers: &SheetDxfLayers,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<SheetDxf, ExportError> {
        self.developed_sheet_dxf_with_backend(
            document,
            revision,
            part,
            SheetDxfOptions {
                backend: GeometryBackend::OpenCascade,
                units,
                layers,
            },
            interrupt,
        )
    }

    /// Evaluate and export the developed blank with an explicitly selected backend.
    pub fn developed_sheet_dxf_with_backend(
        &self,
        document: &Document,
        revision: u64,
        part: EntityId,
        options: SheetDxfOptions<'_>,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<SheetDxf, ExportError> {
        checkpoint(interrupt)?;
        let model = self
            .compiler
            .compile_with_backend(document, revision, options.backend, interrupt)
            .map_err(|e| failure(ErrorKind::Invalid, e.entity, e.message))?;
        checkpoint(interrupt)?;
        let pattern = model.unfold(document, part).map_err(|e| {
            failure(
                ErrorKind::Invalid,
                e.entity.or(Some(part)),
                format!("No developed export: {}", e.message),
            )
        })?;
        let bytes = encode_developed(
            &pattern,
            options.units,
            options.layers,
            self.compiler.policy.linear_tolerance_mm,
        )
        .map_err(|e| failure(ErrorKind::Invalid, Some(pattern.feature), e))?;
        Ok(SheetDxf {
            stamp: ExportStamp::capture(document, revision)?,
            pattern,
            bytes,
        })
    }
}
