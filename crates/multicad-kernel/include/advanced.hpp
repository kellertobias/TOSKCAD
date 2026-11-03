#pragma once
#include "bridge.hpp"
namespace multicad::kernel {
struct CompoundLink { uint8_t kind; uint32_t input; TopoDS_Shape copied; TopAbs_Orientation orientation; };
struct CompoundMember {
  TopoDS_Shape copied;
  std::vector<CompoundLink> links;
  std::optional<std::vector<CachedFace>> faces;
  std::optional<VolumeCertificate> volume;
  uint32_t solids=0;
  bool exact_copy=true;
  bool same_world=true;
  std::optional<std::array<double,6>> bounds;
  std::shared_ptr<MeshIdentity> mesh_identity;
};
struct CompoundJob { std::vector<CompoundMember> members; std::size_t topology_count = 0; };
std::unique_ptr<CompoundJob> compound_job();
void compound_add(CompoundJob &,const Shape &);
std::unique_ptr<Shape> compound_finish(CompoundJob &);
bool compound_property_fallback_probe(const Shape &);
std::unique_ptr<Shape> solid_component_at(const Shape &,uint32_t index);
std::unique_ptr<Shape> shell_solid(const Shape &,rust::Slice<const uint32_t>,double thickness,double tolerance);
std::unique_ptr<Shape> draft_solid(const Shape &,rust::Slice<const uint32_t>,const Point3 &direction,const Frame3 &neutral,double angle);
}
