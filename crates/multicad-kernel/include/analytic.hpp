#pragma once
#include "bridge.hpp"
namespace multicad::kernel {
struct AnalyticEdgeData;
rust::Vec<AnalyticEdgeData> analytic_edges(const Shape &);
}
