//! dca_math — canonical zero-float quantitative library (M01–M20 normative specs).

pub mod fraction;
pub mod instrument;
pub mod m02;
pub mod m03_m07;
pub mod m04;
pub mod m15_m20;
pub mod m16_m18;
pub mod macro_calendar;
pub mod sessions;

pub use fraction::{Fraction, MathError, MAX_BITS};
pub use instrument::{Instrument, MmrBracket};
pub use m02::{build_dca_plan, qty_down, tick_down, tick_up, DcaLevel, Direction, SizingMode};
pub use m16_m18::{Side, VenueOrder};
