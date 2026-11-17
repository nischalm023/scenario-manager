//! Within `site-template`, an `expression` is a knob expression (spec 05 section 3.3): the integer arithmetic over a
//! template's knobs that a binding, a presence condition and a check each hold. This module parses its grammar,
//! evaluates it on signed 64-bit integers with floor division, and checks its dimensions, so that a length can never
//! be multiplied by a length or added to a count.

use thiserror::Error;

/// The longest expression the grammar takes, in characters.
const MAX_CHARS: usize = 512;
/// The deepest nesting of parentheses, a call's included.
const MAX_DEPTH: usize = 32;

/// A parsed knob expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    Integer(i64),
    Knob(String),
    Add(Box<Expr>, Box<Expr>),
    Sub(Box<Expr>, Box<Expr>),
    Mul(Box<Expr>, Box<Expr>),
    Div(Box<Expr>, Box<Expr>),
    Min(Vec<Expr>),
    Max(Vec<Expr>),
}

/// A string the grammar refuses, and why.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("`{text}` is not a knob expression: {why}")]
pub struct GrammarFault {
    pub text: String,
    pub why: String,
}

/// Why an expression has no value at the knobs it was evaluated at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum EvalFault {
    #[error("the arithmetic leaves the signed 64-bit integers")]
    Overflow,
    #[error("it divides by zero")]
    DivisionByZero,
}

/// What a knob or a value measures (spec 05 section 3.3, "Dimensions").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dim {
    Count,
    Length,
    Angle,
}

/// Two dimensions an operator cannot join, in words.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{0}")]
pub struct DimFault(pub String);

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Integer(i64),
    Name(String),
    Plus,
    Minus,
    Star,
    Slash,
    Open,
    Close,
    Comma,
}

impl Expr {
    pub fn parse(text: &str) -> Result<Expr, GrammarFault> {
        let refuse = |why: String| GrammarFault { text: text.to_owned(), why };
        if text.chars().count() > MAX_CHARS {
            return Err(refuse(format!("it is longer than {MAX_CHARS} characters")));
        }
        let tokens = tokens(text).map_err(refuse)?;
        let mut parser = Parser { tokens: &tokens, at: 0, depth: 0 };
        let expr = parser.expression().map_err(refuse)?;
        match parser.tokens.get(parser.at) {
            None => Ok(expr),
            Some(token) => Err(refuse(format!("{} follows a whole expression", describe(token)))),
        }
    }

    /// Every knob the expression names, in the order written, repeats included.
    pub fn knobs(&self) -> Vec<&str> {
        let mut out = Vec::new();
        self.collect_knobs(&mut out);
        out
    }

    fn collect_knobs<'a>(&'a self, out: &mut Vec<&'a str>) {
        match self {
            Expr::Integer(_) => {}
            Expr::Knob(name) => out.push(name),
            Expr::Add(a, b) | Expr::Sub(a, b) | Expr::Mul(a, b) | Expr::Div(a, b) => {
                a.collect_knobs(out);
                b.collect_knobs(out);
            }
            Expr::Min(args) | Expr::Max(args) => args.iter().for_each(|arg| arg.collect_knobs(out)),
        }
    }

    /// The value at the knob values `value` gives, `/` rounding toward negative infinity.
    pub fn eval(&self, value: &impl Fn(&str) -> i64) -> Result<i64, EvalFault> {
        let pair = |a: &Expr, b: &Expr| Ok::<_, EvalFault>((a.eval(value)?, b.eval(value)?));
        match self {
            Expr::Integer(n) => Ok(*n),
            Expr::Knob(name) => Ok(value(name)),
            Expr::Add(a, b) => pair(a, b).and_then(|(a, b)| a.checked_add(b).ok_or(EvalFault::Overflow)),
            Expr::Sub(a, b) => pair(a, b).and_then(|(a, b)| a.checked_sub(b).ok_or(EvalFault::Overflow)),
            Expr::Mul(a, b) => pair(a, b).and_then(|(a, b)| a.checked_mul(b).ok_or(EvalFault::Overflow)),
            Expr::Div(a, b) => pair(a, b).and_then(|(a, b)| floor_div(a, b)),
            Expr::Min(args) => fold(args, value, i64::min),
            Expr::Max(args) => fold(args, value, i64::max),
        }
    }

    /// Check the expression's dimensions where it is read as `want` (`None`: anything), each knob's from `dim_of`.
    /// A literal takes the dimension its place needs, from the knobs it meets and from `want`.
    pub fn check_dims(&self, dim_of: &impl Fn(&str) -> Dim, want: Option<Dim>) -> Result<(), DimFault> {
        match self {
            Expr::Integer(_) => Ok(()),
            Expr::Knob(name) => match want {
                Some(wanted) if wanted != dim_of(name) => {
                    Err(DimFault(format!("`{name}` is {}, where {} is needed", word(dim_of(name)), word(wanted))))
                }
                _ => Ok(()),
            },
            Expr::Add(a, b) | Expr::Sub(a, b) => {
                let dim = match want {
                    Some(wanted) => Some(wanted),
                    None => self.infer(dim_of)?,
                };
                a.check_dims(dim_of, dim)?;
                b.check_dims(dim_of, dim)
            }
            Expr::Min(args) | Expr::Max(args) => {
                let dim = match want {
                    Some(wanted) => Some(wanted),
                    None => self.infer(dim_of)?,
                };
                args.iter().try_for_each(|arg| arg.check_dims(dim_of, dim))
            }
            Expr::Mul(a, b) => match (a.infer(dim_of)?, b.infer(dim_of)?) {
                (Some(x), Some(y)) => settled(times(x, y)?, want),
                (Some(x), None) => {
                    a.check_dims(dim_of, Some(x))?;
                    b.check_dims(dim_of, times_partner(x, want)?)
                }
                (None, Some(y)) => {
                    b.check_dims(dim_of, Some(y))?;
                    a.check_dims(dim_of, times_partner(y, want)?)
                }
                (None, None) => a.check_dims(dim_of, None).and_then(|()| b.check_dims(dim_of, None)),
            },
            Expr::Div(a, b) => match (a.infer(dim_of)?, b.infer(dim_of)?) {
                (Some(x), Some(y)) => settled(over(x, y)?, want),
                (Some(x), None) => {
                    a.check_dims(dim_of, Some(x))?;
                    b.check_dims(dim_of, denominator_for(x, want)?)
                }
                (None, Some(y)) => {
                    b.check_dims(dim_of, Some(y))?;
                    a.check_dims(dim_of, numerator_for(y, want)?)
                }
                (None, None) => a.check_dims(dim_of, None).and_then(|()| b.check_dims(dim_of, None)),
            },
        }
    }

    /// The dimension the expression has whatever its place, read bottom-up, or `None` while only literals decide
    /// it; a fault where two knobs' dimensions cannot meet.
    fn infer(&self, dim_of: &impl Fn(&str) -> Dim) -> Result<Option<Dim>, DimFault> {
        let one_of = |operands: &[&Expr]| -> Result<Option<Dim>, DimFault> {
            let mut found: Option<Dim> = None;
            for operand in operands {
                match (found, operand.infer(dim_of)?) {
                    (Some(seen), Some(dim)) if seen != dim => {
                        return Err(DimFault(format!("{} meets {} where they must be equal", word(seen), word(dim))));
                    }
                    (None, Some(dim)) => found = Some(dim),
                    _ => {}
                }
            }
            Ok(found)
        };
        match self {
            Expr::Integer(_) => Ok(None),
            Expr::Knob(name) => Ok(Some(dim_of(name))),
            Expr::Add(a, b) | Expr::Sub(a, b) => one_of(&[a.as_ref(), b.as_ref()]),
            Expr::Min(args) | Expr::Max(args) => one_of(&args.iter().collect::<Vec<_>>()),
            Expr::Mul(a, b) => Ok(match (a.infer(dim_of)?, b.infer(dim_of)?) {
                (Some(x), Some(y)) => Some(times(x, y)?),
                (Some(Dim::Count), None) | (None, Some(Dim::Count)) | (None, None) => None,
                (Some(dim), None) | (None, Some(dim)) => Some(dim),
            }),
            Expr::Div(a, b) => Ok(match (a.infer(dim_of)?, b.infer(dim_of)?) {
                (Some(x), Some(y)) => Some(over(x, y)?),
                (Some(Dim::Count), None) => Some(Dim::Count),
                (None, Some(Dim::Count)) | (Some(_), None) | (None, None) => None,
                (None, Some(_)) => Some(Dim::Count),
            }),
        }
    }
}

fn floor_div(a: i64, b: i64) -> Result<i64, EvalFault> {
    if b == 0 {
        return Err(EvalFault::DivisionByZero);
    }
    let quotient = a.checked_div(b).ok_or(EvalFault::Overflow)?;
    let rounds_down = a % b != 0 && ((a < 0) != (b < 0));
    Ok(if rounds_down { quotient - 1 } else { quotient })
}

fn fold(args: &[Expr], value: &impl Fn(&str) -> i64, pick: fn(i64, i64) -> i64) -> Result<i64, EvalFault> {
    let mut best: Option<i64> = None;
    for arg in args {
        let n = arg.eval(value)?;
        best = Some(best.map_or(n, |b| pick(b, n)));
    }
    Ok(best.expect("the grammar gives a call two arguments or more"))
}

fn word(dim: Dim) -> &'static str {
    match dim {
        Dim::Count => "a count",
        Dim::Length => "a length",
        Dim::Angle => "an angle",
    }
}

fn settled(dim: Dim, want: Option<Dim>) -> Result<(), DimFault> {
    match want {
        Some(wanted) if wanted != dim => {
            Err(DimFault(format!("the expression gives {}, where {} is needed", word(dim), word(wanted))))
        }
        _ => Ok(()),
    }
}

fn times(x: Dim, y: Dim) -> Result<Dim, DimFault> {
    match (x, y) {
        (Dim::Count, other) | (other, Dim::Count) => Ok(other),
        _ => Err(DimFault(format!("{} times {} is no dimension a knob can set", word(x), word(y)))),
    }
}

fn over(x: Dim, y: Dim) -> Result<Dim, DimFault> {
    match (x, y) {
        (_, Dim::Count) => Ok(x),
        (a, b) if a == b => Ok(Dim::Count),
        _ => Err(DimFault(format!("{} over {} is no dimension a knob can set", word(x), word(y)))),
    }
}

/// What the other factor of a product with one `x` factor must be for the product to be `want`.
fn times_partner(x: Dim, want: Option<Dim>) -> Result<Option<Dim>, DimFault> {
    match (x, want) {
        (Dim::Count, want) => Ok(want),
        (_, None) => Ok(Some(Dim::Count)),
        (x, Some(wanted)) if x == wanted => Ok(Some(Dim::Count)),
        (x, Some(wanted)) => Err(DimFault(format!("{} times anything is never {}", word(x), word(wanted)))),
    }
}

/// The denominator a quotient of an `x` numerator needs to be `want`.
fn denominator_for(x: Dim, want: Option<Dim>) -> Result<Option<Dim>, DimFault> {
    match (x, want) {
        (Dim::Count, None | Some(Dim::Count)) => Ok(Some(Dim::Count)),
        (_, None) => Ok(None),
        (x, Some(wanted)) if x == wanted => Ok(Some(Dim::Count)),
        (x, Some(Dim::Count)) => Ok(Some(x)),
        (x, Some(wanted)) => Err(DimFault(format!("{} over anything is never {}", word(x), word(wanted)))),
    }
}

/// The numerator a quotient over a `y` denominator needs to be `want`.
fn numerator_for(y: Dim, want: Option<Dim>) -> Result<Option<Dim>, DimFault> {
    match (y, want) {
        (Dim::Count, want) => Ok(want),
        (y, None | Some(Dim::Count)) => Ok(Some(y)),
        (y, Some(wanted)) => Err(DimFault(format!("anything over {} is never {}", word(y), word(wanted)))),
    }
}

fn describe(token: &Token) -> String {
    match token {
        Token::Integer(n) => format!("`{n}`"),
        Token::Name(name) => format!("`{name}`"),
        Token::Plus => "`+`".into(),
        Token::Minus => "`-`".into(),
        Token::Star => "`*`".into(),
        Token::Slash => "`/`".into(),
        Token::Open => "`(`".into(),
        Token::Close => "`)`".into(),
        Token::Comma => "`,`".into(),
    }
}

fn tokens(text: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut at = 0;
    while at < chars.len() {
        let c = chars[at];
        let single = match c {
            ' ' => {
                at += 1;
                continue;
            }
            '+' => Some(Token::Plus),
            '-' => Some(Token::Minus),
            '*' => Some(Token::Star),
            '/' => Some(Token::Slash),
            '(' => Some(Token::Open),
            ')' => Some(Token::Close),
            ',' => Some(Token::Comma),
            _ => None,
        };
        if let Some(token) = single {
            out.push(token);
            at += 1;
            continue;
        }
        let start = at;
        if c.is_ascii_digit() {
            while at < chars.len() && chars[at].is_ascii_digit() {
                at += 1;
            }
            let digits: String = chars[start..at].iter().collect();
            if digits.len() > 1 && digits.starts_with('0') {
                return Err(format!("`{digits}` has a leading zero"));
            }
            let n = digits.parse().map_err(|_| format!("`{digits}` is larger than a 64-bit integer"))?;
            out.push(Token::Integer(n));
        } else if c.is_ascii_lowercase() {
            while at < chars.len() && (chars[at].is_ascii_lowercase() || chars[at].is_ascii_digit() || chars[at] == '_')
            {
                at += 1;
            }
            out.push(Token::Name(chars[start..at].iter().collect()));
        } else {
            return Err(format!(
                "`{c}` is outside the grammar, which takes integers, knob keys, + - * /, min, max, parentheses, commas and spaces"
            ));
        }
    }
    Ok(out)
}

struct Parser<'t> {
    tokens: &'t [Token],
    at: usize,
    depth: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn next(&mut self) -> Option<&Token> {
        let token = self.tokens.get(self.at);
        self.at += 1;
        token
    }

    fn expect(&mut self, wanted: Token) -> Result<(), String> {
        match self.next() {
            Some(token) if *token == wanted => Ok(()),
            Some(token) => Err(format!("{} stands where {} belongs", describe(token), describe(&wanted))),
            None => Err(format!("it ends where {} belongs", describe(&wanted))),
        }
    }

    fn open(&mut self) -> Result<(), String> {
        self.expect(Token::Open)?;
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(format!("it nests deeper than {MAX_DEPTH} parentheses"));
        }
        Ok(())
    }

    fn expression(&mut self) -> Result<Expr, String> {
        let mut left = self.term()?;
        while let Some(op @ (Token::Plus | Token::Minus)) = self.peek().cloned() {
            self.at += 1;
            let right = Box::new(self.term()?);
            left = if op == Token::Plus { Expr::Add(Box::new(left), right) } else { Expr::Sub(Box::new(left), right) };
        }
        Ok(left)
    }

    fn term(&mut self) -> Result<Expr, String> {
        let mut left = self.factor()?;
        while let Some(op @ (Token::Star | Token::Slash)) = self.peek().cloned() {
            self.at += 1;
            let right = Box::new(self.factor()?);
            left = if op == Token::Star { Expr::Mul(Box::new(left), right) } else { Expr::Div(Box::new(left), right) };
        }
        Ok(left)
    }

    fn factor(&mut self) -> Result<Expr, String> {
        match self.peek().cloned() {
            Some(Token::Integer(n)) => {
                self.at += 1;
                Ok(Expr::Integer(n))
            }
            Some(Token::Name(name)) if name == "min" || name == "max" => {
                self.at += 1;
                self.open()?;
                let mut args = vec![self.expression()?];
                while self.peek() == Some(&Token::Comma) {
                    self.at += 1;
                    args.push(self.expression()?);
                }
                self.expect(Token::Close)?;
                self.depth -= 1;
                if args.len() < 2 {
                    return Err(format!("`{name}` takes two arguments or more"));
                }
                Ok(if name == "min" { Expr::Min(args) } else { Expr::Max(args) })
            }
            Some(Token::Name(name)) => {
                self.at += 1;
                Ok(Expr::Knob(name))
            }
            Some(Token::Open) => {
                self.open()?;
                let inner = self.expression()?;
                self.expect(Token::Close)?;
                self.depth -= 1;
                Ok(inner)
            }
            Some(Token::Minus) => Err("a sign is not part of the grammar: write `0 - 3` for -3".into()),
            Some(token) => Err(format!("{} stands where a number, a knob or `(` belongs", describe(&token))),
            None => Err("it ends where a number, a knob or `(` belongs".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(name: &str) -> i64 {
        match name {
            "aisles" => 6,
            "width" => 40800,
            "orientation" => 3,
            _ => 0,
        }
    }

    fn dim(name: &str) -> Dim {
        match name {
            "width" | "depth" => Dim::Length,
            _ => Dim::Count,
        }
    }

    /// Spec 05 section 9: the grammar parses every example in the spec.
    #[test]
    fn every_example_of_spec_05_parses() {
        for text in [
            "aisles * 5950 + 13600 - width",
            "1 - min(1, pick_plus_x)",
            "1 - 2 * (orientation - 2 * (orientation / 2))",
            "width * (orientation - 2 * (orientation / 2))",
            "width - 13600",
            "htms",
            "0 - 3400",
            "(0 - 7) / 2",
            "max(1, aisles, 3)",
        ] {
            Expr::parse(text).unwrap_or_else(|fault| panic!("{fault}"));
        }
    }

    /// Spec 05 section 9: the grammar refuses a sign, `min` with one argument, 513 characters and depth 33.
    #[test]
    fn the_grammar_refuses_what_spec_05_refuses() {
        for text in ["-3", "0 - -3", "min(1)", "007", "aisles\t+ 1", "aisles +", "max", "(1"] {
            assert!(Expr::parse(text).is_err(), "{text}");
        }
        let long = format!("1{}", " + 1".repeat(128));
        assert_eq!(long.len(), 513);
        assert!(Expr::parse(&long).is_err());
        assert!(Expr::parse(&long[..509]).is_ok());
        let nest = |depth: usize| format!("{}1{}", "(".repeat(depth), ")".repeat(depth));
        assert!(Expr::parse(&nest(32)).is_ok());
        assert!(Expr::parse(&nest(33)).is_err());
    }

    /// Spec 05 section 9: `(0 - 7) / 2` is -4; an overflow and a zero divisor are named faults.
    #[test]
    fn division_floors_and_faults_are_named() {
        let eval = |text: &str| Expr::parse(text).unwrap().eval(&value);
        assert_eq!(eval("(0 - 7) / 2"), Ok(-4));
        assert_eq!(eval("7 / (0 - 2)"), Ok(-4));
        assert_eq!(eval("7 / 2"), Ok(3));
        assert_eq!(eval("aisles * 5950 + 13600 - width"), Ok(8500));
        assert_eq!(eval("1 - 2 * (orientation - 2 * (orientation / 2))"), Ok(-1));
        assert_eq!(eval("9223372036854775807 + 1"), Err(EvalFault::Overflow));
        assert_eq!(eval("1 / (aisles - aisles)"), Err(EvalFault::DivisionByZero));
    }

    /// Spec 05 section 9: length times length is a dimension mismatch; a literal takes the dimension it meets.
    #[test]
    fn dimensions_meet_or_are_refused() {
        let check = |text: &str, want| Expr::parse(text).unwrap().check_dims(&dim, want);
        assert!(check("aisles * 5950 + 13600 - width", Some(Dim::Length)).is_ok());
        assert!(check("(aisles * 2750 + 2549) / 850 * 850", Some(Dim::Length)).is_ok());
        assert!(check("width / 2", Some(Dim::Length)).is_ok());
        assert!(check("width / depth", Some(Dim::Count)).is_ok());
        assert!(check("width * depth", None).is_err());
        assert!(check("width + aisles", None).is_err());
        assert!(check("aisles", Some(Dim::Length)).is_err());
        assert!(check("aisles / width", None).is_err());
    }
}
