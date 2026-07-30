use std::fmt;

use serde::de::{
    Deserialize, Deserializer, EnumAccess, MapAccess, SeqAccess, VariantAccess, Visitor,
};
use serde_norway::Value;

/// Parse YAML into serde_norway's convenient `Value` tree while preserving
/// integer scalars that are wider than its built-in u64/i64 number type.
///
/// tmuxinator and mux treat names as YAML scalars and accept values such as
/// `111222333444555666777` as a window name. libyaml reports that token as a
/// u128, which `serde_norway::Value` otherwise rejects. Keeping only those
/// out-of-range integers as strings preserves their exact spelling and matches
/// the multiplexer config semantics. The top-level `attach` scalar is also
/// restored from a string-targeted parse so mux's exact `false`/`0` lexical
/// rule is not lost to YAML boolean or integer normalization.
pub fn parse(source: &str) -> Result<Value, serde_norway::Error> {
    let mut value = serde_norway::from_str::<CompatValue>(source)?.0;
    preserve_top_level_attach_lexeme(source, &mut value);
    Ok(value)
}

struct CompatValue(Value);

#[derive(serde::Deserialize)]
struct RawAttach {
    #[serde(default)]
    attach: Option<String>,
}

fn preserve_top_level_attach_lexeme(source: &str, value: &mut Value) {
    let Ok(Some(raw)) = serde_norway::from_str::<RawAttach>(source).map(|parsed| parsed.attach)
    else {
        return;
    };
    let Value::Mapping(mapping) = value else {
        return;
    };
    let key = Value::String("attach".to_string());
    if mapping.contains_key(&key) {
        mapping.insert(key, Value::String(raw));
    }
}

impl<'de> Deserialize<'de> for CompatValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct CompatVisitor;

        impl<'de> Visitor<'de> for CompatVisitor {
            type Value = CompatValue;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("any YAML value")
            }

            fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
                Ok(CompatValue(Value::Bool(value)))
            }

            fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
                Ok(CompatValue(Value::Number(value.into())))
            }

            fn visit_i128<E>(self, value: i128) -> Result<Self::Value, E> {
                match i64::try_from(value) {
                    Ok(value) => Ok(CompatValue(Value::Number(value.into()))),
                    Err(_) => Ok(CompatValue(Value::String(value.to_string()))),
                }
            }

            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
                Ok(CompatValue(Value::Number(value.into())))
            }

            fn visit_u128<E>(self, value: u128) -> Result<Self::Value, E> {
                match u64::try_from(value) {
                    Ok(value) => Ok(CompatValue(Value::Number(value.into()))),
                    Err(_) => Ok(CompatValue(Value::String(value.to_string()))),
                }
            }

            fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E> {
                Ok(CompatValue(Value::Number(value.into())))
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
                Ok(CompatValue(Value::String(value.to_owned())))
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
                Ok(CompatValue(Value::String(value)))
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(CompatValue(Value::Null))
            }

            fn visit_none<E>(self) -> Result<Self::Value, E> {
                Ok(CompatValue(Value::Null))
            }

            fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
            where
                D: Deserializer<'de>,
            {
                CompatValue::deserialize(deserializer)
            }

            fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut values = Vec::with_capacity(sequence.size_hint().unwrap_or(0));
                while let Some(value) = sequence.next_element::<CompatValue>()? {
                    values.push(value.0);
                }
                Ok(CompatValue(Value::Sequence(values)))
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut values = serde_norway::Mapping::new();
                while let Some((key, value)) = map.next_entry::<CompatValue, CompatValue>()? {
                    if values.contains_key(&key.0) {
                        return Err(serde::de::Error::custom("duplicate entry in YAML mapping"));
                    }
                    values.insert(key.0, value.0);
                }
                Ok(CompatValue(Value::Mapping(values)))
            }

            fn visit_enum<A>(self, data: A) -> Result<Self::Value, A::Error>
            where
                A: EnumAccess<'de>,
            {
                let (tag, contents) = data.variant::<String>()?;
                let value = contents.newtype_variant::<CompatValue>()?;
                Ok(CompatValue(Value::Tagged(Box::new(
                    serde_norway::value::TaggedValue {
                        tag: serde_norway::value::Tag::new(tag),
                        value: value.0,
                    },
                ))))
            }
        }

        deserializer.deserialize_any(CompatVisitor)
    }
}

pub fn get<'a>(yaml: &'a Value, key: &str) -> Option<&'a Value> {
    match yaml {
        Value::Mapping(map) => map.get(Value::String(key.to_string())),
        _ => None,
    }
}

/// Resolve scalar canonical keys and deprecated aliases in document order.
/// mux canonicalizes keys while iterating libyaml pairs, so the last value of
/// the expected shape wins when a migration file contains both spellings.
pub fn get_aliased_scalar<'a>(
    yaml: &'a Value,
    canonical: &str,
    aliases: &[&str],
) -> Option<&'a Value> {
    match yaml {
        Value::Mapping(map) => map
            .iter()
            .filter_map(|(key, value)| {
                let key = scalar_to_string(key)?;
                (key == canonical || aliases.contains(&key.as_str())).then_some(value)
            })
            .filter(|value| scalar_to_string(value).is_some())
            .last(),
        _ => None,
    }
}

pub fn get_aliased_nonempty_sequence<'a>(
    yaml: &'a Value,
    canonical: &str,
    aliases: &[&str],
) -> Option<&'a [Value]> {
    match yaml {
        Value::Mapping(map) => map
            .iter()
            .filter_map(|(key, value)| {
                let key = scalar_to_string(key)?;
                (key == canonical || aliases.contains(&key.as_str()))
                    .then(|| value.as_sequence())
                    .flatten()
            })
            .filter(|values| !values.is_empty())
            .last()
            .map(Vec::as_slice),
        _ => None,
    }
}

// Ruby truthiness: only nil and false are falsy (an empty string is truthy).
pub fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => true,
    }
}

// Ruby `to_s` for YAML scalars. Floats use {:?} so 4e5 renders as
// "400000.0" like Ruby's Float#to_s, not "400000".
pub fn scalar_to_string(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Some(i.to_string())
            } else if let Some(u) = n.as_u64() {
                Some(u.to_string())
            } else {
                n.as_f64().map(|f| format!("{f:?}"))
            }
        }
        Value::Tagged(tagged) => scalar_to_string(&tagged.value),
        Value::Sequence(_) | Value::Mapping(_) => None,
    }
}

pub fn get_string(yaml: &Value, key: &str) -> Option<String> {
    get(yaml, key).and_then(scalar_to_string)
}

/// mux treats the scalar spellings `false` and `0` as false for `attach`.
pub fn mux_attach(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => true,
        Some(value) => !matches!(scalar_to_string(value).as_deref(), Some("false" | "0")),
    }
}

// Ruby `parsed_parameters` / `Hooks.commands_from`: arrays are joined with
// the separator (nil entries become empty strings), scalars pass through.
pub fn join_or_string(value: Option<&Value>, separator: &str) -> Option<String> {
    match value {
        None | Some(Value::Null) => None,
        Some(Value::Sequence(items)) => Some(
            items
                .iter()
                .map(|item| scalar_to_string(item).unwrap_or_default())
                .collect::<Vec<_>>()
                .join(separator),
        ),
        Some(other) => scalar_to_string(other),
    }
}

pub fn first_entry(value: &Value) -> (Option<&Value>, Option<&Value>) {
    match value {
        Value::Mapping(map) => match map.iter().next() {
            Some((key, val)) => (Some(key), Some(val)),
            None => (None, None),
        },
        _ => (None, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_integer_scalars_wider_than_u64() {
        let yaml = parse("windows:\n  - 111222333444555666777: echo large\n").unwrap();
        let window = get(&yaml, "windows")
            .and_then(Value::as_sequence)
            .and_then(|windows| windows.first())
            .unwrap();
        let (name, _) = first_entry(window);
        assert_eq!(
            name.and_then(scalar_to_string).as_deref(),
            Some("111222333444555666777")
        );
    }

    #[test]
    fn retains_tags_and_merge_support() {
        let mut yaml = parse(
            "defaults: &defaults\n  root: /tmp\nproject:\n  <<: *defaults\n  tagged: !Thing value\n",
        )
        .unwrap();
        yaml.apply_merge().unwrap();
        assert_eq!(
            get(get(&yaml, "project").unwrap(), "root")
                .and_then(scalar_to_string)
                .as_deref(),
            Some("/tmp")
        );
        assert!(matches!(
            get(get(&yaml, "project").unwrap(), "tagged"),
            Some(Value::Tagged(_))
        ));
    }

    #[test]
    fn rejects_duplicate_mapping_keys() {
        assert!(parse("name: first\nname: second\n").is_err());
    }

    #[test]
    fn aliased_keys_use_the_last_document_spelling() {
        let yaml = parse("name: modern\nproject_name: legacy\n").unwrap();
        assert_eq!(
            get_aliased_scalar(&yaml, "name", &["project_name"])
                .and_then(scalar_to_string)
                .as_deref(),
            Some("legacy")
        );

        let yaml = parse("project_name: legacy\nname: modern\n").unwrap();
        assert_eq!(
            get_aliased_scalar(&yaml, "name", &["project_name"])
                .and_then(scalar_to_string)
                .as_deref(),
            Some("modern")
        );
    }

    #[test]
    fn aliased_helpers_skip_values_of_the_wrong_shape() {
        let yaml =
            parse("tabs:\n  - keep: echo yes\nwindows: []\nname: keep\nproject_name: [wrong]\n")
                .unwrap();
        assert_eq!(
            get_aliased_scalar(&yaml, "name", &["project_name"])
                .and_then(scalar_to_string)
                .as_deref(),
            Some("keep")
        );
        assert_eq!(
            get_aliased_nonempty_sequence(&yaml, "windows", &["tabs"])
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn mux_attach_accepts_boolean_numeric_and_string_false() {
        for source in ["attach: false\n", "attach: 0\n", "attach: \"false\"\n"] {
            let yaml = parse(source).unwrap();
            assert!(!mux_attach(get(&yaml, "attach")), "{source:?}");
        }
        for source in ["attach: true\n", "attach: 1\n", "attach: \"yes\"\n"] {
            let yaml = parse(source).unwrap();
            assert!(mux_attach(get(&yaml, "attach")), "{source:?}");
        }
    }

    #[test]
    fn attach_uses_the_original_scalar_lexeme_like_mux() {
        for source in [
            "attach: False\n",
            "attach: FALSE\n",
            "attach: +0\n",
            "attach: 00\n",
            "attach: 0x0\n",
        ] {
            let yaml = parse(source).unwrap();
            assert!(mux_attach(get(&yaml, "attach")), "{source:?}");
        }
        for source in [
            "attach: false\n",
            "attach: \"false\"\n",
            "attach: 0\n",
            "attach: \"0\"\n",
        ] {
            let yaml = parse(source).unwrap();
            assert!(!mux_attach(get(&yaml, "attach")), "{source:?}");
        }
    }
}
