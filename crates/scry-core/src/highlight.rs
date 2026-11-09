//! Syntax highlighting: the colour role of each part of a line.
//!
//! Computed here, with the grammars that check candidates, so every UI
//! colours code the same way. The core names roles; each UI maps them to
//! colours (portability rule 5).

use tree_sitter::{Parser, Query, QueryCursor, StreamingIterator};

use crate::syntax::Language;

/// The syntax roles of the design tokens (`syntax` group). Text without a
/// span is plain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyntaxRole {
    Keyword,
    String,
    Number,
    Comment,
    Type,
    Function,
    Variable,
    Constant,
    Operator,
    Punctuation,
    /// Attributes, directives and macros: `@MainActor`, `#if`.
    Attribute,
}

/// A run of one role inside a line, in UTF-8 bytes from the line's start,
/// end excluded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub role: SyntaxRole,
}

/// A compiled highlight query for one language. Compiling takes a few
/// milliseconds, so a session keeps one.
pub struct Highlighter {
    language: Language,
    query: Query,
    /// The role of each capture name of the query, by capture index.
    roles: Vec<Option<SyntaxRole>>,
    /// The nesting depth of each pattern, by pattern index.
    depths: Vec<usize>,
}

impl Highlighter {
    /// `None` when Scry has no highlight query for the language (Kotlin).
    pub fn new(language: Language) -> Option<Highlighter> {
        let source = query_source(language)?;
        // The queries ship with the grammars, so a failure is a bug in Scry
        // or in a grammar update, which the tests catch.
        let query = Query::new(&language.grammar(), &source).ok()?;
        let roles = query
            .capture_names()
            .iter()
            .map(|name| role(name))
            .collect();
        let depths = (0..query.pattern_count())
            .map(|index| {
                depth(
                    &source[query.start_byte_for_pattern(index)..query.end_byte_for_pattern(index)],
                )
            })
            .collect();
        Some(Highlighter {
            language,
            query,
            roles,
            depths,
        })
    }

    /// The spans of every line of `text`, one list per line as
    /// `str::lines` splits them. Plain text has no span.
    pub fn lines(&self, text: &str) -> Vec<Vec<Span>> {
        let painted = self.paint(text);
        let mut lines = Vec::new();
        let mut start = 0;
        for line in text.lines() {
            let end = start + line.len();
            lines.push(runs(&painted[start..end]));
            // Skip the line's own text and its "\n" (or "\r\n", which
            // `lines` drops too).
            start = end;
            if text[start..].starts_with("\r\n") {
                start += 2;
            } else if text[start..].starts_with('\n') {
                start += 1;
            }
        }
        lines
    }

    /// The role of every byte of `text`.
    fn paint(&self, text: &str) -> Vec<Option<SyntaxRole>> {
        let mut painted = vec![None; text.len()];
        let mut parser = Parser::new();
        if parser.set_language(&self.language.grammar()).is_err() {
            return painted;
        }
        let Some(tree) = parser.parse(text, None) else {
            return painted;
        };

        // Collect every capture first: which one wins depends on all of
        // them, not on the order the cursor finds them.
        let mut captures = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.query, tree.root_node(), text.as_bytes());
        while let Some(found) = matches.next() {
            for capture in found.captures() {
                let Some(mut role) = self.roles[capture.index as usize] else {
                    continue;
                };
                // The C queries call preprocessor directives keywords; the
                // design shows `#if` and `#import` as attributes, as in Swift.
                if role == SyntaxRole::Keyword && capture.node.kind().starts_with('#') {
                    role = SyntaxRole::Attribute;
                }
                let range = capture.node.byte_range();
                captures.push(Capture {
                    start: range.start,
                    end: range.end,
                    pattern: found.pattern_index,
                    specificity: self.depths[found.pattern_index],
                    role,
                });
            }
        }

        // Paint the winners last. Grammars disagree on query order (Go lists
        // specific patterns first, JavaScript and Swift last), so order alone
        // cannot decide. Instead: inner nodes over the nodes around them (an
        // escape inside a string), then, on the same node, the more nested
        // pattern, which is the more specific one (`(call_expression
        // function: (identifier) @function)` over `(identifier) @variable`),
        // then the later pattern, as in Neovim.
        captures.sort_by_key(|capture| {
            (
                std::cmp::Reverse(capture.end - capture.start),
                capture.specificity,
                capture.pattern,
            )
        });
        for capture in captures {
            for role in &mut painted[capture.start..capture.end] {
                *role = Some(capture.role);
            }
        }
        painted
    }
}

/// How deeply a query pattern nests parentheses: 1 for `(identifier)`, 2
/// for `(call_expression function: (identifier))` or a pattern with a
/// predicate. Alternatives in brackets add nothing, unlike the pattern's
/// length, which a long list of alternatives inflates.
fn depth(pattern: &str) -> usize {
    let mut depth: usize = 0;
    let mut deepest = 0;
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        match c {
            '(' => {
                depth += 1;
                deepest = deepest.max(depth);
            }
            ')' => depth = depth.saturating_sub(1),
            // Skip string literals such as "(" and their escapes.
            '"' => {
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => {
                            chars.next();
                        }
                        '"' => break,
                        _ => {}
                    }
                }
            }
            // Skip comments to the end of the line.
            ';' => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    deepest
}

struct Capture {
    start: usize,
    end: usize,
    pattern: usize,
    specificity: usize,
    role: SyntaxRole,
}

/// Groups equal neighbours into spans; plain bytes give none.
fn runs(painted: &[Option<SyntaxRole>]) -> Vec<Span> {
    let mut spans: Vec<Span> = Vec::new();
    for (index, role) in painted.iter().enumerate() {
        let Some(role) = *role else { continue };
        match spans.last_mut() {
            Some(last) if last.end == index && last.role == role => last.end = index + 1,
            _ => spans.push(Span {
                start: index,
                end: index + 1,
                role,
            }),
        }
    }
    spans
}

/// The highlight query of a language. Some extend another language's query,
/// which comes first so the extension wins ties.
fn query_source(language: Language) -> Option<String> {
    let parts: Vec<&str> = match language {
        Language::Swift => vec![tree_sitter_swift::HIGHLIGHTS_QUERY],
        Language::ObjectiveC => vec![
            tree_sitter_c::HIGHLIGHT_QUERY,
            tree_sitter_objc::HIGHLIGHTS_QUERY,
        ],
        Language::C => vec![tree_sitter_c::HIGHLIGHT_QUERY],
        Language::Cpp => vec![
            tree_sitter_c::HIGHLIGHT_QUERY,
            tree_sitter_cpp::HIGHLIGHT_QUERY,
        ],
        Language::Rust => vec![tree_sitter_rust::HIGHLIGHTS_QUERY],
        Language::JavaScript => vec![
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
        ],
        Language::TypeScript => vec![
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_typescript::HIGHLIGHTS_QUERY,
        ],
        Language::Tsx => vec![
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
            tree_sitter_typescript::HIGHLIGHTS_QUERY,
        ],
        Language::Html => vec![tree_sitter_html::HIGHLIGHTS_QUERY],
        Language::Css => vec![tree_sitter_css::HIGHLIGHTS_QUERY],
        Language::Python => vec![tree_sitter_python::HIGHLIGHTS_QUERY],
        // tree-sitter-kotlin-ng ships no highlight query.
        Language::Kotlin => return None,
        Language::Java => vec![tree_sitter_java::HIGHLIGHTS_QUERY],
        Language::Go => vec![tree_sitter_go::HIGHLIGHTS_QUERY],
        Language::Ruby => vec![tree_sitter_ruby::HIGHLIGHTS_QUERY],
        Language::Bash => vec![tree_sitter_bash::HIGHLIGHT_QUERY],
        Language::Json => vec![tree_sitter_json::HIGHLIGHTS_QUERY],
        Language::Yaml => vec![tree_sitter_yaml::HIGHLIGHTS_QUERY],
        Language::Toml => vec![tree_sitter_toml_ng::HIGHLIGHTS_QUERY],
    };
    Some(parts.join("\n"))
}

/// The role of a capture name. The grammars use both the tree-sitter names
/// (`@function.method`) and older Neovim ones (`@conditional`,
/// `@storageclass`); names without a colour (`@spell`, `@embedded`, `@_x`)
/// give none.
fn role(capture: &str) -> Option<SyntaxRole> {
    let mut parts = capture.split('.');
    let first = parts.next().unwrap_or("");
    let second = parts.next();
    let role = match (first, second) {
        // `self`, `super`, `nil`, `true`: Xcode shows them as keywords.
        ("variable" | "constant", Some("builtin")) | ("boolean", _) => SyntaxRole::Keyword,
        ("keyword", Some("directive")) | ("function", Some("macro")) => SyntaxRole::Attribute,
        ("attribute" | "preproc" | "define", _) => SyntaxRole::Attribute,
        ("type", Some("qualifier")) => SyntaxRole::Keyword,
        ("keyword" | "conditional" | "repeat" | "include" | "exception" | "storageclass", _) => {
            SyntaxRole::Keyword
        }
        ("string" | "character" | "escape", _) => SyntaxRole::String,
        ("number" | "float", _) => SyntaxRole::Number,
        ("comment", _) => SyntaxRole::Comment,
        ("type" | "constructor" | "namespace" | "module" | "tag", _) => SyntaxRole::Type,
        ("function" | "method", _) => SyntaxRole::Function,
        ("variable" | "parameter" | "property" | "field", _) => SyntaxRole::Variable,
        ("constant", _) => SyntaxRole::Constant,
        ("operator", _) => SyntaxRole::Operator,
        ("punctuation", _) => SyntaxRole::Punctuation,
        _ => return None,
    };
    Some(role)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Language; 19] = [
        Language::Swift,
        Language::ObjectiveC,
        Language::C,
        Language::Cpp,
        Language::Rust,
        Language::JavaScript,
        Language::TypeScript,
        Language::Tsx,
        Language::Html,
        Language::Css,
        Language::Python,
        Language::Kotlin,
        Language::Java,
        Language::Go,
        Language::Ruby,
        Language::Bash,
        Language::Json,
        Language::Yaml,
        Language::Toml,
    ];

    /// The text of each span of a line, with its role.
    fn words(text: &str, language: Language) -> Vec<Vec<(String, SyntaxRole)>> {
        let highlighter = Highlighter::new(language).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        highlighter
            .lines(text)
            .iter()
            .zip(lines)
            .map(|(spans, line)| {
                spans
                    .iter()
                    .map(|span| (line[span.start..span.end].to_string(), span.role))
                    .collect()
            })
            .collect()
    }

    fn role_of(text: &str, language: Language, word: &str) -> Option<SyntaxRole> {
        words(text, language)
            .into_iter()
            .flatten()
            .find(|(text, _)| text == word)
            .map(|(_, role)| role)
    }

    #[test]
    fn every_query_compiles_except_kotlin() {
        for language in ALL {
            assert_eq!(
                Highlighter::new(language).is_some(),
                language != Language::Kotlin,
                "{}",
                language.name()
            );
        }
    }

    #[test]
    fn swift_from_the_demo() {
        let text = "@MainActor\nfunc upload(_ item: Item) async throws {\n    // Retry\n    try await client.send(item, timeout: .seconds(30), label: \"a\\n\")\n}\n";
        let swift = Language::Swift;
        assert_eq!(role_of(text, swift, "func"), Some(SyntaxRole::Keyword));
        assert_eq!(role_of(text, swift, "upload"), Some(SyntaxRole::Function));
        assert_eq!(role_of(text, swift, "Item"), Some(SyntaxRole::Type));
        assert_eq!(role_of(text, swift, "// Retry"), Some(SyntaxRole::Comment));
        assert_eq!(role_of(text, swift, "30"), Some(SyntaxRole::Number));
        assert_eq!(role_of(text, swift, "send"), Some(SyntaxRole::Function));
        assert_eq!(
            role_of(text, swift, "@MainActor"),
            Some(SyntaxRole::Attribute)
        );
        // The escape inside the string keeps the string role.
        assert_eq!(role_of(text, swift, "\"a\\n\""), Some(SyntaxRole::String));
    }

    #[test]
    fn specific_pattern_wins_whatever_the_query_order() {
        // JavaScript lists `(identifier) @variable` first, Go last.
        let js = "function load() { return fetch(url); }\n";
        assert_eq!(
            role_of(js, Language::JavaScript, "fetch"),
            Some(SyntaxRole::Function)
        );
        assert_eq!(
            role_of(js, Language::JavaScript, "url"),
            Some(SyntaxRole::Variable)
        );
        let go = "package a\n\nfunc b() { c(d) }\n";
        assert_eq!(role_of(go, Language::Go, "c"), Some(SyntaxRole::Function));
        assert_eq!(role_of(go, Language::Go, "d"), Some(SyntaxRole::Variable));
    }

    #[test]
    fn depth_ignores_alternatives_strings_and_comments() {
        assert_eq!(depth("(identifier) @variable"), 1);
        assert_eq!(
            depth("(call_expression function: (identifier) @function)"),
            2
        );
        assert_eq!(depth("[\"(\" \")\" (a) (b) (c)] @x ; (((comment)))"), 1);
    }

    #[test]
    fn yaml_keys_are_not_strings() {
        // The scalar list is longer than the key pattern but less nested.
        let yaml = "jobs:\n  test: macos\n";
        assert_eq!(
            role_of(yaml, Language::Yaml, "jobs"),
            Some(SyntaxRole::Variable)
        );
        assert_eq!(
            role_of(yaml, Language::Yaml, "macos"),
            Some(SyntaxRole::String)
        );
    }

    #[test]
    fn extended_queries_keep_their_base() {
        let ts = "const a: number = load();\n";
        assert_eq!(
            role_of(ts, Language::TypeScript, "number"),
            Some(SyntaxRole::Type)
        );
        assert_eq!(
            role_of(ts, Language::TypeScript, "const"),
            Some(SyntaxRole::Keyword)
        );
        let objc =
            "#import <Foundation/Foundation.h>\n@interface A : NSObject\n- (void)run;\n@end\n";
        assert_eq!(
            role_of(objc, Language::ObjectiveC, "#import"),
            Some(SyntaxRole::Attribute)
        );
        assert_eq!(
            role_of(objc, Language::ObjectiveC, "void"),
            Some(SyntaxRole::Type)
        );
    }

    #[test]
    fn lines_follow_str_lines_with_crlf_and_utf8() {
        let text = "let é = \"à\"\r\nlet b = 1\r\n";
        let highlighter = Highlighter::new(Language::Swift).unwrap();
        let lines = highlighter.lines(text);
        assert_eq!(lines.len(), 2);
        let second = "let b = 1";
        let number = lines[1]
            .iter()
            .find(|span| span.role == SyntaxRole::Number)
            .unwrap();
        assert_eq!(&second[number.start..number.end], "1");
        // Offsets are UTF-8 bytes, so slicing the line works.
        let first = "let é = \"à\"";
        for span in &lines[0] {
            assert!(first.get(span.start..span.end).is_some());
        }
    }

    #[test]
    fn spans_are_sorted_and_do_not_overlap() {
        let text = "fn main() {\n    let x = vec![1, 2]; // done\n}\n";
        let highlighter = Highlighter::new(Language::Rust).unwrap();
        for spans in highlighter.lines(text) {
            for pair in spans.windows(2) {
                assert!(pair[0].end <= pair[1].start);
            }
        }
    }
}
