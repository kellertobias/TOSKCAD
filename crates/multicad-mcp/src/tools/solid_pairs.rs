//! Read-only whole-Part queries use the same finite-solid inspector as Motion.
use super::*;
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InspectPartPairArgs {
    pub expected_revision: u64,
    pub a_part: String,
    pub b_part: String,
    #[serde(default)]
    pub contact_tolerance: Option<Quantity>,
}
pub(super) fn inspect_part_pair(
    host: &dyn Host,
    call: &Request,
    args: InspectPartPairArgs,
) -> ToolResult {
    let (document, revision, parameters) = snapshot(host)?;
    if args.expected_revision != revision {
        return Err(ToolError::new(
            "stale_revision",
            format!(
                "Request is based on revision {}, but the document is at {revision}",
                args.expected_revision
            ),
        )
        .at_revision(revision));
    }
    let a = id(&args.a_part)?;
    let b = id(&args.b_part)?;
    let a_feature = current_producer(&document, a)?;
    let b_feature = current_producer(&document, b)?;
    let model = compile_without_display(&document, revision, call)?;
    for part in [a, b] {
        if model.snapshot().consumed_bodies.contains(&part) {
            return Err(ToolError::new(
                "missing_current_part",
                "Consumed Parts have no independent current solid",
            )
            .involving(part));
        }
    }
    let left = model
        .solid(a_feature)
        .ok_or_else(|| ToolError::not_found(a, "current physical Part"))?;
    let right = model
        .solid(b_feature)
        .ok_or_else(|| ToolError::not_found(b, "current physical Part"))?;
    let tolerance = args
        .contact_tolerance
        .as_ref()
        .map(|q| q.dimension(&document, &parameters).map(|(_, mm)| mm))
        .transpose()?
        .unwrap_or(
            left.geometry_policy()
                .linear_tolerance_mm
                .max(right.geometry_policy().linear_tolerance_mm),
        );
    call.check()?;
    let result = left
        .inspect_pair(right, tolerance, left.geometry_policy())
        .map_err(|cause| {
            ToolError::new(
                match cause.kind {
                    multicad_kernel::GeometryErrorKind::Unsupported => "unsupported_geometry",
                    multicad_kernel::GeometryErrorKind::Cancelled => "cancelled",
                    multicad_kernel::GeometryErrorKind::Limit => "geometry_limit",
                    multicad_kernel::GeometryErrorKind::Input => "invalid_argument",
                    _ => "inspection_failed",
                },
                cause.to_string(),
            )
        });
    call.check()?;
    with(host, move |target| revision_guard(target, revision))??;
    let result = result?;
    let relation = match result.relation {
        multicad_kernel::SolidPairRelation::Separated => {
            multicad_core::assembly_inspection::PairRelation::Separated
        }
        multicad_kernel::SolidPairRelation::ContactWithinTolerance => {
            multicad_core::assembly_inspection::PairRelation::ContactWithinTolerance
        }
        multicad_kernel::SolidPairRelation::Overlapping => {
            multicad_core::assembly_inspection::PairRelation::Overlapping
        }
    };
    let distance = result.minimum_distance;
    Ok(
        json!({"revision":revision,"scope":"current Part definition coordinates","a_part":a,"b_part":b,"relation":relation,"common_volume_mm3":result.common_volume_mm3,"minimum_distance_mm":distance.distance_mm,"witnesses":distance.witnesses.iter().map(|w|json!({"a_mm":w.a_mm,"b_mm":w.b_mm})).collect::<Vec<_>>(),"witnesses_complete":distance.witnesses_complete,"total_solutions":distance.total_solutions,"inner_solution":distance.inner_solution,"contact_tolerance_mm":result.contact_tolerance_mm}),
    )
}
