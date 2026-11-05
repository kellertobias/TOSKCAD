#pragma once
// Enclosure for the six reviewed normalized ZA source charts only. The
// caller owns their domain/analytic M800 proof and must not substitute an
// arbitrary callback. This header has no OCCT or third-party dependencies.
#include <algorithm>
#include <array>
#include <cmath>
#include <cstddef>
#include <cstdint>
#include <iomanip>
#include <limits>
#include <sstream>
#include <stdexcept>
namespace worm_verification {
inline double lower(double x){if(!std::isfinite(x))throw std::runtime_error("Nonfinite interval operation");return std::nextafter(x,-std::numeric_limits<double>::infinity());}
inline double upper(double x){if(!std::isfinite(x))throw std::runtime_error("Nonfinite interval operation");return std::nextafter(x,std::numeric_limits<double>::infinity());}
struct Interval {
  double lo,hi;
  Interval(double x=0):lo(x),hi(x){if(!std::isfinite(x))throw std::runtime_error("Nonfinite interval input");}
  Interval(double a,double b):lo(a),hi(b){if(!std::isfinite(a)||!std::isfinite(b)||a>b)throw std::runtime_error("Invalid interval");}
};
using I=Interval;
inline I operator+(I a,I b){return {lower(a.lo+b.lo),upper(a.hi+b.hi)};}
inline I operator-(I a){return {-a.hi,-a.lo};}
inline I operator-(I a,I b){return a+(-b);}
inline I operator*(I a,I b){std::array<double,4> p={a.lo*b.lo,a.lo*b.hi,a.hi*b.lo,a.hi*b.hi};return {lower(*std::min_element(p.begin(),p.end())),upper(*std::max_element(p.begin(),p.end()))};}
inline I operator/(I a,I b){if(b.lo<=0&&b.hi>=0)throw std::runtime_error("Interval denominator contains zero");return a*I(lower(1/b.hi),upper(1/b.lo));}
inline double magnitude(I a){return std::max(std::abs(a.lo),std::abs(a.hi));}
inline I square(I a){double low=a.lo<=0&&a.hi>=0?0:std::min(a.lo*a.lo,a.hi*a.hi);return {low==0?0:lower(low),upper(std::max(a.lo*a.lo,a.hi*a.hi))};}
inline I power(I a,unsigned n){I p(1);for(unsigned i=0;i<n;++i)p=p*a;return p;}
// No libm sqrt error assumption: comparisons of outward squared brackets
// establish the enclosure. The midpoints only choose subdivision locations.
inline I scalar_sqrt(double x){
  if(x<0||!std::isfinite(x))throw std::runtime_error("Invalid square root");if(x==0)return I(0);
  double lo=0,hi=std::max(1.,x);
  for(int i=0;i<160;++i){const double mid=lo+(hi-lo)/2;if(mid==lo||mid==hi)break;const I s=square(I(mid));
    if(s.hi<x)lo=mid;else if(s.lo>x)hi=mid;else{
      double a=std::nextafter(mid,0.),b=std::nextafter(mid,std::numeric_limits<double>::infinity());
      while(square(I(a)).hi>x)a=std::nextafter(a,0.);
      while(square(I(b)).lo<x)b=std::nextafter(b,std::numeric_limits<double>::infinity());
      return I(a,b);
    }
  }
  return I(lo,hi);
}
inline I sqrt(I x){if(x.lo<0)throw std::runtime_error("Square root interval crosses zero");return {scalar_sqrt(x.lo).lo,scalar_sqrt(x.hi).hi};}
inline I pi(){constexpr double p=3.141592653589793238462643383279502884;return {lower(p),upper(p)};}
inline std::array<I,2> sincos(I x){
  const double a=magnitude(x);if(a>2)throw std::runtime_error("Taylor trigonometric range exceeds2 radians");
  const I xx=square(x);I st=x,ct(1),s=st,c=ct;
  for(int j=1;j<32;++j){st=(-st)*xx/I((2*j)*(2*j+1));ct=(-ct)*xx/I((2*j-1)*(2*j));s=s+st;c=c+ct;}
  // Both are the degree63 Taylor polynomial (cos coefficient63 is zero).
  I remainder(1);for(int j=1;j<=64;++j)remainder=remainder*I(a)/I(j);
  const I error(-remainder.hi,remainder.hi);return {s+error,c+error};
}
inline I sin(I x){return sincos(x)[0];}
inline I cos(I x){return sincos(x)[1];}
inline I asin(I x){
  const double a=magnitude(x);if(a>=.5)throw std::runtime_error("asin interval exceeds reviewed range");
  const I xx=square(x);I p=x,c(1),s=x;
  for(int j=1;j<64;++j){p=p*xx;c=c*I((2*j-1)*(2*j-1))/I((2*j)*(2*j+1));s=s+c*p;}
  // Every arcsine coefficient is <=1. First omitted power is129.
  I tail=power(I(a),129)/(I(1)-square(I(a)));return s+I(-tail.hi,tail.hi);
}
using Vec=std::array<I,3>;
using Point=std::array<double,3>;
template<std::size_t N>using Lattice=std::array<std::array<Point,N>,N>;
using Poles26=Lattice<26>;
using Poles27=Lattice<27>;
inline double norm_upper(const Vec &v){I n(0);for(const auto &x:v)n=n+square(x);return sqrt(I(std::max(0.,n.lo),n.hi)).hi;}
inline std::array<I,26> exact_nodes(){
  std::array<I,26> nodes;nodes[0]=I(0);nodes[25]=I(1);
  for(int j=1;j<25;++j){const int reflected=std::min(j,25-j);const I angle=pi()*I(reflected)/I(25);I c=cos(angle);if(j>12)c=-c;nodes[j]=(I(1)-c)/I(2);}
  return nodes;
}
template<std::size_t Count>std::array<I,Count> bernstein(I t){
  constexpr std::size_t degree=Count-1;
  if(t.lo<0||t.hi>1)throw std::runtime_error("Node interval outside unit domain");
  std::array<I,Count> w;if(t.lo==0&&t.hi==0){w.fill(I(0));w[0]=I(1);return w;}if(t.lo==1&&t.hi==1){w.fill(I(0));w[degree]=I(1);return w;}
  std::uint64_t binomial=1;
  for(std::size_t i=0;i<Count;++i){if(i)binomial=binomial*std::uint64_t(Count-i)/std::uint64_t(i);I p=I(static_cast<double>(binomial))*power(t,unsigned(i))*power(I(1)-t,unsigned(degree-i));w[i]=I(std::max(0.,p.lo),p.hi);}
  return w;
}
struct NodeCertificate {double max_node_l2,analytic_tail_l2,source_bound_l2;};
inline void require_budget(double bound,double tolerance,const char *what){
  if(!std::isfinite(bound)||!std::isfinite(tolerance)||tolerance<=0||bound>tolerance/2){std::ostringstream s;s<<std::setprecision(17)<<what<<" bound "<<bound<<" exceeds half tolerance "<<tolerance/2;throw std::runtime_error(s.str());}
}
// The reference interpolant remains degree25. P may have any smaller degree:
// P-I25(F) is still degree<=25, so the SAME exact26-node norm bound applies.
// No lower-degree analytic-tail estimate is substituted.
template<std::size_t Count,class Callback>NodeCertificate verify(const Lattice<Count> &poles,Callback source,double module,double tolerance){
  if constexpr(Count<2||Count>26){throw std::runtime_error("Actual Bezier degree exceeds certified1..25 range");}
  else {
    if(!std::isfinite(module)||module<=0)throw std::runtime_error("Invalid physical module");
    for(const auto &row:poles)for(const auto &p:row)for(double x:p)if(!std::isfinite(x))throw std::runtime_error("Nonfinite native pole");
    const auto nodes=exact_nodes();std::array<std::array<I,Count>,26> weights;
    for(int i=0;i<26;++i)weights[i]=bernstein<Count>(nodes[i]);
    double maximum=0;
    for(int u=0;u<26;++u){std::array<Vec,Count> intermediate{};
      for(std::size_t j=0;j<Count;++j)for(std::size_t i=0;i<Count;++i)for(int c=0;c<3;++c)intermediate[j][c]=intermediate[j][c]+weights[u][i]*I(poles[i][j][c]);
      for(int v=0;v<26;++v){Vec evaluated{};for(std::size_t j=0;j<Count;++j)for(int c=0;c<3;++c)evaluated[c]=evaluated[c]+weights[v][j]*intermediate[j][c];
        const Vec target=source(nodes[u],nodes[v]);Vec delta;for(int c=0;c<3;++c)delta[c]=evaluated[c]-target[c];maximum=std::max(maximum,norm_upper(delta));
      }
    }
    constexpr double three25=847288609443.;
    const double tail=(sqrt(I(2))*I(8000)*I(module)/I(three25)).hi;
    const double total=(I(tail)+I(16)*I(maximum)).hi;
    require_budget(total,tolerance,"Stored Bezier source position");return {maximum,tail,total};
  }
}
struct HalfKnotCertificate {double control_l2_bound;};
template<std::size_t Count>HalfKnotCertificate verifyHalfKnots(const Lattice<Count> &before,const Lattice<Count+1> &after,const std::array<double,3>&uk,const std::array<int,3>&um,const std::array<double,3>&vk,const std::array<int,3>&vm,int ud=int(Count)-1,int vd=int(Count)-1,int up=int(Count)+1,int vp=int(Count)+1){
  if constexpr(Count<2||Count>26){throw std::runtime_error("Actual Bezier degree exceeds certified1..25 range");}
  else {
    const std::array<double,3> knots={0,.5,1};const std::array<int,3> mult={int(Count),1,int(Count)};
    if(ud!=int(Count)-1||vd!=int(Count)-1||up!=int(Count)+1||vp!=int(Count)+1||uk!=knots||vk!=knots||um!=mult||vm!=mult)throw std::runtime_error("Unexpected half-knot spline layout");
    std::array<std::array<Vec,Count>,Count+1> u{};std::array<std::array<Vec,Count+1>,Count+1> both{};
    for(std::size_t j=0;j<Count;++j)for(std::size_t i=0;i<Count+1;++i)for(int c=0;c<3;++c)u[i][j][c]=i==0?I(before[0][j][c]):i==Count?I(before[Count-1][j][c]):(I(before[i-1][j][c])+I(before[i][j][c]))/I(2);
    for(std::size_t i=0;i<Count+1;++i)for(std::size_t j=0;j<Count+1;++j)for(int c=0;c<3;++c)both[i][j][c]=j==0?u[i][0][c]:j==Count?u[i][Count-1][c]:(u[i][j-1][c]+u[i][j][c])/I(2);
    double maximum=0;
    for(std::size_t i=0;i<Count+1;++i)for(std::size_t j=0;j<Count+1;++j){Vec difference;for(int c=0;c<3;++c)difference[c]=I(after[i][j][c])-both[i][j][c];maximum=std::max(maximum,norm_upper(difference));}
    return {maximum};
  }
}
inline double combined_bound(NodeCertificate source,HalfKnotCertificate insertion,double tolerance){const double total=(I(source.source_bound_l2)+I(insertion.control_l2_bound)).hi;require_budget(total,tolerance,"Native half-knot source position");return total;}
}
