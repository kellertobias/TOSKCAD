#pragma once
#include "bridge.hpp"
namespace multicad::kernel {
struct ThreadData;
struct WormWheelData;
std::unique_ptr<Shape> za_worm(const ThreadData &,double bore);
std::unique_ptr<Shape> za_worm_wheel(const WormWheelData &);
}
