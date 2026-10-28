#pragma once
#include "bridge.hpp"
namespace multicad::kernel {
struct DistanceData;
struct DistanceOptionsData;
DistanceData minimum_distance(const Shape &,const Shape &,const DistanceOptionsData &);
}
