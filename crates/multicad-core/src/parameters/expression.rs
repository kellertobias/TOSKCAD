use super::{Dimension, ParameterError, ParameterErrorCode, Quantity, Unit};
use std::collections::BTreeSet;

/// A typed formula value in the selected scalar's canonical units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FormulaQuantity<T> {
    pub canonical: T,
    pub dimension: Dimension,
}

impl From<Quantity> for FormulaQuantity<f64> {
    fn from(quantity: Quantity) -> Self {
        Self {
            canonical: quantity.canonical,
            dimension: quantity.dimension,
        }
    }
}

impl From<FormulaQuantity<f64>> for Quantity {
    fn from(quantity: FormulaQuantity<f64>) -> Self {
        Self {
            canonical: quantity.canonical,
            dimension: quantity.dimension,
        }
    }
}

/// Numeric operations for the shared typed formula evaluator. Unit scales
/// and resolved values must use the same canonical representation.
pub trait FormulaScalar: Sized + PartialOrd {
    /// Decode the original numeric token. The parser also supplies the
    /// finite floating-point interpretation for compatibility consumers.
    fn literal(source: &str, parsed_f64: f64) -> Result<Self, ParameterError>;
    fn unit_scale(unit: Unit) -> Result<Self, ParameterError>;
    /// Return the numeric value zero or one.
    fn from_bool(value: bool) -> Self;
    fn negate(self) -> Result<Self, ParameterError>;
    fn add(self, rhs: Self) -> Result<Self, ParameterError>;
    fn subtract(self, rhs: Self) -> Result<Self, ParameterError>;
    fn multiply(self, rhs: Self) -> Result<Self, ParameterError>;
    fn divide(self, rhs: Self) -> Result<Self, ParameterError>;
    fn is_finite(&self) -> bool;
    /// Check that a result can be used as a nonnegative whole count.
    fn is_count(&self) -> bool;
}

impl FormulaScalar for f64 {
    fn literal(_: &str, parsed_f64: f64) -> Result<Self, ParameterError> {
        Ok(parsed_f64)
    }

    fn unit_scale(unit: Unit) -> Result<Self, ParameterError> {
        Ok(unit.scale())
    }

    fn from_bool(value: bool) -> Self {
        f64::from(value)
    }

    fn negate(self) -> Result<Self, ParameterError> {
        Ok(-self)
    }

    fn add(self, rhs: Self) -> Result<Self, ParameterError> {
        Ok(self + rhs)
    }

    fn subtract(self, rhs: Self) -> Result<Self, ParameterError> {
        Ok(self - rhs)
    }

    fn multiply(self, rhs: Self) -> Result<Self, ParameterError> {
        Ok(self * rhs)
    }

    fn divide(self, rhs: Self) -> Result<Self, ParameterError> {
        Ok(self / rhs)
    }

    fn is_finite(&self) -> bool {
        f64::is_finite(*self)
    }

    fn is_count(&self) -> bool {
        *self >= 0. && self.fract().abs() <= 1e-9
    }
}

/// Evaluate a persisted formula using caller-supplied arithmetic and name
/// resolution. A leading '=' enables contextual units in addition and
/// subtraction, as in ordinary project parameter evaluation.
pub fn evaluate_formula<T: FormulaScalar>(
    source: &str,
    unit: Unit,
    resolve: impl FnMut(&str) -> Result<FormulaQuantity<T>, ParameterError>,
) -> Result<FormulaQuantity<T>, ParameterError> {
    parse(source)?.evaluate_declared(unit, source.trim_start().starts_with('='), resolve)
}

#[derive(Clone, Debug)]
pub(crate) enum Expr<'source> {
    Number(&'source str, f64, Option<Unit>),
    Name(String),
    Neg(Box<Self>),
    Binary(char, Box<Self>, Box<Self>),
    Compare(String, Box<Self>, Box<Self>),
}
impl Expr<'_> {
    pub fn names(&self, out: &mut BTreeSet<String>) {
        match self {
            Self::Name(n) => {
                out.insert(n.clone());
            }
            Self::Neg(e) => e.names(out),
            Self::Binary(_, a, b) | Self::Compare(_, a, b) => {
                a.names(out);
                b.names(out);
            }
            _ => {}
        }
    }
    pub fn evaluate(
        &self,
        unit: Unit,
        explicit: bool,
        resolve: &impl Fn(&str) -> Result<Quantity, ParameterError>,
    ) -> Result<(Quantity, bool), ParameterError> {
        let (quantity, implicit) = self.evaluate_with::<f64>(unit, explicit, &mut |name| {
            resolve(name).map(FormulaQuantity::from)
        })?;
        Ok((quantity.into(), implicit))
    }

    pub(crate) fn evaluate_declared<T: FormulaScalar>(
        &self,
        unit: Unit,
        explicit: bool,
        mut resolve: impl FnMut(&str) -> Result<FormulaQuantity<T>, ParameterError>,
    ) -> Result<FormulaQuantity<T>, ParameterError> {
        let (mut quantity, implicit) = self.evaluate_with(unit, explicit, &mut resolve)?;
        if implicit && quantity.dimension == Dimension::RATIO {
            quantity = contextual(quantity, unit)?;
        }
        if quantity.dimension != unit.dimension() {
            return Err(ParameterError::new(
                ParameterErrorCode::Dimension,
                format!(
                    "Expression dimension {:?} does not match unit {}",
                    quantity.dimension,
                    unit.symbol()
                ),
            ));
        }
        if !quantity.canonical.is_finite() {
            return Err(ParameterError::new(
                ParameterErrorCode::NonFinite,
                "Converted result is not finite",
            ));
        }
        if quantity.dimension == Dimension::COUNT && !quantity.canonical.is_count() {
            return Err(ParameterError::new(
                ParameterErrorCode::Count,
                "Counts must be nonnegative whole numbers",
            ));
        }
        Ok(quantity)
    }

    fn evaluate_with<T: FormulaScalar>(
        &self,
        unit: Unit,
        explicit: bool,
        resolve: &mut impl FnMut(&str) -> Result<FormulaQuantity<T>, ParameterError>,
    ) -> Result<(FormulaQuantity<T>, bool), ParameterError> {
        let (q, implicit) = match self {
            Self::Number(raw, parsed, unit) => {
                let value = T::literal(raw, *parsed)?;
                let quantity = FormulaQuantity {
                    canonical: value,
                    dimension: Dimension::RATIO,
                };
                (
                    if let Some(unit) = unit {
                        contextual(quantity, *unit)?
                    } else {
                        quantity
                    },
                    unit.is_none(),
                )
            }
            Self::Name(name) => (resolve(name)?, false),
            Self::Neg(expr) => {
                let (mut q, i) = expr.evaluate_with(unit, explicit, resolve)?;
                q.canonical = q.canonical.negate()?;
                (q, i)
            }
            Self::Compare(op, a, b) => {
                let (a, _) = a.evaluate_with(unit, explicit, resolve)?;
                let (b, _) = b.evaluate_with(unit, explicit, resolve)?;
                if a.dimension != b.dimension {
                    return Err(ParameterError::new(
                        ParameterErrorCode::Dimension,
                        "Comparison requires matching units; give numeric limits explicit units",
                    ));
                }
                let result = match op.as_str() {
                    ">" => a.canonical > b.canonical,
                    "<" => a.canonical < b.canonical,
                    ">=" => a.canonical >= b.canonical,
                    "<=" => a.canonical <= b.canonical,
                    "==" => a.canonical == b.canonical,
                    "!=" => a.canonical != b.canonical,
                    _ => unreachable!(),
                };
                (
                    FormulaQuantity {
                        canonical: T::from_bool(result),
                        dimension: Dimension::RATIO,
                    },
                    false,
                )
            }
            Self::Binary(op, a, b) => {
                let (mut a, ai) = a.evaluate_with(unit, explicit, resolve)?;
                let (mut b, bi) = b.evaluate_with(unit, explicit, resolve)?;
                if explicit && matches!(op, '+' | '-') {
                    if ai && a.dimension != b.dimension {
                        a = contextual(a, unit)?;
                    }
                    if bi && a.dimension != b.dimension {
                        b = contextual(b, unit)?;
                    }
                }
                let dim = match op {
                    '+' | '-' if a.dimension != b.dimension => {
                        return Err(ParameterError::new(
                            ParameterErrorCode::Dimension,
                            "Cannot add or subtract incompatible dimensions; specify units explicitly",
                        ));
                    }
                    '+' | '-' => a.dimension,
                    '*' => a.dimension.combine(b.dimension, 1).ok_or_else(|| {
                        ParameterError::new(
                            ParameterErrorCode::Dimension,
                            "Dimension exponent overflow",
                        )
                    })?,
                    '/' => a.dimension.combine(b.dimension, -1).ok_or_else(|| {
                        ParameterError::new(
                            ParameterErrorCode::Dimension,
                            "Dimension exponent overflow",
                        )
                    })?,
                    _ => unreachable!(),
                };
                let value = match op {
                    '+' => a.canonical.add(b.canonical)?,
                    '-' => a.canonical.subtract(b.canonical)?,
                    '*' => a.canonical.multiply(b.canonical)?,
                    '/' if b.canonical == T::from_bool(false) => {
                        return Err(ParameterError::new(
                            ParameterErrorCode::NonFinite,
                            "Division by zero",
                        ));
                    }
                    '/' => a.canonical.divide(b.canonical)?,
                    _ => unreachable!(),
                };
                (
                    FormulaQuantity {
                        canonical: value,
                        dimension: dim,
                    },
                    ai && bi,
                )
            }
        };
        if !q.canonical.is_finite() {
            return Err(ParameterError::new(
                ParameterErrorCode::NonFinite,
                "Expression result is not finite",
            ));
        }
        Ok((q, implicit))
    }
}
fn contextual<T: FormulaScalar>(
    mut q: FormulaQuantity<T>,
    unit: Unit,
) -> Result<FormulaQuantity<T>, ParameterError> {
    if q.dimension == Dimension::RATIO {
        q.canonical = q.canonical.multiply(T::unit_scale(unit)?)?;
        q.dimension = unit.dimension();
    }
    Ok(q)
}
struct Parser<'a> {
    source: &'a str,
    offset: usize,
    nodes: usize,
    explicit: bool,
}
impl<'source> Parser<'source> {
    fn error(&self, message: &str) -> ParameterError {
        let mut error = ParameterError::new(ParameterErrorCode::Syntax, message);
        error.offset = Some(self.offset);
        error
    }
    fn skip(&mut self) {
        while self
            .source
            .as_bytes()
            .get(self.offset)
            .is_some_and(u8::is_ascii_whitespace)
        {
            self.offset += 1;
        }
    }
    fn peek(&mut self) -> Option<u8> {
        self.skip();
        self.source.as_bytes().get(self.offset).copied()
    }
    fn name(&mut self) -> Option<String> {
        self.skip();
        let start = self.offset;
        while self
            .source
            .as_bytes()
            .get(self.offset)
            .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_')
        {
            self.offset += 1;
        }
        (start != self.offset).then(|| self.source[start..self.offset].to_owned())
    }
    fn expression(&mut self, min: u8, depth: usize) -> Result<Expr<'source>, ParameterError> {
        if depth > 48 || self.nodes >= 256 {
            return Err(self.error("Expression complexity limit exceeded"));
        }
        self.nodes += 1;
        let first = self.peek().ok_or_else(|| self.error("Expected a value"))?;
        let mut left = match first {
            b'-' => {
                self.offset += 1;
                Expr::Neg(Box::new(self.expression(3, depth + 1)?))
            }
            b'+' => {
                self.offset += 1;
                self.expression(3, depth + 1)?
            }
            b'(' => {
                self.offset += 1;
                let value = self.expression(0, depth + 1)?;
                if self.peek() != Some(b')') {
                    return Err(self.error("Expected ')'"));
                }
                self.offset += 1;
                value
            }
            c if c.is_ascii_digit() || c == b'.' => {
                let start = self.offset;
                while self
                    .source
                    .as_bytes()
                    .get(self.offset)
                    .is_some_and(|c| c.is_ascii_digit() || *c == b'.')
                {
                    self.offset += 1;
                }
                if self
                    .source
                    .as_bytes()
                    .get(self.offset)
                    .is_some_and(|c| *c == b'e' || *c == b'E')
                {
                    self.offset += 1;
                    if self
                        .source
                        .as_bytes()
                        .get(self.offset)
                        .is_some_and(|c| *c == b'+' || *c == b'-')
                    {
                        self.offset += 1;
                    }
                    while self
                        .source
                        .as_bytes()
                        .get(self.offset)
                        .is_some_and(u8::is_ascii_digit)
                    {
                        self.offset += 1;
                    }
                }
                let source = self.source;
                let raw = &source[start..self.offset];
                let value = raw
                    .parse::<f64>()
                    .map_err(|_| self.error("Invalid number"))?;
                if !value.is_finite() {
                    return Err(self.error("Number is not finite"));
                }
                let unit = if self.peek() == Some(b'%') {
                    self.offset += 1;
                    Some(Unit::Percent)
                } else if self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
                    let name = self.name().unwrap();
                    Some(
                        Unit::parse(&name)
                            .ok_or_else(|| self.error("Unknown unit after number"))?,
                    )
                } else {
                    None
                };
                Expr::Number(raw, value, unit)
            }
            b'$' => {
                self.offset += 1;
                if !self
                    .peek()
                    .is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
                {
                    return Err(self.error("Expected a parameter name after '$'"));
                }
                Expr::Name(self.name().unwrap())
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                if self.explicit {
                    return Err(self.error("Reference parameters with '$', for example $height"));
                }
                Expr::Name(self.name().unwrap())
            }
            _ => return Err(self.error("Expected a number, parameter name or parentheses")),
        };
        while let Some(op) = self.peek() {
            let precedence = match op {
                b'<' | b'>' | b'=' | b'!' => 0,
                b'+' | b'-' => 1,
                b'*' | b'/' => 2,
                _ => break,
            };
            if precedence < min {
                break;
            }
            let start = self.offset;
            self.offset += 1;
            if precedence == 0 {
                if matches!(left, Expr::Compare(..)) {
                    return Err(self.error("Use one comparison per expression"));
                }
                if self.peek() == Some(b'=') {
                    if self.offset != start + 1 {
                        return Err(self.error(
                            "Keep comparison operator characters together: <=, >=, == or !=",
                        ));
                    }
                    self.offset += 1;
                } else if op == b'=' || op == b'!' {
                    return Err(self.error("Use == or != for equality comparisons"));
                }
            }
            let operator = self.source[start..self.offset].trim().to_string();
            let right = self.expression(precedence + 1, depth + 1)?;
            left = if precedence == 0 {
                Expr::Compare(operator, Box::new(left), Box::new(right))
            } else {
                Expr::Binary(op as char, Box::new(left), Box::new(right))
            };
        }
        Ok(left)
    }
}
pub(crate) fn parse(source: &str) -> Result<Expr<'_>, ParameterError> {
    parse_input(source, false)
}
fn parse_input(source: &str, strict: bool) -> Result<Expr<'_>, ParameterError> {
    if source.len() > 4096 {
        return Err(ParameterError::new(
            ParameterErrorCode::Syntax,
            "Expression exceeds 4096 bytes",
        ));
    }
    let source = source.trim();
    let explicit = strict && source.starts_with('=');
    let source = source.strip_prefix('=').unwrap_or(source);
    let mut parser = Parser {
        source,
        offset: 0,
        nodes: 0,
        explicit,
    };
    let expression = parser.expression(0, 0)?;
    if parser.peek().is_some() {
        return Err(
            parser.error("Unexpected input; function calls and executable code are not supported")
        );
    }
    Ok(expression)
}

impl Expr<'_> {
    fn input(&self) -> String {
        match self {
            Self::Number(_, n, unit) => format!(
                "{}{}",
                n,
                unit.map(|u| format!(" {}", u.symbol())).unwrap_or_default()
            ),
            Self::Name(name) => format!("${name}"),
            Self::Neg(e) => format!("-({})", e.input()),
            Self::Binary(op, a, b) => format!("({} {} {})", a.input(), op, b.input()),
            Self::Compare(op, a, b) => format!("({} {} {})", a.input(), op, b.input()),
        }
    }
    fn literal(&self) -> bool {
        matches!(self, Self::Number(..))
            || matches!(self,Self::Neg(e) if matches!(**e,Self::Number(..)))
    }
}
/// Present legacy formulas using the explicit input notation without changing identity bindings.
pub fn display_input(source: &str) -> String {
    match parse(source) {
        Ok(e) if !e.literal() && validate_input(source).is_err() => format!("={}", e.input()),
        _ => source.into(),
    }
}
/// New UI input distinguishes literals from formulas; persisted legacy sources remain readable.
pub fn validate_input(source: &str) -> Result<(), ParameterError> {
    let e = parse_input(source, true)?;
    if !source.trim_start().starts_with('=') && !e.literal() {
        return Err(ParameterError::new(
            ParameterErrorCode::Syntax,
            "Start expressions with '=' and references with '$'",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
