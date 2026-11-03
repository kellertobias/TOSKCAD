#pragma once
#include "bridge.hpp"
namespace multicad::kernel {
struct ThreadData;
std::unique_ptr<Shape> helical_thread(const ThreadData &);
std::unique_ptr<Shape> helical_thread_fresh(const ThreadData &);
std::unique_ptr<Shape> cached_helical_thread(const ThreadData &);
std::unique_ptr<Shape> helical_thread_after_miss(const ThreadData &);
}
