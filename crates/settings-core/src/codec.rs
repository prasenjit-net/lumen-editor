//! TOML decoding and sparse, comment-preserving encoding. No filesystem access.
use crate::{Control, Layer, Value, catalog};
use toml_edit::{DocumentMut, Item};

pub fn parse(content: Option<&str>) -> Result<(Layer, Vec<String>), String> {
    let document = content
        .unwrap_or("")
        .parse::<DocumentMut>()
        .map_err(|error| error.to_string())?;
    if let Some(version) = document.get("schema_version")
        && version.as_integer() != Some(1)
    {
        return Err("Unsupported settings schema_version; expected 1.".into());
    }
    let mut layer = Layer::new();
    let mut warnings = Vec::new();
    for (section, item) in document.iter() {
        if section == "schema_version" {
            continue;
        }
        if let Some(table) = item.as_table_like() {
            for (name, item) in table.iter() {
                let id = format!("{section}.{name}");
                if let Some(setting) = catalog().find(|setting| setting.id == id) {
                    let value = if let Some(value) = item.as_bool() {
                        Value::Bool(value)
                    } else if let Some(value) = item.as_float() {
                        Value::Number(value)
                    } else if let Some(value) = item.as_integer() {
                        Value::Number(value as f64)
                    } else if let Some(value) = item.as_str() {
                        match setting.control {
                            Control::Text(_) => Value::Text(value.into()),
                            _ => Value::Choice(value.into()),
                        }
                    } else {
                        return Err(format!("Invalid type for {id}"));
                    };
                    setting.validate(&value)?;
                    layer.insert(id, value);
                } else {
                    warnings.push(format!("Unknown setting {id} (preserved)."));
                }
            }
        } else if catalog().any(|setting| setting.id.starts_with(&format!("{section}."))) {
            return Err(format!("{section} must be a table."));
        } else {
            warnings.push(format!("Unknown setting {section} (preserved)."));
        }
    }
    Ok((layer, warnings))
}
pub fn serialize(original: Option<&str>, layer: &Layer) -> Result<String, String> {
    // Parse first: never rewrite unsupported/invalid schemas through the form.
    parse(original)?;
    let mut document = original
        .unwrap_or("")
        .parse::<DocumentMut>()
        .map_err(|error| error.to_string())?;
    document["schema_version"] = toml_edit::value(1);
    for setting in catalog() {
        let (section, name) = setting.id.split_once('.').unwrap();
        if let Some(value) = layer.get(setting.id) {
            if document.get(section).is_none() {
                document[section] = Item::Table(toml_edit::Table::new());
            }
            let mut item = match value {
                Value::Number(value) => toml_edit::value(*value),
                Value::Bool(value) => toml_edit::value(*value),
                Value::Choice(value) | Value::Text(value) => toml_edit::value(value),
            };
            if let Some(previous) = document[section].get(name).and_then(Item::as_value)
                && let Some(next) = item.as_value_mut()
            {
                *next.decor_mut() = previous.decor().clone();
            }
            document[section][name] = item;
        } else if let Some(table) = document.get_mut(section).and_then(Item::as_table_like_mut) {
            table.remove(name);
        }
    }
    Ok(document.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sparse_writes_preserve_comments_unknowns_and_reject_invalid_schema() {
        let source =
            "# personal\nschema_version = 1\n[editor]\nfont_size = 14 # size\nfuture = 'keep me'\n";
        let (mut layer, warnings) = parse(Some(source)).unwrap();
        assert_eq!(warnings.len(), 1);
        layer.insert("editor.font_size".into(), Value::Number(16.0));
        let result = serialize(Some(source), &layer).unwrap();
        assert!(
            result.contains("# personal")
                && result.contains("# size")
                && result.contains("future = 'keep me'")
        );
        assert!(!result.contains("line_height"));
        assert!(parse(Some("schema_version = 2")).is_err());
        assert!(parse(Some("editor = 5")).is_err());
        assert!(parse(Some("[editor]\nfont_size = 'huge'")).is_err());
    }
}
