//! A dependency-free expression evaluator and unit/currency converter, used by
//! the calculator when `qalc` isn't installed.
//!
//! - Arithmetic: `+ - * / % ^`, unary minus, parentheses, and a handful of
//!   functions (`sqrt`, `sin`, `cos`, `tan`, `ln`, `log`, `abs`, `round`,
//!   `floor`, `ceil`) plus constants `pi`, `e`, `tau`.
//! - Conversion: `10 km in mi`, `100 f to c`, `5 gb in mb`, `10 usd in eur`.
//!   Currency uses an embedded approximate rate table (offline).

/// Evaluate an expression or conversion. Returns a display string, or `None`
/// when the input isn't something we can compute.
pub fn evaluate(input: &str) -> Option<String> {
    let s = input.trim();
    if s.is_empty() {
        return None;
    }
    if let Some(r) = try_convert(s) {
        return Some(r);
    }
    let v = eval_arith(s)?;
    Some(fmt_num(v))
}

/// Format a float without a trailing `.0` and without noise digits.
pub fn fmt_num(v: f64) -> String {
    if !v.is_finite() {
        return "∞".into();
    }
    if (v.round() - v).abs() < 1e-9 && v.abs() < 1e15 {
        return format!("{}", v.round() as i64);
    }
    // up to 6 significant decimals, trimmed
    let s = format!("{v:.6}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    s.to_string()
}

// ── arithmetic (recursive-descent) ──────────────────────────────

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

pub fn eval_arith(expr: &str) -> Option<f64> {
    let lower = expr.to_lowercase();
    let mut p = Parser { s: lower.as_bytes(), i: 0 };
    let v = p.expr()?;
    p.ws();
    if p.i != p.s.len() {
        return None; // trailing garbage
    }
    if v.is_finite() {
        Some(v)
    } else {
        None
    }
}

impl<'a> Parser<'a> {
    fn ws(&mut self) {
        while self.i < self.s.len() && (self.s[self.i] as char).is_whitespace() {
            self.i += 1;
        }
    }
    fn peek(&mut self) -> Option<u8> {
        self.ws();
        self.s.get(self.i).copied()
    }
    // expr := term (('+' | '-') term)*
    fn expr(&mut self) -> Option<f64> {
        let mut v = self.term()?;
        while let Some(c) = self.peek() {
            match c {
                b'+' => {
                    self.i += 1;
                    v += self.term()?;
                }
                b'-' => {
                    self.i += 1;
                    v -= self.term()?;
                }
                _ => break,
            }
        }
        Some(v)
    }
    // term := power (('*' | '/' | '%') power)*
    fn term(&mut self) -> Option<f64> {
        let mut v = self.power()?;
        while let Some(c) = self.peek() {
            match c {
                b'*' => {
                    self.i += 1;
                    v *= self.power()?;
                }
                b'/' => {
                    self.i += 1;
                    v /= self.power()?;
                }
                b'%' => {
                    self.i += 1;
                    v %= self.power()?;
                }
                _ => break,
            }
        }
        Some(v)
    }
    // power := unary ('^' power)?   (right-associative)
    fn power(&mut self) -> Option<f64> {
        let base = self.unary()?;
        if let Some(b'^') = self.peek() {
            self.i += 1;
            let exp = self.power()?;
            return Some(base.powf(exp));
        }
        Some(base)
    }
    // unary := ('-' | '+')? atom
    fn unary(&mut self) -> Option<f64> {
        match self.peek() {
            Some(b'-') => {
                self.i += 1;
                Some(-self.unary()?)
            }
            Some(b'+') => {
                self.i += 1;
                self.unary()
            }
            _ => self.atom(),
        }
    }
    // atom := number | constant | func '(' expr ')' | '(' expr ')'
    fn atom(&mut self) -> Option<f64> {
        let c = self.peek()?;
        if c == b'(' {
            self.i += 1;
            let v = self.expr()?;
            if self.peek() == Some(b')') {
                self.i += 1;
                return Some(v);
            }
            return None;
        }
        if c.is_ascii_alphabetic() {
            let name = self.ident();
            match name.as_str() {
                "pi" => return Some(std::f64::consts::PI),
                "tau" => return Some(std::f64::consts::TAU),
                "e" => return Some(std::f64::consts::E),
                _ => {}
            }
            // function call
            if self.peek() == Some(b'(') {
                self.i += 1;
                let arg = self.expr()?;
                if self.peek() != Some(b')') {
                    return None;
                }
                self.i += 1;
                return apply_fn(&name, arg);
            }
            return None;
        }
        self.number()
    }
    fn ident(&mut self) -> String {
        self.ws();
        let start = self.i;
        while self.i < self.s.len() && (self.s[self.i] as char).is_ascii_alphabetic() {
            self.i += 1;
        }
        String::from_utf8_lossy(&self.s[start..self.i]).into_owned()
    }
    fn number(&mut self) -> Option<f64> {
        self.ws();
        let start = self.i;
        let mut seen_dot = false;
        while self.i < self.s.len() {
            let c = self.s[self.i];
            if c.is_ascii_digit() {
                self.i += 1;
            } else if c == b'.' && !seen_dot {
                seen_dot = true;
                self.i += 1;
            } else if (c == b'e') && self.i > start {
                // scientific notation: 1e3, 2.5e-2
                self.i += 1;
                if matches!(self.s.get(self.i), Some(b'+') | Some(b'-')) {
                    self.i += 1;
                }
            } else {
                break;
            }
        }
        if self.i == start {
            return None;
        }
        String::from_utf8_lossy(&self.s[start..self.i]).parse().ok()
    }
}

fn apply_fn(name: &str, x: f64) -> Option<f64> {
    Some(match name {
        "sqrt" => x.sqrt(),
        "abs" => x.abs(),
        "sin" => x.sin(),
        "cos" => x.cos(),
        "tan" => x.tan(),
        "ln" => x.ln(),
        "log" => x.log10(),
        "round" => x.round(),
        "floor" => x.floor(),
        "ceil" => x.ceil(),
        _ => return None,
    })
}

// ── conversions ─────────────────────────────────────────────────

/// `<number> <unit> (in|to) <unit>` → formatted result, or `None`.
fn try_convert(s: &str) -> Option<String> {
    let lower = s.to_lowercase();
    // split on " in " or " to "
    let (lhs, to_unit) = split_conv(&lower)?;
    let lhs = lhs.trim();
    // parse leading number, rest is the from-unit
    let num_end = lhs
        .char_indices()
        .take_while(|(_, c)| c.is_ascii_digit() || *c == '.' || *c == '-' || *c == '+')
        .last()
        .map(|(i, c)| i + c.len_utf8())?;
    let num: f64 = lhs[..num_end].trim().parse().ok()?;
    let from_unit = lhs[num_end..].trim();
    if from_unit.is_empty() {
        return None;
    }
    convert(num, from_unit, to_unit.trim())
}

fn split_conv(s: &str) -> Option<(&str, &str)> {
    if let Some(idx) = s.find(" in ") {
        return Some((&s[..idx], &s[idx + 4..]));
    }
    if let Some(idx) = s.find(" to ") {
        return Some((&s[..idx], &s[idx + 4..]));
    }
    None
}

/// A physical dimension: everything is stored as a factor to a base unit.
struct Dim {
    /// (aliases, factor-to-base)
    units: &'static [(&'static [&'static str], f64)],
}

// length base = metre
const LENGTH: Dim = Dim {
    units: &[
        (&["mm", "millimeter", "millimetre"], 0.001),
        (&["cm", "centimeter", "centimetre"], 0.01),
        (&["m", "meter", "metre"], 1.0),
        (&["km", "kilometer", "kilometre"], 1000.0),
        (&["in", "inch", "inches"], 0.0254),
        (&["ft", "foot", "feet"], 0.3048),
        (&["yd", "yard", "yards"], 0.9144),
        (&["mi", "mile", "miles"], 1609.344),
        (&["nmi", "nauticalmile"], 1852.0),
    ],
};
// mass base = gram
const MASS: Dim = Dim {
    units: &[
        (&["mg", "milligram"], 0.001),
        (&["g", "gram", "grams"], 1.0),
        (&["kg", "kilogram", "kilograms"], 1000.0),
        (&["t", "tonne", "ton"], 1_000_000.0),
        (&["oz", "ounce", "ounces"], 28.349523125),
        (&["lb", "lbs", "pound", "pounds"], 453.59237),
        (&["st", "stone"], 6350.29318),
    ],
};
// data base = byte
const DATA: Dim = Dim {
    units: &[
        (&["b", "byte", "bytes"], 1.0),
        (&["kb", "kilobyte"], 1000.0),
        (&["mb", "megabyte"], 1_000_000.0),
        (&["gb", "gigabyte"], 1_000_000_000.0),
        (&["tb", "terabyte"], 1_000_000_000_000.0),
        (&["kib"], 1024.0),
        (&["mib"], 1_048_576.0),
        (&["gib"], 1_073_741_824.0),
        (&["tib"], 1_099_511_627_776.0),
        (&["bit", "bits"], 0.125),
    ],
};
// time base = second
const TIME: Dim = Dim {
    units: &[
        (&["ms", "millisecond", "milliseconds"], 0.001),
        (&["s", "sec", "second", "seconds"], 1.0),
        (&["min", "minute", "minutes"], 60.0),
        (&["h", "hr", "hour", "hours"], 3600.0),
        (&["d", "day", "days"], 86400.0),
        (&["w", "week", "weeks"], 604800.0),
        (&["y", "yr", "year", "years"], 31_557_600.0),
    ],
};

const DIMS: &[&Dim] = &[&LENGTH, &MASS, &DATA, &TIME];

/// Approximate currency rates relative to USD (offline fallback). Updated
/// occasionally; not for financial use. Rate = units per 1 USD.
const CURRENCY: &[(&str, f64)] = &[
    ("usd", 1.0),
    ("eur", 0.92),
    ("gbp", 0.79),
    ("try", 32.5),
    ("jpy", 157.0),
    ("cny", 7.24),
    ("inr", 83.3),
    ("cad", 1.36),
    ("aud", 1.51),
    ("chf", 0.89),
    ("rub", 92.0),
    ("brl", 5.1),
    ("krw", 1360.0),
];

fn convert(num: f64, from: &str, to: &str) -> Option<String> {
    // temperature is affine, handle specially
    if let Some(r) = convert_temp(num, from, to) {
        return Some(format!("{} {}", fmt_num(r), canonical_temp(to)));
    }
    // currency
    if let (Some(fr), Some(tr)) = (currency_rate(from), currency_rate(to)) {
        let usd = num / fr;
        let out = usd * tr;
        return Some(format!("{} {} (approx)", fmt_num(out), to.to_uppercase()));
    }
    // dimensional
    for d in DIMS {
        if let (Some(ff), Some(tf)) = (unit_factor(d, from), unit_factor(d, to)) {
            let base = num * ff;
            return Some(format!("{} {}", fmt_num(base / tf), to));
        }
    }
    None
}

fn unit_factor(d: &Dim, name: &str) -> Option<f64> {
    d.units
        .iter()
        .find(|(aliases, _)| aliases.contains(&name))
        .map(|(_, f)| *f)
}

fn currency_rate(name: &str) -> Option<f64> {
    CURRENCY.iter().find(|(c, _)| *c == name).map(|(_, r)| *r)
}

fn convert_temp(num: f64, from: &str, to: &str) -> Option<f64> {
    let celsius = match from {
        "c" | "celsius" => num,
        "f" | "fahrenheit" => (num - 32.0) * 5.0 / 9.0,
        "k" | "kelvin" => num - 273.15,
        _ => return None,
    };
    Some(match to {
        "c" | "celsius" => celsius,
        "f" | "fahrenheit" => celsius * 9.0 / 5.0 + 32.0,
        "k" | "kelvin" => celsius + 273.15,
        _ => return None,
    })
}

fn canonical_temp(to: &str) -> &'static str {
    match to {
        "c" | "celsius" => "°C",
        "f" | "fahrenheit" => "°F",
        "k" | "kelvin" => "K",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arithmetic() {
        assert_eq!(evaluate("2+2").unwrap(), "4");
        assert_eq!(evaluate("2 * (3 + 4)").unwrap(), "14");
        assert_eq!(evaluate("2^10").unwrap(), "1024");
        assert_eq!(evaluate("10 % 3").unwrap(), "1");
        assert_eq!(evaluate("sqrt(144)").unwrap(), "12");
        assert_eq!(evaluate("-5 + 3").unwrap(), "-2");
    }

    #[test]
    fn constants_and_funcs() {
        assert_eq!(evaluate("round(pi)").unwrap(), "3");
        assert!(evaluate("abc").is_none());
        assert!(evaluate("2 +").is_none());
    }

    #[test]
    fn length_conversion() {
        assert_eq!(evaluate("1 km in m").unwrap(), "1000 m");
        assert_eq!(evaluate("100 cm to m").unwrap(), "1 m");
    }

    #[test]
    fn temperature_conversion() {
        assert_eq!(evaluate("100 c in f").unwrap(), "212 °F");
        assert_eq!(evaluate("32 f to c").unwrap(), "0 °C");
    }

    #[test]
    fn data_conversion() {
        assert_eq!(evaluate("5 gb in mb").unwrap(), "5000 mb");
    }

    #[test]
    fn currency_conversion() {
        let r = evaluate("10 usd in eur").unwrap();
        assert!(r.contains("EUR"));
    }
}
