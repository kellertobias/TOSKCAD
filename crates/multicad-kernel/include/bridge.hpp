#pragma once
#include "rust/cxx.h"
#include <TopoDS_Shape.hxx>
#include <memory>
#include <vector>
#include <stdexcept>
#include <array>
#include <mutex>
#include <optional>

class TopoDS_Face;
class gp_Pnt;
namespace multicad::kernel {
class VolumeIntegrationFailure : public std::runtime_error {
public:
  VolumeIntegrationFailure():std::runtime_error("Native volume integration did not converge"){}
};
struct Mesh;
struct MeshCacheStats;
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
struct ThreadData;
struct ThreadFactoryStats;
struct ThreadFactoryTag;
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
struct VolumeCertificate {
  double mass,relative_error;
};
struct VolumeCache {
  std::mutex mutex;
  std::optional<VolumeCertificate> certificate;
  uint64_t integrations=0;
};
struct CachedFace {
  uint32_t index;
  bool planar;
  std::array<double,3> origin,normal,centroid;
  double area;
};
CachedFace describe_face(const TopoDS_Face &,uint32_t,const gp_Pnt &,double);
struct FaceCache {
  std::mutex mutex;
  std::optional<std::vector<CachedFace>> value;
  uint64_t integrations=0;
};
struct ValidityCache {
  std::mutex mutex;
  bool accepted=false;
  uint64_t checks=0;
};
struct MeshKey {
  uint64_t linear,angular;
  uint32_t triangles;
  bool operator==(const MeshKey &other)const {return linear==other.linear&&angular==other.angular&&triangles==other.triangles;}
};
struct MeshBuffer;
struct MeshIdentity {
  std::mutex mutex;
  std::optional<MeshKey> key;
  std::weak_ptr<const MeshBuffer> receipt;
};
struct Shape {
  TopoDS_Shape value;
  std::vector<NativeAncestry> ancestry;
  std::vector<NativeSeed> seeds;
  // New geometry gets a fresh cache; sharing the same immutable B-rep shares
  // its lock and result. Bounds can also be queried by native worker threads.
  std::shared_ptr<BoundsCache> bounds_cache=std::make_shared<BoundsCache>();
  // Only completed positive, finite native integrations are memoized. New
  // geometry/imports start empty; identical immutable owners share the cache.
  std::shared_ptr<VolumeCache> volume_cache=std::make_shared<VolumeCache>();
  // Completed numeric face properties belong to this exact immutable geometry.
  // Memory is bounded by its face count and lifetime; new geometry starts cold.
  std::shared_ptr<FaceCache> face_cache=std::make_shared<FaceCache>();
  // Only successful native validation of this exact immutable geometry.
  std::shared_ptr<ValidityCache> validity_cache=std::make_shared<ValidityCache>();
  // Identity and a weak numeric receipt only; strong mesh retention is bounded
  // by the owning thread LRU and never retains native geometry handles.
  std::shared_ptr<MeshIdentity> mesh_identity=std::make_shared<MeshIdentity>();
  // Only an original local thread result has this opaque factory identity.
  // Geometry-changing operations/imports never inherit it.
  std::shared_ptr<const ThreadFactoryTag> thread_factory_tag = {};
};
std::unique_ptr<Shape> extrude(double width, double depth, double height);
std::unique_ptr<Shape> cut(const Shape &, double x, double width, double depth,
                           double height);
Mesh mesh(const Shape &, double deflection);
Mesh tessellate_fresh(const Shape &,double,double,uint32_t);
bool remember_validated_mesh(const Shape &,double,double,uint32_t,const Mesh &);
MeshCacheStats mesh_cache_stats();
void reset_mesh_cache();
void mesh_cache_test_limits(uint32_t,uint64_t,uint64_t);
bool shares_mesh_identity(const Shape &,const Shape &);
double volume(const Shape &);
uint64_t volume_integration_count(const Shape &);
bool shares_volume_cache(const Shape &,const Shape &);
uint64_t face_integration_count(const Shape &);
bool shares_face_cache(const Shape &,const Shape &);
bool face_cache_concurrent_probe(const Shape &);
bool volume_cache_concurrent_probe(const Shape &);
ThreadFactoryStats thread_factory_stats();
bool remember_validated_thread(const Shape &);
bool remember_thread_prototype(const Shape &,const ThreadData &);
void reset_thread_factory_cache();
void thread_factory_test_limits(uint32_t entries,uint64_t bytes);
bool thread_factory_contains(const ThreadData &);
bool valid(const Shape &);
void record_validated_shape(Shape &);
uint64_t validity_check_count(const Shape &);
bool shares_validity_cache(const Shape &,const Shape &);
bool validity_cache_concurrent_probe(const Shape &);
bool validity_cache_rejection_probe();
void write_step(const Shape &, rust::Str path);
std::unique_ptr<Shape> read_step(rust::Str path);
rust::Vec<StepSolidData> step_solids(rust::Str path);
rust::String shape_to_brep(const Shape &);
std::unique_ptr<Shape> shape_from_brep(rust::Str text);
std::unique_ptr<Shape> shape_from_thread_prototype(rust::Str,rust::Slice<const Seed>);
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
