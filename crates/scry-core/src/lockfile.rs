//! Lockfiles (`package-lock.json`, `Cargo.lock`, `Podfile.lock`…): the
//! package manager writes them from the manifest, so a merged lockfile is
//! never edited by hand. Take one side, then let the tool add the other
//! side's changes back from the merged manifest. This reads each version's
//! packages, to show what each side changed and what the tool must redo.

use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

/// A lockfile format, by file name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Cargo,
    Npm,
    Yarn,
    Pnpm,
    CocoaPods,
    Bundler,
    Poetry,
    Composer,
    GoSum,
    Uv,
    Pipenv,
    Pub,
    Carthage,
    Gradle,
    Nix,
    Bun,
    Deno,
    Mix,
}

impl Format {
    /// The format of the file at `path`, when it is a lockfile.
    pub fn of(path: &str) -> Option<Format> {
        let name = path.rsplit('/').next().unwrap_or(path);
        Some(match name {
            "Cargo.lock" => Format::Cargo,
            "package-lock.json" | "npm-shrinkwrap.json" => Format::Npm,
            "yarn.lock" => Format::Yarn,
            "pnpm-lock.yaml" => Format::Pnpm,
            "Podfile.lock" => Format::CocoaPods,
            "Gemfile.lock" => Format::Bundler,
            "poetry.lock" => Format::Poetry,
            "composer.lock" => Format::Composer,
            "go.sum" => Format::GoSum,
            "uv.lock" => Format::Uv,
            "Pipfile.lock" => Format::Pipenv,
            "pubspec.lock" => Format::Pub,
            "Cartfile.resolved" => Format::Carthage,
            "gradle.lockfile" => Format::Gradle,
            "flake.lock" => Format::Nix,
            "bun.lock" => Format::Bun,
            "deno.lock" => Format::Deno,
            "mix.lock" => Format::Mix,
            _ => return None,
        })
    }

    /// The package manager, as people name it.
    pub fn tool(self) -> &'static str {
        match self {
            Format::Cargo => "Cargo",
            Format::Npm => "npm",
            Format::Yarn => "Yarn",
            Format::Pnpm => "pnpm",
            Format::CocoaPods => "CocoaPods",
            Format::Bundler => "Bundler",
            Format::Poetry => "Poetry",
            Format::Composer => "Composer",
            Format::GoSum => "Go",
            Format::Uv => "uv",
            Format::Pipenv => "Pipenv",
            Format::Pub => "Flutter",
            Format::Carthage => "Carthage",
            Format::Gradle => "Gradle",
            Format::Nix => "Nix",
            Format::Bun => "Bun",
            Format::Deno => "Deno",
            Format::Mix => "Mix",
        }
    }

    /// The files the lockfile may be written from, beside it, the usual
    /// one first: the overview names the one that exists.
    pub fn manifests(self) -> &'static [&'static str] {
        match self {
            Format::Cargo => &["Cargo.toml"],
            Format::Npm | Format::Yarn | Format::Pnpm | Format::Bun => &["package.json"],
            Format::CocoaPods => &["Podfile"],
            Format::Bundler => &["Gemfile", "gems.rb"],
            Format::Poetry | Format::Uv => &["pyproject.toml"],
            Format::Composer => &["composer.json"],
            Format::GoSum => &["go.mod"],
            Format::Pipenv => &["Pipfile"],
            Format::Pub => &["pubspec.yaml"],
            Format::Carthage => &["Cartfile"],
            Format::Gradle => &["build.gradle.kts", "build.gradle"],
            Format::Nix => &["flake.nix"],
            Format::Deno => &["deno.json", "deno.jsonc"],
            Format::Mix => &["mix.exs"],
        }
    }

    /// The command that writes the lockfile again from the manifest,
    /// keeping the versions already locked where the tool can. `manifest`
    /// is the manifest's text (empty when it cannot be read): a Dart
    /// package without Flutter is fetched with `dart`, as `flutter` may
    /// not be installed.
    pub fn command(self, manifest: &str) -> &'static str {
        match self {
            Format::Pub if !depends_on_flutter(manifest) => "dart pub get",
            Format::Cargo => "cargo update --workspace",
            Format::Npm => "npm install",
            Format::Yarn => "yarn install",
            Format::Pnpm => "pnpm install",
            Format::CocoaPods => "pod install",
            Format::Bundler => "bundle install",
            // Poetry 2 keeps locked versions by default and refuses
            // `--no-update`; Poetry 1 needs it (see `caveat`).
            Format::Poetry => "poetry lock",
            Format::Composer => "composer update --lock",
            Format::GoSum => "go mod tidy",
            Format::Uv => "uv lock",
            Format::Pipenv => "pipenv lock",
            Format::Pub => "flutter pub get",
            // Carthage has no lock-only update: this resolves every
            // dependency again, within the Cartfile's requirements.
            Format::Carthage => "carthage update --no-build",
            Format::Gradle => "./gradlew dependencies --write-locks",
            Format::Nix => "nix flake lock",
            Format::Bun => "bun install",
            Format::Deno => "deno install",
            Format::Mix => "mix deps.get",
        }
    }

    /// Where `command` may not keep the locked versions, what to know
    /// before running it.
    pub fn caveat(self) -> Option<&'static str> {
        match self {
            Format::Poetry => Some(
                "With Poetry 1.x, run poetry lock --no-update instead: without it, Poetry 1 moves every package to its newest allowed version.",
            ),
            Format::Pipenv => Some(
                "Pipenv has no command that keeps the locked versions: it locks every package again at the newest version the Pipfile allows. Check the lockfile's changes before staging it.",
            ),
            Format::Carthage => Some(
                "Carthage has no lock-only update: it resolves every dependency again within the Cartfile's requirements. Check the Cartfile.resolved changes before staging it.",
            ),
            _ => None,
        }
    }

    /// `go.sum` only lists checksums: both sides' lines can be kept.
    pub fn merges_by_lines(self) -> bool {
        self == Format::GoSum
    }
}

/// Whether a `pubspec.yaml` depends on the Flutter SDK
/// (`sdk: flutter` under a dependency).
fn depends_on_flutter(pubspec: &str) -> bool {
    pubspec.lines().any(|line| {
        let line = line.trim();
        line.strip_prefix("sdk:")
            .is_some_and(|value| value.trim().trim_matches(['"', '\'']) == "flutter")
    })
}

/// A package one side or both changed: its version in each, joined
/// ("1.2.0, 2.0.1") when a version holds it several times; `None` where
/// that version does not have it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PackageChange {
    pub name: String,
    pub base: Option<String>,
    pub ours: Option<String>,
    pub theirs: Option<String>,
}

impl PackageChange {
    /// Both sides changed it, differently: the side taken keeps its
    /// version, since package managers keep a locked version the merged
    /// manifest still allows.
    pub fn both_changed(&self) -> bool {
        self.ours != self.base && self.theirs != self.base && self.ours != self.theirs
    }
}

/// The packages each side changed since the base, by name.
pub fn changes(format: Format, base: &str, ours: &str, theirs: &str) -> Vec<PackageChange> {
    let base = packages(format, base);
    let ours = packages(format, ours);
    let theirs = packages(format, theirs);
    let names: BTreeSet<&String> = base
        .keys()
        .chain(ours.keys())
        .chain(theirs.keys())
        .collect();
    names
        .into_iter()
        .filter_map(|name| {
            let change = PackageChange {
                name: name.clone(),
                base: base.get(name).cloned(),
                ours: ours.get(name).cloned(),
                theirs: theirs.get(name).cloned(),
            };
            (change.ours != change.base || change.theirs != change.base).then_some(change)
        })
        .collect()
}

/// Lines both sides have, and those one side added; a line one side
/// removed goes. Sorted, as `go.sum` is.
pub fn merge_lines(base: &str, ours: &str, theirs: &str) -> String {
    let set = |text: &str| -> BTreeSet<String> {
        text.lines()
            .filter(|line| !line.trim().is_empty())
            .map(str::to_string)
            .collect()
    };
    let (base, ours, theirs) = (set(base), set(ours), set(theirs));
    let kept = ours.iter().chain(theirs.iter()).filter(|line| {
        let both = ours.contains(*line) && theirs.contains(*line);
        both || !base.contains(*line)
    });
    let lines: BTreeSet<&String> = kept.collect();
    let mut text = String::new();
    for line in lines {
        text.push_str(line);
        text.push('\n');
    }
    text
}

/// Each package's versions, by name.
pub fn packages(format: Format, text: &str) -> BTreeMap<String, String> {
    let mut found: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut add = |name: &str, version: &str| {
        if !name.is_empty() && !version.is_empty() {
            found
                .entry(name.to_string())
                .or_default()
                .insert(version.to_string());
        }
    };
    match format {
        Format::Cargo | Format::Poetry | Format::Uv => toml_packages(text, &mut add),
        Format::Npm => npm_packages(text, &mut add),
        Format::Composer => composer_packages(text, &mut add),
        Format::Yarn => yarn_packages(text, &mut add),
        Format::Pnpm => pnpm_packages(text, &mut add),
        Format::CocoaPods => indented_list(text, "PODS:", "  - ", &mut add),
        Format::Bundler => {
            indented_list(text, "  specs:", "    ", &mut add);
            // The Bundler version that wrote it, which two people on
            // different versions keep changing.
            let mut lines = text.lines();
            while let Some(line) = lines.next() {
                if line.trim_end() == "BUNDLED WITH" {
                    add("bundler", lines.next().unwrap_or("").trim());
                }
            }
        }
        Format::Pipenv => pipenv_packages(text, &mut add),
        Format::Pub => pub_packages(text, &mut add),
        Format::Carthage => {
            for line in text.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if let [_, name, version, ..] = parts.as_slice() {
                    add(name.trim_matches('"'), &short(version.trim_matches('"')));
                }
            }
        }
        Format::Gradle => {
            for line in text.lines().filter(|line| !line.starts_with('#')) {
                let coordinates = line.split('=').next().unwrap_or("");
                if let Some((name, version)) = coordinates.rsplit_once(':') {
                    add(name, version);
                }
            }
        }
        Format::Nix => nix_packages(text, &mut add),
        Format::Bun => bun_packages(text, &mut add),
        Format::Deno => deno_packages(text, &mut add),
        Format::Mix => mix_packages(text, &mut add),
        Format::GoSum => {
            for line in text.lines() {
                let mut parts = line.split_whitespace();
                if let (Some(module), Some(version)) = (parts.next(), parts.next()) {
                    add(module, version.trim_end_matches("/go.mod"));
                }
            }
        }
    }
    found
        .into_iter()
        .map(|(name, versions)| (name, versions.into_iter().collect::<Vec<_>>().join(", ")))
        .collect()
}

/// `[[package]]` tables with `name = "…"` and `version = "…"`.
fn toml_packages(text: &str, add: &mut dyn FnMut(&str, &str)) {
    let mut name: Option<String> = None;
    let mut version: Option<String> = None;
    let mut flush = |name: &mut Option<String>, version: &mut Option<String>| {
        if let (Some(name), Some(version)) = (name.take(), version.take()) {
            add(&name, &version);
        }
    };
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            flush(&mut name, &mut version);
            continue;
        }
        if let Some(value) = toml_string(line, "name") {
            name = Some(value);
        } else if let Some(value) = toml_string(line, "version") {
            version = Some(value);
        }
    }
    flush(&mut name, &mut version);
}

fn toml_string(line: &str, key: &str) -> Option<String> {
    let rest = line
        .strip_prefix(key)?
        .trim_start()
        .strip_prefix('=')?
        .trim();
    Some(rest.trim_matches('"').to_string())
}

/// `packages` ("node_modules/a/node_modules/b" → b) in lockfile v2 and
/// v3, `dependencies` in v1.
fn npm_packages(text: &str, add: &mut dyn FnMut(&str, &str)) {
    let Ok(json) = serde_json::from_str::<Value>(text) else {
        return;
    };
    if let Some(Value::Object(packages)) = json.get("packages") {
        for (path, package) in packages {
            let Some(name) = path
                .rsplit("node_modules/")
                .next()
                .filter(|_| !path.is_empty())
            else {
                continue;
            };
            if let Some(version) = package.get("version").and_then(Value::as_str) {
                add(name, version);
            }
        }
    } else if let Some(Value::Object(dependencies)) = json.get("dependencies") {
        for (name, package) in dependencies {
            if let Some(version) = package.get("version").and_then(Value::as_str) {
                add(name, version);
            }
        }
    }
}

/// `default` and `develop`, each package with `"version": "==1.2.3"`.
fn pipenv_packages(text: &str, add: &mut dyn FnMut(&str, &str)) {
    let Ok(json) = serde_json::from_str::<Value>(text) else {
        return;
    };
    for group in ["default", "develop"] {
        let Some(Value::Object(packages)) = json.get(group) else {
            continue;
        };
        for (name, package) in packages {
            if let Some(version) = package.get("version").and_then(Value::as_str) {
                add(name, version.trim_start_matches("=="));
            }
        }
    }
}

/// `packages:` entries at two spaces, each with `    version: "1.2.3"`.
fn pub_packages(text: &str, add: &mut dyn FnMut(&str, &str)) {
    let mut inside = false;
    let mut name: Option<String> = None;
    for line in text.lines() {
        if !line.starts_with(' ') && !line.is_empty() {
            inside = line == "packages:";
            name = None;
            continue;
        }
        if !inside {
            continue;
        }
        if let Some(entry) = line
            .strip_prefix("  ")
            .filter(|entry| !entry.starts_with(' '))
        {
            name = Some(entry.trim_end_matches(':').trim_matches('"').to_string());
        } else if let (Some(version), Some(name)) = (line.strip_prefix("    version:"), &name) {
            add(name, version.trim().trim_matches('"'));
        }
    }
}

/// `nodes`, each input locked to a revision; `root` is the flake itself.
fn nix_packages(text: &str, add: &mut dyn FnMut(&str, &str)) {
    let Ok(json) = serde_json::from_str::<Value>(text) else {
        return;
    };
    let Some(Value::Object(nodes)) = json.get("nodes") else {
        return;
    };
    for (name, node) in nodes {
        let locked = node.get("locked");
        let field = |key: &str| {
            locked
                .and_then(|locked| locked.get(key))
                .and_then(Value::as_str)
        };
        if let Some(revision) = field("rev").or(field("narHash")) {
            add(name, &short(revision));
        }
    }
}

/// Lines `"react": ["react@18.3.1", …]` (the file allows trailing
/// commas, so it is read by lines rather than as JSON).
fn bun_packages(text: &str, add: &mut dyn FnMut(&str, &str)) {
    for line in text.lines() {
        let Some((_, rest)) = line.split_once("\": [\"") else {
            continue;
        };
        let spec = rest.split('"').next().unwrap_or("");
        if let Some(name) = package_name(spec) {
            add(name, &spec[name.len() + 1..]);
        }
    }
}

/// The keys of `npm` and `jsr` ("react@18.3.1", "@std/path@1.0.8"), at
/// the top (version 4) or under `packages` (version 3).
fn deno_packages(text: &str, add: &mut dyn FnMut(&str, &str)) {
    let Ok(json) = serde_json::from_str::<Value>(text) else {
        return;
    };
    let groups = json.get("packages").unwrap_or(&json);
    for registry in ["npm", "jsr"] {
        let Some(Value::Object(packages)) = groups.get(registry) else {
            continue;
        };
        for spec in packages.keys() {
            if let Some(name) = package_name(spec) {
                add(name, &spec[name.len() + 1..]);
            }
        }
    }
}

/// Lines `  "jason": {:hex, :jason, "1.4.1", …}` or `  "x": {:git, "url",
/// "revision", …}`.
fn mix_packages(text: &str, add: &mut dyn FnMut(&str, &str)) {
    for line in text.lines() {
        let Some((key, rest)) = line.trim().split_once("\": {") else {
            continue;
        };
        let name = key.trim_start_matches('"');
        let quoted: Vec<&str> = rest.split('"').skip(1).step_by(2).collect();
        let version = if rest.starts_with(":git") {
            quoted.get(1).map(|revision| short(revision))
        } else {
            quoted.first().map(|version| version.to_string())
        };
        if let Some(version) = version {
            add(name, &version);
        }
    }
}

/// A commit or hash shortened as git shows it; a version as it is.
fn short(revision: &str) -> String {
    let hex = revision.len() >= 32 && revision.chars().all(|c| c.is_ascii_hexdigit());
    if hex {
        revision[..7].to_string()
    } else {
        revision.to_string()
    }
}

/// `packages` and `packages-dev`, each a list of `{name, version}`.
fn composer_packages(text: &str, add: &mut dyn FnMut(&str, &str)) {
    let Ok(json) = serde_json::from_str::<Value>(text) else {
        return;
    };
    for list in ["packages", "packages-dev"] {
        for package in json
            .get(list)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let (Some(name), Some(version)) = (
                package.get("name").and_then(Value::as_str),
                package.get("version").and_then(Value::as_str),
            ) {
                add(name, version);
            }
        }
    }
}

/// Entries such as `"lodash@^4.17.0", lodash@^4.0.0:` followed by
/// `  version "4.17.21"` (Yarn 1) or `  version: 4.17.21` (Yarn 2+).
fn yarn_packages(text: &str, add: &mut dyn FnMut(&str, &str)) {
    let mut name: Option<String> = None;
    for line in text.lines() {
        if !line.starts_with(' ') && line.ends_with(':') && !line.starts_with('#') {
            let first = line.trim_end_matches(':').split(',').next().unwrap_or("");
            let spec = first.trim().trim_matches('"');
            name = package_name(spec)
                .filter(|name| *name != "__metadata")
                .map(str::to_string);
        } else if let Some(rest) = line.strip_prefix("  version") {
            let version = rest.trim_start_matches(':').trim().trim_matches('"');
            if let Some(name) = &name {
                add(name, version);
            }
        }
    }
}

/// `packages:` entries keyed `lodash@4.17.21:` (v9), `/lodash@4.17.21:`
/// (v6), quoted when scoped, with peer suffixes `(react@18.2.0)`.
fn pnpm_packages(text: &str, add: &mut dyn FnMut(&str, &str)) {
    let mut inside = false;
    for line in text.lines() {
        if !line.starts_with(' ') && !line.is_empty() {
            inside = line == "packages:";
            continue;
        }
        let Some(entry) = line.strip_prefix("  ") else {
            continue;
        };
        if !inside || entry.starts_with(' ') || !entry.ends_with(':') {
            continue;
        }
        let key = entry
            .trim_end_matches(':')
            .trim_matches('\'')
            .trim_matches('"');
        let key = key.trim_start_matches('/');
        let key = key.split('(').next().unwrap_or(key);
        if let Some(at) = key.rfind('@').filter(|at| *at > 0) {
            add(&key[..at], &key[at + 1..]);
        }
    }
}

/// `name (version)` lines right under `section`, at exactly `prefix`'s
/// indent: `  - Alamofire (5.8.1)` in a Podfile.lock, `    rack (3.0.8)`
/// in a Gemfile.lock. Deeper lines are dependencies' requirements.
fn indented_list(text: &str, section: &str, prefix: &str, add: &mut dyn FnMut(&str, &str)) {
    let mut inside = false;
    for line in text.lines() {
        if line.trim_end() == section {
            inside = true;
            continue;
        }
        if !inside {
            continue;
        }
        if line.trim().is_empty()
            || line.len() - line.trim_start().len() < prefix.len() - prefix.trim_start().len()
        {
            inside = false;
            continue;
        }
        let Some(entry) = line.strip_prefix(prefix) else {
            continue;
        };
        if entry.starts_with(' ') {
            continue;
        }
        let entry = entry.trim_end_matches(':').trim().trim_matches('"');
        if let Some((name, version)) = entry.split_once(" (") {
            add(name, version.trim_end_matches(')'));
        }
    }
}

/// "lodash" from "lodash@^4.17.0", "@babel/core" from
/// "@babel/core@npm:^7.0.0".
fn package_name(spec: &str) -> Option<&str> {
    // A scoped name starts with its own `@`.
    let start = usize::from(spec.starts_with('@'));
    let at = spec[start..].find('@')? + start;
    Some(&spec[..at])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lockfiles are told apart by name, so the overview can offer the
    /// right command; other files are not lockfiles.
    #[test]
    fn lockfiles_are_recognised_by_name() {
        let names = [
            ("Cargo.lock", Format::Cargo),
            ("web/package-lock.json", Format::Npm),
            ("npm-shrinkwrap.json", Format::Npm),
            ("yarn.lock", Format::Yarn),
            ("pnpm-lock.yaml", Format::Pnpm),
            ("ios/Podfile.lock", Format::CocoaPods),
            ("Gemfile.lock", Format::Bundler),
            ("poetry.lock", Format::Poetry),
            ("composer.lock", Format::Composer),
            ("go.sum", Format::GoSum),
            ("backend/uv.lock", Format::Uv),
            ("Pipfile.lock", Format::Pipenv),
            ("pubspec.lock", Format::Pub),
            ("Cartfile.resolved", Format::Carthage),
            ("gradle.lockfile", Format::Gradle),
            ("flake.lock", Format::Nix),
            ("bun.lock", Format::Bun),
            ("deno.lock", Format::Deno),
            ("mix.lock", Format::Mix),
        ];
        for (path, format) in names {
            assert_eq!(Format::of(path), Some(format), "{path}");
        }
        assert_eq!(Format::of("package.json"), None);
        assert_eq!(Format::CocoaPods.command(""), "pod install");
        assert_eq!(Format::of("backend/uv.lock"), Some(Format::Uv));
        assert_eq!(
            Format::Gradle.manifests(),
            ["build.gradle.kts", "build.gradle"]
        );
    }

    /// The command offered keeps the locked versions where the tool can:
    /// a pure Dart package is fetched with `dart`, which needs no Flutter,
    /// and Poetry and Pipenv, whose command may move every package, say so.
    #[test]
    fn commands_keep_locked_versions_or_say_they_do_not() {
        let flutter = "name: app\ndependencies:\n  flutter:\n    sdk: flutter\n";
        let dart = "name: tool\ndependencies:\n  args: ^2.4.0\n";
        assert_eq!(Format::Pub.command(flutter), "flutter pub get");
        assert_eq!(Format::Pub.command(dart), "dart pub get");
        assert_eq!(Format::Poetry.command(""), "poetry lock");
        assert!(Format::Poetry.caveat().unwrap().contains("--no-update"));
        assert!(Format::Pipenv.caveat().is_some());
        assert!(Format::Carthage.caveat().is_some());
        assert_eq!(Format::Npm.caveat(), None);
    }

    /// Only go.sum, a list of checksums, merges by keeping both sides' lines: in any
    /// other lockfile that would leave two versions of a package.
    #[test]
    fn only_go_sum_merges_by_lines() {
        assert!(Format::GoSum.merges_by_lines());
        assert!(!Format::Npm.merges_by_lines());
        assert!(!Format::Cargo.merges_by_lines());
    }

    /// Composer lists packages and dev packages; an entry without a version is not a
    /// package to compare.
    #[test]
    fn composer_lists_packages_and_dev_packages() {
        let text = r#"{"packages":[{"name":"monolog/monolog","version":"3.5.0"},{"name":"vendor/unversioned","version":""}],
            "packages-dev":[{"name":"phpunit/phpunit","version":"10.5.10"}]}"#;
        let packages = packages(Format::Composer, text);
        assert_eq!(packages["monolog/monolog"], "3.5.0");
        assert_eq!(packages["phpunit/phpunit"], "10.5.10");
        assert_eq!(packages.len(), 2);
    }

    /// A full commit hash is shortened as git shows it; a shorter hex string or a long
    /// name that is not hex is a version or a branch, kept as it is.
    #[test]
    fn only_full_hashes_are_shortened() {
        assert_eq!(short("0123456789abcdef0123456789abcdef01234567"), "0123456");
        assert_eq!(short("abcdef12"), "abcdef12");
        assert_eq!(
            short("release/2024-05-offline-queue-and-forecast"),
            "release/2024-05-offline-queue-and-forecast"
        );
    }

    /// The `PODS:` list goes on past a pod's own dependencies, indented
    /// deeper, and ends at the next section even without a blank line.
    #[test]
    fn cocoapods_reads_the_whole_pods_list_and_only_it() {
        let lock = "PODS:\n  - SnapKit (5.6.0):\n    - Alamofire (~> 5.0)\n  - Zip (2.1.0)\nDEPENDENCIES:\n  - Kingfisher (~> 7.0)\n";
        let found = packages(Format::CocoaPods, lock);
        let names: Vec<&str> = found.keys().map(String::as_str).collect();
        assert_eq!(names, ["SnapKit", "Zip"]);
    }

    /// Each side's upgrades are listed by package, and a package both
    /// sides moved to different versions is flagged: the tool decides. The
    /// requirements under DEPENDENCIES are not installed versions.
    #[test]
    fn package_changes_are_listed_by_name() {
        let lock = |alamofire: &str, kingfisher: &str| {
            format!(
                "PODS:\n  - Alamofire ({alamofire})\n  - Kingfisher ({kingfisher})\n  - SnapKit (5.6.0):\n    - Alamofire (~> 5.0)\n\nDEPENDENCIES:\n  - Alamofire (~> 5.8)\n  - Kingfisher\n"
            )
        };
        let changes = changes(
            Format::CocoaPods,
            &lock("5.8.0", "7.9.0"),
            &lock("5.8.1", "7.9.0"),
            &lock("5.9.0", "7.10.0"),
        );
        assert_eq!(
            packages(Format::CocoaPods, &lock("5.8.0", "7.9.0"))["Alamofire"],
            "5.8.0"
        );
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].name, "Alamofire");
        assert!(changes[0].both_changed());
        assert_eq!(changes[1].name, "Kingfisher");
        assert_eq!(changes[1].ours.as_deref(), Some("7.9.0"));
        assert!(!changes[1].both_changed());
    }

    /// npm's nested installs count under the package's own name, and a
    /// package held twice shows both versions.
    #[test]
    fn npm_packages_are_read_from_their_paths() {
        let text = r#"{"lockfileVersion":3,"packages":{"":{"name":"web"},
            "node_modules/lodash":{"version":"4.17.21"},
            "node_modules/@babel/core":{"version":"7.24.0"},
            "node_modules/a/node_modules/lodash":{"version":"3.10.1"}}}"#;
        let packages = packages(Format::Npm, text);
        assert_eq!(packages["lodash"], "3.10.1, 4.17.21");
        assert_eq!(packages["@babel/core"], "7.24.0");
        assert_eq!(packages.len(), 2);
    }

    /// Yarn and pnpm name packages inside their keys, scoped or not.
    #[test]
    fn yarn_and_pnpm_keys_give_the_package() {
        let yarn = "# yarn lockfile v1\n\n\"@babel/core@^7.0.0\", \"@babel/core@^7.1.0\":\n  version \"7.24.0\"\n\nlodash@^4.17.0:\n  version \"4.17.21\"\n";
        let packages_found = packages(Format::Yarn, yarn);
        assert_eq!(packages_found["@babel/core"], "7.24.0");
        assert_eq!(packages_found["lodash"], "4.17.21");
        let pnpm = "lockfileVersion: '9.0'\n\npackages:\n\n  '@babel/core@7.24.0':\n    resolution: {integrity: x}\n\n  react-dom@18.2.0(react@18.2.0):\n    resolution: {integrity: y}\n";
        let packages_found = packages(Format::Pnpm, pnpm);
        assert_eq!(packages_found["@babel/core"], "7.24.0");
        assert_eq!(packages_found["react-dom"], "18.2.0");
        // Only the `packages:` section lists packages.
        let pnpm = "lockfileVersion: '9.0'\n\nsettings:\n  other@1.0.0:\n\npackages:\n\n  lodash@4.17.21:\n    resolution: {integrity: z}\n";
        let packages_found = packages(Format::Pnpm, pnpm);
        assert_eq!(packages_found.len(), 1, "{packages_found:?}");
        assert_eq!(packages_found["lodash"], "4.17.21");
    }

    /// Bundler's own version counts as a package, since both sides often
    /// change it; nested requirements are not packages.
    #[test]
    fn gemfile_lock_lists_gems_and_bundler() {
        let text = "GEM\n  remote: https://rubygems.org/\n  specs:\n    fastlane (2.220.0)\n      xcpretty (~> 0.3.0)\n    xcpretty (0.3.0)\n\nPLATFORMS\n  ruby\n\nBUNDLED WITH\n   2.5.9\n";
        let packages = packages(Format::Bundler, text);
        assert_eq!(packages["fastlane"], "2.220.0");
        assert_eq!(packages["xcpretty"], "0.3.0");
        assert_eq!(packages["bundler"], "2.5.9");
        assert_eq!(packages.len(), 3);
    }

    /// The other formats each give their packages: uv, Pipenv, Flutter,
    /// Carthage, Gradle, Nix, Bun, Deno and Mix.
    #[test]
    fn other_formats_give_their_packages() {
        let read = |path: &str, text: &str| packages(Format::of(path).unwrap(), text);
        let uv = "version = 1\n\n[[package]]\nname = \"fastapi\"\nversion = \"0.111.0\"\nsource = { registry = \"https://pypi.org/simple\" }\ndependencies = [\n    { name = \"starlette\" },\n]\n\n[package.metadata]\nrequires-dist = [{ name = \"starlette\", specifier = \">=0.37\" }]\n";
        assert_eq!(read("uv.lock", uv)["fastapi"], "0.111.0");
        assert_eq!(read("uv.lock", uv).len(), 1);
        let pipenv = r#"{"_meta":{},"default":{"requests":{"version":"==2.31.0"}},"develop":{"pytest":{"version":"==8.1.1"}}}"#;
        assert_eq!(read("Pipfile.lock", pipenv)["pytest"], "8.1.1");
        let pubspec = "packages:\n  http:\n    dependency: \"direct main\"\n    source: hosted\n    version: \"1.2.1\"\nsdks:\n  dart: \">=3.3.0 <4.0.0\"\n";
        assert_eq!(read("pubspec.lock", pubspec)["http"], "1.2.1");
        assert_eq!(read("pubspec.lock", pubspec).len(), 1);
        let cartfile = "github \"Alamofire/Alamofire\" \"5.8.1\"\ngit \"https://example.com/kit.git\" \"0123456789abcdef0123456789abcdef01234567\"\n";
        assert_eq!(
            read("Cartfile.resolved", cartfile)["Alamofire/Alamofire"],
            "5.8.1"
        );
        assert_eq!(
            read("Cartfile.resolved", cartfile)["https://example.com/kit.git"],
            "0123456"
        );
        let gradle = "# This is a Gradle generated file\ncom.squareup.okhttp3:okhttp:4.12.0=debugRuntimeClasspath\nempty=\n";
        assert_eq!(
            read("gradle.lockfile", gradle)["com.squareup.okhttp3:okhttp"],
            "4.12.0"
        );
        assert_eq!(read("gradle.lockfile", gradle).len(), 1);
        let flake = r#"{"nodes":{"nixpkgs":{"locked":{"rev":"0123456789abcdef0123456789abcdef01234567"}},"root":{"inputs":{}}},"version":7}"#;
        assert_eq!(read("flake.lock", flake)["nixpkgs"], "0123456");
        let bun = "{\n  \"packages\": {\n    \"@babel/core\": [\"@babel/core@7.24.0\", \"\", {}, \"sha512-x\"],\n    \"react\": [\"react@18.3.1\", \"\", {}, \"sha512-y\"],\n  },\n}\n";
        assert_eq!(read("bun.lock", bun)["@babel/core"], "7.24.0");
        assert_eq!(read("bun.lock", bun)["react"], "18.3.1");
        let deno = r#"{"version":"4","jsr":{"@std/path@1.0.8":{}},"npm":{"react@18.3.1":{}}}"#;
        assert_eq!(read("deno.lock", deno)["@std/path"], "1.0.8");
        let mix = "%{\n  \"jason\": {:hex, :jason, \"1.4.1\", \"hash\", [:mix], [], \"hexpm\", \"hash\"},\n  \"kit\": {:git, \"https://example.com/kit.git\", \"0123456789abcdef0123456789abcdef01234567\", []},\n}\n";
        assert_eq!(read("mix.lock", mix)["jason"], "1.4.1");
        assert_eq!(read("mix.lock", mix)["kit"], "0123456");
    }

    /// go.sum keeps the checksums both sides added and drops the ones a
    /// side removed, sorted as Go writes them.
    #[test]
    fn go_sum_lines_merge() {
        let base = "a v1 h1:a\nold v1 h1:o\n";
        let ours = "a v1 h1:a\nold v1 h1:o\nz v2 h1:z\n";
        let theirs = "a v1 h1:a\nb v1 h1:b\n";
        assert_eq!(
            merge_lines(base, ours, theirs),
            "a v1 h1:a\nb v1 h1:b\nz v2 h1:z\n"
        );
    }
}
