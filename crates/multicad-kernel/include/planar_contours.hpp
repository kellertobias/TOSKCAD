#pragma once
#include "bridge.hpp"
namespace multicad::kernel {
struct PlanarWireData;
rust::Vec<PlanarWireData> planar_face_contours(const Shape &,uint32_t,const Frame3 &,double);
}
