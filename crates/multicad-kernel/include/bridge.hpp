#pragma once
#include "rust/cxx.h"
#include <TopoDS_Shape.hxx>
#include <memory>

namespace multicad::kernel {
struct Mesh;
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
} // namespace multicad::kernel
