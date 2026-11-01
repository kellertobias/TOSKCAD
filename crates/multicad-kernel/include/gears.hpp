#pragma once
#include "bridge.hpp"
namespace multicad::kernel {
struct GearData;
std::unique_ptr<Shape> involute_gear(const GearData &);
}
