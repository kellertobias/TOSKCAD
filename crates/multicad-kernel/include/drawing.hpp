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
  uint32_t max_curves, max_points;
  std::vector<DrawingInput> inputs;
};
std::unique_ptr<DrawingJob> drawing_job(const DrawingOptionsData &);
void drawing_add(DrawingJob &, const Shape &, bool hatch, bool cross, double angle, double spacing);
rust::Vec<DrawingCurveData> drawing_run(DrawingJob &);
}
