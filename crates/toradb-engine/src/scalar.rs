//! Evaluation of SQL scalar (built-in) functions.
use std::collections::HashMap;

use toradb_core::{civil_from_days, parse_timestamp_millis, ColumnType};
use toradb_sql::ast::Expr;

#[derive(Clone, Debug, PartialEq)]
pub enum ScalarValue {
    Null,
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
}

impl ScalarValue {
    pub fn is_null(&self) -> bool {
        matches!(self, ScalarValue::Null)
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            ScalarValue::Null => None,
            ScalarValue::Int(i) => Some(*i as f64),
            ScalarValue::Float(f) => Some(*f),
            ScalarValue::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            ScalarValue::Str(s) => s.trim().parse::<f64>().ok(),
        }
    }

    pub fn as_string(&self) -> Option<String> {
        match self {
            ScalarValue::Null => None,
            ScalarValue::Str(s) => Some(s.clone()),
            ScalarValue::Int(i) => Some(i.to_string()),
            ScalarValue::Float(f) => Some(format_f64(*f)),
            ScalarValue::Bool(b) => Some(b.to_string()),
        }
    }

    pub fn column_type(&self) -> ColumnType {
        match self {
            ScalarValue::Int(_) => ColumnType::Int,
            ScalarValue::Float(_) => ColumnType::Float,
            ScalarValue::Bool(_) => ColumnType::Bool,
            ScalarValue::Str(_) | ScalarValue::Null => ColumnType::Text,
        }
    }
}

fn format_f64(f: f64) -> String {
    if f.fract() == 0.0 && f.is_finite() && f.abs() < 1e15 {
        format!("{}", f as i64)
    } else {
        format!("{f}")
    }
}

pub struct Row<'a> {
    pub id: u64,
    pub metadata: &'a HashMap<String, String>,
    pub text: Option<&'a str>,
    pub score: Option<f32>,
}

pub fn func_return_type(name: &str, _arg_types: &[ColumnType]) -> ColumnType {
    match name {
        "lower" | "upper" | "trim" | "substr" | "concat" | "date_trunc" => ColumnType::Text,
        "length" | "floor" | "ceil" | "now" | "extract" | "age" => ColumnType::Int,
        "abs" | "round" | "mod" => ColumnType::Float,
        // coalesce/ifnull/nullif take the type of the chosen argument; default Text.
        _ => ColumnType::Text,
    }
}

pub fn eval_expr(
    expr: &Expr,
    row: &Row,
    col_types: &HashMap<String, ColumnType>,
    now_millis: i64,
) -> ScalarValue {
    match expr {
        Expr::Literal(s) => ScalarValue::Str(s.clone()),
        Expr::Column(name) => eval_column(name, row, col_types),
        Expr::Func { name, args } => {
            let vals: Vec<ScalarValue> = args
                .iter()
                .map(|a| eval_expr(a, row, col_types, now_millis))
                .collect();
            eval_func(name, &vals, now_millis)
        }
    }
}

fn eval_column(name: &str, row: &Row, col_types: &HashMap<String, ColumnType>) -> ScalarValue {
    if name == "id" {
        return ScalarValue::Int(row.id as i64);
    }
    if name == "score" {
        if let Some(s) = row.score {
            return ScalarValue::Float(s as f64);
        }
    }
    if name == "text" {
        if let Some(t) = row.text {
            return ScalarValue::Str(t.to_string());
        }
    }
    match row.metadata.get(name) {
        Some(v) => coerce_stored(v, col_types.get(name).copied()),
        None => ScalarValue::Null,
    }
}

fn coerce_stored(v: &str, ty: Option<ColumnType>) -> ScalarValue {
    match ty {
        Some(ColumnType::Int) => v
            .trim()
            .parse::<i64>()
            .map(ScalarValue::Int)
            .unwrap_or_else(|_| ScalarValue::Str(v.to_string())),
        Some(ColumnType::Float) => v
            .trim()
            .parse::<f64>()
            .map(ScalarValue::Float)
            .unwrap_or_else(|_| ScalarValue::Str(v.to_string())),
        Some(ColumnType::Bool) => toradb_core::parse_bool(v)
            .map(ScalarValue::Bool)
            .unwrap_or_else(|| ScalarValue::Str(v.to_string())),
        _ => ScalarValue::Str(v.to_string()),
    }
}

fn eval_func(name: &str, args: &[ScalarValue], now_millis: i64) -> ScalarValue {
    match name {
        // ---- String ----
        "lower" => str_map(&args[0], |s| s.to_lowercase()),
        "upper" => str_map(&args[0], |s| s.to_uppercase()),
        "trim" => str_map(&args[0], |s| s.trim().to_string()),
        "length" => match args[0].as_string() {
            Some(s) => ScalarValue::Int(s.chars().count() as i64),
            None => ScalarValue::Null,
        },
        "substr" => eval_substr(args),
        "concat" => {
            let mut out = String::new();
            for a in args {
                if let Some(s) = a.as_string() {
                    out.push_str(&s);
                }
            }
            ScalarValue::Str(out)
        }
        "coalesce" => args
            .iter()
            .find(|a| !a.is_null())
            .cloned()
            .unwrap_or(ScalarValue::Null),

        // ---- Numeric ----
        "abs" => num_map(&args[0], |x| x.abs()),
        "floor" => match args[0].as_f64() {
            Some(x) => ScalarValue::Int(x.floor() as i64),
            None => ScalarValue::Null,
        },
        "ceil" => match args[0].as_f64() {
            Some(x) => ScalarValue::Int(x.ceil() as i64),
            None => ScalarValue::Null,
        },
        "round" => eval_round(args),
        "mod" => match (args[0].as_f64(), args[1].as_f64()) {
            (Some(_), Some(b)) if b == 0.0 => ScalarValue::Null,
            (Some(a), Some(b)) => {
                if let (ScalarValue::Int(x), ScalarValue::Int(y)) = (&args[0], &args[1]) {
                    ScalarValue::Int(x % y)
                } else {
                    ScalarValue::Float(a % b)
                }
            }
            _ => ScalarValue::Null,
        },

        // ---- Date / time ----
        "now" => ScalarValue::Int(now_millis),
        "date_trunc" => eval_date_trunc(args),
        "extract" => eval_extract(args),
        "age" => match ts_millis(&args[0]) {
            Some(ts) => ScalarValue::Int(now_millis - ts),
            None => ScalarValue::Null,
        },

        // ---- Conditional ----
        "nullif" => {
            if scalar_eq(&args[0], &args[1]) {
                ScalarValue::Null
            } else {
                args[0].clone()
            }
        }
        "ifnull" => {
            if args[0].is_null() {
                args[1].clone()
            } else {
                args[0].clone()
            }
        }

        _ => ScalarValue::Null,
    }
}

fn str_map(v: &ScalarValue, f: impl Fn(&str) -> String) -> ScalarValue {
    match v.as_string() {
        Some(s) if !v.is_null() => ScalarValue::Str(f(&s)),
        _ => ScalarValue::Null,
    }
}

fn num_map(v: &ScalarValue, f: impl Fn(f64) -> f64) -> ScalarValue {
    match v {
        ScalarValue::Int(i) => ScalarValue::Int(f(*i as f64) as i64),
        _ => match v.as_f64() {
            Some(x) => ScalarValue::Float(f(x)),
            None => ScalarValue::Null,
        },
    }
}

fn eval_substr(args: &[ScalarValue]) -> ScalarValue {
    let Some(s) = (if args[0].is_null() {
        None
    } else {
        args[0].as_string()
    }) else {
        return ScalarValue::Null;
    };
    let chars: Vec<char> = s.chars().collect();
    // 1-based start, SQL semantics; clamp to bounds.
    let start_1 = args[1].as_f64().map(|f| f as i64).unwrap_or(1);
    let start = if start_1 < 1 {
        0
    } else {
        (start_1 - 1) as usize
    };
    if start >= chars.len() {
        return ScalarValue::Str(String::new());
    }
    let end = if args.len() == 3 {
        let len = args[2].as_f64().map(|f| f as i64).unwrap_or(0);
        if len <= 0 {
            return ScalarValue::Str(String::new());
        }
        (start + len as usize).min(chars.len())
    } else {
        chars.len()
    };
    ScalarValue::Str(chars[start..end].iter().collect())
}

fn eval_round(args: &[ScalarValue]) -> ScalarValue {
    let Some(x) = args[0].as_f64() else {
        return ScalarValue::Null;
    };
    if args.len() == 2 {
        let d = args[1].as_f64().map(|f| f as i32).unwrap_or(0);
        let factor = 10f64.powi(d);
        ScalarValue::Float((x * factor).round() / factor)
    } else {
        // No precision → integer result.
        ScalarValue::Int(x.round() as i64)
    }
}

fn ts_millis(v: &ScalarValue) -> Option<i64> {
    match v {
        ScalarValue::Null => None,
        ScalarValue::Int(i) => Some(*i),
        _ => v.as_string().and_then(|s| parse_timestamp_millis(&s)),
    }
}

fn eval_date_trunc(args: &[ScalarValue]) -> ScalarValue {
    let Some(unit) = args[0].as_string() else {
        return ScalarValue::Null;
    };
    let Some(ms) = ts_millis(&args[1]) else {
        return ScalarValue::Null;
    };
    let days = ms.div_euclid(86_400_000);
    let rem = ms.rem_euclid(86_400_000);
    let (y, m, d) = civil_from_days(days);
    let (hh, mm) = (rem / 3_600_000, (rem % 3_600_000) / 60_000);
    let s = match unit.to_lowercase().as_str() {
        "year" => format!("{y:04}-01-01T00:00:00"),
        "month" => format!("{y:04}-{m:02}-01T00:00:00"),
        "day" => format!("{y:04}-{m:02}-{d:02}T00:00:00"),
        "hour" => format!("{y:04}-{m:02}-{d:02}T{hh:02}:00:00"),
        "minute" => format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:00"),
        _ => return ScalarValue::Null,
    };
    ScalarValue::Str(s)
}

fn eval_extract(args: &[ScalarValue]) -> ScalarValue {
    let Some(field) = args[0].as_string() else {
        return ScalarValue::Null;
    };
    let Some(ms) = ts_millis(&args[1]) else {
        return ScalarValue::Null;
    };
    let days = ms.div_euclid(86_400_000);
    let rem = ms.rem_euclid(86_400_000);
    let (y, m, d) = civil_from_days(days);
    let val = match field.to_lowercase().as_str() {
        "year" => y,
        "month" => m as i64,
        "day" => d as i64,
        "hour" => rem / 3_600_000,
        "minute" => (rem % 3_600_000) / 60_000,
        "second" => (rem % 60_000) / 1_000,
        "epoch" => ms / 1_000,
        _ => return ScalarValue::Null,
    };
    ScalarValue::Int(val)
}

/// Typed equality for `nullif`: numeric when both coerce, else string.
fn scalar_eq(a: &ScalarValue, b: &ScalarValue) -> bool {
    if a.is_null() || b.is_null() {
        return false;
    }
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => x == y,
        _ => a.as_string() == b.as_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lit(s: &str) -> Expr {
        Expr::Literal(s.into())
    }
    fn func(name: &str, args: Vec<Expr>) -> Expr {
        Expr::Func {
            name: name.into(),
            args,
        }
    }
    fn ev(e: &Expr) -> ScalarValue {
        let meta = HashMap::new();
        let row = Row {
            id: 1,
            metadata: &meta,
            text: None,
            score: None,
        };
        eval_expr(e, &row, &HashMap::new(), 1_700_000_000_000)
    }

    #[test]
    fn string_funcs() {
        assert_eq!(
            ev(&func("lower", vec![lit("AbC")])),
            ScalarValue::Str("abc".into())
        );
        assert_eq!(
            ev(&func("upper", vec![lit("AbC")])),
            ScalarValue::Str("ABC".into())
        );
        assert_eq!(
            ev(&func("trim", vec![lit("  hi ")])),
            ScalarValue::Str("hi".into())
        );
        assert_eq!(ev(&func("length", vec![lit("héllo")])), ScalarValue::Int(5));
        assert_eq!(
            ev(&func("substr", vec![lit("abcdef"), lit("2"), lit("3")])),
            ScalarValue::Str("bcd".into())
        );
        // out-of-range start
        assert_eq!(
            ev(&func("substr", vec![lit("ab"), lit("9")])),
            ScalarValue::Str(String::new())
        );
        assert_eq!(
            ev(&func("concat", vec![lit("a"), lit("b"), lit("c")])),
            ScalarValue::Str("abc".into())
        );
    }

    #[test]
    fn numeric_funcs() {
        assert_eq!(ev(&func("abs", vec![lit("-5")])), ScalarValue::Float(5.0));
        assert_eq!(ev(&func("floor", vec![lit("3.9")])), ScalarValue::Int(3));
        assert_eq!(ev(&func("ceil", vec![lit("3.1")])), ScalarValue::Int(4));
        assert_eq!(ev(&func("round", vec![lit("3.6")])), ScalarValue::Int(4));
        assert_eq!(
            ev(&func("round", vec![lit("3.14159"), lit("2")])),
            ScalarValue::Float(3.14)
        );
        // Literal args are untyped strings → float arithmetic; the Int branch is
        // reserved for typed-column values. Display still renders "1".
        assert_eq!(
            ev(&func("mod", vec![lit("7"), lit("3")])),
            ScalarValue::Float(1.0)
        );
        assert_eq!(
            ev(&func("mod", vec![lit("7"), lit("3")]))
                .as_string()
                .unwrap(),
            "1"
        );
        assert_eq!(
            ev(&func("mod", vec![lit("7"), lit("0")])),
            ScalarValue::Null
        );
        // Typed Int args take the integer modulo path.
        assert_eq!(
            eval_func("mod", &[ScalarValue::Int(7), ScalarValue::Int(3)], 0),
            ScalarValue::Int(1)
        );
        // nested
        assert_eq!(
            ev(&func("round", vec![func("abs", vec![lit("-3.6")])])),
            ScalarValue::Int(4)
        );
        // non-numeric → null
        assert_eq!(ev(&func("abs", vec![lit("foo")])), ScalarValue::Null);
    }

    #[test]
    fn conditional_funcs() {
        assert_eq!(
            ev(&func("coalesce", vec![lit("x"), lit("y")])),
            ScalarValue::Str("x".into())
        );
        assert_eq!(
            ev(&func("ifnull", vec![lit("a"), lit("b")])),
            ScalarValue::Str("a".into())
        );
        assert_eq!(
            ev(&func("nullif", vec![lit("5"), lit("5")])),
            ScalarValue::Null
        );
        assert_eq!(
            ev(&func("nullif", vec![lit("5"), lit("6")])),
            ScalarValue::Str("5".into())
        );
    }

    #[test]
    fn null_propagation_from_missing_column() {
        let meta = HashMap::new();
        let row = Row {
            id: 1,
            metadata: &meta,
            text: None,
            score: None,
        };
        // missing column → Null, propagates through lower()
        let e = func("lower", vec![Expr::Column("missing".into())]);
        assert_eq!(eval_expr(&e, &row, &HashMap::new(), 0), ScalarValue::Null);
        // coalesce skips the null
        let e = func(
            "coalesce",
            vec![Expr::Column("missing".into()), lit("fallback")],
        );
        assert_eq!(
            eval_expr(&e, &row, &HashMap::new(), 0),
            ScalarValue::Str("fallback".into())
        );
    }

    #[test]
    fn date_funcs() {
        // 2023-11-14T22:13:20Z = 1_700_000_000_000 ms
        let ts = "2023-11-14T22:13:20";
        assert_eq!(
            ev(&func("date_trunc", vec![lit("month"), lit(ts)])),
            ScalarValue::Str("2023-11-01T00:00:00".into())
        );
        assert_eq!(
            ev(&func("extract", vec![lit("year"), lit(ts)])),
            ScalarValue::Int(2023)
        );
        assert_eq!(
            ev(&func("extract", vec![lit("day"), lit(ts)])),
            ScalarValue::Int(14)
        );
        assert_eq!(
            ev(&func("extract", vec![lit("hour"), lit(ts)])),
            ScalarValue::Int(22)
        );
        // now() returns the threaded clock
        assert_eq!(
            ev(&func("now", vec![])),
            ScalarValue::Int(1_700_000_000_000)
        );
        // age() = now - ts
        let age = ev(&func("age", vec![lit(ts)]));
        assert_eq!(age, ScalarValue::Int(0));
    }
}
