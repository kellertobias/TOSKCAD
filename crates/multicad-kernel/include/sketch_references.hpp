#pragma once
#include "bridge.hpp"
namespace multicad::kernel {
struct SketchReferenceCatalogData;
SketchReferenceCatalogData sketch_reference_geometry(const Shape &,uint8_t,uint32_t,const Frame3 &,uint8_t,double);
}
