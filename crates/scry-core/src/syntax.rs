//! Whether a text parses, with tree-sitter grammars compiled into Scry.
//!
//! The same grammars will drive syntax highlighting (decision 0003).

use std::path::Path;

use tree_sitter::{Node, Parser};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Swift,
    ObjectiveC,
    C,
    Cpp,
    Rust,
    JavaScript,
    TypeScript,
    Tsx,
    Html,
    Css,
    Python,
    Kotlin,
    Java,
    Go,
    Ruby,
    Bash,
    Json,
    Yaml,
    Toml,
}

impl Language {
    /// The language of a file, from its name or extension; `None` when Scry
    /// has no grammar for it.
    pub fn from_path(path: &Path) -> Option<Language> {
        let name = path.file_name()?.to_str()?;
        let by_name = match name {
            "Podfile" | "Gemfile" | "Fastfile" | "Appfile" | "Matchfile" | "Rakefile"
            | "Brewfile" | "Dangerfile" => Some(Language::Ruby),
            "Cargo.lock" => Some(Language::Toml),
            "Package.resolved" => Some(Language::Json),
            _ => None,
        };
        if by_name.is_some() {
            return by_name;
        }
        let extension = path.extension()?.to_str()?.to_ascii_lowercase();
        match extension.as_str() {
            "swift" => Some(Language::Swift),
            // In Apple projects, headers are mostly Objective-C, which the
            // Objective-C grammar parses along with plain C.
            "m" | "h" => Some(Language::ObjectiveC),
            "c" => Some(Language::C),
            "cc" | "cpp" | "cxx" | "hh" | "hpp" | "hxx" => Some(Language::Cpp),
            "rs" => Some(Language::Rust),
            "js" | "mjs" | "cjs" | "jsx" => Some(Language::JavaScript),
            "ts" | "mts" | "cts" => Some(Language::TypeScript),
            "tsx" => Some(Language::Tsx),
            "html" | "htm" => Some(Language::Html),
            "css" => Some(Language::Css),
            "py" | "pyi" => Some(Language::Python),
            "kt" | "kts" => Some(Language::Kotlin),
            "java" => Some(Language::Java),
            "go" => Some(Language::Go),
            "rb" | "podspec" | "gemspec" => Some(Language::Ruby),
            "sh" | "bash" => Some(Language::Bash),
            // String catalogs are JSON.
            "json" | "xcstrings" => Some(Language::Json),
            "yml" | "yaml" => Some(Language::Yaml),
            "toml" => Some(Language::Toml),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Language::Swift => "Swift",
            Language::ObjectiveC => "Objective-C",
            Language::C => "C",
            Language::Cpp => "C++",
            Language::Rust => "Rust",
            Language::JavaScript => "JavaScript",
            Language::TypeScript => "TypeScript",
            Language::Tsx => "TSX",
            Language::Html => "HTML",
            Language::Css => "CSS",
            Language::Python => "Python",
            Language::Kotlin => "Kotlin",
            Language::Java => "Java",
            Language::Go => "Go",
            Language::Ruby => "Ruby",
            Language::Bash => "Bash",
            Language::Json => "JSON",
            Language::Yaml => "YAML",
            Language::Toml => "TOML",
        }
    }

    pub(crate) fn grammar(self) -> tree_sitter::Language {
        match self {
            Language::Swift => tree_sitter_swift::LANGUAGE.into(),
            Language::ObjectiveC => tree_sitter_objc::LANGUAGE.into(),
            Language::C => tree_sitter_c::LANGUAGE.into(),
            Language::Cpp => tree_sitter_cpp::LANGUAGE.into(),
            Language::Rust => tree_sitter_rust::LANGUAGE.into(),
            Language::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Language::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Language::Html => tree_sitter_html::LANGUAGE.into(),
            Language::Css => tree_sitter_css::LANGUAGE.into(),
            Language::Python => tree_sitter_python::LANGUAGE.into(),
            Language::Kotlin => tree_sitter_kotlin_ng::LANGUAGE.into(),
            Language::Java => tree_sitter_java::LANGUAGE.into(),
            Language::Go => tree_sitter_go::LANGUAGE.into(),
            Language::Ruby => tree_sitter_ruby::LANGUAGE.into(),
            Language::Bash => tree_sitter_bash::LANGUAGE.into(),
            Language::Json => tree_sitter_json::LANGUAGE.into(),
            Language::Yaml => tree_sitter_yaml::LANGUAGE.into(),
            Language::Toml => tree_sitter_toml_ng::LANGUAGE.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseCheck {
    /// No grammar for the file, or no reference version of the file that
    /// parses, so a failure could not be blamed on this text.
    NotChecked,
    Parses,
    /// `line` is the first line with an error, from 1.
    DoesNotParse {
        line: usize,
    },
}

/// Parses `text` as `language`.
pub fn check(language: Language, text: &str) -> ParseCheck {
    let mut parser = Parser::new();
    if parser.set_language(&language.grammar()).is_err() {
        return ParseCheck::NotChecked;
    }
    let Some(tree) = parser.parse(text, None) else {
        return ParseCheck::NotChecked;
    };
    match first_error(tree.root_node()) {
        None => ParseCheck::Parses,
        Some(node) => ParseCheck::DoesNotParse {
            line: error_row(node) + 1,
        },
    }
}

/// Where to point the user: a missing token where it should be; an error
/// region where the parser gave up, at its end (for an unclosed brace, the
/// region starts at the opening line and ends where the brace is missing).
fn error_row(node: Node<'_>) -> usize {
    if node.is_missing() {
        return node.start_position().row;
    }
    let end = node.end_position();
    if end.column == 0 && end.row > node.start_position().row {
        end.row - 1
    } else {
        end.row
    }
}

/// The first node, in text order, that is an error or a token the parser
/// had to invent (such as a missing closing brace).
fn first_error(node: Node<'_>) -> Option<Node<'_>> {
    if node.is_error() || node.is_missing() {
        return Some(node);
    }
    if !node.has_error() {
        return None;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(error) = first_error(child) {
            return Some(error);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The language comes from the file extension, and unknown files get none.
    #[test]
    fn language_from_extension() {
        assert_eq!(
            Language::from_path(Path::new("a/SyncEngine.swift")),
            Some(Language::Swift)
        );
        assert_eq!(
            Language::from_path(Path::new("main.rs")),
            Some(Language::Rust)
        );
        assert_eq!(Language::from_path(Path::new("README.md")), None);
        assert_eq!(Language::from_path(Path::new("Makefile")), None);
    }

    /// Apple and tooling files map to their grammar by name or extension (headers as
    /// Objective-C, Podfile as Ruby, `.xcstrings` as JSON), and `.pbxproj` to none.
    #[test]
    fn names_and_extensions() {
        let cases = [
            ("Sources/AppDelegate.m", Language::ObjectiveC),
            ("Bridging-Header.h", Language::ObjectiveC),
            ("engine.cpp", Language::Cpp),
            ("App.tsx", Language::Tsx),
            ("Localizable.xcstrings", Language::Json),
            ("ios/Podfile", Language::Ruby),
            ("fastlane/Fastfile", Language::Ruby),
            ("Cargo.lock", Language::Toml),
            ("ci.YML", Language::Yaml),
        ];
        for (path, language) in cases {
            assert_eq!(
                Language::from_path(Path::new(path)),
                Some(language),
                "{path}"
            );
        }
        assert_eq!(
            Language::from_path(Path::new("App.xcodeproj/project.pbxproj")),
            None
        );
    }

    /// One valid and one broken sample per grammar: a grammar that refused
    /// valid code would block the user's merge, the worst failure here.
    #[test]
    fn every_grammar_accepts_valid_code_and_rejects_broken_code() {
        let cases = [
            (
                Language::Swift,
                "struct A { let b: Int }\n",
                "struct A { let b: Int\n",
            ),
            (
                Language::ObjectiveC,
                "@interface A : NSObject\n- (void)run;\n@end\n",
                "@interface A : NSObject\n- (void)run\n",
            ),
            (
                Language::C,
                "int main(void) { return 0; }\n",
                "int main(void) { return 0;\n",
            ),
            (
                Language::Cpp,
                "class A { public: int b; };\n",
                "class A { public: int b;\n",
            ),
            (Language::Rust, "fn main() {}\n", "fn main() {\n"),
            (
                Language::JavaScript,
                "const a = () => <div>{b}</div>;\n",
                "const a = (;\n",
            ),
            (
                Language::TypeScript,
                "const a: number = 1;\n",
                "const a: = 1;\n",
            ),
            (
                Language::Tsx,
                "const a = <A b={1} />;\n",
                "const a = <A b={1} ;\n",
            ),
            (Language::Html, "<p>a</p>\n", "<p a=\"b>a</p>\n"),
            (Language::Css, "a { color: red; }\n", "a { color: red;\n"),
            (
                Language::Python,
                "def a():\n    return 1\n",
                "def a(:\n    return 1\n",
            ),
            (Language::Kotlin, "fun a(): Int = 1\n", "fun a(: Int = 1\n"),
            (Language::Java, "class A { int b; }\n", "class A { int b;\n"),
            (
                Language::Go,
                "package a\n\nfunc b() {}\n",
                "package a\n\nfunc b() {\n",
            ),
            (Language::Ruby, "def a\n  1\nend\n", "def a\n  1\n"),
            (
                Language::Bash,
                "if true; then echo a; fi\n",
                "if true; then echo a\n",
            ),
            (Language::Json, "{\"a\": [1, 2]}\n", "{\"a\": [1, 2}\n"),
            (Language::Yaml, "a:\n  - b\n", "a: [b\n"),
            (Language::Toml, "[a]\nb = 1\n", "[a\nb = 1\n"),
        ];
        for (language, valid, broken) in cases {
            assert_eq!(
                check(language, valid),
                ParseCheck::Parses,
                "{} valid",
                language.name()
            );
            assert!(
                matches!(check(language, broken), ParseCheck::DoesNotParse { .. }),
                "{} broken",
                language.name()
            );
        }
    }

    /// The demo's Swift parses, so checks on the demo are meaningful.
    #[test]
    fn swift_from_the_demo_parses() {
        let text = "func upload(_ item: Item) async throws {\n    try await retry(times: 3, backoff: .exponential) {\n        try await client.send(item, timeout: .seconds(30))\n    }\n}\n";
        assert_eq!(check(Language::Swift, text), ParseCheck::Parses);
    }

    /// A missing closing brace is reported, with the line where it is missing.
    #[test]
    fn missing_brace_is_reported_with_a_line() {
        let text =
            "func cancel() {\n    queue.cancelAll()\nfunc pause() {\n    queue.suspend()\n}\n";
        assert!(matches!(
            check(Language::Swift, text),
            ParseCheck::DoesNotParse { .. }
        ));
        assert_eq!(
            check(Language::Rust, "fn main() {\n    let x = 1;\n"),
            ParseCheck::DoesNotParse { line: 2 }
        );
    }
}
