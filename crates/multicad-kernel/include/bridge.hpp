#pragma once
#include "rust/cxx.h"
#include <TopoDS_Shape.hxx>
#include <memory>
#include <vector>
#include <stdexcept>
#include <array>
#include <mutex>
#include <optional>

namespace multicad::kernel {
class VolumeIntegrationFailure : public std::runtime_error {
public:
  VolumeIntegrationFailure():std::runtime_error("Native volume integration did not converge"){}
};
struct Mesh;
struct Point3;
struct Frame3;
struct CurveEdge;
struct Bounds3;
struct FaceInfo;
struct TopologyNode;
struct Ancestry;
struct Seed;
struct EdgeInfo;
struct VertexInfo;
struct StepSolidData;
struct NativeAncestry {
  uint8_t kind, operand;
  uint32_t output, input;
};
struct NativeSeed {
  uint32_t face, source_index;
  uint8_t role;
};
struct BoundsCache {
  std::mutex mutex;
  std::optional<std::array<double,6>> value;
};
struct Shape {
  TopoDS_Shape value;
  std::vector<NativeAncestry> ancestry;
  std::vector<NativeSeed> seeds;
  // New geometry gets a fresh cache; sharing the same immutable B-rep shares
  // its lock and result. Bounds can also be queried by native worker threads.
  std::shared_ptr<BoundsCache> bounds_cache=std::make_shared<BoundsCache>();
};
std::unique_ptr<Shape> extrude(double width, double depth, double height);
std::unique_ptr<Shape> cut(const Shape &, double x, double width, double depth,
                           double height);
Mesh mesh(const Shape &, double deflection);
double volume(const Shape &);
bool valid(const Shape &);
void write_step(const Shape &, rust::Str path);
std::unique_ptr<Shape> read_step(rust::Str path);
rust::Vec<StepSolidData> step_solids(rust::Str path);
rust::String shape_to_brep(const Shape &);
std::unique_ptr<Shape> shape_from_brep(rust::Str text);
void route_native_output_to_stderr();
std::unique_ptr<Shape> cylinder(const Frame3 &,double radius,double height);
std::unique_ptr<Shape> profile_extrude(const Frame3 &,rust::Slice<const CurveEdge>,rust::Slice<const uint32_t>,double height);
std::unique_ptr<Shape> boolean_shapes(const Shape &,const Shape &,uint8_t operation,double fuzzy);
std::unique_ptr<Shape> clip_half_space(const Shape &,const Frame3 &,bool positive,double fuzzy);
std::unique_ptr<Shape> inspection_union(const Shape &);
std::unique_ptr<Shape> share_shape(const Shape &);
std::unique_ptr<Shape> transform_shape(const Shape &,rust::Slice<const double>,const Point3 &);
std::unique_ptr<Shape> affine_transform_shape(const Shape &,rust::Slice<const double>,const Point3 &);
std::unique_ptr<Shape> mirror_shape(const Shape &,const Frame3 &);
std::unique_ptr<Shape> treat_edges(const Shape &,rust::Slice<const uint32_t>,uint8_t mode,double size);
std::unique_ptr<Shape> treat_edges_sided(const Shape &,rust::Slice<const uint32_t>,rust::Slice<const uint32_t>,uint8_t mode,double first,double second);
Bounds3 bounds(const Shape &);
bool bounds_cache_concurrent_probe(const Shape &);
rust::Vec<FaceInfo> faces(const Shape &);
uint32_t solid_count(const Shape &);
Mesh tessellate(const Shape &,double deflection,double angle,uint32_t max_triangles);
rust::Vec<TopologyNode> topology(const Shape &);
rust::Vec<Ancestry> ancestry(const Shape &);
rust::Vec<Seed> seeds(const Shape &);
rust::Vec<EdgeInfo> display_edges(const Shape &, double deflection);
rust::Vec<VertexInfo> vertices(const Shape &);
} // namespace multicad::kernel
