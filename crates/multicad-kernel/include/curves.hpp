#pragma once
#include "bridge.hpp"
namespace multicad::kernel {
struct CurvedEdgeData;
struct CurveSampleData;
struct CurveQueryData;
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
} // namespace multicad::kernel
