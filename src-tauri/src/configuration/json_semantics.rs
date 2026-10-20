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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn only_equal_numeric_values_survive_ipc_representation_changes() {
        for (left, right) in [
            ("20.0", "20"), ("-0.0", "0"), ("-12.0", "-12"),
            ("9007199254740992.0", "9007199254740992"),
        ] {
            let a: Value = serde_json::from_str(left).unwrap();
            let b: Value = serde_json::from_str(right).unwrap();
            assert!(equivalent(&a, &b));
            assert!(equivalent(&b, &a));
        }
        for (left, right) in [
            ("20.1", "20"), ("20.0", "21"),
            ("9007199254740992.0", "9007199254740993"),
            ("9223372036854775808.0", "9223372036854775807"),
            ("18446744073709551616.0", "18446744073709551615"),
        ] {
            assert!(!equivalent(&serde_json::from_str(left).unwrap(), &serde_json::from_str(right).unwrap()));
        }
        assert!(equivalent(&json!({"nested":[20.0,{"rate":10.0}]}), &json!({"nested":[20,{"rate":10}]})));
        assert!(!equivalent(&json!([1,2]), &json!([2,1])));
        assert!(!equivalent(&json!(20), &json!("20")));
        assert!(!equivalent(&json!({"rate":20}), &json!({"rate":20,"extra":false})));
    }
}
