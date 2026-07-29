use serde_norway::Value;

pub fn get<'a>(yaml: &'a Value, key: &str) -> Option<&'a Value> {
    match yaml {
        Value::Mapping(map) => map.get(Value::String(key.to_string())),
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
