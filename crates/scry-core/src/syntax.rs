//! Whether a text parses, and its highlight spans, with the tree-sitter
//! grammars compiled into Scry (decision 0003).

use serde::Serialize;
use std::ops::Range;
use std::path::Path;
use std::sync::OnceLock;

use tree_sitter::{InputEdit, Node, Parser, Point, Tree};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
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
    match ParsedFile::new(language, text.to_string()) {
        Some(parsed) => parsed.check(),
        None => ParseCheck::NotChecked,
    }
}

/// A text and its syntax tree. Variants of it, with some byte ranges
/// replaced, are parsed incrementally: tree-sitter reuses the tree and
/// reparses only around the replaced text, so checking each candidate of a
/// large file costs little more than the candidate itself.
#[derive(Clone)]
pub(crate) struct ParsedFile {
    language: Language,
    text: String,
    tree: Tree,
    /// Where each line starts, to find an edit's row without scanning the
    /// text before it; computed the first time this file is edited, since
    /// most variants are only checked.
    line_starts: OnceLock<Vec<usize>>,
}

impl ParsedFile {
    pub(crate) fn new(language: Language, text: String) -> Option<ParsedFile> {
        let tree = parser(language)?.parse(&text, None)?;
        Some(ParsedFile::from_parts(language, text, tree))
    }

    fn from_parts(language: Language, text: String, tree: Tree) -> ParsedFile {
        ParsedFile {
            language,
            text,
            tree,
            line_starts: OnceLock::new(),
        }
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    pub(crate) fn tree(&self) -> &Tree {
        &self.tree
    }

    pub(crate) fn check(&self) -> ParseCheck {
        match first_error(self.tree.root_node()) {
            None => ParseCheck::Parses,
            Some(node) => ParseCheck::DoesNotParse {
                line: error_row(node) + 1,
            },
        }
    }

    /// This file with each range replaced by its text. The ranges are byte
    /// ranges of this file, in order and not overlapping.
    pub(crate) fn with(&self, replacements: &[(Range<usize>, &str)]) -> Option<ParsedFile> {
        self.with_changes(replacements).map(|(parsed, _)| parsed)
    }

    /// `with`, and the byte ranges of the new file whose syntax may differ
    /// from this one's: the replaced text, and what tree-sitter reports as
    /// changed around it (a string left open colours the lines after it).
    pub(crate) fn with_changes(
        &self,
        replacements: &[(Range<usize>, &str)],
    ) -> Option<(ParsedFile, Vec<Range<usize>>)> {
        let mut text = String::with_capacity(self.text.len());
        let mut copied = 0;
        for (range, replacement) in replacements {
            text.push_str(&self.text[copied..range.start]);
            text.push_str(replacement);
            copied = range.end;
        }
        text.push_str(&self.text[copied..]);

        // Edits are described from the last to the first, so the text
        // before each one is still this file's.
        let mut tree = self.tree.clone();
        for (range, replacement) in replacements.iter().rev() {
            let start = self.point(range.start);
            tree.edit(&InputEdit {
                start_byte: range.start,
                old_end_byte: range.end,
                new_end_byte: range.start + replacement.len(),
                start_position: start,
                old_end_position: advance(start, &self.text[range.clone()]),
                new_end_position: advance(start, replacement),
            });
        }
        let new_tree = parser(self.language)?.parse(&text, Some(&tree))?;
        let mut changed: Vec<Range<usize>> = tree
            .changed_ranges(&new_tree)
            .map(|range| range.start_byte..range.end_byte)
            .collect();
        let mut shift: isize = 0;
        for (range, replacement) in replacements {
            let start = (range.start as isize + shift) as usize;
            changed.push(start..start + replacement.len());
            shift += replacement.len() as isize - range.len() as isize;
        }
        Some((
            ParsedFile::from_parts(self.language, text, new_tree),
            changed,
        ))
    }

    /// The row of the line holding byte `offset`.
    pub(crate) fn row(&self, offset: usize) -> usize {
        self.point(offset).row
    }

    /// The bytes of lines `rows`, line breaks included.
    pub(crate) fn line_bytes(&self, rows: Range<usize>) -> Range<usize> {
        let start = self.point_of_row(rows.start);
        let end = self.point_of_row(rows.end);
        start..end
    }

    /// Where line `row` starts; the text's end past the last line.
    fn point_of_row(&self, row: usize) -> usize {
        self.point(0);
        let line_starts = self
            .line_starts
            .get()
            .map(Vec::as_slice)
            .unwrap_or_default();
        line_starts.get(row).copied().unwrap_or(self.text.len())
    }

    /// Whether this file parses with each range replaced by its text, as
    /// `with(replacements).check()` says, without building that file.
    ///
    /// Once every way the parser was trying is in error, the file cannot
    /// parse, so it stops there: past an error tree-sitter can rarely reuse
    /// the old tree, and parsing on to the end of a large file for each
    /// candidate made opening a file quadratic in its conflicts. The line
    /// reported is then where the parser gave up, which can be dozens of
    /// lines past the error: right for the verdict, not for pointing the
    /// user at it (`locate_with` does).
    pub(crate) fn check_with(&self, replacements: &[(Range<usize>, &str)]) -> ParseCheck {
        self.check_with_stop(replacements).0
    }

    /// Like `check_with`, but the line of an error is always the full
    /// check's: when the quick check stopped early, the edited file is
    /// parsed to its end. For what the user reads, one text at a time.
    pub(crate) fn locate_with(&self, replacements: &[(Range<usize>, &str)]) -> ParseCheck {
        match self.check_with_stop(replacements) {
            (check, true) => self
                .with(replacements)
                .map_or(check, |edited| edited.check()),
            (check, false) => check,
        }
    }

    /// `check_with`, and whether it stopped before the end of the file.
    fn check_with_stop(&self, replacements: &[(Range<usize>, &str)]) -> (ParseCheck, bool) {
        let mut tree = self.tree.clone();
        let pieces = landings(replacements);
        for (range, replacement) in replacements.iter().rev() {
            let start = self.point(range.start);
            tree.edit(&InputEdit {
                start_byte: range.start,
                old_end_byte: range.end,
                new_end_byte: range.start + replacement.len(),
                start_position: start,
                old_end_position: advance(start, &self.text[range.clone()]),
                new_end_position: advance(start, replacement),
            });
        }
        let old = self.text.as_bytes();
        // The new text from `offset` on, up to the next boundary between
        // this file's text and a replacement.
        let mut read = |offset: usize, _: Point| -> &[u8] {
            let mut shift: isize = 0;
            for (index, (start, old_start, bytes)) in pieces.iter().enumerate() {
                if offset < *start {
                    let from = (offset as isize - shift) as usize;
                    return &old[from..*old_start];
                }
                if offset < start + bytes.len() {
                    return &bytes[offset - start..];
                }
                shift += bytes.len() as isize - replacements[index].0.len() as isize;
            }
            let from = (offset as isize - shift) as usize;
            &old[from.min(old.len())..]
        };
        let mut failed_at: Option<usize> = None;
        let mut stop = |state: &tree_sitter::ParseState| {
            if state.has_error() {
                failed_at = Some(state.current_byte_offset());
                std::ops::ControlFlow::Break(())
            } else {
                std::ops::ControlFlow::Continue(())
            }
        };
        let options = tree_sitter::ParseOptions::new().progress_callback(&mut stop);
        let Some(mut parser) = parser(self.language) else {
            return (ParseCheck::NotChecked, false);
        };
        let parsed = parser.parse_with_options(&mut read, Some(&tree), Some(options));
        match (parsed, failed_at) {
            (Some(tree), _) => {
                let check = match first_error(tree.root_node()) {
                    None => ParseCheck::Parses,
                    Some(node) => ParseCheck::DoesNotParse {
                        line: error_row(node) + 1,
                    },
                };
                (check, false)
            }
            (None, Some(offset)) => (
                ParseCheck::DoesNotParse {
                    line: row_in(&pieces, replacements, &self.text, offset) + 1,
                },
                true,
            ),
            (None, None) => (ParseCheck::NotChecked, false),
        }
    }

    /// The row and byte column of `offset`.
    fn point(&self, offset: usize) -> Point {
        let line_starts = self.line_starts.get_or_init(|| {
            std::iter::once(0)
                .chain(self.text.match_indices('\n').map(|(index, _)| index + 1))
                .collect()
        });
        // The last line starting at or before `offset`.
        let row = line_starts.partition_point(|&start| start <= offset) - 1;
        Point {
            row,
            column: offset - line_starts[row],
        }
    }
}

fn parser(language: Language) -> Option<Parser> {
    let mut parser = Parser::new();
    parser.set_language(&language.grammar()).ok()?;
    Some(parser)
}

/// Where each replacement lands in the new text: its start there, its start
/// in the old text, and its bytes.
fn landings<'a>(replacements: &[(Range<usize>, &'a str)]) -> Vec<(usize, usize, &'a [u8])> {
    let mut shift: isize = 0;
    let mut pieces = Vec::new();
    for (range, replacement) in replacements {
        let start = (range.start as isize + shift) as usize;
        pieces.push((start, range.start, replacement.as_bytes()));
        shift += replacement.len() as isize - range.len() as isize;
    }
    pieces
}

/// The row of `offset` in `text` with `replacements` made, `pieces` giving
/// where each lands (`landings`).
fn row_in(
    pieces: &[(usize, usize, &[u8])],
    replacements: &[(Range<usize>, &str)],
    text: &str,
    offset: usize,
) -> usize {
    let mut row = 0;
    let mut copied = 0;
    let mut position = 0;
    for ((start, old_start, bytes), (range, _)) in pieces.iter().zip(replacements) {
        let before = &text[copied..*old_start];
        if offset < *start {
            return row + count_lines(&before.as_bytes()[..offset - position]);
        }
        row += count_lines(before.as_bytes());
        if offset < start + bytes.len() {
            return row + count_lines(&bytes[..offset - start]);
        }
        row += count_lines(bytes);
        copied = range.end;
        position = start + bytes.len();
    }
    let rest = &text.as_bytes()[copied..];
    row + count_lines(&rest[..(offset - position).min(rest.len())])
}

fn count_lines(bytes: &[u8]) -> usize {
    bytes.iter().filter(|byte| **byte == b'\n').count()
}

/// Where `start` ends up after `text`.
fn advance(start: Point, text: &str) -> Point {
    match text.rfind('\n') {
        Some(last) => Point {
            row: start.row + text.matches('\n').count(),
            column: text.len() - last - 1,
        },
        None => Point {
            row: start.row,
            column: start.column + text.len(),
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
///
/// Walks the tree with a cursor rather than by recursion: a deeply nested
/// file (JSON arrays thousands deep) would overflow the stack.
fn first_error(node: Node<'_>) -> Option<Node<'_>> {
    let mut cursor = node.walk();
    loop {
        let current = cursor.node();
        if current.is_error() || current.is_missing() {
            return Some(current);
        }
        // Only a subtree with an error is worth going into.
        if current.has_error() && cursor.goto_first_child() {
            continue;
        }
        while !cursor.goto_next_sibling() {
            if !cursor.goto_parent() {
                return None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deeply nested file is checked without overflowing the stack, even
    /// on a thread with a small one: a recursive walk would crash the app.
    #[test]
    fn deeply_nested_files_are_checked() {
        let depth = 100_000;
        // One closing bracket short.
        let text = format!("{}1{}", "[".repeat(depth), "]".repeat(depth - 1));
        let check = std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(move || check(Language::Json, &text))
            .unwrap()
            .join()
            .unwrap();
        assert!(matches!(check, ParseCheck::DoesNotParse { .. }));
    }

    /// A file reparsed incrementally after edits must give the same tree,
    /// and so the same verdict, as parsing the edited text from scratch:
    /// every candidate check and the result check rely on it.
    #[test]
    fn incremental_parse_matches_a_fresh_parse() {
        // Functions after the edits, which tree-sitter reuses as they are:
        // an edit described wrongly would shift them.
        let functions: String = (0..100)
            .map(|index| format!("func f{index}() {{\n    g({index})\n}}\n"))
            .collect();
        let text = format!(
            "struct A {{\n    let a = 1\n}}\n\nfunc b() {{\n    print(\"b\")\n}}\n\nlet c = [1, 2]\n{functions}"
        );
        let base = ParsedFile::new(Language::Swift, text.clone()).unwrap();
        let a = text.find("let a = 1").unwrap();
        let print = text.find("print").unwrap();
        let c = text.find("[1, 2]").unwrap();
        let cases: [Vec<(Range<usize>, &str)>; 4] = [
            // One line changed, still valid.
            vec![(a..a + 9, "let a = 2")],
            // An unclosed brace, invalid.
            vec![(print..print + 10, "if x {")],
            // Lines added and removed in several places, with non-ASCII text.
            vec![
                (a..a + 9, "let é = \"…\"\n    let b = 2"),
                (print..print + 10, ""),
                (c..c + 6, "[\n  1,\n  2,\n]"),
            ],
            // Nothing replaced.
            vec![],
        ];
        for replacements in cases {
            let edited = base.with(&replacements).unwrap();
            let fresh = ParsedFile::new(Language::Swift, edited.text().to_string()).unwrap();
            assert_eq!(nodes(&edited), nodes(&fresh), "{:?}", edited.text());
            assert_eq!(edited.check(), fresh.check());
        }
    }

    /// `check_with` stops at the first sure error instead of parsing on: it
    /// must give the same verdict as checking the edited file, whatever the
    /// edits, with the error line at most one line from the full check's.
    #[test]
    fn check_with_gives_the_full_checks_verdict() {
        let functions: String = (0..200)
            .map(|index| format!("func f{index}() {{\n    g({index})\n}}\n"))
            .collect();
        let text = format!("let a = 1\nfunc b() {{\n    print(\"b\")\n}}\n{functions}");
        let base = ParsedFile::new(Language::Swift, text.clone()).unwrap();
        let a = text.find("let a = 1").unwrap();
        let print = text.find("print").unwrap();
        let far = text.find("g(150)").unwrap();
        let cases: Vec<Vec<(Range<usize>, &str)>> = vec![
            vec![(a..a + 9, "let a = 2")],
            vec![(print..print + 10, "if x {")],
            vec![(print..print + 10, "let = = 3")],
            vec![
                (a..a + 9, "let é = \"…\"\nlet b = 2"),
                (far..far + 6, "g(150))"),
            ],
            vec![(far..far + 6, "\"open")],
            vec![(a..a + 9, ""), (print..print + 10, "}\n}\n")],
            // Several replacements of other lengths that parse: the text
            // after each one is read shifted by the ones before.
            vec![
                (a..a + 9, "let a = 22222"),
                (print..print + 10, "print(1)"),
                (far..far + 6, "g(1)"),
            ],
            vec![(a..a + 9, ""), (far..far + 6, "g(150, 151)")],
            vec![],
        ];
        for replacements in cases {
            let full = base.with(&replacements).unwrap().check();
            let quick = base.check_with(&replacements);
            match (full, quick) {
                (
                    ParseCheck::DoesNotParse { line: full },
                    ParseCheck::DoesNotParse { line: quick },
                ) => {
                    assert!(
                        full.abs_diff(quick) <= 1,
                        "{replacements:?}: line {full} or {quick}"
                    );
                }
                (full, quick) => assert_eq!(full, quick, "{replacements:?}"),
            }
        }
    }

    /// Every node of a file with its kind and where it starts and ends, in
    /// bytes and in rows and columns: an edit described wrongly shifts
    /// the nodes tree-sitter reuses.
    fn nodes(parsed: &ParsedFile) -> Vec<(String, Range<usize>, Point, Point)> {
        let mut nodes = Vec::new();
        let mut pending = vec![parsed.tree().root_node()];
        while let Some(node) = pending.pop() {
            nodes.push((
                node.kind().to_string(),
                node.byte_range(),
                node.start_position(),
                node.end_position(),
            ));
            let mut cursor = node.walk();
            pending.extend(node.children(&mut cursor));
        }
        nodes
    }

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
            ("Package.resolved", Language::Json),
            ("hash.c", Language::C),
            ("app.js", Language::JavaScript),
            ("worker.mjs", Language::JavaScript),
            ("config.cjs", Language::JavaScript),
            ("View.jsx", Language::JavaScript),
            ("api.ts", Language::TypeScript),
            ("api.mts", Language::TypeScript),
            ("api.cts", Language::TypeScript),
            ("index.html", Language::Html),
            ("index.htm", Language::Html),
            ("site.css", Language::Css),
            ("tool.py", Language::Python),
            ("stubs.pyi", Language::Python),
            ("Main.kt", Language::Kotlin),
            ("build.gradle.kts", Language::Kotlin),
            ("Main.java", Language::Java),
            ("main.go", Language::Go),
            ("lib.rb", Language::Ruby),
            ("Alamofire.podspec", Language::Ruby),
            ("tool.gemspec", Language::Ruby),
            ("build.sh", Language::Bash),
            ("setup.bash", Language::Bash),
            ("pyproject.toml", Language::Toml),
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

    /// The line `check_with` reports when it stops early is the line of that
    /// byte in the edited text: compared with counting the lines of the
    /// edited text itself, at every offset, for replacements that add,
    /// remove and keep lines.
    #[test]
    fn rows_of_the_edited_text_are_counted_across_replacements() {
        let text = "a\nbb\nccc\ndddd\neeeee\n";
        let cases: Vec<Vec<(Range<usize>, &str)>> = vec![
            vec![],
            vec![(2..4, "x\ny\nz")],
            vec![(0..2, ""), (5..9, "q")],
            vec![(2..5, "1\n2\n"), (9..14, ""), (14..20, "end\n")],
            // Several lines between two replacements.
            vec![(0..1, "A"), (14..19, "E")],
        ];
        for replacements in cases {
            let mut edited = String::new();
            let mut copied = 0;
            for (range, replacement) in &replacements {
                edited.push_str(&text[copied..range.start]);
                edited.push_str(replacement);
                copied = range.end;
            }
            edited.push_str(&text[copied..]);
            let pieces = landings(&replacements);
            for offset in 0..=edited.len() {
                assert_eq!(
                    row_in(&pieces, &replacements, text, offset),
                    edited[..offset].matches('\n').count(),
                    "{replacements:?} at {offset} in {edited:?}"
                );
            }
        }
    }

    /// `with_changes` names every replaced range in the new file, even
    /// where tree-sitter sees the same structure (a renamed call), so the
    /// result pane repaints it; ranges after a replacement of another
    /// length are shifted by it.
    #[test]
    fn changed_ranges_include_every_replacement() {
        let text = "let a = 1\nfunc b() {\n    g(1)\n}\nfunc c() {\n    g(2)\n}\n";
        let base = ParsedFile::new(Language::Swift, text.to_string()).unwrap();
        let a = text.find("let a = 1").unwrap();
        let g = text.find("g(2)").unwrap();
        let replacements = [(a..a + 9, "let a = 100000"), (g..g + 4, "h(3)")];
        let (edited, changed) = base.with_changes(&replacements).unwrap();
        for (start, replacement) in [(a, "let a = 100000"), (g + 5, "h(3)")] {
            let range = start..start + replacement.len();
            assert_eq!(&edited.text()[range.clone()], replacement);
            assert!(
                changed
                    .iter()
                    .any(|found| found.start <= range.start && range.end <= found.end),
                "{range:?} not in {changed:?}"
            );
        }
    }

    /// The line reported for a file that does not parse is where the user
    /// must look: a missing token where it is missing, an error on one line
    /// at that line, and an unclosed block at its last line, not the empty
    /// line after the end of the file.
    #[test]
    fn errors_are_reported_on_the_line_to_fix() {
        assert_eq!(
            check(Language::C, "int a(\n\nint b;\n"),
            ParseCheck::DoesNotParse { line: 3 }
        );
        assert_eq!(
            check(
                Language::Java,
                "class A {\n  void a() {\n    int x = ;\n  }\n}\n"
            ),
            ParseCheck::DoesNotParse { line: 3 }
        );
        assert_eq!(
            check(Language::Swift, "func a() {\n    b()\n"),
            ParseCheck::DoesNotParse { line: 2 }
        );
    }

    /// On a large file the quick check stops where the parser gives up,
    /// dozens of lines past the error; `locate_with` gives the full check's
    /// line, the one to show the user.
    #[test]
    fn an_early_stop_is_located_at_the_error() {
        let entries: Vec<String> = (0..100)
            .map(|index| format!("  \"k{index}\" : {{\n    \"v\" : {index}\n  }}"))
            .collect();
        let text = format!("{{\n{}\n}}\n", entries.join(",\n"));
        let base = ParsedFile::new(Language::Json, text.clone()).unwrap();
        let at = text.find("\"k3\"").unwrap();
        let broken = [(at..at + 6, "\"k3\" { ")];
        let full = base.with(&broken).unwrap().check();
        assert_eq!(full, ParseCheck::DoesNotParse { line: 11 });
        let (quick, stopped) = base.check_with_stop(&broken);
        assert!(stopped);
        // The quick check names the line, counted from 1, where the parser
        // gave up: further down than the error.
        assert_eq!(quick, ParseCheck::DoesNotParse { line: 52 });
        assert_eq!(base.locate_with(&broken), full);
        // A check that ran to the end already has the full check's line.
        let whole = [(at..at + 6, "\"k3\" : ,")];
        assert!(!base.check_with_stop(&whole).1);
        assert_eq!(base.locate_with(&whole), base.with(&whole).unwrap().check());
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
