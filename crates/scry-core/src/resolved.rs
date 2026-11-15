//! `Package.resolved`: the Swift packages a project is pinned to. Git
//! merges it line by line, so two branches bumping different packages
//! conflict, and a line merge can mix one side's version with the other
//! side's revision. This merges it package by package instead, each pin
//! whole: a pin one side changed is taken from that side, and only a
//! package both sides moved differently is left to the user.

use serde_json::{Map, Value};

use crate::catalog::{Pick, apple_json, check_picks};

/// A package one side or both changed, as people read its pin: "1.2.0",
/// "main @ 3f2a9c1" or "3f2a9c1"; `None` where that version has none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinChange {
    pub identity: String,
    pub base: Option<String>,
    pub ours: Option<String>,
    pub theirs: Option<String>,
}

impl PinChange {
    /// Ours moved it: an automatic pin is taken from the side that moved
    /// it, ours unless only theirs did.
    pub fn moved_by_ours(&self) -> bool {
        self.ours != self.base
    }
}

/// The three versions merged.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedMerge {
    /// Pins taken from the one side that changed them.
    pub automatic: Vec<PinChange>,
    /// Pins both sides changed differently.
    pub conflicts: Vec<PinChange>,
    /// Both sides changed `originHash`, the fingerprint of the manifests'
    /// dependencies: ours is written, and Xcode computes it again on the
    /// next resolution.
    pub origin_hash_stale: bool,
    /// Ours, with every pin settled but the conflicting ones.
    merged: Value,
    /// Where the pins are: `["pins"]` (versions 2 and 3) or
    /// `["object", "pins"]` (version 1).
    pins_path: Vec<String>,
    /// The key naming a pin: "identity", or "package" in version 1.
    identity_key: String,
    /// Each pin, by identity, once settled (`None`: removed).
    pins: Vec<(String, Option<Value>)>,
    /// The conflicting pins: ours and theirs.
    choices: Vec<(Option<Value>, Option<Value>)>,
}

/// Merges `base`, `ours` and `theirs`, each the text of a
/// `Package.resolved`; an empty base stands for a file both sides added.
pub fn merge(base: &str, ours: &str, theirs: &str) -> Result<ResolvedMerge, String> {
    let parse = |text: &str, which: &str| -> Result<Value, String> {
        if text.trim().is_empty() {
            return Ok(Value::Object(Map::new()));
        }
        serde_json::from_str(text).map_err(|error| format!("{which} is not valid JSON: {error}"))
    };
    let base = parse(base, "the base")?;
    let ours = parse(ours, "ours")?;
    let theirs = parse(theirs, "theirs")?;

    // Each format version places and names pins its own way; merging pins
    // across them would write a file of neither format.
    let format = |file: &Value| file.get("version").map(Value::to_string);
    if let (Some(ours_format), Some(theirs_format)) = (format(&ours), format(&theirs))
        && ours_format != theirs_format
    {
        return Err(format!(
            "ours is format {ours_format} of Package.resolved and theirs format {theirs_format}, \
             so its pins cannot be merged one by one: take one side's file, then let Xcode \
             resolve the packages again"
        ));
    }

    let version_one = ours.get("object").is_some() || theirs.get("object").is_some();
    let (pins_path, identity_key) = if version_one {
        (
            vec![String::from("object"), String::from("pins")],
            "package",
        )
    } else {
        (vec![String::from("pins")], "identity")
    };
    let base_pins = pins(&base, &pins_path, identity_key);
    let ours_pins = pins(&ours, &pins_path, identity_key);
    let theirs_pins = pins(&theirs, &pins_path, identity_key);

    let mut identities: Vec<String> = Vec::new();
    for (identity, _) in ours_pins.iter().chain(&theirs_pins).chain(&base_pins) {
        if !identities.contains(identity) {
            identities.push(identity.clone());
        }
    }
    // SwiftPM sorts pins by identity.
    identities.sort_by_key(|identity| identity.to_lowercase());

    let find = |list: &[(String, Value)], identity: &str| -> Option<Value> {
        list.iter()
            .find(|(name, _)| name == identity)
            .map(|(_, pin)| pin.clone())
    };
    let mut merge = ResolvedMerge {
        automatic: Vec::new(),
        conflicts: Vec::new(),
        origin_hash_stale: false,
        merged: ours.clone(),
        pins_path,
        identity_key: identity_key.to_string(),
        pins: Vec::new(),
        choices: Vec::new(),
    };
    for identity in identities {
        let base_pin = find(&base_pins, &identity);
        let ours_pin = find(&ours_pins, &identity);
        let theirs_pin = find(&theirs_pins, &identity);
        let change = || PinChange {
            identity: identity.clone(),
            base: base_pin.as_ref().map(describe),
            ours: ours_pin.as_ref().map(describe),
            theirs: theirs_pin.as_ref().map(describe),
        };
        let settled = if ours_pin == theirs_pin {
            ours_pin.clone()
        } else if ours_pin == base_pin {
            merge.automatic.push(change());
            theirs_pin.clone()
        } else if theirs_pin == base_pin {
            merge.automatic.push(change());
            ours_pin.clone()
        } else {
            merge.conflicts.push(change());
            merge.choices.push((ours_pin.clone(), theirs_pin.clone()));
            ours_pin.clone()
        };
        merge.pins.push((identity, settled));
    }

    // The other top-level values (originHash, version), each whole.
    if let (Value::Object(merged), Value::Object(theirs_map)) = (&mut merge.merged, &theirs) {
        let empty = Map::new();
        let base_map = base.as_object().unwrap_or(&empty);
        for (key, theirs_value) in theirs_map {
            if key == &merge.pins_path[0] {
                continue;
            }
            let ours_value = merged.get(key).cloned();
            let base_value = base_map.get(key);
            if ours_value.as_ref() == base_value && Some(theirs_value) != base_value {
                merged.insert(key.clone(), theirs_value.clone());
            } else if ours_value.as_ref() != Some(theirs_value)
                && Some(theirs_value) != base_value
                && key == "originHash"
            {
                merge.origin_hash_stale = true;
            }
        }
    }
    Ok(merge)
}

impl ResolvedMerge {
    /// The file with one pick per conflicting package, in order, written
    /// as SwiftPM writes it. Every conflict needs its pick.
    pub fn resolve(&self, picks: &[Pick]) -> Result<String, String> {
        check_picks(picks.len(), self.conflicts.len())?;
        let mut conflict = 0;
        let mut list = Vec::new();
        for (identity, settled) in &self.pins {
            let pin = if self
                .conflicts
                .get(conflict)
                .is_some_and(|change| &change.identity == identity)
            {
                let (ours, theirs) = &self.choices[conflict];
                let pick = picks.get(conflict).copied();
                conflict += 1;
                match pick {
                    Some(Pick::Theirs) => theirs.clone(),
                    _ => ours.clone(),
                }
            } else {
                settled.clone()
            };
            if let Some(pin) = pin {
                list.push(pin);
            }
        }
        let mut value = self.merged.clone();
        let mut node = &mut value;
        let (last, parents) = self.pins_path.split_last().expect("a pins path");
        for part in parents {
            let Value::Object(map) = node else { break };
            node = map
                .entry(part.clone())
                .or_insert_with(|| Value::Object(Map::new()));
        }
        if let Value::Object(map) = node {
            map.insert(last.clone(), Value::Array(list));
        }
        Ok(apple_json(&value))
    }

    /// The key naming a pin, for messages.
    pub fn identity_key(&self) -> &str {
        &self.identity_key
    }
}

/// The pins of one version, by identity.
fn pins(root: &Value, path: &[String], identity_key: &str) -> Vec<(String, Value)> {
    let mut node = root;
    for part in path {
        match node.get(part) {
            Some(next) => node = next,
            None => return Vec::new(),
        }
    }
    node.as_array()
        .into_iter()
        .flatten()
        .filter_map(|pin| {
            let identity = pin.get(identity_key)?.as_str()?.to_string();
            Some((identity, pin.clone()))
        })
        .collect()
}

/// A pin's state in a few characters.
fn describe(pin: &Value) -> String {
    let state = pin.get("state");
    let text = |key: &str| {
        state
            .and_then(|state| state.get(key))
            .and_then(Value::as_str)
    };
    let revision = text("revision").map(|revision| revision.chars().take(7).collect::<String>());
    match (text("version"), text("branch"), revision) {
        (Some(version), _, _) => version.to_string(),
        (None, Some(branch), Some(revision)) => format!("{branch} @ {revision}"),
        (None, Some(branch), None) => branch.to_string(),
        (None, None, Some(revision)) => revision,
        (None, None, None) => String::from("?"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pin(identity: &str, version: &str, revision: &str) -> String {
        format!(
            r#"{{
      "identity" : "{identity}",
      "kind" : "remoteSourceControl",
      "location" : "https://github.com/example/{identity}",
      "state" : {{
        "revision" : "{revision}",
        "version" : "{version}"
      }}
    }}"#
        )
    }

    fn resolved(hash: &str, pins: &[String]) -> String {
        format!(
            "{{\n  \"originHash\" : \"{hash}\",\n  \"pins\" : [\n    {}\n  ],\n  \"version\" : 3\n}}\n",
            pins.join(",\n    ")
        )
    }

    /// Two branches bumping different packages merge by themselves, and
    /// the file is written exactly as SwiftPM writes it.
    #[test]
    fn different_packages_bumped_on_each_side_merge() {
        let base = resolved(
            "h0",
            &[
                pin("alamofire", "5.8.0", "a0"),
                pin("kingfisher", "7.9.0", "k0"),
            ],
        );
        let ours = resolved(
            "h0",
            &[
                pin("alamofire", "5.8.1", "a1"),
                pin("kingfisher", "7.9.0", "k0"),
            ],
        );
        let theirs = resolved(
            "h0",
            &[
                pin("alamofire", "5.8.0", "a0"),
                pin("kingfisher", "7.10.0", "k1"),
            ],
        );
        let merge = merge(&base, &ours, &theirs).unwrap();
        assert!(merge.conflicts.is_empty());
        assert_eq!(merge.automatic.len(), 2);
        assert_eq!(
            merge.resolve(&[]).unwrap(),
            resolved(
                "h0",
                &[
                    pin("alamofire", "5.8.1", "a1"),
                    pin("kingfisher", "7.10.0", "k1")
                ]
            )
        );
    }

    /// A package both sides moved is one choice, taken whole: the version
    /// never travels without its revision.
    #[test]
    fn a_package_bumped_twice_is_a_choice_of_whole_pins() {
        let base = resolved("h0", &[pin("alamofire", "5.8.0", "a0")]);
        let ours = resolved("h0", &[pin("alamofire", "5.8.1", "a1")]);
        let theirs = resolved("h0", &[pin("alamofire", "5.9.0", "a2")]);
        let merge = merge(&base, &ours, &theirs).unwrap();
        assert_eq!(merge.conflicts.len(), 1);
        let conflict = &merge.conflicts[0];
        assert_eq!(conflict.identity, "alamofire");
        assert_eq!(conflict.ours.as_deref(), Some("5.8.1"));
        assert_eq!(conflict.theirs.as_deref(), Some("5.9.0"));
        assert_eq!(
            merge.resolve(&[Pick::Theirs]).unwrap(),
            resolved("h0", &[pin("alamofire", "5.9.0", "a2")])
        );
    }

    /// Packages added on each side are both kept, in identity order; one
    /// removed on one side and untouched on the other goes.
    #[test]
    fn additions_and_removals_are_kept() {
        let base = resolved(
            "h0",
            &[
                pin("alamofire", "5.8.0", "a0"),
                pin("snapkit", "5.6.0", "s0"),
            ],
        );
        let ours = resolved(
            "h1",
            &[
                pin("alamofire", "5.8.0", "a0"),
                pin("lottie", "4.4.0", "l0"),
                pin("snapkit", "5.6.0", "s0"),
            ],
        );
        let theirs = resolved(
            "h2",
            &[
                pin("alamofire", "5.8.0", "a0"),
                pin("kingfisher", "7.10.0", "k1"),
            ],
        );
        let merge = merge(&base, &ours, &theirs).unwrap();
        assert!(merge.conflicts.is_empty());
        assert!(merge.origin_hash_stale);
        assert_eq!(
            merge.resolve(&[]).unwrap(),
            resolved(
                "h1",
                &[
                    pin("alamofire", "5.8.0", "a0"),
                    pin("kingfisher", "7.10.0", "k1"),
                    pin("lottie", "4.4.0", "l0")
                ]
            )
        );
    }

    /// Version 1 files name pins by `package`, under `object`.
    /// Sides written by different Xcode versions use different formats
    /// (version 2 and 3): merging their pins would give a file of neither,
    /// so the merge is refused and says why.
    #[test]
    fn sides_of_different_formats_are_refused() {
        let ours = resolved("h0", &[pin("alamofire", "5.8.0", "a0")]);
        let theirs = ours.replace("\"version\" : 3", "\"version\" : 2");
        assert_ne!(ours, theirs);
        let error = merge(&ours, &ours, &theirs).unwrap_err();
        assert!(error.contains("format 3"), "{error}");
        assert!(error.contains("format 2"), "{error}");
    }

    /// Older projects keep version 1 of the format, with the pins under
    /// `object` and named by `package`: they merge pin by pin like the
    /// current versions, and the bump is written back.
    #[test]
    fn version_one_files_are_read() {
        let file = |version: &str| {
            format!(
                r#"{{"object":{{"pins":[{{"package":"Alamofire","repositoryURL":"u","state":{{"branch":null,"revision":"r{version}","version":"{version}"}}}}]}},"version":1}}"#
            )
        };
        let merge = merge(&file("5.8.0"), &file("5.8.1"), &file("5.8.0")).unwrap();
        assert_eq!(merge.identity_key(), "package");
        assert_eq!(merge.automatic[0].identity, "Alamofire");
        assert!(
            merge
                .resolve(&[])
                .unwrap()
                .contains("\"version\" : \"5.8.1\"")
        );
    }
}
