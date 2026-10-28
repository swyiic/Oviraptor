//! Compare JSON values across the JavaScript IPC boundary without treating
//! integral floats (20.0) as changes to the same integer (20). Never round a
//! different integer or a fractional value into an equal value.
use serde_json::{Number, Value};

fn numbers_equal(left: &Number, right: &Number) -> bool {
    if left == right {
        return true;
    }
    let (float, integer) = if left.is_f64() && !right.is_f64() {
        (left.as_f64().unwrap(), right)
    } else if right.is_f64() && !left.is_f64() {
        (right.as_f64().unwrap(), left)
    } else {
        return false;
    };
    if !float.is_finite() || float.fract() != 0.0 {
        return false;
    }
    // The upper bounds are exclusive: i64::MAX/u64::MAX round upward as f64.
    if let Some(value) = integer.as_i64() {
        return (-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0)
            .contains(&float)
            && float as i64 == value;
    }
    integer.as_u64().is_some_and(|value| {
        (0.0..18_446_744_073_709_551_616.0).contains(&float) && float as u64 == value
    })
}

pub fn equivalent(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Number(left), Value::Number(right)) => numbers_equal(left, right),
        (Value::Array(left), Value::Array(right)) => {
            left.len() == right.len() && left.iter().zip(right).all(|(a, b)| equivalent(a, b))
        }
        (Value::Object(left), Value::Object(right)) => {
            left.len() == right.len()
                && left.iter().all(|(key, value)| {
                    right.get(key).is_some_and(|other| equivalent(value, other))
                })
        }
        _ => left == right,
    }
}

