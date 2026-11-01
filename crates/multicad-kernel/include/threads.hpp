#pragma once
#include "bridge.hpp"
namespace multicad::kernel {
struct ThreadData;
std::unique_ptr<Shape> helical_thread(const ThreadData &);
}
