use super::{ParameterError, ParameterErrorCode, Quantity, Unit};
use std::collections::BTreeSet;
#[derive(Clone, Debug)]
pub(crate) enum Expr {
    Number(f64, Option<Unit>),
    Name(String),
    Neg(Box<Self>),
    Binary(char, Box<Self>, Box<Self>),
    Compare(String, Box<Self>, Box<Self>),
}
impl Expr {
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
        let (q, implicit) = match self {
            Self::Number(value, unit) => {
                let unit_value = unit.unwrap_or(Unit::Ratio);
                (
                    Quantity {
                        canonical: value * unit_value.scale(),
                        dimension: unit_value.dimension(),
                    },
                    unit.is_none(),
                )
            }
            Self::Name(name) => (resolve(name)?, false),
            Self::Neg(expr) => {
                let (mut q, i) = expr.evaluate(unit, explicit, resolve)?;
                q.canonical = -q.canonical;
                (q, i)
            }
            Self::Compare(op, a, b) => {
                let (a, _) = a.evaluate(unit, explicit, resolve)?;
                let (b, _) = b.evaluate(unit, explicit, resolve)?;
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
                    Quantity {
                        canonical: if result { 1. } else { 0. },
                        dimension: super::Dimension::RATIO,
                    },
                    false,
                )
            }
            Self::Binary(op, a, b) => {
                let (mut a, ai) = a.evaluate(unit, explicit, resolve)?;
                let (mut b, bi) = b.evaluate(unit, explicit, resolve)?;
                if explicit && matches!(op, '+' | '-') {
                    if ai && a.dimension != b.dimension {
                        a = contextual(a, unit);
                    }
                    if bi && a.dimension != b.dimension {
                        b = contextual(b, unit);
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
                    '+' => a.canonical + b.canonical,
                    '-' => a.canonical - b.canonical,
                    '*' => a.canonical * b.canonical,
                    '/' if b.canonical == 0.0 => {
                        return Err(ParameterError::new(
                            ParameterErrorCode::NonFinite,
                            "Division by zero",
                        ));
                    }
                    '/' => a.canonical / b.canonical,
                    _ => unreachable!(),
                };
                (
                    Quantity {
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
fn contextual(mut q: Quantity, unit: Unit) -> Quantity {
    if q.dimension == super::Dimension::RATIO {
        q.canonical *= unit.scale();
        q.dimension = unit.dimension();
    }
    q
}
struct Parser<'a> {
    source: &'a str,
    offset: usize,
    nodes: usize,
    explicit: bool,
}
impl Parser<'_> {
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
    fn expression(&mut self, min: u8, depth: usize) -> Result<Expr, ParameterError> {
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
                let value = self.source[start..self.offset]
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
                Expr::Number(value, unit)
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
pub(crate) fn parse(source: &str) -> Result<Expr, ParameterError> {
    if source.len() > 4096 {
        return Err(ParameterError::new(
            ParameterErrorCode::Syntax,
            "Expression exceeds 4096 bytes",
        ));
    }
    let source = source.trim();
    let explicit = source.starts_with('=');
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

impl Expr {
    fn input(&self) -> String {
        match self {
            Self::Number(n, unit) => format!(
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
        Ok(e) if !source.trim_start().starts_with('=') && !e.literal() => format!("={}", e.input()),
        _ => source.into(),
    }
}
/// New UI input distinguishes literals from formulas; persisted legacy sources remain readable.
pub fn validate_input(source: &str) -> Result<(), ParameterError> {
    let e = parse(source)?;
    if !source.trim_start().starts_with('=') && !e.literal() {
        return Err(ParameterError::new(
            ParameterErrorCode::Syntax,
            "Start expressions with '=' and references with '$'",
        ));
    }
    Ok(())
}
