#pragma once
#include "bridge.hpp"
#include <TopoDS_Edge.hxx>
#include <TopoDS_Face.hxx>
#include <utility>
namespace multicad::kernel {
struct CurvedEdgeData;
std::unique_ptr<Shape> ruled_profile_solid(
    const TopoDS_Face &start, const TopoDS_Face &end,
    const std::vector<std::pair<TopoDS_Edge, uint32_t>> &start_edges,
    const std::vector<std::pair<TopoDS_Edge, uint32_t>> &end_edges,
    rust::Slice<const CurvedEdgeData> kinds, double tolerance);
} // namespace multicad::kernel
