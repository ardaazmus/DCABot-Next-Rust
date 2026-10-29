//! dca_math — canonical zero-float quantitative library (M01–M20 normative specs).

// Faz 21 (CI): M02/M03/M07/M15 matematik imzaları normatif speç sözleşmesidir —
// parametre sayısı domain dilini yansıtır; sarmalama struct'ı API kırıcı olur.
#![allow(clippy::too_many_arguments)]

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
