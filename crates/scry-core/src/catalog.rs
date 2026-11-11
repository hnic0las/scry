//! String catalogs (`.xcstrings`): JSON that git merges line by line, so
//! two translators touching nearby keys conflict. This merges the three
//! versions key by key instead: a change on one side only is taken, and
//! only a value both sides changed differently is left to the user.

use serde_json::{Map, Value};

/// Which version a choice takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    Ours,
    Theirs,
}

/// A value both sides changed differently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogConflict {
    /// Where it is, from the root ("strings", "sync.done", "localizations",
    /// "fr", "stringUnit", "value").
    pub path: Vec<String>,
    /// The string's key ("sync.done"), when the value is under "strings".
    pub key: Option<String>,
    /// The rest of the path, as people read it ("fr › value").
    pub detail: String,
    /// Each version's value as text; `None` where that version has none.
    pub base: Option<String>,
    pub ours: Option<String>,
    pub theirs: Option<String>,
}

/// The three versions merged.
#[derive(Debug, Clone, PartialEq)]
pub struct CatalogMerge {
    /// The merge, with ours where values conflict.
    pub merged: Value,
    /// Values taken from the side that alone changed them.
    pub automatic: usize,
    pub conflicts: Vec<CatalogConflict>,
    /// The conflicting values, by conflict: ours and theirs, `None` where
    /// a side removed it.
    choices: Vec<(Option<Value>, Option<Value>)>,
}

/// Merges `base`, `ours` and `theirs`, each the text of a catalog; an
/// empty base stands for a file both sides added.
pub fn merge(base: &str, ours: &str, theirs: &str) -> Result<CatalogMerge, String> {
    let parse = |text: &str, which: &str| -> Result<Value, String> {
        if text.trim().is_empty() {
            return Ok(Value::Object(Map::new()));
        }
        serde_json::from_str(text).map_err(|error| format!("{which} is not valid JSON: {error}"))
    };
    let base = parse(base, "the base")?;
    let ours = parse(ours, "ours")?;
    let theirs = parse(theirs, "theirs")?;
    let mut merge = CatalogMerge {
        merged: Value::Null,
        automatic: 0,
        conflicts: Vec::new(),
        choices: Vec::new(),
    };
    let merged = merge_value(
        &mut merge,
        &mut Vec::new(),
        Some(&base),
        Some(&ours),
        Some(&theirs),
    );
    merge.merged = merged.unwrap_or(Value::Object(Map::new()));
    Ok(merge)
}

impl CatalogMerge {
    /// The catalog with one pick per conflict, in order, written as Xcode
    /// writes catalogs. Fewer picks than conflicts keep ours for the rest.
    pub fn resolve(&self, picks: &[Pick]) -> String {
        let mut value = self.merged.clone();
        for (index, conflict) in self.conflicts.iter().enumerate() {
            let (ours, theirs) = &self.choices[index];
            let chosen = match picks.get(index) {
                Some(Pick::Theirs) => theirs,
                _ => ours,
            };
            set(&mut value, &conflict.path, chosen.clone());
        }
        apple_json(&value)
    }
}

/// JSON as Apple's tools write it (`JSONEncoder` pretty-printed, as in
/// string catalogs and `Package.resolved`), with a final newline.
pub(crate) fn apple_json(value: &Value) -> String {
    let mut text = String::new();
    write(value, 0, &mut text);
    text.push('\n');
    text
}

fn merge_value(
    merge: &mut CatalogMerge,
    path: &mut Vec<String>,
    base: Option<&Value>,
    ours: Option<&Value>,
    theirs: Option<&Value>,
) -> Option<Value> {
    if ours == theirs {
        return ours.cloned();
    }
    if ours == base {
        merge.automatic += 1;
        return theirs.cloned();
    }
    if theirs == base {
        merge.automatic += 1;
        return ours.cloned();
    }
    if let (Some(Value::Object(ours_map)), Some(Value::Object(theirs_map))) = (ours, theirs) {
        let empty = Map::new();
        let base_map = match base {
            Some(Value::Object(map)) => map,
            _ => &empty,
        };
        // Ours' order, then the keys only theirs has.
        let mut keys: Vec<&String> = ours_map.keys().collect();
        for key in theirs_map.keys().chain(base_map.keys()) {
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
        let mut merged = Map::new();
        for key in keys {
            path.push(key.clone());
            let value = merge_value(
                merge,
                path,
                base_map.get(key),
                ours_map.get(key),
                theirs_map.get(key),
            );
            path.pop();
            if let Some(value) = value {
                merged.insert(key.clone(), value);
            }
        }
        return Some(Value::Object(merged));
    }
    merge.conflicts.push(conflict(path, base, ours, theirs));
    merge.choices.push((ours.cloned(), theirs.cloned()));
    ours.cloned()
}

fn conflict(
    path: &[String],
    base: Option<&Value>,
    ours: Option<&Value>,
    theirs: Option<&Value>,
) -> CatalogConflict {
    let (key, rest) = match path {
        [strings, key, rest @ ..] if strings == "strings" => (Some(key.clone()), rest),
        _ => (None, path),
    };
    // The containers every translation goes through say nothing.
    let detail: Vec<&str> = rest
        .iter()
        .map(String::as_str)
        .filter(|part| !matches!(*part, "localizations" | "stringUnit"))
        .collect();
    CatalogConflict {
        path: path.to_vec(),
        key,
        detail: detail.join(" › "),
        base: base.map(display),
        ours: ours.map(display),
        theirs: theirs.map(display),
    }
}

/// A value as people read it: a string as it is, anything else as JSON.
fn display(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// Sets, or removes with `None`, the value at `path`.
fn set(root: &mut Value, path: &[String], value: Option<Value>) {
    let Some((last, parents)) = path.split_last() else {
        if let Some(value) = value {
            *root = value;
        }
        return;
    };
    let mut node = root;
    for part in parents {
        let Value::Object(map) = node else { return };
        node = map
            .entry(part.clone())
            .or_insert_with(|| Value::Object(Map::new()));
    }
    if let Value::Object(map) = node {
        match value {
            Some(value) => {
                map.insert(last.clone(), value);
            }
            None => {
                map.shift_remove(last);
            }
        }
    }
}

/// JSON as Xcode writes string catalogs: two-space indents and `" : "`
/// between a key and its value.
fn write(value: &Value, depth: usize, out: &mut String) {
    let indent = |depth: usize| "  ".repeat(depth);
    match value {
        Value::Object(map) if map.is_empty() => out.push_str("{\n\n}"),
        Value::Object(map) => {
            out.push_str("{\n");
            for (index, (key, value)) in map.iter().enumerate() {
                out.push_str(&indent(depth + 1));
                out.push_str(&Value::String(key.clone()).to_string());
                out.push_str(" : ");
                write(value, depth + 1, out);
                if index + 1 < map.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str(&indent(depth));
            out.push('}');
        }
        Value::Array(items) if items.is_empty() => out.push_str("[\n\n]"),
        Value::Array(items) => {
            out.push_str("[\n");
            for (index, item) in items.iter().enumerate() {
                out.push_str(&indent(depth + 1));
                write(item, depth + 1, out);
                if index + 1 < items.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str(&indent(depth));
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog(done: &str, pending: &str) -> String {
        format!(
            r#"{{
  "sourceLanguage" : "en",
  "strings" : {{
    "sync.done" : {{
      "localizations" : {{
        "fr" : {{
          "stringUnit" : {{
            "state" : "translated",
            "value" : "{done}"
          }}
        }}
      }}
    }},
    "sync.pending" : {{
      "localizations" : {{
        "fr" : {{
          "stringUnit" : {{
            "state" : "translated",
            "value" : "{pending}"
          }}
        }}
      }}
    }}
  }},
  "version" : "1.0"
}}
"#
        )
    }

    /// One translation changed on each side merges by itself; the
    /// result is written exactly as Xcode writes it.
    #[test]
    fn changes_to_different_keys_merge() {
        let base = catalog("Synchronisé", "%lld en attente");
        let ours = catalog("Tout est synchronisé", "%lld en attente");
        let theirs = catalog("Synchronisé", "%lld à envoyer");
        let merge = merge(&base, &ours, &theirs).unwrap();
        assert!(merge.conflicts.is_empty());
        assert_eq!(merge.automatic, 2);
        assert_eq!(
            merge.resolve(&[]),
            catalog("Tout est synchronisé", "%lld à envoyer")
        );
    }

    /// The same translation changed differently on each side is the one
    /// choice left: named by key and language, with the three values, and
    /// the user's pick is written.
    #[test]
    fn the_same_translation_changed_twice_is_a_choice() {
        let base = catalog("Synchronisé", "%lld en attente");
        let ours = catalog("Tout est synchronisé", "%lld en attente");
        let theirs = catalog("Synchronisation terminée", "%lld en attente");
        let merge = merge(&base, &ours, &theirs).unwrap();
        assert_eq!(merge.conflicts.len(), 1);
        let conflict = &merge.conflicts[0];
        assert_eq!(conflict.key.as_deref(), Some("sync.done"));
        assert_eq!(conflict.detail, "fr › value");
        assert_eq!(conflict.base.as_deref(), Some("Synchronisé"));
        assert_eq!(conflict.ours.as_deref(), Some("Tout est synchronisé"));
        assert_eq!(conflict.theirs.as_deref(), Some("Synchronisation terminée"));
        assert_eq!(
            merge.resolve(&[Pick::Theirs]),
            catalog("Synchronisation terminée", "%lld en attente")
        );
    }

    /// A key one side removed and the other changed is a choice too, and
    /// picking the removal removes it.
    #[test]
    fn a_removed_key_changed_on_the_other_side() {
        let base = r#"{"strings":{"a":{"comment":"x"},"b":{}}}"#;
        let ours = r#"{"strings":{"b":{}}}"#;
        let theirs = r#"{"strings":{"a":{"comment":"y"},"b":{}}}"#;
        let merge = merge(base, ours, theirs).unwrap();
        assert_eq!(merge.conflicts.len(), 1);
        assert_eq!(merge.conflicts[0].ours, None);
        assert!(!merge.resolve(&[Pick::Ours]).contains("\"a\""));
        assert!(
            merge
                .resolve(&[Pick::Theirs])
                .contains("\"comment\" : \"y\"")
        );
    }
}
