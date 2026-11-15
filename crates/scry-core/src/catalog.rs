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
    /// writes catalogs. Every conflict needs its pick: a missing one is
    /// refused rather than silently taken from ours.
    pub fn resolve(&self, picks: &[Pick]) -> Result<String, String> {
        check_picks(picks.len(), self.conflicts.len())?;
        let mut value = self.merged.clone();
        for (index, conflict) in self.conflicts.iter().enumerate() {
            let (ours, theirs) = &self.choices[index];
            let chosen = match picks.get(index) {
                Some(Pick::Theirs) => theirs,
                _ => ours,
            };
            set(&mut value, &conflict.path, chosen.clone());
        }
        Ok(apple_json(&value))
    }
}

/// How Apple's tools sort keys: `JSONEncoder` by bytes (`Package.resolved`),
/// `JSONSerialization` ignoring case with numbers in order ("a2" before
/// "a10").
type KeyOrder = fn(&str, &str) -> std::cmp::Ordering;

/// The order `keys` are in, of the two Apple's tools write, bytes first
/// when both fit; `None` when they follow neither, and new keys go last.
fn key_order(keys: &[&String]) -> Option<KeyOrder> {
    let orders: [KeyOrder; 2] = [|a, b| a.cmp(b), natural];
    orders
        .into_iter()
        .find(|order| keys.windows(2).all(|pair| order(pair[0], pair[1]).is_le()))
}

/// Ignoring case, with runs of digits compared as numbers.
fn natural(a: &str, b: &str) -> std::cmp::Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, Some(_)) => return std::cmp::Ordering::Less,
            (Some(_), None) => return std::cmp::Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let number = |chars: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut digits = String::new();
                    while let Some(digit) = chars.next_if(char::is_ascii_digit) {
                        digits.push(digit);
                    }
                    digits
                };
                let (x, y) = (number(&mut a), number(&mut b));
                let (x, y) = (x.trim_start_matches('0'), y.trim_start_matches('0'));
                let order = x.len().cmp(&y.len()).then_with(|| x.cmp(y));
                if order.is_ne() {
                    return order;
                }
            }
            (Some(x), Some(y)) => {
                let order = x.to_lowercase().cmp(y.to_lowercase());
                if order.is_ne() {
                    return order;
                }
                a.next();
                b.next();
            }
        }
    }
}

/// One pick per conflict, no more, no fewer.
pub(crate) fn check_picks(picks: usize, conflicts: usize) -> Result<(), String> {
    if picks == conflicts {
        Ok(())
    } else {
        Err(format!(
            "{conflicts} value(s) to choose, {picks} choice(s) given"
        ))
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
    if let (Some(Value::Object(ours_map)), Some(Value::Object(theirs_map))) = (ours, theirs)
        && !(is_localization(path) && shape(ours_map) != shape(theirs_map))
    {
        let empty = Map::new();
        let base_map = match base {
            Some(Value::Object(map)) => map,
            _ => &empty,
        };
        // Ours' order, with the keys only theirs (or the base) has put
        // where the order ours follows puts them.
        let mut keys: Vec<&String> = ours_map.keys().collect();
        let order = key_order(&keys);
        for key in theirs_map.keys().chain(base_map.keys()) {
            if keys.contains(&key) {
                continue;
            }
            match order {
                Some(order) => {
                    let at = keys.partition_point(|known| order(known, key).is_lt());
                    keys.insert(at, key);
                }
                None => keys.push(key),
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

/// Whether `path` leads to one language's translation of a string
/// ("strings", key, "localizations", "fr").
fn is_localization(path: &[String]) -> bool {
    path.len() >= 2 && path[path.len() - 2] == "localizations"
}

/// What a translation is made of: a single `stringUnit`, or `variations`
/// (plural, device). A translation is one or the other; when the sides
/// disagree, it is chosen whole, since merging its keys one by one could
/// write both, which Xcode does not expect.
fn shape(localization: &Map<String, Value>) -> (bool, bool) {
    (
        localization.contains_key("stringUnit"),
        localization.contains_key("variations"),
    )
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
        // As Foundation writes them: an empty line, then the closing
        // bracket at the container's own indent.
        Value::Object(map) if map.is_empty() => {
            out.push_str("{\n\n");
            out.push_str(&indent(depth));
            out.push('}');
        }
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
        Value::Array(items) if items.is_empty() => {
            out.push_str("[\n\n");
            out.push_str(&indent(depth));
            out.push(']');
        }
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
            merge.resolve(&[]).unwrap(),
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
            merge.resolve(&[Pick::Theirs]).unwrap(),
            catalog("Synchronisation terminée", "%lld en attente")
        );
    }

    /// A missing pick is refused instead of silently keeping ours, so a
    /// caller that forgot a value cannot write a merge nobody chose.
    #[test]
    fn every_conflict_needs_its_pick() {
        let base = catalog("Synchronisé", "%lld en attente");
        let ours = catalog("Tout est synchronisé", "%lld en attente");
        let theirs = catalog("Synchronisation terminée", "%lld en attente");
        let merge = merge(&base, &ours, &theirs).unwrap();
        assert!(merge.resolve(&[]).is_err());
        assert!(merge.resolve(&[Pick::Ours, Pick::Ours]).is_err());
        assert!(merge.resolve(&[Pick::Ours]).is_ok());
    }

    /// One side turns a translation into a plural while the other edits
    /// its single value: the translation is one choice, taken whole, so
    /// the file never gets both a `stringUnit` and `variations`.
    #[test]
    fn a_translation_changed_into_a_plural_is_chosen_whole() {
        let base = r#"{"strings":{"n":{"localizations":{"fr":{"stringUnit":{"state":"translated","value":"%lld fichiers"}}}}}}"#;
        let ours = r#"{"strings":{"n":{"localizations":{"fr":{"variations":{"plural":{"one":{"stringUnit":{"state":"translated","value":"%lld fichier"}}}}}}}}}"#;
        let theirs = r#"{"strings":{"n":{"localizations":{"fr":{"stringUnit":{"state":"translated","value":"%lld documents"}}}}}}"#;
        let merge = merge(base, ours, theirs).unwrap();
        assert_eq!(merge.conflicts.len(), 1);
        assert_eq!(merge.conflicts[0].key.as_deref(), Some("n"));
        assert_eq!(merge.conflicts[0].detail, "fr");

        let theirs_picked = merge.resolve(&[Pick::Theirs]).unwrap();
        assert!(theirs_picked.contains("%lld documents"));
        assert!(!theirs_picked.contains("variations"));
        let ours_picked = merge.resolve(&[Pick::Ours]).unwrap();
        assert!(ours_picked.contains("variations"));
        assert!(!ours_picked.contains("%lld fichiers"));
    }

    /// A merge is written as Xcode writes the file, so it adds no noise to the diff: an
    /// empty container closes at its own indent, and a key only theirs added goes where
    /// the order ours follows puts it, by bytes or ignoring case with numbers in order.
    #[test]
    fn the_merge_is_written_as_xcode_writes_it() {
        // Ignoring case with numbers in order, as `JSONSerialization` sorts.
        // Both sides change the strings, so their keys are merged (with
        // ours unchanged, theirs would be taken whole, in its own order).
        let base = r#"{"strings":{"a2":{},"a10":{},"c":{}}}"#;
        let ours = r#"{"strings":{"a2":{},"a10":{},"c":{"x":[]}}}"#;
        let theirs = r#"{"strings":{"a2":{},"a3":{},"a10":{},"B":{},"c":{}}}"#;
        let written = merge(base, ours, theirs).unwrap().resolve(&[]).unwrap();
        assert_eq!(
            written,
            "{\n  \"strings\" : {\n    \"a2\" : {\n\n    },\n    \"a3\" : {\n\n    },\n    \"a10\" : {\n\n    },\n    \"B\" : {\n\n    },\n    \"c\" : {\n      \"x\" : [\n\n      ]\n    }\n  }\n}\n"
        );

        // Sorted by bytes, as `JSONEncoder` sorts `Package.resolved`'s keys.
        let ours = r#"{"Zed":1,"a":1}"#;
        let theirs = r#"{"Zed":1,"a":1,"B":1}"#;
        let written = merge("{}", ours, theirs).unwrap().resolve(&[]).unwrap();
        assert_eq!(written, "{\n  \"B\" : 1,\n  \"Zed\" : 1,\n  \"a\" : 1\n}\n");
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
        assert!(!merge.resolve(&[Pick::Ours]).unwrap().contains("\"a\""));
        assert!(
            merge
                .resolve(&[Pick::Theirs])
                .unwrap()
                .contains("\"comment\" : \"y\"")
        );
    }
}
