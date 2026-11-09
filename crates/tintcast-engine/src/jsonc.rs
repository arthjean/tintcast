use jsonc_parser::ParseOptions;
use jsonc_parser::cst::{CstInputValue, CstObject, CstRootNode};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum JsoncError {
    #[error("syntax error at line {line}, column {column}: {message}")]
    Syntax {
        line: usize,
        column: usize,
        message: String,
    },
    #[error("{pointer} is not a JSON pointer")]
    InvalidPointer { pointer: String },
    #[error("{at} is not an object, so {pointer} can't be set")]
    NotAnObject { pointer: String, at: String },
}

fn syntax(error: &jsonc_parser::errors::ParseError) -> JsoncError {
    JsoncError::Syntax {
        line: error.line_display(),
        column: error.column_display(),
        message: error.kind().to_string(),
    }
}

pub fn segments(pointer: &str) -> Result<Vec<String>, JsoncError> {
    if pointer.is_empty() {
        return Ok(Vec::new());
    }
    let rest = pointer
        .strip_prefix('/')
        .ok_or_else(|| JsoncError::InvalidPointer {
            pointer: pointer.to_owned(),
        })?;
    Ok(rest
        .split('/')
        .map(|s| s.replace("~1", "/").replace("~0", "~"))
        .collect())
}

pub fn ancestors(pointer: &str) -> Vec<&str> {
    let mut found: Vec<&str> = pointer
        .char_indices()
        .filter(|&(index, c)| c == '/' && index > 0)
        .filter_map(|(index, _)| pointer.get(..index))
        .collect();
    found.reverse();
    found
}

pub fn strip_bom(text: &str) -> &str {
    text.strip_prefix('\u{feff}').unwrap_or(text)
}

pub fn parse(text: &str) -> Result<Option<Value>, JsoncError> {
    let text = strip_bom(text);
    if text.trim().is_empty() {
        return Ok(None);
    }
    jsonc_parser::parse_to_serde_value::<Value>(text, &ParseOptions::default())
        .map(Some)
        .map_err(|e| syntax(&e))
}

pub fn get(text: &str, pointer: &str) -> Result<Option<Value>, JsoncError> {
    segments(pointer)?;
    Ok(parse(text)?.and_then(|root| root.pointer(pointer).cloned()))
}

fn input(value: &Value) -> CstInputValue {
    match value {
        Value::Null => CstInputValue::Null,
        Value::Bool(b) => CstInputValue::Bool(*b),
        Value::Number(n) => CstInputValue::Number(n.to_string()),
        Value::String(s) => CstInputValue::String(s.clone()),
        Value::Array(items) => CstInputValue::Array(items.iter().map(input).collect()),
        Value::Object(map) => {
            CstInputValue::Object(map.iter().map(|(k, v)| (k.clone(), input(v))).collect())
        }
    }
}

pub fn set(text: &str, pointer: &str, value: Option<&Value>) -> Result<String, JsoncError> {
    let path = segments(pointer)?;
    let bom = text.starts_with('\u{feff}');
    let body = strip_bom(text);
    let root = CstRootNode::parse(body, &ParseOptions::default()).map_err(|e| syntax(&e))?;
    let Some((last, parents)) = path.split_last() else {
        match value {
            Some(value) => root.set_value(input(value)),
            None => root.clear_children(),
        }
        return Ok(finish(bom, root.to_string()));
    };
    let not_object = |depth: usize| JsoncError::NotAnObject {
        pointer: pointer.to_owned(),
        at: if depth == 0 {
            "The document".to_owned()
        } else {
            format!("/{}", path[..depth].join("/"))
        },
    };
    let mut object: CstObject = match root.value() {
        None if value.is_none() => return Ok(text.to_owned()),
        None => root.object_value_or_set(),
        Some(node) => node.as_object().ok_or_else(|| not_object(0))?,
    };
    for (depth, segment) in parents.iter().enumerate() {
        object = match object.get(segment) {
            Some(prop) => match prop.value() {
                Some(node) => node.as_object().ok_or_else(|| not_object(depth + 1))?,
                None => return Err(not_object(depth + 1)),
            },
            None if value.is_none() => return Ok(text.to_owned()),
            None => object
                .append(segment, CstInputValue::Object(Vec::new()))
                .object_value()
                .ok_or_else(|| not_object(depth + 1))?,
        };
    }
    match (object.get(last), value) {
        (Some(prop), Some(value)) => prop.set_value(input(value)),
        (None, Some(value)) => {
            object.append(last, input(value));
        }
        (Some(prop), None) => prop.remove(),
        (None, None) => return Ok(text.to_owned()),
    }
    Ok(finish(bom, root.to_string()))
}

fn finish(bom: bool, text: String) -> String {
    if bom { format!("\u{feff}{text}") } else { text }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const SETTINGS: &str = "{\n    // Terminal settings\n    \"profiles\": {\n        \"defaults\": {\n            \"font\": { \"size\": 11 } /* keep */\n        },\n        \"list\": []\n    },\n    \"theme\": \"dark\",\n}\n";

    #[test]
    fn reads_a_value_through_a_pointer() {
        assert_eq!(
            get(SETTINGS, "/profiles/defaults/font/size").unwrap(),
            Some(json!(11))
        );
        assert_eq!(
            get(SETTINGS, "/profiles/defaults/colorScheme").unwrap(),
            None
        );
        assert_eq!(get("", "/a").unwrap(), None);
    }

    #[test]
    fn setting_a_new_key_keeps_comments_and_other_bytes() {
        let out = set(
            SETTINGS,
            "/profiles/defaults/colorScheme",
            Some(&json!("Tintcast Moondust")),
        )
        .unwrap();
        assert!(out.contains("// Terminal settings"));
        assert!(out.contains("/* keep */"));
        assert!(out.contains("\"theme\": \"dark\",\n}"));
        assert_eq!(
            get(&out, "/profiles/defaults/colorScheme").unwrap(),
            Some(json!("Tintcast Moondust"))
        );
        assert_eq!(
            get(&out, "/profiles/defaults/font/size").unwrap(),
            Some(json!(11))
        );
    }

    #[test]
    fn replacing_an_existing_value_changes_only_that_value() {
        let out = set(SETTINGS, "/theme", Some(&json!("light"))).unwrap();
        assert_eq!(
            out,
            SETTINGS.replace("\"theme\": \"dark\"", "\"theme\": \"light\"")
        );
    }

    #[test]
    fn removing_a_key_keeps_the_rest() {
        let out = set(SETTINGS, "/theme", None).unwrap();
        assert_eq!(get(&out, "/theme").unwrap(), None);
        assert!(out.contains("// Terminal settings"));
        assert!(out.contains("/* keep */"));
    }

    #[test]
    fn removing_a_missing_key_changes_nothing() {
        assert_eq!(set(SETTINGS, "/nothing/here", None).unwrap(), SETTINGS);
        assert_eq!(set(SETTINGS, "/missing", None).unwrap(), SETTINGS);
    }

    #[test]
    fn creates_missing_parents_and_an_empty_document() {
        let out = set("", "/a/b", Some(&json!(true))).unwrap();
        assert_eq!(parse(&out).unwrap(), Some(json!({"a": {"b": true}})));
    }

    #[test]
    fn keeps_a_byte_order_mark() {
        let text = format!("\u{feff}{SETTINGS}");
        let out = set(&text, "/theme", Some(&json!("light"))).unwrap();
        assert!(out.starts_with('\u{feff}'));
        assert_eq!(get(&out, "/theme").unwrap(), Some(json!("light")));
    }

    #[test]
    fn lists_ancestors_deepest_first() {
        assert_eq!(
            ancestors("/profiles/defaults/colorScheme"),
            ["/profiles/defaults", "/profiles"]
        );
        assert!(ancestors("/theme").is_empty());
        assert!(ancestors("").is_empty());
        assert_eq!(ancestors("/a~1b/c"), ["/a~1b"]);
    }

    #[test]
    fn unescapes_pointer_segments() {
        let out = set("{}", "/a~1b/c~0d", Some(&json!(1))).unwrap();
        assert_eq!(parse(&out).unwrap(), Some(json!({"a/b": {"c~d": 1}})));
    }

    #[test]
    fn reports_the_line_and_column_of_a_syntax_error() {
        let error = get("{\n  \"a\": 1,\n  \"b\": ]\n}", "/a").unwrap_err();
        match error {
            JsoncError::Syntax { line, column, .. } => {
                assert_eq!(line, 3);
                assert!(column > 1);
            }
            other => panic!("unexpected error {other:?}"),
        }
        assert!(set("{ \"a\": ", "/a", Some(&json!(1))).is_err());
    }

    #[test]
    fn refuses_to_set_through_a_value_that_is_not_an_object() {
        let error = set(SETTINGS, "/theme/x", Some(&json!(1))).unwrap_err();
        assert!(matches!(error, JsoncError::NotAnObject { .. }));
        assert!(matches!(
            set(SETTINGS, "no-slash", Some(&json!(1))).unwrap_err(),
            JsoncError::InvalidPointer { .. }
        ));
    }
}
