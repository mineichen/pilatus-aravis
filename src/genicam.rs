use std::fmt::Debug;

use anyhow::Context;
use aravis::{Camera, CameraExt};
use serde::{de::Unexpected, Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct GenicamFeatureCollection(Vec<(FeatureName, FeatureValue)>);

impl GenicamFeatureCollection {
    pub fn apply(&self, cam: &mut Camera) -> Result<usize, anyhow::Error> {
        for (key, value) in self.0.iter() {
            match value {
                FeatureValue::Bool(v) => cam.set_boolean(&key.0, *v),
                FeatureValue::Integer(v) => cam.set_integer(&key.0, *v),
                FeatureValue::Float(v) => cam.set_float(&key.0, *v),
                FeatureValue::String(v) => cam.set_string(&key.0, v),
            }
            .with_context(|| format!("key: {key:?}, value: {value:?}"))?
        }
        Ok(self.0.len())
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Invalid genicam feature key '{0}'")]
pub struct InvalidGenicamKey(String);

#[derive(Serialize, Clone)]
pub struct FeatureName(String);

impl Debug for FeatureName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl<'de> Deserialize<'de> for FeatureName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .try_into()
            .map_err(<D::Error as serde::de::Error>::custom)
    }
}

impl TryFrom<String> for FeatureName {
    type Error = InvalidGenicamKey;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value
            .chars()
            .any(|c| !c.is_alphanumeric() && !matches!(c, '_' | '[' | ']'))
        {
            Err(InvalidGenicamKey(value))
        } else {
            Ok(Self(value))
        }
    }
}

#[derive(Debug, Clone)]
enum FeatureValue {
    Bool(bool),
    Integer(i64),
    Float(f64),
    String(String),
}

impl Serialize for FeatureValue {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            FeatureValue::Bool(v) => serializer.serialize_bool(*v),
            FeatureValue::Integer(v) => serializer.serialize_i64(*v),
            FeatureValue::Float(v) => serializer.serialize_f64(*v),
            FeatureValue::String(v) => serializer.serialize_str(v),
        }
    }
}

struct GenicamValueVisitor;
impl<'de> serde::de::Visitor<'de> for GenicamValueVisitor {
    type Value = FeatureValue;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("Invalid data")
    }
    fn visit_bool<E>(self, v: bool) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(FeatureValue::Bool(v))
    }
    fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        if v > i64::MAX as u64 {
            return Err(serde::de::Error::invalid_value(
                Unexpected::Unsigned(v),
                &self,
            ));
        }
        Ok(FeatureValue::Integer(v as _))
    }
    fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(FeatureValue::Integer(v))
    }
    fn visit_f64<E>(self, v: f64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(FeatureValue::Float(v))
    }
    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(FeatureValue::String(v.into()))
    }
    fn visit_string<E>(self, v: String) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(FeatureValue::String(v))
    }
}

impl<'de> Deserialize<'de> for FeatureValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(GenicamValueVisitor)
    }
}

#[cfg(test)]
mod tests {
    use serde::de::DeserializeOwned;

    use super::*;

    #[test]
    fn back_and_forth_bool() {
        back_and_forth_value(true);
    }

    #[test]
    fn back_and_forth_f64() {
        back_and_forth_value(1.0f64);
    }

    #[test]
    fn back_and_forth_i32() {
        back_and_forth_value(42i32);
    }

    #[test]
    fn back_and_forth_str() {
        back_and_forth_value("test[x]".to_string());
    }

    fn back_and_forth_value<T: Serialize + DeserializeOwned>(v: T) {
        let original = serde_json::json!(["foobar", v]);
        let raw = <(FeatureName, FeatureValue)>::deserialize(&original).unwrap();
        assert_eq!(original, serde_json::to_value(raw).unwrap());
    }

    #[test]
    fn deserialize_invalid() {
        let raw = <(FeatureName, FeatureValue)>::deserialize(serde_json::json!(["foo@bar", 1]))
            .unwrap_err();
        let error_msg = format!("{raw:?}");
        assert!(error_msg.contains("@"), "{error_msg}");
    }
}
