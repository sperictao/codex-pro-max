//! TOML 路径读写（toml_edit 保留注释与格式）

use toml_edit::{DocumentMut, Item, Table};

use crate::i18n::{tr, trf};

pub(crate) fn get_toml_path<'a>(doc: &'a DocumentMut, path: &str) -> Option<&'a Item> {
    let mut item = doc.as_item();
    for seg in path.split('.') {
        item = item.get(seg)?;
    }
    Some(item)
}

pub(crate) fn set_toml_path(doc: &mut DocumentMut, path: &str, val: Item) -> Result<(), String> {
    let segs: Vec<&str> = path.split('.').collect();
    let mut item = doc.as_item_mut();
    for seg in &segs[..segs.len() - 1] {
        if item.as_table_like().is_none() {
            return Err(trf(
                "Intermediate path node {node} is not a table",
                &[("node", seg.to_string())],
            ));
        }
        if item.get(seg).is_none() {
            item.as_table_like_mut()
                .unwrap()
                .insert(seg, Item::Table(Table::new()));
        }
        item = item.get_mut(seg).unwrap();
        if item.as_table_like().is_none() {
            return Err(trf(
                "Intermediate path node {node} is not a table",
                &[("node", seg.to_string())],
            ));
        }
    }
    let last = segs[segs.len() - 1];
    item.as_table_like_mut()
        .ok_or_else(|| tr("Path endpoint is not a table"))?
        .insert(last, val);
    Ok(())
}

pub(crate) fn remove_toml_path(doc: &mut DocumentMut, path: &str) {
    let segs: Vec<&str> = path.split('.').collect();
    let mut item = doc.as_item_mut();
    for seg in &segs[..segs.len() - 1] {
        match item.get_mut(seg) {
            Some(i) => item = i,
            None => return,
        }
    }
    if let Some(t) = item.as_table_like_mut() {
        t.remove(segs[segs.len() - 1]);
    }
}

pub(crate) fn json_to_toml(v: &serde_json::Value) -> Result<Item, String> {
    match v {
        serde_json::Value::Bool(b) => Ok(toml_edit::value(*b)),
        serde_json::Value::Number(n) => n
            .as_i64()
            .map(toml_edit::value)
            .ok_or_else(|| tr("Integer parameters do not support decimals")),
        serde_json::Value::String(s) => Ok(toml_edit::value(s.as_str())),
        _ => Err(tr("Unsupported value type")),
    }
}

pub(crate) fn toml_matches_json(item: &Item, v: &serde_json::Value) -> bool {
    match (item, v) {
        (Item::Value(toml_edit::Value::Boolean(b)), serde_json::Value::Bool(x)) => *b.value() == *x,
        (Item::Value(toml_edit::Value::Integer(i)), serde_json::Value::Number(n)) => {
            n.as_i64() == Some(*i.value())
        }
        (Item::Value(toml_edit::Value::String(s)), serde_json::Value::String(x)) => s.value() == x,
        _ => false,
    }
}

pub(crate) fn render_toml_value(item: &Item) -> String {
    match item {
        Item::Value(toml_edit::Value::Boolean(b)) => b.value().to_string(),
        Item::Value(toml_edit::Value::Integer(i)) => i.value().to_string(),
        Item::Value(toml_edit::Value::String(s)) => format!("\"{}\"", s.value()),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_set_get_remove_roundtrip() {
        let mut doc = "[mcp_servers.x]\nurl = \"u\"\n"
            .parse::<DocumentMut>()
            .unwrap();
        set_toml_path(
            &mut doc,
            "features.multi_agent_v2.enabled",
            json_to_toml(&serde_json::json!(true)).unwrap(),
        )
        .unwrap();
        let item = get_toml_path(&doc, "features.multi_agent_v2.enabled").unwrap();
        assert!(toml_matches_json(item, &serde_json::json!(true)));
        // 既有内容未被破坏
        assert!(get_toml_path(&doc, "mcp_servers.x.url").is_some());
        // 注释保留（不紧贴被删表）
        let mut doc2 = "# 注释\nx = 1\n[agents]\nmax_threads = 6\n"
            .parse::<DocumentMut>()
            .unwrap();
        remove_toml_path(&mut doc2, "agents");
        assert!(get_toml_path(&doc2, "agents").is_none());
        assert!(doc2.to_string().contains("# 注释"));
        assert!(get_toml_path(&doc2, "x").is_some());
    }

    #[test]
    fn toml_remove_nested() {
        let mut doc = "[a.b]\nx = 1\n".parse::<DocumentMut>().unwrap();
        remove_toml_path(&mut doc, "a.b");
        assert!(get_toml_path(&doc, "a.b").is_none());
    }

    #[test]
    fn toml_matches_json_compares_scalar_types() {
        // 漂移判定的核心比对：bool/int/string 按值比，类型不符或非标量一律 false
        let doc = "b = true\ni = 42\ns = \"hi\"\nf = 1.5\n[a]\nx = 1\n"
            .parse::<DocumentMut>()
            .unwrap();
        let at = |p: &str| get_toml_path(&doc, p).unwrap();
        assert!(toml_matches_json(at("b"), &serde_json::json!(true)));
        assert!(!toml_matches_json(at("b"), &serde_json::json!(false)));
        assert!(toml_matches_json(at("i"), &serde_json::json!(42)));
        assert!(!toml_matches_json(at("i"), &serde_json::json!(43)));
        assert!(toml_matches_json(at("s"), &serde_json::json!("hi")));
        // 类型不符（int 对 string）→ false
        assert!(!toml_matches_json(at("i"), &serde_json::json!("42")));
        // 非标量（float / table）永不判 match
        assert!(!toml_matches_json(at("f"), &serde_json::json!(1.5)));
        assert!(!toml_matches_json(at("a"), &serde_json::json!({})));
    }

    #[test]
    fn json_to_toml_rejects_decimals_and_arrays() {
        assert!(json_to_toml(&serde_json::json!(42)).is_ok());
        assert!(json_to_toml(&serde_json::json!(true)).is_ok());
        assert!(json_to_toml(&serde_json::json!("s")).is_ok());
        assert!(json_to_toml(&serde_json::json!(1.5)).is_err());
        assert!(json_to_toml(&serde_json::json!([1, 2])).is_err());
    }

    #[test]
    fn render_toml_value_formats_scalars() {
        let doc = "s = \"hi\"\nb = true\ni = 7\n"
            .parse::<DocumentMut>()
            .unwrap();
        let at = |p: &str| get_toml_path(&doc, p).unwrap();
        assert_eq!(render_toml_value(at("s")), "\"hi\"");
        assert_eq!(render_toml_value(at("b")), "true");
        assert_eq!(render_toml_value(at("i")), "7");
    }
}
