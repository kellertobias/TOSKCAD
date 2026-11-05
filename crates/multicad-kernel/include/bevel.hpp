#pragma once
#include "bridge.hpp"
namespace multicad::kernel {
struct BevelData;
std::unique_ptr<Shape> spherical_bevel_gear(const BevelData &);
}
