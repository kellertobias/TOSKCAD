#include <STEPControl_Reader.hxx>
#include <TopExp_Explorer.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Solid.hxx>
#include <BRepClass3d_SolidClassifier.hxx>
#include <BRepGProp.hxx>
#include <GProp_GProps.hxx>
#include <Standard_Failure.hxx>
#include <cmath>
#include <iostream>
#include <iomanip>
#include <stdexcept>
const double pi=std::acos(-1.),R=4.,P=1.25,H=8.,D=.5,C=.25;
double profile(double u){return R-std::max(0.,std::abs(u)-C/2)/std::tan(pi/12);}
double simple(double phase,double z,double k){double u=phase<=P/2?phase:phase-P;return std::max(R-D,profile(u))+k*z;}
double transport(double phase,double z,double k,int n,bool left){
 double u=phase<=P/2?phase:phase-P,lead=P*n,h=(left?-1:1)*lead/(2*pi),root=R-D,d=u,r=root+k*z;
 for(int i=0;i<12;++i){double zc=z-d,rc=root+k*zc,psi=std::atan(k*h/(root-k*lead))-std::atan(k*h/rc),dr=profile(d)-root+k*d,re=rc+dr*std::cos(psi),im=dr*std::sin(psi);r=std::hypot(re,im);d=u+h*std::atan2(im,re);}return std::max(root+k*z,r);
}
int main(int argc,char**argv){try{
 if(argc!=4)return 2;
 double k=std::stod(argv[2]);int n=std::stoi(argv[3]); bool left=false;
 STEPControl_Reader reader;if(reader.ReadFile(argv[1])!=IFSelect_RetDone)return 3;reader.TransferRoots();auto shape=reader.OneShape();TopExp_Explorer solids(shape,TopAbs_SOLID);if(!solids.More())return 4;auto solid=TopoDS::Solid(solids.Current());solids.Next();if(solids.More())return 5;
 GProp_GProps mass;double integration=BRepGProp::VolumePropertiesGK(solid,mass,1e-9,true,false);std::cout<<std::setprecision(15)<<"READ_VOLUME k="<<k<<" starts="<<n<<" volume="<<mass.Mass()<<" integration="<<integration<<"\n";
 BRepClass3d_SolidClassifier classifier(solid);auto inside=[&](double r,double theta,double z){classifier.Perform(gp_Pnt(r*std::cos(theta),r*std::sin(theta),z),1e-9);return classifier.State()==TopAbs_IN||classifier.State()==TopAbs_ON;};
 for(double z:{1.7,6.8})for(double phase:{.21,1.04}){double h=P*n/(2*pi),theta=(z-phase)/h,pred=simple(phase,z,k),exact=transport(phase,z,k,n,left),lo=pred-.003,hi=pred+.003;if(!inside(lo,theta,z)||inside(hi,theta,z))throw std::runtime_error("bad bracket");for(int i=0;i<22;++i){double m=(lo+hi)/2;if(inside(m,theta,z))lo=m;else hi=m;}double actual=(lo+hi)/2;std::cout<<"BOUNDARY k="<<k<<" starts="<<n<<" z="<<z<<" phase="<<phase<<" simple="<<pred<<" transport="<<exact<<" actual="<<actual<<" simple_delta="<<actual-pred<<" transport_delta="<<actual-exact<<"\n";}
}catch(const Standard_Failure&e){std::cerr<<e.GetMessageString();return 6;}catch(const std::exception&e){std::cerr<<e.what();return 7;}return 0;}
