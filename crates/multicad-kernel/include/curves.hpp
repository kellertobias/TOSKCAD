#pragma once
#include "bridge.hpp"
namespace multicad::kernel {
struct CurvedEdgeData;
struct CurveSampleData;
struct CurveQueryData;
struct SweepLimits;
struct LoftSectionData;
std::unique_ptr<Shape> curved_profile_pipe(const Frame3 &,rust::Slice<const CurvedEdgeData>,rust::Slice<const Point3>,double tolerance);
std::unique_ptr<Shape> curved_profile_loft(rust::Slice<const LoftSectionData>,bool ruled,double tolerance);
CurveSampleData curve_evaluate(const CurvedEdgeData &, double parameter);
CurveQueryData curve_intersections(const CurvedEdgeData &,
                                   const CurvedEdgeData &, double tolerance,
                                   bool self_query);
double curved_profile_area(const Frame3 &, rust::Slice<const CurvedEdgeData>,
                           rust::Slice<const uint32_t>, double tolerance);
double curved_profile_signed_area(const Frame3 &,
                                  rust::Slice<const CurvedEdgeData>,
                                  rust::Slice<const uint32_t>,
                                  double tolerance);
uint8_t curved_profile_classify(const Frame3 &,
                                rust::Slice<const CurvedEdgeData>,
                                rust::Slice<const uint32_t>, double tolerance,
                                const Point3 &);
std::unique_ptr<Shape>
curved_profile_extrude_vector(const Frame3 &, rust::Slice<const CurvedEdgeData>,
                              rust::Slice<const uint32_t>, double tolerance,
                              const Point3 &displacement);
std::unique_ptr<Shape>
curved_profile_revolve(const Frame3 &, rust::Slice<const CurvedEdgeData>,
                       rust::Slice<const uint32_t>, double tolerance,
                       const Point3 &axis_origin, const Point3 &axis_direction,
                       double angle);
std::unique_ptr<Shape>
curved_profile_to_plane(const Frame3 &, rust::Slice<const CurvedEdgeData>,
                        rust::Slice<const uint32_t>, const SweepLimits &,
                        const Point3 &direction, const Frame3 &termination);
std::unique_ptr<Shape>
curved_profile_ruled(const Frame3 &start, const Frame3 &end,
                     rust::Slice<const CurvedEdgeData> start_edges,
                     rust::Slice<const CurvedEdgeData> end_edges,
                     rust::Slice<const uint32_t> offsets, double tolerance);
} // namespace multicad::kernel
