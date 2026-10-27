#pragma once
#include "rust/cxx.h"
#include <TopoDS_Shape.hxx>
#include <memory>

namespace multicad::kernel {
struct Mesh;
struct Point3;
struct Frame3;
struct CurveEdge;
struct Bounds3;
struct FaceInfo;
struct Shape {
  TopoDS_Shape value;
};
std::unique_ptr<Shape> extrude(double width, double depth, double height);
std::unique_ptr<Shape> cut(const Shape &, double x, double width, double depth,
                           double height);
Mesh mesh(const Shape &, double deflection);
double volume(const Shape &);
bool valid(const Shape &);
void write_step(const Shape &, rust::Str path);
std::unique_ptr<Shape> read_step(rust::Str path);
std::unique_ptr<Shape> cylinder(const Frame3 &,double radius,double height);
std::unique_ptr<Shape> profile_extrude(const Frame3 &,rust::Slice<const CurveEdge>,rust::Slice<const uint32_t>,double height);
std::unique_ptr<Shape> boolean_shapes(const Shape &,const Shape &,uint8_t operation,double fuzzy);
std::unique_ptr<Shape> transform_shape(const Shape &,rust::Slice<const double>,const Point3 &);
Bounds3 bounds(const Shape &);
rust::Vec<FaceInfo> faces(const Shape &);
uint32_t solid_count(const Shape &);
Mesh tessellate(const Shape &,double deflection,double angle,uint32_t max_triangles);
} // namespace multicad::kernel
