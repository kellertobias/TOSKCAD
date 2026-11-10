//! Bounded parser for a single ISO 10303-21 DATA section.

use super::ReadError;
use std::collections::BTreeMap;

pub(super) const MAX_BYTES: usize = 16 * 1024 * 1024;
pub(super) const MAX_ENTITIES: usize = 131_072;
const MAX_VALUES: usize = 1_048_576;
const MAX_DEPTH: usize = 24;
const MAX_TOKEN_BYTES: usize = 64 * 1024;
const MAX_COMPONENTS: usize = 64;

#[derive(Debug, PartialEq)]
pub(super) struct Entity<'a> {
    pub(super) name: &'a str,
    pub(super) args: Vec<Value<'a>>,
}

#[derive(Debug, PartialEq)]
pub(super) enum Value<'a> {
    /// The borrowed spelling between quotes, retaining doubled apostrophes.
    String(&'a str),
    /// The enum identifier without its surrounding dots.
    Enum(&'a str),
    Reference(u32),
    Number(f64),
    List(Vec<Value<'a>>),
    Omitted,
    Derived,
    Typed(Entity<'a>),
}

pub(super) fn parse(input: &str) -> Result<BTreeMap<u32, Vec<Entity<'_>>>, ReadError> {
    if input.len() > MAX_BYTES {
        return Err(ReadError::Limit);
    }
    Parser {
        input,
        offset: 0,
        values: 0,
    }
    .file()
}

struct Parser<'a> {
    input: &'a str,
    offset: usize,
    values: usize,
}

impl<'a> Parser<'a> {
    fn rest(&self) -> &'a str {
        &self.input[self.offset..]
    }

    fn skip_trivia(&mut self) -> Result<(), ReadError> {
        loop {
            self.offset += self
                .rest()
                .bytes()
                .take_while(u8::is_ascii_whitespace)
                .count();
            let Some(comment) = self.rest().strip_prefix("/*") else {
                return Ok(());
            };
            self.offset += 4 + comment.find("*/").ok_or(ReadError::Invalid)?;
        }
    }

    fn peek(&mut self) -> Result<Option<u8>, ReadError> {
        self.skip_trivia()?;
        Ok(self.rest().as_bytes().first().copied())
    }

    fn take(&mut self, expected: u8) -> Result<bool, ReadError> {
        if self.peek()? != Some(expected) {
            return Ok(false);
        }
        self.offset += 1;
        Ok(true)
    }

    fn require(&mut self, expected: u8) -> Result<(), ReadError> {
        self.take(expected)?.then_some(()).ok_or(ReadError::Invalid)
    }

    fn keyword(&mut self, expected: &str) -> Result<(), ReadError> {
        self.skip_trivia()?;
        if !self.rest().starts_with(expected) {
            return Err(ReadError::Invalid);
        }
        self.offset += expected.len();
        self.require(b';')
    }

    fn token(&mut self, matches: impl Fn(u8) -> bool) -> Result<&'a str, ReadError> {
        let start = self.offset;
        let count = self
            .rest()
            .bytes()
            .take_while(|b| matches(*b))
            .take(MAX_TOKEN_BYTES + 1)
            .count();
        if count == 0 {
            return Err(ReadError::Invalid);
        }
        if count > MAX_TOKEN_BYTES {
            return Err(ReadError::Limit);
        }
        self.offset += count;
        Ok(&self.input[start..self.offset])
    }

    fn name(&mut self) -> Result<&'a str, ReadError> {
        if !self
            .peek()?
            .is_some_and(|b| b.is_ascii_uppercase() || b == b'_')
        {
            return Err(ReadError::Invalid);
        }
        self.token(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
    }

    fn id(&mut self) -> Result<u32, ReadError> {
        // Reference names cannot have whitespace after the prefix.
        let number = self
            .token(|b| b.is_ascii_digit())?
            .parse::<u32>()
            .map_err(|_| ReadError::Limit)?;
        (number != 0).then_some(number).ok_or(ReadError::Invalid)
    }

    fn quoted(&mut self) -> Result<&'a str, ReadError> {
        let start = self.offset;
        loop {
            let remaining = MAX_TOKEN_BYTES - (self.offset - start);
            // A byte search keeps the token limit independent of the encoding
            // inside the borrowed string. ASCII quote bytes are UTF-8 boundaries.
            let next = self
                .rest()
                .as_bytes()
                .iter()
                .take(remaining + 1)
                .position(|b| *b == b'\'');
            let end = match next {
                Some(next) => self.offset + next,
                None if self.rest().len() > remaining => return Err(ReadError::Limit),
                None => return Err(ReadError::Invalid),
            };
            self.offset = end + 1;
            if !self.rest().starts_with('\'') {
                return Ok(&self.input[start..end]);
            }
            self.offset += 1;
            if self.offset - start > MAX_TOKEN_BYTES {
                return Err(ReadError::Limit);
            }
        }
    }

    fn charge(&mut self) -> Result<(), ReadError> {
        if self.values == MAX_VALUES {
            return Err(ReadError::Limit);
        }
        self.values += 1;
        Ok(())
    }

    fn value(&mut self, depth: usize) -> Result<Value<'a>, ReadError> {
        self.charge()?;
        match self.peek()? {
            Some(b'\'') => {
                self.offset += 1;
                Ok(Value::String(self.quoted()?))
            }
            Some(b'.') => {
                self.offset += 1;
                let name =
                    self.token(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')?;
                if !self.rest().starts_with('.') {
                    return Err(ReadError::Invalid);
                }
                self.offset += 1;
                Ok(Value::Enum(name))
            }
            Some(b'#') => {
                self.offset += 1;
                Ok(Value::Reference(self.id()?))
            }
            Some(b'+' | b'-' | b'0'..=b'9') => {
                let token = self.token(|b| matches!(b, b'+' | b'-' | b'.' | b'E' | b'0'..=b'9'))?;
                let number = token.parse::<f64>().map_err(|_| ReadError::Invalid)?;
                number
                    .is_finite()
                    .then_some(Value::Number(number))
                    .ok_or(ReadError::Invalid)
            }
            Some(b'(') => Ok(Value::List(self.arguments(depth)?)),
            Some(b'$') => {
                self.offset += 1;
                Ok(Value::Omitted)
            }
            Some(b'*') => {
                self.offset += 1;
                Ok(Value::Derived)
            }
            Some(b'A'..=b'Z' | b'_') => Ok(Value::Typed(self.entity(depth)?)),
            Some(b'"') => Err(ReadError::Unsupported),
            _ => Err(ReadError::Invalid),
        }
    }

    fn arguments(&mut self, depth: usize) -> Result<Vec<Value<'a>>, ReadError> {
        if depth >= MAX_DEPTH {
            return Err(ReadError::Limit);
        }
        self.require(b'(')?;
        let mut args = Vec::new();
        if !self.take(b')')? {
            loop {
                args.push(self.value(depth + 1)?);
                if self.take(b')')? {
                    break;
                }
                self.require(b',')?;
            }
        }
        Ok(args)
    }

    fn entity(&mut self, depth: usize) -> Result<Entity<'a>, ReadError> {
        self.charge()?;
        Ok(Entity {
            name: self.name()?,
            args: self.arguments(depth)?,
        })
    }

    fn record(&mut self) -> Result<Vec<Entity<'a>>, ReadError> {
        if !self.take(b'(')? {
            return Ok(vec![self.entity(0)?]);
        }
        let mut components = Vec::new();
        loop {
            if components.len() == MAX_COMPONENTS {
                return Err(ReadError::Limit);
            }
            let entity = self.entity(0)?;
            if components
                .iter()
                .any(|e: &Entity<'_>| e.name == entity.name)
            {
                return Err(ReadError::Invalid);
            }
            components.push(entity);
            if self.take(b')')? {
                return Ok(components);
            }
        }
    }

    fn file(&mut self) -> Result<BTreeMap<u32, Vec<Entity<'a>>>, ReadError> {
        self.keyword("ISO-10303-21")?;
        self.keyword("HEADER")?;
        for required in ["FILE_DESCRIPTION", "FILE_NAME", "FILE_SCHEMA"] {
            if self.entity(0)?.name != required {
                return Err(ReadError::Invalid);
            }
            self.require(b';')?;
        }
        self.skip_trivia()?;
        while !self.rest().starts_with("ENDSEC") {
            if matches!(
                self.entity(0)?.name,
                "FILE_DESCRIPTION" | "FILE_NAME" | "FILE_SCHEMA"
            ) {
                return Err(ReadError::Invalid);
            }
            self.require(b';')?;
            self.skip_trivia()?;
        }
        self.keyword("ENDSEC")?;
        self.skip_trivia()?;
        if !self.rest().starts_with("DATA") {
            return Err(ReadError::Invalid);
        }
        self.offset += "DATA".len();
        if self.peek()? == Some(b'(') {
            return Err(ReadError::Unsupported);
        }
        self.require(b';')?;
        let mut entities = BTreeMap::new();
        while self.take(b'#')? {
            if entities.len() == MAX_ENTITIES {
                return Err(ReadError::Limit);
            }
            let id = self.id()?;
            self.require(b'=')?;
            let record = self.record()?;
            self.require(b';')?;
            if entities.insert(id, record).is_some() {
                return Err(ReadError::Invalid);
            }
        }
        self.keyword("ENDSEC")?;
        self.skip_trivia()?;
        if self.rest().starts_with("DATA") {
            return Err(ReadError::Unsupported);
        }
        self.keyword("END-ISO-10303-21")?;
        if self.peek()?.is_some() {
            return Err(ReadError::Invalid);
        }
        Ok(entities)
    }
}

#[cfg(test)]
mod tests;
