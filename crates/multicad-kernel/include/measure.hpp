#pragma once
#include <BRepGProp.hxx>
#include <GProp_GProps.hxx>
#include <TopoDS_Face.hxx>
#include <cmath>
#include <stdexcept>
namespace multicad::kernel {
// Measurement only: exact surfaces/trim curves are integrated adaptively;
// no geometry, triangulation or stored topology is changed. A single face has
// positive area, so its relative convergence estimate is meaningful.
inline GProp_GProps measured_face(const TopoDS_Face &face) {
  GProp_GProps properties;
  const double relativeError =
      BRepGProp::SurfaceProperties(face, properties, 1e-12);
  const auto center = properties.CentreOfMass();
  if (!std::isfinite(relativeError) || relativeError < 0. ||
      relativeError > 1e-7 || !std::isfinite(properties.Mass()) ||
      properties.Mass() <= 0. || !std::isfinite(center.X()) ||
      !std::isfinite(center.Y()) || !std::isfinite(center.Z()))
    throw std::runtime_error(
        "Native face area/centroid integration did not converge");
  return properties;
}
} // namespace multicad::kernel
