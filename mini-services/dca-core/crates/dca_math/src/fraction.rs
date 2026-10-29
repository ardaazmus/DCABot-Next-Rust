//! M01 — Exact Numeric Representation (4096-bit rational fractions, zero-float).
//!
//! Canonical type: Fraction { num: BigInt, den: BigInt>0, gcd(|num|,den)=1 }.
//! IEEE-754 floats are strictly forbidden in quantitative logic. All parsing is
//! exact decimal string parsing; rounding only happens at display boundary
//! (half-even, banker's rounding).

use num_bigint::BigInt;
use num_integer::Integer;
use num_traits::{One, Signed, Zero};
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;
use thiserror::Error;

pub const MAX_BITS: u64 = 4096;

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum MathError {
    #[error("M01: Division by zero")]
    DivisionByZero,
    #[error("M01: 4096-bit integer limit exceeded")]
    BitLimitExceeded,
    #[error("M01: Invalid decimal string: {0}")]
    InvalidDecimalString(String),
    #[error("M01: Negative or fractional exponent not allowed")]
    InvalidExponent,
    #[error("M01: Domain violation: {0}")]
    DomainViolation(String),
}

/// Exact rational fraction. Invariant: den > 0, gcd(|num|, den) = 1, bits <= 4096.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Fraction {
    num: BigInt,
    den: BigInt,
}

impl Fraction {
    pub fn new(num: BigInt, den: BigInt) -> Result<Self, MathError> {
        if den.is_zero() {
            return Err(MathError::DivisionByZero);
        }
        let (num, den) = if den.is_negative() { (-num, -den) } else { (num, den) };
        let gcd = num.abs().gcd(&den);
        let (num, den) = if gcd.is_zero() { (num, BigInt::one()) } else { (num / &gcd, den / &gcd) };
        if num.bits() > MAX_BITS || den.bits() > MAX_BITS {
            return Err(MathError::BitLimitExceeded);
        }
        Ok(Self { num, den })
    }

    pub fn zero() -> Self {
        Self { num: BigInt::zero(), den: BigInt::one() }
    }

    pub fn one() -> Self {
        Self { num: BigInt::one(), den: BigInt::one() }
    }

    pub fn neg_one() -> Self {
        Self { num: -BigInt::one(), den: BigInt::one() }
    }

    pub fn from_i64(v: i64) -> Self {
        Self { num: BigInt::from(v), den: BigInt::one() }
    }

    pub fn from_u64(v: u64) -> Self {
        Self { num: BigInt::from(v), den: BigInt::one() }
    }

    /// Exact decimal string parsing ("123.45", "-0.007", "1e-4", "+2.50").
    /// Rejects NaN/Inf/empty/garbage. Scientific notation with explicit exponent allowed.
    pub fn from_decimal_str(s: &str) -> Result<Self, MathError> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err(MathError::InvalidDecimalString(s.to_string()));
        }
        let parts: Vec<&str> = trimmed.split(['e', 'E']).collect();
        let (mantissa_str, exp_val) = match parts.len() {
            1 => (parts[0], 0i64),
            2 => {
                if parts[1].is_empty() {
                    return Err(MathError::InvalidDecimalString(s.to_string()));
                }
                let exp = parts[1].parse::<i64>().map_err(|_| MathError::InvalidDecimalString(s.to_string()))?;
                (parts[0], exp)
            }
            _ => return Err(MathError::InvalidDecimalString(s.to_string())),
        };

        let is_negative = mantissa_str.starts_with('-');
        let clean = mantissa_str.trim_start_matches(['+', '-']);
        let dec_parts: Vec<&str> = clean.split('.').collect();
        let (int_part, frac_part) = match dec_parts.len() {
            1 => (dec_parts[0], ""),
            2 => (dec_parts[0], dec_parts[1]),
            _ => return Err(MathError::InvalidDecimalString(s.to_string())),
        };
        if int_part.is_empty() && frac_part.is_empty() {
            return Err(MathError::InvalidDecimalString(s.to_string()));
        }
        let digits = format!("{}{}", int_part, frac_part);
        if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
            return Err(MathError::InvalidDecimalString(s.to_string()));
        }

        let mut num = BigInt::from_str(&digits).map_err(|_| MathError::InvalidDecimalString(s.to_string()))?;
        let frac_len = frac_part.len() as i64;
        let mut den = BigInt::from(10u32).pow(frac_len.max(0) as u32);

        // Faz 23 (derin inceleme): üs i64→u32 cast'i wrap yapıyordu —
        // "1e4294967297" sessizce 10'a düşüyordu (exactness sözleşmesi ihlali)
        // ve aşırı büyük üs parse anında ~GB'lık BigInt ayırıyordu (DoS).
        // 4096-bit sınır için |üs| > 1233 yeterli; aşılırsa dürüst hata.
        if exp_val.abs() > 1233 {
            return Err(MathError::InvalidDecimalString(s.to_string()));
        }
        if exp_val > 0 {
            num *= BigInt::from(10u32).pow(exp_val as u32);
        } else if exp_val < 0 {
            den *= BigInt::from(10u32).pow((-exp_val) as u32);
        }
        if is_negative {
            num = -num;
        }
        Self::new(num, den)
    }

    /// Parse either exact decimal ("123.45") or canonical rational ("124901/2").
    pub fn from_any_str(s: &str) -> Result<Self, MathError> {
        let t = s.trim();
        if let Some((n, d)) = t.split_once('/') {
            let num = BigInt::from_str(n.trim()).map_err(|_| MathError::InvalidDecimalString(s.to_string()))?;
            let den = BigInt::from_str(d.trim()).map_err(|_| MathError::InvalidDecimalString(s.to_string()))?;
            return Self::new(num, den);
        }
        Self::from_decimal_str(t)
    }

    pub fn num(&self) -> &BigInt {
        &self.num
    }

    pub fn den(&self) -> &BigInt {
        &self.den
    }

    pub fn is_zero(&self) -> bool {
        self.num.is_zero()
    }

    pub fn is_positive(&self) -> bool {
        self.num.is_positive()
    }

    pub fn is_negative(&self) -> bool {
        self.num.is_negative()
    }

    pub fn is_integer(&self) -> bool {
        self.den == BigInt::one()
    }

    pub fn add(&self, other: &Self) -> Result<Self, MathError> {
        Self::new(&self.num * &other.den + &other.num * &self.den, &self.den * &other.den)
    }

    pub fn sub(&self, other: &Self) -> Result<Self, MathError> {
        Self::new(&self.num * &other.den - &other.num * &self.den, &self.den * &other.den)
    }

    pub fn mul(&self, other: &Self) -> Result<Self, MathError> {
        Self::new(&self.num * &other.num, &self.den * &other.den)
    }

    pub fn div(&self, other: &Self) -> Result<Self, MathError> {
        if other.num.is_zero() {
            return Err(MathError::DivisionByZero);
        }
        Self::new(&self.num * &other.den, &self.den * &other.num)
    }

    pub fn neg(&self) -> Result<Self, MathError> {
        Self::new(-&self.num, self.den.clone())
    }

    pub fn abs(&self) -> Result<Self, MathError> {
        Self::new(self.num.abs(), self.den.clone())
    }

    /// Integer exponentiation (exp >= 0). 0^0 = 1 by convention.
    pub fn pow_u(&self, exp: u32) -> Result<Self, MathError> {
        if exp == 0 {
            return Ok(Self::one());
        }
        Self::new(self.num.pow(exp), self.den.pow(exp))
    }

    /// Exact floor division: returns floor(self / other) as BigInt.
    pub fn floor_div(&self, other: &Self) -> Result<BigInt, MathError> {
        if other.num.is_zero() {
            return Err(MathError::DivisionByZero);
        }
        // self.num * other.den / (self.den * other.num); handle sign of other.num
        let numer = &self.num * &other.den;
        let denom = &self.den * &other.num;
        if denom.is_zero() {
            return Err(MathError::DivisionByZero);
        }
        if denom.is_positive() {
            Ok(numer.div_floor(&denom))
        } else {
            let q = (-numer).div_floor(&(-&denom));
            Ok(q)
        }
    }

    /// Exact ceiling division: returns ceil(self / other) as BigInt.
    pub fn ceil_div(&self, other: &Self) -> Result<BigInt, MathError> {
        let fd = self.floor_div(other)?;
        // ceil = floor when exact, else floor + 1
        let prod = &fd * &other.num * &self.den;
        let cross = &self.num * &other.den;
        if prod == cross {
            Ok(fd)
        } else {
            Ok(fd + 1)
        }
    }

    /// M01 display: half-even (banker's) rounding to `decimals` decimal places.
    /// Rounding only at reporting boundary; internal state stays exact.
    pub fn to_half_even_display(&self, decimals: usize) -> String {
        if self.is_zero() {
            return if decimals == 0 { "0".into() } else { format!("0.{}", "0".repeat(decimals)) };
        }
        let scale = BigInt::from(10u32).pow(decimals as u32);
        let t = &self.num * &scale; // signed
        let (t_abs, sign) = if t.is_negative() { (-&t, "-") } else { (t.clone(), "") };
        let d = &self.den;
        let q = &t_abs / d;
        let r = &t_abs % d;
        let r2 = &r * 2u32;
        let mut q = q.clone();
        if &r2 > d || (&r2 == d && q.is_odd()) {
            q += 1u32;
        }
        let digits = q.to_string();
        if decimals == 0 {
            format!("{}{}", sign, digits)
        } else if digits.len() <= decimals {
            format!("{}0.{}{}", sign, "0".repeat(decimals - digits.len()), digits)
        } else {
            let split = digits.len() - decimals;
            format!("{}{}.{}", sign, &digits[..split], &digits[split..])
        }
    }

    /// Truncate toward zero to `decimals` (used for fee display in some ledgers).
    pub fn to_trunc_display(&self, decimals: usize) -> String {
        let scale = BigInt::from(10u32).pow(decimals as u32);
        let t = &self.num * &scale;
        let q = t.div_floor(&self.den);
        let (q_abs, sign) = if q.is_negative() { (-q, "-") } else { (q, "") };
        let digits = q_abs.to_string();
        if decimals == 0 {
            format!("{}{}", sign, digits)
        } else if digits.len() <= decimals {
            format!("{}0.{}{}", sign, "0".repeat(decimals - digits.len()), digits)
        } else {
            let split = digits.len() - decimals;
            format!("{}{}.{}", sign, &digits[..split], &digits[split..])
        }
    }

    pub fn to_f64_display(&self) -> f64 {
        // REPORTING ONLY — never used in quantitative decisions.
        self.to_half_even_display(12).parse::<f64>().unwrap_or(0.0)
    }

    pub fn min(a: &Self, b: &Self) -> Self {
        if a <= b {
            a.clone()
        } else {
            b.clone()
        }
    }

    pub fn max(a: &Self, b: &Self) -> Self {
        if a >= b {
            a.clone()
        } else {
            b.clone()
        }
    }
}

impl Default for Fraction {
    fn default() -> Self {
        Self::zero()
    }
}

impl fmt::Display for Fraction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.den.is_one() {
            write!(f, "{}", self.num)
        } else {
            write!(f, "{}/{}", self.num, self.den)
        }
    }
}

impl fmt::Debug for Fraction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Fraction({})", self)
    }
}

impl PartialOrd for Fraction {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Fraction {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Cross-multiply comparison: a.num*b.den <=> b.num*a.den (both dens > 0)
        let lhs = &self.num * &other.den;
        let rhs = &other.num * &self.den;
        lhs.cmp(&rhs)
    }
}

impl Serialize for Fraction {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Fraction {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Fraction::from_any_str(&s).map_err(de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(s: &str) -> Fraction {
        Fraction::from_any_str(s).unwrap()
    }

    #[test]
    fn parse_exact_decimal() {
        assert_eq!(f("0.1").to_string(), "1/10");
        assert_eq!(f("123.45").to_string(), "2469/20");
        assert_eq!(f("-0.007").to_string(), "-7/1000");
        assert_eq!(f("2e3").to_string(), "2000");
        assert_eq!(f("1.5e-2").to_string(), "3/200");
        assert_eq!(f("+2.50").to_string(), "5/2");
    }

    #[test]
    fn parse_rejects_garbage() {
        assert!(Fraction::from_decimal_str("NaN").is_err());
        assert!(Fraction::from_decimal_str("Infinity").is_err());
        assert!(Fraction::from_decimal_str("").is_err());
        assert!(Fraction::from_decimal_str("1.2.3").is_err());
        assert!(Fraction::from_decimal_str("abc").is_err());
        assert!(Fraction::from_decimal_str("0x10").is_err());
    }

    #[test]
    fn arithmetic_exact() {
        let a = f("0.1");
        let b = f("0.2");
        // 0.1 + 0.2 = 0.3 EXACTLY (float would give 0.30000000000000004)
        assert_eq!(a.add(&b).unwrap().to_string(), "3/10");
        assert_eq!(f("1").div(&f("3")).unwrap().to_string(), "1/3");
        assert_eq!(f("1").div(&f("3")).unwrap().mul(&f("3")).unwrap().to_string(), "1");
    }

    #[test]
    fn division_by_zero() {
        assert_eq!(f("1").div(&Fraction::zero()).unwrap_err(), MathError::DivisionByZero);
    }

    #[test]
    fn comparison_cross_multiply() {
        let a = f("1/3");
        let b = f("0.3333333333");
        assert!(a > b);
        assert!(f("-1/2") < f("-1/3"));
    }

    #[test]
    fn half_even_display() {
        assert_eq!(f("2.5").to_half_even_display(0), "2"); // tie -> even
        assert_eq!(f("3.5").to_half_even_display(0), "4"); // tie -> even
        assert_eq!(f("2.625").to_half_even_display(2), "2.62"); // tie -> even
        assert_eq!(f("2.626").to_half_even_display(2), "2.63");
        assert_eq!(f("0.125").to_half_even_display(2), "0.12");
        assert_eq!(f("-2.5").to_half_even_display(0), "-2");
        assert_eq!(f("1/3").to_half_even_display(6), "0.333333");
        assert_eq!(f("0").to_half_even_display(2), "0.00");
        assert_eq!(f("10").to_half_even_display(2), "10.00");
        assert_eq!(f("0.005").to_half_even_display(2), "0.00"); // exact tie, 0 even
        assert_eq!(f("0.015").to_half_even_display(2), "0.02"); // exact tie, 1 odd -> 2
    }

    #[test]
    fn floor_ceil_div() {
        assert_eq!(f("7").floor_div(&f("2")).unwrap().to_string(), "3");
        assert_eq!(f("-7").floor_div(&f("2")).unwrap().to_string(), "-4");
        assert_eq!(f("7").ceil_div(&f("2")).unwrap().to_string(), "4");
        assert_eq!(f("-7").ceil_div(&f("2")).unwrap().to_string(), "-3");
        assert_eq!(f("6").ceil_div(&f("2")).unwrap().to_string(), "3");
    }

    #[test]
    fn serde_string_roundtrip() {
        let x = f("62450.5");
        let json = serde_json::to_string(&x).unwrap();
        assert_eq!(json, "\"124901/2\"");
        let y: Fraction = serde_json::from_str(&json).unwrap();
        assert_eq!(x, y);
        let z: Fraction = serde_json::from_str("\"62450.50\"").unwrap();
        assert_eq!(x, z);
    }

    #[test]
    fn big_values_no_overflow_u64() {
        // Values beyond u64 must stay exact
        let big = f("123456789012345678901234567890.5");
        assert_eq!(big.mul(&f("2")).unwrap().to_string(), "246913578024691357802469135781");
    }

    #[test]
    fn bit_limit_enforced() {
        let huge = BigInt::from(2u32).pow(4100u32);
        let err = Fraction::new(huge, BigInt::one()).unwrap_err();
        assert_eq!(err, MathError::BitLimitExceeded);
    }

    #[test]
    fn pow_exact() {
        assert_eq!(f("1.1").pow_u(3).unwrap().to_string(), "1331/1000");
    }
}
