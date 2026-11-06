//! Detect ambiguous billing fields before JSON object parsing drops duplicates.
use serde::de::{DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};
use std::{collections::HashSet, fmt};

struct UniqueUsage;
impl<'de> DeserializeSeed<'de> for UniqueUsage {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        d.deserialize_any(self)
    }
}
impl<'de> Visitor<'de> for UniqueUsage {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("usage with unique object keys")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        let mut keys = HashSet::new();
        while let Some(key) = map.next_key::<String>()? {
            if !keys.insert(key) {
                return Err(serde::de::Error::custom("duplicate_usage_key"));
            }
            map.next_value_seed(UniqueUsage)?;
        }
        Ok(())
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        while seq.next_element_seed(UniqueUsage)?.is_some() {}
        Ok(())
    }
    fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<(), E> {
        Ok(())
    }
    fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<(), E> {
        Ok(())
    }
    fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<(), E> {
        Ok(())
    }
    fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<(), E> {
        Ok(())
    }
    fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<(), E> {
        Ok(())
    }
    fn visit_unit<E: serde::de::Error>(self) -> Result<(), E> {
        Ok(())
    }
}
struct UsageEnvelope;
impl<'de> Visitor<'de> for UsageEnvelope {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("one original usage member")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        let mut seen = false;
        while let Some(key) = map.next_key::<String>()? {
            if key == "usage" {
                if seen {
                    return Err(serde::de::Error::custom("duplicate_usage_member"));
                }
                seen = true;
                map.next_value_seed(UniqueUsage)?;
            } else {
                map.next_value::<IgnoredAny>()?;
            }
        }
        if !seen {
            return Err(serde::de::Error::custom("usage_member_missing"));
        }
        Ok(())
    }
}

/// Wire structure only. Numeric/cache semantics are checked independently by
/// usage_is_reported; this check neither settles a bill nor grants execution.
pub(crate) fn usage_wire_is_unambiguous(text: &str) -> bool {
    use serde::Deserializer;
    let mut d = serde_json::Deserializer::from_str(text);
    d.deserialize_map(UsageEnvelope).is_ok() && d.end().is_ok()
}

