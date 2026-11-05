#include "bridge.hpp"
#include "measure.hpp"
#include <tuple>
#include <cstring>
#include <cstdio>
#include <cstdlib>
#include <limits>
#include "multicad-kernel/src/lib.rs.h"
#include <BRepAlgoAPI_Cut.hxx>
#include <BRepAlgoAPI_BooleanOperation.hxx>
#include <gp_Pln.hxx>
#include <BRepAlgoAPI_Fuse.hxx>
#include <BRepAlgoAPI_Common.hxx>
#include <BRepBuilderAPI_Copy.hxx>
#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_MakeWire.hxx>
#include <BRepBuilderAPI_Transform.hxx>
#include <BRepBuilderAPI_GTransform.hxx>
#include <BRepFilletAPI_MakeFillet.hxx>
#include <BRepFilletAPI_MakeChamfer.hxx>
#include <BRepAdaptor_Surface.hxx>
#include <BRepAdaptor_Curve.hxx>
#include <GCPnts_QuasiUniformDeflection.hxx>
#include <BRepBndLib.hxx>
#include <BRepExtrema_DistShapeShape.hxx>
#include <Precision.hxx>
#include <Bnd_Box.hxx>
#include <GC_MakeArcOfCircle.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <BRepPrimAPI_MakeHalfSpace.hxx>
#include <StepData_StepModel.hxx>
#include <gp_Circ.hxx>
#include <gp_Ax3.hxx>
#include <gp_Trsf.hxx>
#include <gp_GTrsf.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_MakePolygon.hxx>
#include <BRepCheck_Analyzer.hxx>
#include <BRepCheck_Shell.hxx>
#include <Poly_Triangulation.hxx>
#include <TopoDS_Shell.hxx>
#include <TopoDS_Vertex.hxx>
#include <BRepGProp.hxx>
#include <BRepMesh_IncrementalMesh.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakePrism.hxx>
#include <BRep_Tool.hxx>
#include <GProp_GProps.hxx>
#include <Message.hxx>
#include <Message_Messenger.hxx>
#include <Message_PrinterOStream.hxx>
#include <STEPControl_Reader.hxx>
#include <STEPControl_Writer.hxx>
#include <Standard_Failure.hxx>
#include <TopExp_Explorer.hxx>
#include <BRepTools.hxx>
#include <BRep_Builder.hxx>
#include <StepRepr_RepresentationItem.hxx>
#include <TCollection_HAsciiString.hxx>
#include <XSControl_TransferReader.hxx>
#include <XSControl_WorkSession.hxx>
#include <sstream>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopTools_IndexedDataMapOfShapeListOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Solid.hxx>
#include <TopoDS_Shell.hxx>
#include <array>
#include <iostream>
#include <thread>
#include <atomic>
#include <algorithm>
#include <exception>
#include <mutex>
#include <set>
#include <stdexcept>

namespace multicad::kernel {
// Standard_Failure is translated explicitly; no native exception crosses FFI.
template <class F> auto guard(F fn) -> decltype(fn()) {
  try {
    return fn();
  } catch (const Standard_Failure &e) {
    throw std::runtime_error(e.GetMessageString() ? e.GetMessageString()
                                                  : "OCCT operation failed");
  }
}
static std::unique_ptr<Shape> checked(const TopoDS_Shape &s) {
  if (s.IsNull() || !BRepCheck_Analyzer(s).IsValid())
    throw std::runtime_error("OCCT produced an invalid shape");
  auto result=std::make_unique<Shape>(Shape{s, {}, {}});
  record_validated_shape(*result);
  return result;
}
static const std::array<TopAbs_ShapeEnum, 3> topologyKinds = {
    TopAbs_FACE, TopAbs_EDGE, TopAbs_VERTEX};
static TopTools_IndexedMapOfShape topology_map(const TopoDS_Shape &s,
                                             uint8_t kind) {
  TopTools_IndexedMapOfShape map;
  TopExp::MapShapes(s, topologyKinds.at(kind), map);
  if (map.Extent() > 100000)
    throw std::runtime_error("Topology budget exceeded");
  return map;
}
// These ordinals live only within this result. Rust assigns persistent source
// intent and follows these exact OCCT history relations; no geometric matching.
template <class Operation>
static void record_history(Shape &result, const Shape &input, uint8_t operand,
                           Operation &operation) {
  for (uint8_t kind = 0; kind < 3; ++kind) {
    auto in = topology_map(input.value, kind);
    auto out = topology_map(result.value, kind);
    for (int i = 1; i <= in.Extent(); ++i) {
      std::set<int> descendants;
      auto append = [&](const TopoDS_Shape &candidate) {
        auto index = out.FindIndex(candidate);
        if (index > 0)
          descendants.insert(index);
      };
      append(in(i));
      for (const auto &candidate : operation.Modified(in(i))) append(candidate);
      for (const auto &candidate : operation.Generated(in(i))) append(candidate);
      for (auto index : descendants) {
        if (result.ancestry.size() >= 1000000)
          throw std::runtime_error("Topology history budget exceeded");
        result.ancestry.push_back(NativeAncestry{kind, operand,
            static_cast<uint32_t>(index - 1), static_cast<uint32_t>(i - 1)});
      }
    }
  }
}

// Compose the deep-copy history with treatment history. No enumeration-order
// assumption connects the original and copied shapes, and the caller's input
// is never given to the mutating native builder.
template <class Operation>
static std::unique_ptr<Shape> finish_treatment(
    const Shape &input, BRepBuilderAPI_Copy &copy, Operation &operation,
    const std::vector<TopoDS_Edge> &selected) {
  operation.Build();
  if (!operation.IsDone())
    throw std::runtime_error("Edge treatment failed; no partial result was accepted");
  auto result = checked(operation.Shape());
  for (uint8_t kind = 0; kind < 3; ++kind) {
    const auto source = topology_map(input.value, kind);
    const auto output = topology_map(result->value, kind);
    for (int i = 1; i <= source.Extent(); ++i) {
      const auto copied = copy.ModifiedShape(source(i));
      if (copied.IsNull()) throw std::runtime_error("Edge-treatment copy history is incomplete");
      std::set<int> descendants;
      auto append = [&](const TopoDS_Shape &candidate) {
        const int index = output.FindIndex(candidate);
        if (index > 0) descendants.insert(index);
      };
      append(copied);
      for (const auto &shape : operation.Modified(copied)) append(shape);
      for (const auto &shape : operation.Generated(copied)) append(shape);
      for (int index : descendants) {
        if (result->ancestry.size() >= 1000000)
          throw std::runtime_error("Edge-treatment history budget exceeded");
        result->ancestry.push_back(NativeAncestry{kind, 0,
            static_cast<uint32_t>(index - 1), static_cast<uint32_t>(i - 1)});
      }
    }
  }
  const auto faces = topology_map(result->value, 0);
  std::set<std::pair<uint32_t, uint32_t>> seeds;
  for (uint32_t i = 0; i < selected.size(); ++i) {
    auto append_generated = [&](const TopoDS_Shape &source) {
      for (const auto &shape : operation.Generated(source)) {
        if (shape.ShapeType() != TopAbs_FACE) continue;
        const int face = faces.FindIndex(shape);
        if (face > 0) seeds.emplace(static_cast<uint32_t>(face - 1), i);
      }
    };
    append_generated(selected[i]);
    // Blend corners can be Generated(vertex), rather than Generated(edge).
    // Qualify those exact vertices by their selected incident edge origins.
    for (TopExp_Explorer vertex(selected[i], TopAbs_VERTEX); vertex.More(); vertex.Next())
      append_generated(vertex.Current());
  }
  for (const auto &[face, source] : seeds)
    result->seeds.push_back(NativeSeed{face, source, 3});
  return result;
}

std::unique_ptr<Shape> treat_edges_sided(const Shape &input,
    rust::Slice<const uint32_t> indices, rust::Slice<const uint32_t> sides, uint8_t mode, double size, double second) {
  return guard([&]() -> std::unique_ptr<Shape> {
    if (indices.empty() || indices.size() > 128 || mode > 3 ||
        !std::isfinite(size) || size <= 0. || size > 1e7)
      throw std::runtime_error("Invalid bounded edge-treatment request");
    if (mode >= 2 && (sides.size() != indices.size() || !std::isfinite(second) || second <= 0. ||
        (mode == 2 && second > 1e7) || (mode == 3 && second >= std::acos(-1.) / 2.)))
      throw std::runtime_error("Invalid sided chamfer parameters");
    const auto edges = topology_map(input.value, 1);
    const auto faces = topology_map(input.value, 0);
    TopTools_IndexedDataMapOfShapeListOfShape adjacency;
    TopExp::MapShapesAndAncestors(input.value, TopAbs_EDGE, TopAbs_FACE, adjacency);
    BRepBuilderAPI_Copy copy(input.value, true, false);
    if (!copy.IsDone()) throw std::runtime_error("Could not isolate edge-treatment input");
    std::vector<TopoDS_Edge> selected;
    std::vector<TopoDS_Face> referenceFaces;
    std::set<uint32_t> unique;
    for (uint32_t index : indices) {
      if (index >= static_cast<uint32_t>(edges.Extent()) || !unique.insert(index).second)
        throw std::runtime_error("Selected edge is absent or duplicated");
      const auto edge = TopoDS::Edge(edges(static_cast<int>(index + 1)));
      if (BRep_Tool::Degenerated(edge) || !adjacency.Contains(edge))
        throw std::runtime_error("Degenerate or unbounded edges cannot be treated");
      TopTools_IndexedMapOfShape incident;
      for (const auto &face : adjacency.FindFromKey(edge)) incident.Add(face);
      if (incident.Extent() != 2)
        throw std::runtime_error("Treatment needs an edge between two distinct faces; seams are unsupported");
      const auto copied = copy.ModifiedShape(edge);
      if (copied.IsNull() || copied.ShapeType() != TopAbs_EDGE)
        throw std::runtime_error("Selected edge copy history is incomplete");
      if (mode >= 2) {
        const uint32_t faceIndex = sides[selected.size()];
        if (faceIndex >= static_cast<uint32_t>(faces.Extent()) || !incident.Contains(faces(static_cast<int>(faceIndex+1))))
          throw std::runtime_error("The explicit chamfer reference face is not adjacent to the selected edge");
        referenceFaces.push_back(TopoDS::Face(copy.ModifiedShape(faces(static_cast<int>(faceIndex+1)))));
      }
      selected.push_back(TopoDS::Edge(copied));
    }
    if (mode == 0) {
      BRepFilletAPI_MakeFillet operation(copy.Shape());
      for (const auto &edge : selected) operation.Add(size, edge);
      for (const auto &edge : selected)
        if (operation.Contour(edge) == 0)
          throw std::runtime_error("Selected edge is not a supported fillet contour");
      return finish_treatment(input, copy, operation, selected);
    }
    BRepFilletAPI_MakeChamfer operation(copy.Shape());
    for (size_t i=0; i<selected.size(); ++i) {
      if (mode == 1) operation.Add(size, selected[i]);
      else if (mode == 2) operation.Add(size, second, selected[i], referenceFaces[i]);
      else operation.AddDA(size, second, selected[i], referenceFaces[i]);
    }
    for (const auto &edge : selected)
      if (operation.Contour(edge) == 0)
        throw std::runtime_error("Selected edge is not a supported chamfer contour");
    return finish_treatment(input, copy, operation, selected);
  });
}
std::unique_ptr<Shape> treat_edges(const Shape &input, rust::Slice<const uint32_t> indices, uint8_t mode, double size) {
  if (mode > 1) throw std::runtime_error("Explicit face sides are required for asymmetric chamfers");
  return treat_edges_sided(input, indices, indices, mode, size, 0.);
}
std::unique_ptr<Shape> extrude(double w, double d, double h) {
  return guard([&] {
    BRepBuilderAPI_MakePolygon polygon;
    polygon.Add(gp_Pnt(0, 0, 0));
    polygon.Add(gp_Pnt(w, 0, 0));
    polygon.Add(gp_Pnt(w, d, 0));
    polygon.Add(gp_Pnt(0, d, 0));
    polygon.Close();
    BRepBuilderAPI_MakeFace face(polygon.Wire());
    BRepPrimAPI_MakePrism prism(face.Face(), gp_Vec(0, 0, h));
    if (!prism.IsDone())
      throw std::runtime_error("Extrusion failed");
    return checked(prism.Shape());
  });
}
std::unique_ptr<Shape> cut(const Shape &shape, double x, double w, double d,
                           double h) {
  return guard([&] {
    auto tool = BRepPrimAPI_MakeBox(gp_Pnt(x, 0, 0), w, d, h).Shape();
    BRepAlgoAPI_Cut operation;
    TopTools_ListOfShape arguments, tools;
    arguments.Append(shape.value);
    tools.Append(tool);
    operation.SetArguments(arguments);
    operation.SetTools(tools);
    operation.SetNonDestructive(true);
    operation.Build();
    if (!operation.IsDone())
      throw std::runtime_error("Boolean failed");
    return checked(operation.Shape());
  });
}
void record_validated_shape(Shape &s) {
  // Caller has just completed BRepCheck_Analyzer on this new native shape.
  std::lock_guard<std::mutex> lock(s.validity_cache->mutex);
  s.validity_cache->accepted=true;++s.validity_cache->checks;
}
bool valid(const Shape &s) {
  return guard([&] {
    std::lock_guard<std::mutex> lock(s.validity_cache->mutex);
    if(s.validity_cache->accepted)return true;
    if(s.value.IsNull())return false;
    ++s.validity_cache->checks;
    const bool accepted=BRepCheck_Analyzer(s.value).IsValid();
    if(accepted)s.validity_cache->accepted=true;
    return accepted;
  });
}
uint64_t validity_check_count(const Shape &s) {
  std::lock_guard<std::mutex> lock(s.validity_cache->mutex);
  return s.validity_cache->checks;
}
bool shares_validity_cache(const Shape &a,const Shape &b) {return a.validity_cache==b.validity_cache;}
bool validity_cache_concurrent_probe(const Shape &source) {
  Shape cold{source.value,{},{}};const auto shared=share_shape(cold);
  std::array<bool,8> results{};std::array<std::exception_ptr,8> errors{};
  std::vector<std::thread> workers;std::atomic<bool> start{false};
  try {for(int i=0;i<8;++i)workers.emplace_back([&,i] {
    while(!start.load(std::memory_order_acquire))std::this_thread::yield();
    try {results[i]=valid(i%2?*shared:cold);}catch(...) {errors[i]=std::current_exception();}
  });}catch(...) {start.store(true,std::memory_order_release);for(auto &w:workers)w.join();throw;}
  start.store(true,std::memory_order_release);for(auto &w:workers)w.join();
  for(int i=0;i<8;++i)if(errors[i])std::rethrow_exception(errors[i]);
  return std::all_of(results.begin(),results.end(),[](bool value){return value;})&&validity_check_count(cold)==1;
}
bool validity_cache_rejection_probe() {
  BRep_Builder builder;TopoDS_Solid solid;TopoDS_Shell shell;
  builder.MakeSolid(solid);builder.MakeShell(shell);
  const auto box=BRepPrimAPI_MakeBox(1.,1.,1.).Shape();
  TopExp_Explorer first(box,TopAbs_FACE);builder.Add(shell,first.Current());builder.Add(solid,shell);
  Shape bad{solid,{},{}};
  return !valid(bad)&&!valid(bad)&&validity_check_count(bad)==2&&!bad.validity_cache->accepted;
}
// Match the pinned public GK wrapper's per-shell reference, including repeated
// vertex occurrences and its no-vertex triangulation fallback.
static gp_Pnt volume_shell_reference(const TopoDS_Shape &shell) {
  gp_XYZ sum(0,0,0);size_t count=0;
  for(TopExp_Explorer vertices(shell,TopAbs_VERTEX);vertices.More();vertices.Next()) {
    sum+=BRep_Tool::Pnt(TopoDS::Vertex(vertices.Current())).XYZ();++count;
  }
  if(count)sum/=static_cast<double>(count);
  else for(TopExp_Explorer faces(shell,TopAbs_FACE);faces.More();faces.Next()) {
    TopLoc_Location location;
    const auto &triangulation=BRep_Tool::Triangulation(TopoDS::Face(faces.Current()),location);
    if(!triangulation.IsNull()&&triangulation->NbNodes()>0) {
      sum=triangulation->Node(1).XYZ();
      if(!location.IsIdentity())location.Transformation().Transforms(sum);
      break;
    }
  }
  return gp_Pnt(sum);
}
static std::pair<double,double> point_equivalent_volume(const TopoDS_Shape &shape) {
  const gp_Dir first(1.,std::sqrt(2.),std::sqrt(3.)),second(-std::sqrt(2.),1.,0.);
  const std::array<gp_Dir,3> directions={first,second,first.Crossed(second)};
  long double total_mass=0,total_absolute_error=0;bool closed=false;
  // Keep the original closed-shell selection, orientation and multiplicity.
  for(TopExp_Explorer shells(shape,TopAbs_SHELL);shells.More();shells.Next()) {
    const auto &shell=shells.Current();BRepCheck_Shell checker(TopoDS::Shell(shell));
    if(checker.Closed(false)!=BRepCheck_NoError)continue;
    closed=true;const auto reference=volume_shell_reference(shell);
    long double shell_mass=0,shell_absolute_error=0;
    for(const auto &direction:directions) {
      // In pinned VinertGK, gp_Pln coefficient D gives flux n.dot(x)-D.
      const double coefficient=direction.X()*reference.X()+direction.Y()*reference.Y()+direction.Z()*reference.Z();
      GProp_GProps properties;
      const double reported=BRepGProp::VolumePropertiesGK(shell,properties,gp_Pln(direction.X(),direction.Y(),direction.Z(),coefficient),1e-10,true,true);
      const double mass=properties.Mass();
      if(!std::isfinite(reported)||reported<0||!std::isfinite(mass))throw VolumeIntegrationFailure();
      // The public wrapper divides by mass only when mass>Epsilon(1).
      // An inward void shell keeps its signed mass and absolute error.
      const long double absolute_error=mass>Epsilon(1.)?static_cast<long double>(reported)*mass:reported;
      shell_mass+=static_cast<long double>(mass)/3;
      shell_absolute_error+=absolute_error/3;
    }
    total_mass+=shell_mass;total_absolute_error+=shell_absolute_error;
  }
  if(!closed||!std::isfinite(total_mass)||total_mass<=0||!std::isfinite(total_absolute_error))throw VolumeIntegrationFailure();
  return {static_cast<double>(total_mass),static_cast<double>(total_absolute_error/total_mass)};
}
double volume(const Shape &s) {
  return guard([&] {
    std::lock_guard<std::mutex> lock(s.volume_cache->mutex);
    if(s.volume_cache->certificate) {
      const auto &certificate=*s.volume_cache->certificate;
      if(!std::isfinite(certificate.relative_error)||certificate.relative_error<0.||certificate.relative_error>1e-7)
        throw VolumeIntegrationFailure();
      return certificate.mass;
    }
    ++s.volume_cache->integrations;
    // Three orthonormal fluxes at the SAME per-shell reference average to
    // (x-reference).N/3, preserving the original point integrand even when
    // the actual trimmed boundary has a small residual directional flux.
    // Requested tolerance, spline spans and the accepted error remain fixed.
    const auto measured=point_equivalent_volume(s.value);
    double mass=measured.first,relativeError=measured.second;
    if (!std::isfinite(relativeError) || relativeError < 0.)
      throw VolumeIntegrationFailure();
    if(relativeError>1e-7) {
      // Independent directional solves can report a larger quadrature error
      // than the original point formulation on the same closed native solid.
      // Retry that original integration, never the rejected mass or its guard.
      GProp_GProps properties;
      const double pointError=BRepGProp::VolumePropertiesGK(s.value,properties,1e-10,true,true);
      if(const auto *profile=std::getenv("MULTICAD_PROFILE_NATIVE_VOLUME");profile&&std::strcmp(profile,"0")!=0)
        std::fprintf(stderr,"multicad.native.volume method=point_retry basis_error=%.17g point_error=%.17g\n",relativeError,pointError);
      mass=properties.Mass();relativeError=pointError;
      if(!std::isfinite(mass)||mass<=0.||!std::isfinite(relativeError)||relativeError<0.||relativeError>1e-7)
        throw VolumeIntegrationFailure();
    }
    if(std::isfinite(mass)&&mass>0.)s.volume_cache->certificate=VolumeCertificate{mass,relativeError};
    return mass;
  });
}
uint64_t volume_integration_count(const Shape &s) {
  std::lock_guard<std::mutex> lock(s.volume_cache->mutex);
  return s.volume_cache->integrations;
}
bool shares_volume_cache(const Shape &a,const Shape &b) {
  return a.volume_cache==b.volume_cache;
}
// Private regression support: native resources remain inside the audited
// adapter. Rust Solid remains !Send/!Sync and never crosses worker threads.
bool volume_cache_concurrent_probe(const Shape &source) {
  Shape cold{source.value,{},{}};
  const auto shared=share_shape(cold);
  std::array<double,8> values;
  std::array<std::exception_ptr,8> errors;
  std::vector<std::thread> workers;std::atomic<bool> start{false};
  try {for(int i=0;i<8;++i)workers.emplace_back([&,i] {
    while(!start.load(std::memory_order_acquire))std::this_thread::yield();
    try {
      for(int repeat=0;repeat<16;++repeat) {
        const double value=volume(i%2?*shared:cold);
        if(repeat&&value!=values[i])throw std::runtime_error("Shared native volume changed");
        values[i]=value;
      }
    } catch(...) {errors[i]=std::current_exception();}
  });} catch(...) {
    start.store(true,std::memory_order_release);
    for(auto &worker:workers)worker.join();
    throw;
  }
  start.store(true,std::memory_order_release);
  for(auto &worker:workers)worker.join();
  for(int i=0;i<8;++i)if(errors[i])std::rethrow_exception(errors[i]);
  for(int i=1;i<8;++i)if(values[i]!=values[0])return false;
  return shares_volume_cache(cold,*shared)&&volume_integration_count(cold)==1&&
      std::isfinite(values[0])&&values[0]>0.;
}
Mesh mesh(const Shape &s, double tolerance) {
  return tessellate(s, tolerance, 0.5, 1000000);
}
struct MeshBuffer {
  // Exact-sized numeric arrays: no geometric/native handles or unaccounted
  // vector growth, including while a new bounded receipt is being prepared.
  std::unique_ptr<double[]> vertices;
  std::unique_ptr<uint32_t[]> indices,faces;
  size_t vertex_size=0,index_size=0,face_size=0;
};
namespace {
constexpr uint64_t meshTotalBytes=32*1024*1024,meshReceiptBytes=4*1024*1024;
constexpr uint32_t meshEntries=16;
uint64_t mesh_bits(double value) {uint64_t bits;std::memcpy(&bits,&value,sizeof(bits));return bits;}
MeshKey mesh_key(double linear,double angular,uint32_t triangles) {return {mesh_bits(linear),mesh_bits(angular),triangles};}
struct MeshEntry {std::weak_ptr<MeshIdentity> identity;MeshKey key;std::shared_ptr<const MeshBuffer> data;uint64_t bytes,used;};
struct MeshMemo {
  std::vector<MeshEntry> entries;
  MeshCacheStats stats{};
  uint64_t bytes=0,clock=0,total_limit=meshTotalBytes,receipt_limit=meshReceiptBytes;
  uint32_t entry_limit=meshEntries;
};
MeshMemo &mesh_memo() {thread_local MeshMemo memo;return memo;}
void purge_meshes(MeshMemo &memo) {
  for(auto it=memo.entries.begin();it!=memo.entries.end();) {
    if(it->identity.expired()) {memo.bytes-=it->bytes;it=memo.entries.erase(it);}else ++it;
  }
}
Mesh detached_mesh(const MeshBuffer &buffer) {
  Mesh result;
  result.vertices.reserve(buffer.vertex_size);result.indices.reserve(buffer.index_size);result.face_indices.reserve(buffer.face_size);
  for(size_t i=0;i<buffer.vertex_size;++i)result.vertices.push_back(buffer.vertices[i]);
  for(size_t i=0;i<buffer.index_size;++i)result.indices.push_back(buffer.indices[i]);
  for(size_t i=0;i<buffer.face_size;++i)result.face_indices.push_back(buffer.faces[i]);
  return result;
}
}
static Mesh tessellate_impl(const Shape &s,double tolerance,double angle,uint32_t limit,bool cached) {
  return guard([&] {
    auto &memo=mesh_memo();++memo.stats.requests;purge_meshes(memo);
    const auto key=mesh_key(tolerance,angle,limit);
    if(cached)for(auto &entry:memo.entries)if(entry.identity.lock()==s.mesh_identity&&entry.key==key) {
      auto result=detached_mesh(*entry.data);entry.used=++memo.clock;++memo.stats.hits;return result;
    }
    ++memo.stats.builds;
    // Meshing mutates triangulations. Detach geometry so a display request
    // cannot modify a cached modeling operand or its manufacturing geometry.
    BRepBuilderAPI_Copy copy(s.value, true, false);
    auto display = copy.Shape();
    auto exactFaces = topology_map(s.value, 0);
    // A valid curved face can have an open discretized wire at a coarse
    // deflection. Retry a bounded finer mesh on this detached copy; every
    // exact face must still contribute triangles. The requested tolerances
    // remain upper bounds, and the same triangle/index budget applies.
    bool complete=false;
    double meshTolerance=tolerance,meshAngle=angle;
    for(int attempt=0;attempt<3;++attempt) {
      if(attempt)BRepTools::Clean(display);
      BRepMesh_IncrementalMesh mesher(display,meshTolerance,false,meshAngle,false);
      if(!mesher.IsDone())throw std::runtime_error("Tessellation failed");
      complete=true;uint64_t count=0;
      for(int i=1;i<=exactFaces.Extent();++i) {
        auto face=TopoDS::Face(copy.ModifiedShape(exactFaces(i)));
        TopLoc_Location location;auto triangulation=BRep_Tool::Triangulation(face,location);
        if(triangulation.IsNull()||triangulation->NbTriangles()==0)complete=false;
        else count+=triangulation->NbTriangles();
      }
      if(count>limit)throw std::runtime_error("Display mesh exceeds triangle/index budget");
      if(complete)break;
      meshTolerance*=.25;meshAngle*=.5;
    }
    if(!complete)throw std::runtime_error("Face missing triangulation after bounded mesh refinement");
    Mesh out;
    uint64_t triangles = 0;
    for (int faceIndex = 1; faceIndex <= exactFaces.Extent(); ++faceIndex) {
      auto face = TopoDS::Face(copy.ModifiedShape(exactFaces(faceIndex)));
      // The copy's modified sub-shape need not carry the orientation the face
      // has inside the solid; the exact topology map does. Winding follows it
      // so the tessellation is a consistently outward-oriented closed surface.
      const bool reversed = exactFaces(faceIndex).Orientation() == TopAbs_REVERSED;
      TopLoc_Location location;
      auto tri = BRep_Tool::Triangulation(face, location);
      if (tri.IsNull())
        throw std::runtime_error("Face missing triangulation");
      triangles += tri->NbTriangles();
      if (triangles > limit || out.vertices.size() / 3 + tri->NbNodes() > UINT32_MAX)
        throw std::runtime_error("Display mesh exceeds triangle/index budget");
      const auto offset = static_cast<uint32_t>(out.vertices.size() / 3);
      for (int i = 1; i <= tri->NbNodes(); ++i) {
        auto p = tri->Node(i).Transformed(location.Transformation());
        out.vertices.push_back(p.X());
        out.vertices.push_back(p.Y());
        out.vertices.push_back(p.Z());
      }
      for (int i = 1; i <= tri->NbTriangles(); ++i) {
        int a, b, c;
        tri->Triangle(i).Get(a, b, c);
        if (reversed)
          std::swap(b, c);
        out.indices.push_back(offset + a - 1);
        out.indices.push_back(offset + b - 1);
        out.indices.push_back(offset + c - 1);
        out.face_indices.push_back(static_cast<uint32_t>(faceIndex - 1));
      }
    }
    return out;
  });
}
Mesh tessellate(const Shape &s,double tolerance,double angle,uint32_t limit) {return tessellate_impl(s,tolerance,angle,limit,true);}
Mesh tessellate_fresh(const Shape &s,double tolerance,double angle,uint32_t limit) {return tessellate_impl(s,tolerance,angle,limit,false);}
bool remember_validated_mesh(const Shape &s,double tolerance,double angle,uint32_t limit,const Mesh &mesh) {
  // Called only after the complete Rust TriangleMesh checks. Cache operations
  // cannot convert accepted display data into a geometry/modeling failure.
  auto &memo=mesh_memo();purge_meshes(memo);
  const auto key=mesh_key(tolerance,angle,limit);
  for(auto &entry:memo.entries)if(entry.identity.lock()==s.mesh_identity&&entry.key==key) {entry.used=++memo.clock;return true;}
  const auto unretained=[&](){++memo.stats.unretained;return false;};
  if(mesh.vertices.size()>meshReceiptBytes/sizeof(double)||mesh.indices.size()>meshReceiptBytes/sizeof(uint32_t)||mesh.face_indices.size()>meshReceiptBytes/sizeof(uint32_t))return unretained();
  const uint64_t bytes=mesh.vertices.size()*sizeof(double)+(mesh.indices.size()+mesh.face_indices.size())*sizeof(uint32_t);
  if(!memo.entry_limit||!bytes||bytes>memo.receipt_limit||bytes>memo.total_limit||mesh.indices.size()/3>limit)return unretained();
  try {
    // Free budget before allocating a pending numeric receipt. Failure may
    // evict optional old data but never invalidates the accepted caller mesh.
    memo.entries.reserve(memo.entry_limit);
    while(memo.entries.size()>=memo.entry_limit||memo.bytes>memo.total_limit-bytes) {
      auto oldest=std::min_element(memo.entries.begin(),memo.entries.end(),[](const auto &a,const auto &b){return a.used<b.used;});
      memo.bytes-=oldest->bytes;memo.entries.erase(oldest);++memo.stats.evictions;
    }
    auto data=std::make_shared<MeshBuffer>();
    data->vertex_size=mesh.vertices.size();data->index_size=mesh.indices.size();data->face_size=mesh.face_indices.size();
    data->vertices=std::make_unique<double[]>(data->vertex_size);
    data->indices=std::make_unique<uint32_t[]>(data->index_size);
    data->faces=std::make_unique<uint32_t[]>(data->face_size);
    std::copy(mesh.vertices.begin(),mesh.vertices.end(),data->vertices.get());
    std::copy(mesh.indices.begin(),mesh.indices.end(),data->indices.get());
    std::copy(mesh.face_indices.begin(),mesh.face_indices.end(),data->faces.get());
    memo.entries.push_back({s.mesh_identity,key,data,bytes,++memo.clock});memo.bytes+=bytes;
    {std::lock_guard<std::mutex> lock(s.mesh_identity->mutex);s.mesh_identity->key=key;s.mesh_identity->receipt=data;}
    return true;
  }catch(const std::bad_alloc &) {return unretained();}
}
MeshCacheStats mesh_cache_stats() {auto &memo=mesh_memo();purge_meshes(memo);auto result=memo.stats;result.entries=memo.entries.size();result.bytes=memo.bytes;return result;}
void reset_mesh_cache() {mesh_memo()=MeshMemo{};}
void mesh_cache_test_limits(uint32_t entries,uint64_t total,uint64_t receipt) {
  if(entries>meshEntries||total>meshTotalBytes||receipt>meshReceiptBytes)throw std::runtime_error("Mesh test limits cannot exceed production bounds");
  reset_mesh_cache();auto &memo=mesh_memo();memo.entry_limit=entries;memo.total_limit=total;memo.receipt_limit=receipt;
}
bool shares_mesh_identity(const Shape &a,const Shape &b) {return a.mesh_identity==b.mesh_identity;}
// STEP reader/writer initialization and shape-fix parameters use shared OCCT
// translator state. Acquire before constructing either translator and retain
// through destruction; unrelated geometry and owned handles remain independent.
static std::mutex step_exchange_mutex;
void write_step(const Shape &s, rust::Str path) {
  guard([&] {
    const std::lock_guard<std::mutex> exchange_lock(step_exchange_mutex);
    STEPControl_Writer writer;
    writer.Model()->SetLocalLengthUnit(1.0);
    writer.Model()->SetWriteLengthUnit(1.0);
    if (writer.Transfer(s.value, STEPControl_AsIs) != IFSelect_RetDone ||
        writer.Write(std::string(path).c_str()) != IFSelect_RetDone)
      throw std::runtime_error("STEP write failed");
  });
}
void route_native_output_to_stderr() {
  static std::once_flag once;
  std::call_once(once, [] {
    const Handle(Message_Messenger) &messenger = Message::DefaultMessenger();
    messenger->RemovePrinters(STANDARD_TYPE(Message_PrinterOStream));
    messenger->AddPrinter(
        new Message_PrinterOStream("cerr", Standard_False, Message_Warning));
    // Direct C++ stream writes; Rust writes to the file descriptor itself.
    std::cout.rdbuf(std::cerr.rdbuf());
  });
}
std::unique_ptr<Shape> read_step(rust::Str path) {
  return guard([&] {
    const std::lock_guard<std::mutex> exchange_lock(step_exchange_mutex);
    STEPControl_Reader reader;
    if (reader.ReadFile(std::string(path).c_str()) != IFSelect_RetDone)
      throw std::runtime_error("STEP read failed");
    reader.SetSystemLengthUnit(1.0);
    if (reader.TransferRoots() == 0)
      throw std::runtime_error("STEP contains no transferable geometry");
    return checked(reader.OneShape());
  });
}
// Every solid of a STEP file in millimeters, located as in the file, with the
// name of the STEP representation item that produced it when there is one.
rust::Vec<StepSolidData> step_solids(rust::Str path) {
  return guard([&] {
    const std::lock_guard<std::mutex> exchange_lock(step_exchange_mutex);
    STEPControl_Reader reader;
    if (reader.ReadFile(std::string(path).c_str()) != IFSelect_RetDone)
      throw std::runtime_error("STEP read failed");
    reader.SetSystemLengthUnit(1.0);
    if (reader.TransferRoots() == 0)
      throw std::runtime_error("STEP contains no transferable geometry");
    const TopoDS_Shape all = reader.OneShape();
    const Handle(XSControl_TransferReader) transfer =
        reader.WS()->TransferReader();
    rust::Vec<StepSolidData> out;
    for (TopExp_Explorer e(all, TopAbs_SOLID); e.More(); e.Next()) {
      if (!BRepCheck_Analyzer(e.Current()).IsValid())
        throw std::runtime_error("STEP contains an invalid solid");
      std::string name;
      if (!transfer.IsNull()) {
        const auto item = Handle(StepRepr_RepresentationItem)::DownCast(
            transfer->EntityFromShapeResult(e.Current(), 1));
        if (!item.IsNull() && !item->Name().IsNull())
          name = item->Name()->ToCString();
      }
      std::ostringstream text;
      BRepTools::Write(e.Current(), text);
      out.push_back(StepSolidData{rust::String(name), rust::String(text.str())});
    }
    return out;
  });
}
rust::String shape_to_brep(const Shape &s) {
  return guard([&] {
    std::ostringstream text;
    BRepTools::Write(s.value, text);
    return rust::String(text.str());
  });
}
std::unique_ptr<Shape> shape_from_brep(rust::Str text) {
  return guard([&] {
    std::istringstream stream{std::string(text)};
    TopoDS_Shape shape;
    BRep_Builder builder;
    BRepTools::Read(shape, stream, builder);
    if (shape.IsNull())
      throw std::runtime_error("BRep text holds no shape");
    return checked(shape);
  });
}
std::unique_ptr<Shape> shape_from_thread_prototype(rust::Str text,rust::Slice<const Seed> seeds) {
  return guard([&] {
    auto result=shape_from_brep(text);
    const auto map=topology_map(result->value,0);
    if(map.IsEmpty()||seeds.empty())throw std::runtime_error("Thread prototype has no face ancestry");
    std::vector<bool> covered(map.Extent(),false);
    std::set<std::tuple<uint32_t,uint32_t,uint8_t>> unique;
    for(const auto &seed:seeds) {
      const bool role=(seed.role<=1&&seed.source_index==0)||
          (seed.role==2&&seed.source_index/4<8&&seed.source_index%4<3)||
          (seed.role==3&&seed.source_index==0)||(seed.role==4&&seed.source_index<2);
      if(seed.face>=static_cast<uint32_t>(map.Extent())||!role||
          !unique.emplace(seed.face,seed.source_index,seed.role).second)
        throw std::runtime_error("Thread prototype has invalid or duplicate face ancestry");
      covered[seed.face]=true;
      result->seeds.push_back({seed.face,seed.source_index,seed.role});
    }
    if(std::find(covered.begin(),covered.end(),false)!=covered.end())
      throw std::runtime_error("Thread prototype face ancestry is incomplete");
    return result;
  });
}
static gp_Pnt point(const Point3 &p) { return gp_Pnt(p.x, p.y, p.z); }
static gp_Vec vec(const Point3 &p) { return gp_Vec(p.x, p.y, p.z); }
static Point3 data(const gp_Pnt &p) { return Point3{p.X(), p.Y(), p.Z()}; }
// Preserve the exact native planar descriptor path independently of numeric
// quadrature: persistent references consume these output-surface descriptors.
CachedFace describe_face(const TopoDS_Face &face,uint32_t index,
                                const gp_Pnt &center,double area) {
  CachedFace result{index,false,{0,0,0},{0,0,0},{center.X(),center.Y(),center.Z()},area};
  BRepAdaptor_Surface surface(face,true);
  if(surface.GetType()==GeomAbs_Plane) {
    const auto plane=surface.Plane();
    gp_Pnt location;gp_Vec du,dv;surface.D1(0.,0.,location,du,dv);
    gp_Dir normal(du.Crossed(dv));
    if(face.Orientation()==TopAbs_REVERSED)normal.Reverse();
    result.planar=true;
    result.origin={plane.Location().X(),plane.Location().Y(),plane.Location().Z()};
    result.normal={normal.X(),normal.Y(),normal.Z()};
  }
  return result;
}
static void carry_rigid_face_properties(Shape &result,const Shape &source,
    BRepBuilderAPI_Transform &operation,const gp_Trsf &transform) {
  // This optional optimization never changes geometry or accepts a partial
  // mapping. No source lock is held during native topology/surface queries.
  try {
    std::optional<std::vector<CachedFace>> accepted;
    {std::lock_guard<std::mutex> lock(source.face_cache->mutex);accepted=source.face_cache->value;}
    if(!accepted)return;
    const auto input=topology_map(source.value,0),output=topology_map(result.value,0);
    if(input.Extent()!=output.Extent()||accepted->size()!=static_cast<size_t>(input.Extent()))return;
    std::vector<CachedFace> completed(output.Extent());
    std::vector<bool> covered(output.Extent(),false);
    for(const auto &properties:*accepted) {
      if(properties.index>=static_cast<uint32_t>(input.Extent()))return;
      const auto &before=input(properties.index+1);
      const auto mapped=operation.ModifiedShape(before);
      if(mapped.IsNull()||mapped.ShapeType()!=TopAbs_FACE||mapped.Orientation()!=before.Orientation())return;
      const int index=output.FindIndex(mapped);
      if(index<=0||covered[index-1]||output(index).Orientation()!=mapped.Orientation())return;
      const gp_Pnt center=gp_Pnt(properties.centroid[0],properties.centroid[1],properties.centroid[2]).Transformed(transform);
      if(!std::isfinite(center.X())||!std::isfinite(center.Y())||!std::isfinite(center.Z()))return;
      completed[index-1]=describe_face(TopoDS::Face(output(index)),index-1,center,properties.area);
      covered[index-1]=true;
    }
    if(std::find(covered.begin(),covered.end(),false)!=covered.end())return;
    // Fresh result-owned certificate: only accepted area/centroid are carried.
    // Origins/normals are recomputed on the actual output native surface.
    result.face_cache->value=std::move(completed);
  } catch(const Standard_Failure &) { /* Leave a fresh cache for ordinary measurement. */ }
    catch(const std::bad_alloc &) { /* Optional retention must not invalidate geometry. */ }
}
static gp_Ax2 axes(const Frame3 &f) {
  return gp_Ax2(point(f.origin), gp_Dir(vec(f.u).Crossed(vec(f.v))),
                gp_Dir(vec(f.u)));
}
std::unique_ptr<Shape> cylinder(const Frame3 &f, double radius, double height) {
  return guard([&] {
    return checked(BRepPrimAPI_MakeCylinder(axes(f), radius, height).Shape());
  });
}
std::unique_ptr<Shape> profile_extrude(
    const Frame3 &f, rust::Slice<const CurveEdge> edges,
    rust::Slice<const uint32_t> offsets, double height) {
  return guard([&] {
    if (offsets.size() < 2 || offsets[0] != 0 ||
        offsets[offsets.size() - 1] != edges.size())
      throw std::runtime_error("Invalid profile wire offsets");
    BRepBuilderAPI_MakeFace face;
    std::vector<std::pair<TopoDS_Edge, uint32_t>> sourceEdges;
    for (size_t wireIndex = 0; wireIndex + 1 < offsets.size(); ++wireIndex) {
      if (offsets[wireIndex] >= offsets[wireIndex + 1] ||
          offsets[wireIndex + 1] > edges.size())
        throw std::runtime_error("Empty/invalid profile wire");
      BRepBuilderAPI_MakeWire wire;
      for (uint32_t i = offsets[wireIndex]; i < offsets[wireIndex + 1]; ++i) {
        const auto &e = edges[i];
        if (e.kind == 0) {
          wire.Add(BRepBuilderAPI_MakeEdge(point(e.a), point(e.b)).Edge());
        } else if (e.kind == 1) {
          GC_MakeArcOfCircle arc(point(e.a), point(e.mid), point(e.b));
          if (!arc.IsDone())
            throw std::runtime_error("Arc construction failed");
          wire.Add(BRepBuilderAPI_MakeEdge(arc.Value()).Edge());
        } else if (e.kind == 2) {
          auto ax = axes(f);
          ax.SetLocation(point(e.a));
          wire.Add(BRepBuilderAPI_MakeEdge(gp_Circ(ax, e.radius)).Edge());
        } else {
          throw std::runtime_error("Unknown profile edge");
        }
        if (!wire.IsDone())
          throw std::runtime_error("Profile edges are not connected");
        sourceEdges.emplace_back(wire.Edge(), e.source_index);
      }
      auto w = wire.Wire();
      if (wireIndex == 0) {
        face = BRepBuilderAPI_MakeFace(gp_Pln(gp_Ax3(axes(f))), w, true);
      } else {
        w.Reverse();
        face.Add(w);
      }
    }
    if (!face.IsDone() || !BRepCheck_Analyzer(face.Face()).IsValid())
      throw std::runtime_error(
          "Profile is invalid, self-intersecting, or has invalid holes");
    BRepPrimAPI_MakePrism prism(face.Face(), gp_Vec(axes(f).Direction()) * height);
    if (!prism.IsDone())
      throw std::runtime_error("Profile extrusion failed");
    auto result = checked(prism.Shape());
    auto faces = topology_map(result->value, 0);
    auto seed = [&](const TopoDS_Shape &shape, uint8_t role, uint32_t index) {
      const auto faceIndex = faces.FindIndex(shape);
      if (faceIndex > 0)
        result->seeds.push_back(NativeSeed{
            static_cast<uint32_t>(faceIndex - 1), index, role});
    };
    seed(prism.FirstShape(), 0, 0);
    seed(prism.LastShape(), 1, 0);
    for (const auto &entry : sourceEdges)
      for (const auto &generated : prism.Generated(entry.first))
        seed(generated, 2, entry.second);
    return result;
  });
}
std::unique_ptr<Shape> boolean_shapes(const Shape &a, const Shape &b,
                                      uint8_t operation, double fuzzy) {
  return guard([&]() -> std::unique_ptr<Shape> {
    BRepAlgoAPI_BooleanOperation op;
    TopTools_ListOfShape arguments, tools;
    arguments.Append(a.value);
    tools.Append(b.value);
    op.SetArguments(arguments);
    op.SetTools(tools);
    if (operation == 0)
      op.SetOperation(BOPAlgo_FUSE);
    else if (operation == 1)
      op.SetOperation(BOPAlgo_CUT);
    else if (operation == 2)
      op.SetOperation(BOPAlgo_COMMON);
    else
      throw std::runtime_error("Unknown Boolean operation");
    op.SetNonDestructive(true);
    op.SetRunParallel(false);
    op.SetFuzzyValue(fuzzy);
    op.Build();
    if (!op.IsDone() || op.HasErrors())
      throw std::runtime_error("Solid Boolean failed");
    auto result = op.Shape();
    if (!TopExp_Explorer(result, TopAbs_SOLID).More())
      return nullptr;
    auto owned = checked(result);
    record_history(*owned, a, 0, op);
    record_history(*owned, b, 1, op);
    return owned;
  });
}
// Exact bounded target material on one side of an infinite authored plane.
// The half-space is an operand only; no artificial cutting box enters a model.
std::unique_ptr<Shape> clip_half_space(const Shape &target, const Frame3 &frame,
                                      bool positive, double fuzzy) {
  return guard([&]() -> std::unique_ptr<Shape> {
    gp_Pln plane{gp_Ax3(axes(frame))};
    BRepBuilderAPI_MakeFace boundary(plane);
    if (!boundary.IsDone())
      throw std::runtime_error("Split plane construction failed");
    const auto point = plane.Location().Translated(
        gp_Vec(plane.Axis().Direction()) * (positive ? 1. : -1.));
    BRepPrimAPI_MakeHalfSpace half(boundary.Face(), point);
    if (!half.IsDone())
      throw std::runtime_error("Split half-space construction failed");
    BRepAlgoAPI_Common operation;
    TopTools_ListOfShape arguments, tools;
    arguments.Append(target.value);
    tools.Append(half.Solid());
    operation.SetArguments(arguments);
    operation.SetTools(tools);
    operation.SetNonDestructive(true);
    operation.SetRunParallel(false);
    operation.SetFuzzyValue(fuzzy);
    operation.Build();
    if (!operation.IsDone() || operation.HasErrors())
      throw std::runtime_error("Exact plane split failed");
    if (!TopExp_Explorer(operation.Shape(), TopAbs_SOLID).More())
      return nullptr;
    auto result = checked(operation.Shape());
    record_history(*result, target, 0, operation);
    const auto faces = topology_map(result->value, 0);
    const auto sourceFaces = topology_map(half.Solid(), 0);
    std::set<int> caps;
    auto append = [&](const TopoDS_Shape &face) {
      const auto index = faces.FindIndex(face);
      if (index > 0) caps.insert(index);
    };
    for (int i = 1; i <= sourceFaces.Extent(); ++i) {
      append(sourceFaces(i));
      for (const auto &face : operation.Modified(sourceFaces(i))) append(face);
      for (const auto &face : operation.Generated(sourceFaces(i))) append(face);
    }
    // This adapter has one seed family: the exact generated plane boundary.
    for (int index : caps)
      result->seeds.push_back(NativeSeed{static_cast<uint32_t>(index - 1), 0, 0});
    return result;
  });
}
std::unique_ptr<Shape> transform_shape(const Shape &s,
                                     rust::Slice<const double> r,
                                     const Point3 &t) {
  return guard([&] {
    if (r.size() != 9)
      throw std::runtime_error("Rotation requires nine coefficients");
    gp_Trsf tr;
    tr.SetValues(r[0], r[1], r[2], t.x, r[3], r[4], r[5], t.y,
                 r[6], r[7], r[8], t.z);
    // Validated proper rigid transforms use OCCT's exact location path. Avoid
    // recopying BSpline geometry/pcurves, which can change STEP trim behavior
    // even for an identity pose. Shared geometry remains immutable; the result
    // still receives its own world-space bounds cache and native face history.
    BRepBuilderAPI_Transform operation(s.value, tr, false);
    if (!operation.IsDone())
      throw std::runtime_error("Rigid transform failed");
    const auto transformed=operation.Shape();
    bool identity=true;
    const auto beforeLocation=s.value.Location().Transformation();
    const auto afterLocation=transformed.Location().Transformation();
    for(int row=1;row<=3;++row)for(int col=1;col<=4;++col) {
      identity=identity&&(tr.Value(row,col)==(row==col?1.:0.))&&
          beforeLocation.Value(row,col)==afterLocation.Value(row,col);
    }
    identity=identity&&transformed.IsPartner(s.value)&&transformed.Orientation()==s.value.Orientation();
    bool acceptedIdentity=false;
    if(identity) {std::lock_guard<std::mutex> lock(s.validity_cache->mutex);acceptedIdentity=s.validity_cache->accepted;}
    std::unique_ptr<Shape> result;
    if(acceptedIdentity) {
      result=std::make_unique<Shape>(Shape{transformed,{},{}});
      result->validity_cache=s.validity_cache;
    } else result=checked(transformed);
    record_history(*result, s, 0, operation);
    // Exact identity also preserves face order/world descriptors. All other
    // new geometries validate independently and retain fresh result caches.
    if(identity) {
      result->bounds_cache=s.bounds_cache;
      result->face_cache=s.face_cache;
      result->mesh_identity=s.mesh_identity;
    }
    // A completed native mass certificate is invariant under this exact
    // unit-scale proper isometry. Other transforms and all new geometry keep
    // fresh caches; imported STEP/BRep must independently integrate again.
    if(tr.ScaleFactor()==1.&&!tr.IsNegative()) {
      std::lock_guard<std::mutex> lock(s.volume_cache->mutex);
      if(s.volume_cache->certificate)result->volume_cache=s.volume_cache;
    }
    if(result->face_cache!=s.face_cache&&tr.ScaleFactor()==1.&&!tr.IsNegative())
      carry_rigid_face_properties(*result,s,operation,tr);
    return result;
  });
}
// A second owner of the same immutable B-rep: TopoDS handles are reference
// counted and every kernel operation builds new shapes (non-destructive
// Booleans, tessellation of a copy), so sharing never changes the source.
std::unique_ptr<Shape> share_shape(const Shape &source) {
  return guard([&] { return std::make_unique<Shape>(source); });
}
// Query-only occupied space. Compound members remain distinct in canonical
// source; normalize deep copies so overlapping members cannot invalidate COMMON
// or count the same physical overlap twice.
std::unique_ptr<Shape> inspection_union(const Shape &source) {
  return guard([&] {
    TopTools_IndexedMapOfShape solids;TopExp::MapShapes(source.value,TopAbs_SOLID,solids);
    if(solids.IsEmpty()||solids.Extent()>128)throw std::runtime_error("Occupied-space inspection supports 1 to 128 solids per Part");
    BRepBuilderAPI_Copy first(solids(1),true,false);
    if(!first.IsDone())throw std::runtime_error("Inspection input copy failed");
    auto result=checked(first.Shape());
    for(int i=2;i<=solids.Extent();++i) {
      BRepBuilderAPI_Copy copy(solids(i),true,false);
      if(!copy.IsDone())throw std::runtime_error("Inspection member copy failed");
      BRepAlgoAPI_Fuse operation;
      TopTools_ListOfShape arguments,tools;arguments.Append(result->value);tools.Append(copy.Shape());
      operation.SetArguments(arguments);operation.SetTools(tools);
      operation.SetNonDestructive(true);operation.SetRunParallel(false);operation.SetFuzzyValue(0.);
      operation.Build();
      if(!operation.IsDone()||operation.HasErrors())throw std::runtime_error("Native occupied-space normalization failed");
      result=checked(operation.Shape());
    }
    return result;
  });
}
std::unique_ptr<Shape> affine_transform_shape(const Shape &s,
                                            rust::Slice<const double> m,
                                            const Point3 &t) {
  return guard([&] {
    if (m.size() != 9)
      throw std::runtime_error("Affine transform requires nine coefficients");
    // Only literal scalar matrices take this path. Near-uniform or conjugated
    // matrices keep every supplied coefficient in the general transform.
    if (m[0] == m[4] && m[4] == m[8] && m[1] == 0. && m[2] == 0. &&
        m[3] == 0. && m[5] == 0. && m[6] == 0. && m[7] == 0.) {
      gp_Trsf tr;
      tr.SetScale(gp::Origin(), m[0]);
      tr.SetTranslationPart(gp_Vec(t.x, t.y, t.z));
      BRepBuilderAPI_Transform operation(s.value, tr, true);
      if (!operation.IsDone()) throw std::runtime_error("Uniform affine transform failed");
      auto result = checked(operation.Shape());
      record_history(*result, s, 0, operation);
      return result;
    }
    gp_GTrsf tr(gp_Mat(m[0], m[1], m[2], m[3], m[4], m[5], m[6], m[7], m[8]),
                gp_XYZ(t.x, t.y, t.z));
    if (tr.IsNegative() || tr.IsSingular())
      throw std::runtime_error("Affine transform is singular or reflects the solid");
    BRepBuilderAPI_GTransform operation(s.value, tr, true);
    if (!operation.IsDone()) throw std::runtime_error("General affine transform failed");
    auto result = checked(operation.Shape());
    // OCCT 7.9.3 GTransform::Modified() does not expose its collected list.
    // Its ModifiedShape() composes the exact NURBS-conversion and transform
    // histories. Affine maps preserve topology; require a complete bijection.
    for (uint8_t kind = 0; kind < 3; ++kind) {
      const auto input = topology_map(s.value, kind);
      const auto output = topology_map(result->value, kind);
      std::set<int> mapped;
      for (int i = 1; i <= input.Extent(); ++i) {
        const auto modified = operation.ModifiedShape(input(i));
        const int index = output.FindIndex(modified);
        if (index <= 0 || !mapped.insert(index).second)
          throw std::runtime_error("Affine transform lacks unique exact topology ancestry");
        result->ancestry.push_back(NativeAncestry{kind, 0,
            static_cast<uint32_t>(index - 1), static_cast<uint32_t>(i - 1)});
      }
      if (mapped.size() != static_cast<size_t>(output.Extent()))
        throw std::runtime_error("Affine transform added unsupported topology");
    }
    return result;
  });
}
std::unique_ptr<Shape> mirror_shape(const Shape &s, const Frame3 &f) {
  return guard([&] {
    gp_Trsf mirror;
    mirror.SetMirror(axes(f));
    BRepBuilderAPI_Transform operation(s.value, mirror, true);
    if (!operation.IsDone()) throw std::runtime_error("Native reflection failed");
    auto result = checked(operation.Shape());
    record_history(*result, s, 0, operation);
    return result;
  });
}
namespace {
bool needs_trimmed_support(const TopoDS_Shape &shape) {
  // Bounded coverage for the observed curved-profile sweep failure. Preserve
  // established analytic and ruled-surface paths; this is not a claim that
  // polynomial degree characterizes every possible trimming-domain problem.
  for(TopExp_Explorer it(shape,TopAbs_FACE);it.More();it.Next()) {
    BRepAdaptor_Surface surface(TopoDS::Face(it.Current()));
    if((surface.GetType()==GeomAbs_BSplineSurface||surface.GetType()==GeomAbs_BezierSurface)
        &&surface.UDegree()>1&&surface.VDegree()>1)return true;
  }
  return false;
}
std::array<double,6> trimmed_support_bounds(const TopoDS_Shape &shape) {
  // Add (not AddOptimal) gives a conservative, non-triangulated enclosure.
  // Finite planes beyond it cover the complete orthogonal projection. Thus
  // nearest distance to each plane measures a directional support coordinate,
  // including trimming curves rather than the underlying surface's UV box.
  Bnd_Box box;BRepBndLib::Add(shape,box,false);
  if(box.IsVoid()||box.IsOpen())throw std::runtime_error("Shape has no finite support enclosure");
  double lo[3],hi[3];box.Get(lo[0],lo[1],lo[2],hi[0],hi[1],hi[2]);
  for(int i=0;i<3;++i)if(!std::isfinite(lo[i])||!std::isfinite(hi[i])||lo[i]>hi[i])
    throw std::runtime_error("Shape support enclosure is nonfinite");
  const double tolerance=Precision::Confusion();
  std::array<double,6> result;
  for(int axis=0;axis<3;++axis)for(int side=0;side<2;++side) {
    const int u=(axis+1)%3,v=(axis+2)%3;
    double center[3],normal[3]={0,0,0},udir[3]={0,0,0};
    for(int i=0;i<3;++i)center[i]=(lo[i]+hi[i])/2;
    center[axis]=side?hi[axis]+1:lo[axis]-1;normal[axis]=1;udir[u]=1;
    const auto plane=BRepBuilderAPI_MakeFace(gp_Pln(gp_Ax3(gp_Pnt(center[0],center[1],center[2]),gp_Dir(normal[0],normal[1],normal[2]),gp_Dir(udir[0],udir[1],udir[2]))),-(hi[u]-lo[u])/2-1,(hi[u]-lo[u])/2+1,-(hi[v]-lo[v])/2-1,(hi[v]-lo[v])/2+1).Face();
    BRepExtrema_DistShapeShape query(shape,plane,tolerance);
    if(!query.IsDone()||query.NbSolution()<1||!std::isfinite(query.Value())||query.Value()<=0)
      throw std::runtime_error("Trimmed support query did not converge");
    const double extent=center[axis]+(side?-query.Value():query.Value());
    double residual=0;
    for(int i=1;i<=query.NbSolution();++i) {
      const auto a=query.PointOnShape1(i),b=query.PointOnShape2(i);
      for(int coordinate=1;coordinate<=3;++coordinate)
        if(!std::isfinite(a.Coord(coordinate))||!std::isfinite(b.Coord(coordinate)))
          throw std::runtime_error("Trimmed support witness is nonfinite");
      if(std::abs(b.Coord(u+1)-center[u])>(hi[u]-lo[u])/2+1+tolerance||
          std::abs(b.Coord(v+1)-center[v])>(hi[v]-lo[v])/2+1+tolerance)
        throw std::runtime_error("Trimmed support witness is outside its finite plane");
      // Equal minima on a flat plane can differ tangentially by more than the
      // distance deflection. Validate the directional support and actual
      // witness distance, rather than unrelated tangent coordinates.
      residual=std::max({residual,std::abs(a.Coord(axis+1)-extent),std::abs(b.Coord(axis+1)-center[axis]),std::abs(a.Distance(b)-query.Value())});
    }
    if(!std::isfinite(extent)||residual>tolerance)
      throw std::runtime_error("Trimmed support witnesses are inconsistent");
    // Outward numerical enclosure: solver deflection plus measured witness
    // projection residual. This does not replace the underlying exact solid.
    result[axis+side*3]=extent+(side?1:-1)*(tolerance+residual);
  }
  for(int axis=0;axis<3;++axis)if(result[axis]>result[axis+3])
    throw std::runtime_error("Trimmed support bounds are inverted");
  return result;
}
}
Bounds3 bounds(const Shape &s) {
  return guard([&] {
    std::lock_guard<std::mutex> lock(s.bounds_cache->mutex);
    if(!s.bounds_cache->value) {
      if(needs_trimmed_support(s.value))s.bounds_cache->value=trimmed_support_bounds(s.value);
      else {
        Bnd_Box b;BRepBndLib::AddOptimal(s.value,b,false,false);
        if(b.IsVoid()||b.IsOpen())throw std::runtime_error("Shape has no finite bounds");
        std::array<double,6> values;
        b.Get(values[0],values[1],values[2],values[3],values[4],values[5]);
        s.bounds_cache->value=values;
      }
    }
    const auto &b=*s.bounds_cache->value;
    return Bounds3{Point3{b[0],b[1],b[2]},Point3{b[3],b[4],b[5]}};
  });
}
// Private FFI unit-test support: race the cold cache through identical owners.
// Rust Solid remains !Send/!Sync; no cross-thread native handle escapes here.
bool bounds_cache_concurrent_probe(const Shape &s) {
  Shape cold{s.value,{},{}};
  const auto shared=share_shape(cold);
  if(shared->bounds_cache!=cold.bounds_cache)return false;
  std::array<std::array<double,6>,8> values;
  std::array<std::exception_ptr,8> errors;
  std::vector<std::thread> workers;std::atomic<bool> start{false};
  try {for(int i=0;i<8;++i)workers.emplace_back([&,i] {
    while(!start.load(std::memory_order_acquire))std::this_thread::yield();
    try {
      for(int repeat=0;repeat<16;++repeat) {
        const auto b=bounds(i%2?*shared:cold);
        values[i]={b.min.x,b.min.y,b.min.z,b.max.x,b.max.y,b.max.z};
      }
    } catch(...) {errors[i]=std::current_exception();}
  });} catch(...) {
    start.store(true,std::memory_order_release);
    for(auto &worker:workers)worker.join();
    throw;
  }
  start.store(true,std::memory_order_release);
  for(auto &worker:workers)worker.join();
  for(int i=0;i<8;++i)if(errors[i])std::rethrow_exception(errors[i]);
  for(int i=1;i<8;++i)if(values[i]!=values[0])return false;
  return true;
}
uint32_t solid_count(const Shape &s) {
  return guard([&] {
    uint32_t count = 0;
    for (TopExp_Explorer e(s.value, TopAbs_SOLID); e.More(); e.Next())
      ++count;
    return count;
  });
}
rust::Vec<FaceInfo> faces(const Shape &s) {
  return guard([&] {
    std::lock_guard<std::mutex> lock(s.face_cache->mutex);
    if(!s.face_cache->value) {
    std::vector<CachedFace> completed;
    auto map = topology_map(s.value, 0);
    for (int index = 1; index <= map.Extent(); ++index) {
      auto face = TopoDS::Face(map(index));
      ++s.face_cache->integrations;
      const auto p = measured_face(face);
      completed.push_back(describe_face(face,index-1,p.CentreOfMass(),p.Mass()));
    }
    // Publish only a complete successful measurement. An exception leaves no
    // partial certificate; existing adaptive integration guards stay unchanged.
    s.face_cache->value=std::move(completed);
    }
    rust::Vec<FaceInfo> out;
    for(const auto &f:*s.face_cache->value)out.push_back({f.index,f.planar,
        Point3{f.origin[0],f.origin[1],f.origin[2]},
        Point3{f.normal[0],f.normal[1],f.normal[2]},
        Point3{f.centroid[0],f.centroid[1],f.centroid[2]},f.area});
    return out;
  });
}
uint64_t face_integration_count(const Shape &s) {
  std::lock_guard<std::mutex> lock(s.face_cache->mutex);
  return s.face_cache->integrations;
}
bool shares_face_cache(const Shape &a,const Shape &b) {return a.face_cache==b.face_cache;}
bool face_cache_concurrent_probe(const Shape &source) {
  Shape cold{source.value};
  const auto shared=share_shape(cold);
  std::array<std::vector<double>,8> values;
  std::array<std::exception_ptr,8> errors{};
  std::vector<std::thread> workers;
  std::atomic<bool> start{false};
  try {for(int i=0;i<8;++i)workers.emplace_back([&,i] {
    while(!start.load(std::memory_order_acquire))std::this_thread::yield();
    try {
      for(const auto &f:faces(i%2?*shared:cold)) {
        auto &v=values[i];
        v.insert(v.end(),{double(f.index),double(f.planar),f.origin.x,f.origin.y,f.origin.z,
            f.normal.x,f.normal.y,f.normal.z,f.center.x,f.center.y,f.center.z,f.area});
      }
    } catch(...) {errors[i]=std::current_exception();}
  });} catch(...) {
    start.store(true,std::memory_order_release);
    for(auto &worker:workers)worker.join();
    throw;
  }
  start.store(true,std::memory_order_release);
  for(auto &worker:workers)worker.join();
  for(int i=0;i<8;++i)if(errors[i])std::rethrow_exception(errors[i]);
  for(int i=1;i<8;++i)if(values[i]!=values[0])return false;
  return !values[0].empty()&&face_integration_count(cold)==values[0].size()/12;
}
rust::Vec<TopologyNode> topology(const Shape &s) {
  return guard([&] {
    rust::Vec<TopologyNode> result;
    auto faces = topology_map(s.value, 0);
    TopTools_IndexedDataMapOfShapeListOfShape edgeFaces, vertexFaces;
    TopExp::MapShapesAndAncestors(s.value, TopAbs_EDGE, TopAbs_FACE, edgeFaces);
    TopExp::MapShapesAndAncestors(s.value, TopAbs_VERTEX, TopAbs_FACE, vertexFaces);
    for (uint8_t kind = 0; kind < 3; ++kind) {
      auto elements = topology_map(s.value, kind);
      for (int i = 1; i <= elements.Extent(); ++i) {
        std::set<uint32_t> incident;
        auto append = [&](const TopTools_ListOfShape &list) {
          for (const auto &face : list) {
            auto index = faces.FindIndex(face);
            if (index > 0 && !(kind == 0 && index == i))
              incident.insert(static_cast<uint32_t>(index - 1));
          }
        };
        if (kind == 0) {
          for (TopExp_Explorer e(elements(i), TopAbs_EDGE); e.More(); e.Next())
            if (edgeFaces.Contains(e.Current())) append(edgeFaces.FindFromKey(e.Current()));
        } else {
          const auto &ancestors = kind == 1 ? edgeFaces : vertexFaces;
          if (ancestors.Contains(elements(i))) append(ancestors.FindFromKey(elements(i)));
        }
        TopologyNode node{kind, static_cast<uint32_t>(i - 1), {}};
        for (auto index : incident) node.incident_faces.push_back(index);
        result.push_back(std::move(node));
      }
    }
    return result;
  });
}
rust::Vec<Ancestry> ancestry(const Shape &s) {
  rust::Vec<Ancestry> result;
  for (const auto &entry : s.ancestry)
    result.push_back(Ancestry{entry.kind, entry.operand, entry.output, entry.input});
  return result;
}
rust::Vec<Seed> seeds(const Shape &s) {
  rust::Vec<Seed> result;
  for (const auto &entry : s.seeds)
    result.push_back(Seed{entry.face, entry.source_index, entry.role});
  return result;
}
rust::Vec<EdgeInfo> display_edges(const Shape &s, double deflection) {
  return guard([&] {
    rust::Vec<EdgeInfo> result;
    auto edges = topology_map(s.value, 1);
    uint64_t points = 0;
    for (int i = 1; i <= edges.Extent(); ++i) {
      auto edge = TopoDS::Edge(edges(i));
      if (BRep_Tool::Degenerated(edge)) continue;
      BRepAdaptor_Curve curve(edge);
      GCPnts_QuasiUniformDeflection sample(curve, deflection);
      if (!sample.IsDone() || sample.NbPoints() < 2)
        throw std::runtime_error("B-rep edge display sampling failed");
      points += sample.NbPoints();
      if (points > 1000000)
        throw std::runtime_error("B-rep edge display budget exceeded");
      EdgeInfo info{static_cast<uint32_t>(i - 1), {}};
      for (int p = 1; p <= sample.NbPoints(); ++p) {
        auto v = sample.Value(p);
        info.points.push_back(v.X()); info.points.push_back(v.Y()); info.points.push_back(v.Z());
      }
      result.push_back(std::move(info));
    }
    return result;
  });
}
rust::Vec<VertexInfo> vertices(const Shape &s) {
  return guard([&] {
    rust::Vec<VertexInfo> result;
    auto vertices = topology_map(s.value, 2);
    for (int i = 1; i <= vertices.Extent(); ++i)
      result.push_back(VertexInfo{static_cast<uint32_t>(i - 1),
          data(BRep_Tool::Pnt(TopoDS::Vertex(vertices(i))))});
    return result;
  });
}
} // namespace multicad::kernel
