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
uint8_t curved_profile_classify(const Frame3 &,
                                rust::Slice<const CurvedEdgeData>,
                                rust::Slice<const uint32_t>, double tolerance,
                                const Point3 &);
std::unique_ptr<Shape> curved_profile_extrude(const Frame3 &,
                                              rust::Slice<const CurvedEdgeData>,
                                              rust::Slice<const uint32_t>,
                                              double tolerance, double height);
} // namespace multicad::kernel
