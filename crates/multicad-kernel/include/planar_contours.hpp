#pragma once
#include "bridge.hpp"
namespace multicad::kernel {
struct PlanarWireData;
struct HoleAxisData;
HoleAxisData transfer_hole_axis(const Shape &,uint8_t,uint32_t,double);
rust::Vec<PlanarWireData> planar_face_contours(const Shape &,uint32_t,const Frame3 &,double);
}
