//! Owned generic planning families. These defaults are editable context sizes,
//! not manufacturer products, connector fit geometry or installation approval.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    ElectricalLighting,
    Heating,
    Sanitary,
    Appliances,
}
impl Category {
    pub const ALL: [Self; 4] = [
        Self::ElectricalLighting,
        Self::Heating,
        Self::Sanitary,
        Self::Appliances,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::ElectricalLighting => "Electrical and Lighting",
            Self::Heating => "Heating",
            Self::Sanitary => "Sanitary",
            Self::Appliances => "Appliances",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    PowerOutletTypeF,
    DoublePowerOutletTypeF,
    TriplePowerOutletTypeF,
    DataOutletRj45,
    TvAntennaOutlet,
    Switch,
    CeilingLuminaire,
    WallLuminaire,
    LampHolderE27,
    Radiator,
    KitchenSink,
    Washbasin,
    Toilet,
    Bathtub,
    Shower,
    CookerOven,
    Cooktop,
    WashingMachine,
    Dryer,
    Refrigerator,
    Dishwasher,
    JunctionBox,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MountClass {
    WallCenter,
    WallBottom,
    CeilingTop,
    FloorBottom,
    CounterTop,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanConvention {
    ToskCadGenericV1,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PowerConnector {
    TypeF,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Installation {
    Standalone,
    BuiltIn,
}
#[derive(Clone, Copy, Debug)]
pub struct Preset {
    pub name: &'static str,
    pub category: Category,
    /// Width X, depth Y, height Z, in millimeters.
    pub size_mm: [f64; 3],
    pub mount: MountClass,
    /// Center/bottom/top above the labeled floor, according to mount class.
    pub mounting_height_mm: f64,
    pub supports_built_in: bool,
    pub supports_standalone: bool,
    pub power_connector: Option<PowerConnector>,
    /// An optional fixed-paper electrical mark; the physical footprint keeps
    /// its true model dimensions separately.
    pub mark_paper_mm: Option<f64>,
    pub service_front_mm: f64,
    pub door_front_mm: f64,
}
impl Kind {
    pub fn search_terms(self) -> &'static str {
        match self {
            Self::PowerOutletTypeF
            | Self::DoublePowerOutletTypeF
            | Self::TriplePowerOutletTypeF => "power socket receptacle schuko type f outlet",
            Self::DataOutletRj45 => "ethernet lan network data rj45 socket outlet",
            Self::TvAntennaOutlet => "television tv antenna coax socket outlet",
            Self::Switch => "electrical light switch",
            Self::JunctionBox => "electrical junction box connection enclosure fitting",
            Self::CeilingLuminaire => "ceiling light lamp luminaire",
            Self::WallLuminaire => "wall light sconce luminaire",
            Self::LampHolderE27 => "bulb light socket lamp holder e27",
            Self::Radiator => "radiator heater heating",
            Self::KitchenSink => "kitchen sink draining board basin",
            Self::Washbasin => "washbasin hand washing sink bathroom basin",
            Self::Toilet => "toilet wc cistern bathroom",
            Self::Bathtub => "bathtub bath bathroom",
            Self::Shower => "shower tray enclosure bathroom",
            Self::CookerOven => "full cooker stove oven range hob",
            Self::Cooktop => "cooktop hob built in no oven",
            Self::WashingMachine => "washing machine washer laundry",
            Self::Dryer => "dryer tumble laundry",
            Self::Refrigerator => "fridge refrigerator kitchen",
            Self::Dishwasher => "dishwasher dishes kitchen",
        }
    }
    // Append only: published generic definition and Library identities use these slots.
    pub const ALL: [Self; 22] = [
        Self::PowerOutletTypeF,
        Self::DoublePowerOutletTypeF,
        Self::TriplePowerOutletTypeF,
        Self::DataOutletRj45,
        Self::TvAntennaOutlet,
        Self::Switch,
        Self::CeilingLuminaire,
        Self::WallLuminaire,
        Self::LampHolderE27,
        Self::Radiator,
        Self::KitchenSink,
        Self::Washbasin,
        Self::Toilet,
        Self::Bathtub,
        Self::Shower,
        Self::CookerOven,
        Self::Cooktop,
        Self::WashingMachine,
        Self::Dryer,
        Self::Refrigerator,
        Self::Dishwasher,
        Self::JunctionBox,
    ];
    pub fn preset(self) -> Preset {
        use Category::*;
        use MountClass::*;
        let (name, category, size_mm, mount, mounting_height_mm) = match self {
            Self::PowerOutletTypeF => (
                "Power outlet — Type F, generic",
                ElectricalLighting,
                [80., 45., 80.],
                WallCenter,
                300.,
            ),
            Self::DoublePowerOutletTypeF => (
                "Double power outlet — Type F, generic",
                ElectricalLighting,
                [150., 45., 80.],
                WallCenter,
                300.,
            ),
            Self::TriplePowerOutletTypeF => (
                "Triple power outlet — Type F, generic",
                ElectricalLighting,
                [220., 45., 80.],
                WallCenter,
                300.,
            ),
            Self::DataOutletRj45 => (
                "Data / network outlet — RJ45, generic",
                ElectricalLighting,
                [80., 35., 80.],
                WallCenter,
                300.,
            ),
            Self::TvAntennaOutlet => (
                "TV / antenna outlet, generic",
                ElectricalLighting,
                [80., 35., 80.],
                WallCenter,
                300.,
            ),
            Self::JunctionBox => (
                "Junction box, generic",
                ElectricalLighting,
                [80., 40., 80.],
                WallCenter,
                2000.,
            ),
            Self::Switch => (
                "Wall switch, generic",
                ElectricalLighting,
                [80., 35., 80.],
                WallCenter,
                1100.,
            ),
            Self::CeilingLuminaire => (
                "Ceiling luminaire, generic",
                ElectricalLighting,
                [300., 300., 90.],
                CeilingTop,
                0.,
            ),
            Self::WallLuminaire => (
                "Wall luminaire, generic",
                ElectricalLighting,
                [220., 130., 160.],
                WallCenter,
                1800.,
            ),
            Self::LampHolderE27 => (
                "Lamp holder — E27 planning envelope",
                ElectricalLighting,
                [55., 55., 80.],
                CeilingTop,
                0.,
            ),
            Self::Radiator => (
                "Panel radiator, generic",
                Heating,
                [1000., 100., 600.],
                WallBottom,
                150.,
            ),
            Self::KitchenSink => (
                "Kitchen sink, generic",
                Sanitary,
                [800., 500., 180.],
                CounterTop,
                900.,
            ),
            Self::Washbasin => (
                "Washbasin / washing sink, generic",
                Sanitary,
                [600., 450., 200.],
                CounterTop,
                850.,
            ),
            Self::Toilet => (
                "Toilet with cistern, generic",
                Sanitary,
                [380., 700., 780.],
                FloorBottom,
                0.,
            ),
            Self::Bathtub => (
                "Bathtub, generic",
                Sanitary,
                [1700., 750., 550.],
                FloorBottom,
                0.,
            ),
            Self::Shower => (
                "Shower tray and enclosure, generic",
                Sanitary,
                [900., 900., 2000.],
                FloorBottom,
                0.,
            ),
            Self::CookerOven => (
                "Full cooker / stove with oven, generic",
                Appliances,
                [600., 600., 850.],
                FloorBottom,
                0.,
            ),
            Self::Cooktop => (
                "Cooktop only — no oven, generic",
                Appliances,
                [600., 520., 60.],
                CounterTop,
                900.,
            ),
            Self::WashingMachine => (
                "Front-loading washing machine, generic",
                Appliances,
                [600., 600., 850.],
                FloorBottom,
                0.,
            ),
            Self::Dryer => (
                "Front-loading tumble dryer, generic",
                Appliances,
                [600., 600., 850.],
                FloorBottom,
                0.,
            ),
            Self::Refrigerator => (
                "Refrigerator, generic",
                Appliances,
                [600., 650., 1800.],
                FloorBottom,
                0.,
            ),
            Self::Dishwasher => (
                "Dishwasher, generic",
                Appliances,
                [600., 600., 850.],
                FloorBottom,
                0.,
            ),
        };
        let electrical = category == ElectricalLighting;
        let supports_built_in = matches!(
            self,
            Self::CookerOven
                | Self::Cooktop
                | Self::WashingMachine
                | Self::Dryer
                | Self::Refrigerator
                | Self::Dishwasher
        );
        let power_connector = matches!(
            self,
            Self::PowerOutletTypeF | Self::DoublePowerOutletTypeF | Self::TriplePowerOutletTypeF
        )
        .then_some(PowerConnector::TypeF);
        let service_front_mm = if supports_built_in { 600. } else { 0. };
        let door_front_mm = match self {
            Self::Refrigerator => 600.,
            Self::Dishwasher => 700.,
            Self::WashingMachine | Self::Dryer => 450.,
            Self::CookerOven => 450.,
            _ => 0.,
        };
        Preset {
            name,
            category,
            size_mm,
            mount,
            mounting_height_mm,
            supports_built_in,
            supports_standalone: self != Self::Cooktop,
            power_connector,
            mark_paper_mm: electrical.then_some(3.5),
            service_front_mm,
            door_front_mm,
        }
    }
}
