#pragma once
#include "bridge.hpp"
#include <gp_Ax2.hxx>
#include <memory>
#include <vector>
namespace multicad::kernel {
struct DrawingOptionsData;
struct DrawingCurveData;
struct DrawingInput {
  TopoDS_Shape shape;
  bool hatch, cross;
  double angle, spacing;
};
struct DrawingJob {
  gp_Ax2 frame;
  bool section, detail, hidden, tangents;
  double offset, cx, cy, radius, tolerance;
  // Section limits; see DrawingProjection. Unlimited sections keep the
  // original half-space path.
  bool section_only = false, depth = false, window_u = false, window_v = false;
  double depth_mm = 0, u_min = 0, u_max = 0, v_min = 0, v_max = 0;
  uint32_t max_curves, max_points;
  std::vector<DrawingInput> inputs;
};
std::unique_ptr<DrawingJob> drawing_job(const DrawingOptionsData &);
void drawing_add(DrawingJob &, const Shape &, bool hatch, bool cross, double angle, double spacing);
rust::Vec<DrawingCurveData> drawing_run(DrawingJob &);
}
