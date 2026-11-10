//! Indentation for text typed by hand: the file's indent unit, what Return
//! inserts, and when a line moves back because it closes a block.
//!
//! Brackets work the same in every language. Keyword and tag blocks follow
//! each language's rules: Ruby (`def` … `end`), Bash (`then` … `fi`,
//! `do` … `done`), Python (a colon opens a block, `return` ends one),
//! YAML, HTML tags, and `case` labels in Swift, Go and the C family.
//!
//! A line that closes a block takes the indentation of the line that opened
//! it, found by walking back and skipping nested blocks, rather than one
//! unit less: that stays right when the lines above are not.

use crate::syntax::Language;

/// What Return does. `text` replaces the current line from its start up to
/// the cursor: the line itself, moved back first when it closes a block
/// (Ruby's `end`), then the line break and the new line's indentation, and
/// for a block opened between a pair (`{|}`, `<div>|</div>`) the closing
/// part on a line of its own. `cursor` is where the cursor goes, in UTF-8
/// bytes from the start of `text`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Newline {
    pub text: String,
    pub cursor: usize,
}

/// The file's indent unit: a tab when most indented lines start with one,
/// otherwise the most common step between a line's indentation and the
/// next deeper one (2 or 4 spaces, say). Four spaces when the file says
/// nothing.
pub fn indent_unit(text: &str) -> String {
    let mut tabs = 0;
    let mut spaces = 0;
    // How often each step (in spaces) appears between consecutive lines.
    let mut steps = [0usize; 9];
    let mut previous = 0;
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if line.starts_with('\t') {
            tabs += 1;
            continue;
        }
        let width = line.len() - line.trim_start_matches(' ').len();
        if width > 0 {
            spaces += 1;
        }
        if width > previous && width - previous < steps.len() {
            steps[width - previous] += 1;
        }
        previous = width;
    }
    if tabs > spaces {
        return String::from("\t");
    }
    // The most common step; ties go to the smaller one.
    let mut best = 0;
    for step in 1..steps.len() {
        if steps[step] > steps[best] {
            best = step;
        }
    }
    if best == 0 {
        best = 4;
    }
    " ".repeat(best)
}

/// What Return inserts, with `before` all the text before the cursor and
/// `after` the rest of the cursor's line.
pub fn newline(before: &str, after: &str, unit: &str, language: Option<Language>) -> Newline {
    let line_start = before.rfind('\n').map_or(0, |index| index + 1);
    let above = &before[..line_start];
    let mut line = before[line_start..].to_string();
    if let Some(indentation) = closing_line_indent(above, &line, unit, language) {
        line = format!("{indentation}{}", line.trim_start());
    }
    let indentation = leading_whitespace(&line);
    let trimmed = line.trim();
    let after = after.trim_start();

    let mut next = indentation.clone();
    // The indentation of a closing part pushed to its own line.
    let mut closing = None;
    if let Some(opener) = trailing_opener(trimmed) {
        next.push_str(unit);
        if after.starts_with(closer_of(opener)) {
            closing = Some(indentation.clone());
        }
    } else if opens_block(trimmed, language) {
        next.push_str(unit);
        if language == Some(Language::Html) && after.starts_with("</") {
            closing = Some(indentation.clone());
        }
    } else if ends_flow(trimmed, language) {
        next = next.strip_suffix(unit).unwrap_or(&next).to_string();
    }

    let mut text = line;
    text.push('\n');
    text.push_str(&next);
    let cursor = text.len();
    if let Some(closing) = closing {
        text.push('\n');
        text.push_str(&closing);
    }
    Newline { text, cursor }
}

/// The indentation for the current line once `typed` is typed, with
/// `before` all the text before the cursor; `None` to type it as it is.
///
/// A closing bracket on a blank line, a colon ending a `case` label or
/// Python's `else:`, and `>` ending an HTML closing tag move the line back
/// to its block's opening line. Keywords alone (`end`, `fi`) wait for
/// Return, so `ending` is not moved while it is typed.
pub fn typed_indent(
    before: &str,
    typed: char,
    unit: &str,
    language: Option<Language>,
) -> Option<String> {
    let line_start = before.rfind('\n').map_or(0, |index| index + 1);
    let above = &before[..line_start];
    let current = &before[line_start..];
    match typed {
        '}' | ')' | ']' => {
            if !is_blank(current) {
                return None;
            }
            let opener = match typed {
                '}' => '{',
                ')' => '(',
                _ => '[',
            };
            Some(
                bracket_opener_indent(above, opener, typed)
                    .unwrap_or_else(|| current.strip_suffix(unit).unwrap_or(current).to_string()),
            )
        }
        ':' | '>' => {
            let line = format!("{current}{typed}");
            closing_line_indent(above, &line, unit, language)
        }
        _ => None,
    }
}

/// The indentation `line` should have when it closes a block, given the
/// text `above` it; `None` when it closes nothing.
fn closing_line_indent(
    above: &str,
    line: &str,
    unit: &str,
    language: Option<Language>,
) -> Option<String> {
    let trimmed = line.trim();
    let word = first_word(trimmed);
    match language? {
        Language::Ruby => match word {
            "end" | "else" | "elsif" | "when" | "rescue" | "ensure" => {
                keyword_opener_indent(above, ruby_starts_block, |line| first_word(line) == "end")
            }
            _ => None,
        },
        Language::Bash => match word {
            "fi" | "done" | "esac" | "else" | "elif" => {
                keyword_opener_indent(above, bash_starts_block, |line| {
                    matches!(first_word(line), "fi" | "done" | "esac")
                })
            }
            _ => None,
        },
        Language::Python => {
            let openers: &[&str] = match word {
                "else" => &["if", "elif", "for", "while", "try", "except"],
                "elif" => &["if", "elif"],
                "except" => &["try", "except"],
                "finally" => &["try", "except", "else"],
                _ => return None,
            };
            python_opener_indent(above, leading_whitespace(line).len(), openers)
        }
        Language::Swift | Language::Go if is_case_label(trimmed) => {
            // Swift and Go align `case` with its `switch`.
            bracket_opener_indent(above, '{', '}')
        }
        Language::C
        | Language::Cpp
        | Language::ObjectiveC
        | Language::Java
        | Language::JavaScript
        | Language::TypeScript
        | Language::Tsx
            if is_case_label(trimmed) =>
        {
            // The C family indents `case` one unit inside its `switch`.
            bracket_opener_indent(above, '{', '}').map(|indentation| indentation + unit)
        }
        Language::Html => {
            let name = trimmed.strip_prefix("</")?.strip_suffix('>')?.trim();
            if name.is_empty() {
                return None;
            }
            html_opener_indent(above, name)
        }
        _ => None,
    }
}

/// Whether the line opens a block of its language, so the next line goes
/// one unit deeper. Brackets are handled apart.
fn opens_block(trimmed: &str, language: Option<Language>) -> bool {
    match language {
        Some(Language::Python) => trimmed.ends_with(':'),
        Some(Language::Yaml) => {
            trimmed.ends_with(':')
                || [": |", ": >", ": |-", ": >-", ": |+", ": >+"]
                    .iter()
                    .any(|block| trimmed.ends_with(block))
        }
        Some(Language::Ruby) => {
            ruby_starts_block(trimmed)
                || matches!(
                    first_word(trimmed),
                    "else" | "elsif" | "when" | "rescue" | "ensure"
                )
        }
        Some(Language::Bash) => {
            matches!(last_word(trimmed), "then" | "do" | "else" | "in")
                || first_word(trimmed) == "else"
        }
        Some(Language::Html) => html_opening_tag(trimmed).is_some(),
        Some(
            Language::Swift
            | Language::Go
            | Language::C
            | Language::Cpp
            | Language::ObjectiveC
            | Language::Java
            | Language::JavaScript
            | Language::TypeScript
            | Language::Tsx,
        ) => is_case_label(trimmed),
        _ => false,
    }
}

/// Whether the line ends its block's flow, so the next line goes one unit
/// back: Python's `return`, `pass`, `break`, `continue` and `raise`.
fn ends_flow(trimmed: &str, language: Option<Language>) -> bool {
    language == Some(Language::Python)
        && matches!(
            first_word(trimmed),
            "return" | "pass" | "break" | "continue" | "raise"
        )
}

/// A Ruby line that starts a block closed by `end`: `def`, `class`,
/// `if`… at its start (not as a modifier such as `return if x`), or a block
/// opened by `do`. A one-line block ending in `end` starts nothing.
fn ruby_starts_block(line: &str) -> bool {
    if last_word(line) == "end" {
        return false;
    }
    let starts = matches!(
        first_word(line),
        "def" | "class" | "module" | "if" | "unless" | "while" | "until" | "case" | "begin" | "for"
    );
    let block = last_word(line) == "do" || (line.ends_with('|') && line.contains(" do |"));
    starts || block
}

/// A Bash line that starts a block closed by `fi`, `done` or `esac`,
/// unless it closes it on the same line.
fn bash_starts_block(line: &str) -> bool {
    matches!(
        first_word(line),
        "if" | "for" | "while" | "until" | "case" | "select"
    ) && !matches!(last_word(line), "fi" | "done" | "esac")
}

/// `case …:` or `default:`.
fn is_case_label(trimmed: &str) -> bool {
    matches!(first_word(trimmed), "case" | "default") && trimmed.ends_with(':')
}

/// The opening bracket that ends the line, if any.
fn trailing_opener(trimmed: &str) -> Option<char> {
    match trimmed.chars().last()? {
        bracket @ ('{' | '(' | '[') => Some(bracket),
        _ => None,
    }
}

fn closer_of(opener: char) -> char {
    match opener {
        '{' => '}',
        '(' => ')',
        _ => ']',
    }
}

/// The indentation of the line holding the opening bracket that a closing
/// bracket typed after `above` would close, skipping nested pairs. Brackets
/// inside strings and comments are counted too.
fn bracket_opener_indent(above: &str, opener: char, closer: char) -> Option<String> {
    let mut depth = 0;
    for (index, c) in above.char_indices().rev() {
        if c == closer {
            depth += 1;
        } else if c == opener {
            if depth == 0 {
                let start = above[..index].rfind('\n').map_or(0, |found| found + 1);
                return Some(leading_whitespace(&above[start..]));
            }
            depth -= 1;
        }
    }
    None
}

/// The indentation of the nearest line above that starts a keyword block
/// still open, skipping blocks opened and closed in between.
fn keyword_opener_indent(
    above: &str,
    starts: impl Fn(&str) -> bool,
    ends: impl Fn(&str) -> bool,
) -> Option<String> {
    let mut depth = 0;
    for line in above.lines().rev() {
        let trimmed = line.trim();
        if ends(trimmed) {
            depth += 1;
        } else if starts(trimmed) {
            if depth == 0 {
                return Some(leading_whitespace(line));
            }
            depth -= 1;
        }
    }
    None
}

/// Python has no closing keyword: the opener is the nearest line above,
/// less indented than the current line, that starts with one of `openers`.
fn python_opener_indent(above: &str, current: usize, openers: &[&str]) -> Option<String> {
    for line in above.lines().rev() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let indentation = leading_whitespace(line);
        if indentation.len() < current && openers.contains(&first_word(trimmed)) {
            return Some(indentation);
        }
    }
    None
}

/// The indentation of the nearest line above with a `<name>` tag still
/// open, skipping pairs opened and closed in between.
fn html_opener_indent(above: &str, name: &str) -> Option<String> {
    let opening = format!("<{name}");
    let closing = format!("</{name}");
    let mut depth = 0;
    for line in above.lines().rev() {
        let opens = count_tags(line, &opening);
        let closes = count_tags(line, &closing);
        if opens > closes {
            let open = opens - closes;
            if depth < open {
                return Some(leading_whitespace(line));
            }
            depth -= open;
        } else {
            depth += closes - opens;
        }
    }
    None
}

/// How many tags start with `prefix` and end their name there, not counting
/// self-closing ones.
fn count_tags(line: &str, prefix: &str) -> usize {
    let mut count = 0;
    let mut rest = line;
    while let Some(found) = rest.find(prefix) {
        let tail = &rest[found + prefix.len()..];
        let ends_name = tail
            .chars()
            .next()
            .is_none_or(|c| c == '>' || c.is_whitespace());
        let self_closing = tail.split('>').next().is_some_and(|tag| tag.ends_with('/'));
        if ends_name && !self_closing {
            count += 1;
        }
        rest = tail;
    }
    count
}

/// The name of the opening tag that ends the line, unless it is void
/// (`<br>`), self-closing or closed on the same line.
fn html_opening_tag(trimmed: &str) -> Option<&str> {
    if !trimmed.ends_with('>') || trimmed.ends_with("/>") {
        return None;
    }
    let start = trimmed.rfind('<')?;
    let tag = &trimmed[start + 1..];
    if tag.starts_with('/') || tag.starts_with('!') {
        return None;
    }
    let name_end = tag
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
        .unwrap_or(tag.len());
    let name = &tag[..name_end];
    const VOID: [&str; 14] = [
        "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param",
        "source", "track", "wbr",
    ];
    if name.is_empty() || VOID.contains(&name.to_ascii_lowercase().as_str()) {
        return None;
    }
    Some(name)
}

fn first_word(trimmed: &str) -> &str {
    let end = trimmed
        .find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .unwrap_or(trimmed.len());
    &trimmed[..end]
}

fn last_word(trimmed: &str) -> &str {
    // Starts after the last character that is not part of a word, which can
    // take more than one byte (`…`, `’`).
    let start = trimmed
        .char_indices()
        .rev()
        .find(|&(_, c)| !(c.is_alphanumeric() || c == '_'))
        .map_or(0, |(index, c)| index + c.len_utf8());
    &trimmed[start..]
}

fn is_blank(text: &str) -> bool {
    text.chars().all(|c| c == ' ' || c == '\t')
}

fn leading_whitespace(line: &str) -> String {
    line.chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const UNIT: &str = "    ";

    /// What Return leaves in place of the current line: its text, then `|`
    /// at the cursor.
    fn enter(before: &str, after: &str, language: Language) -> String {
        let newline = newline(before, after, UNIT, Some(language));
        format!(
            "{}|{}",
            &newline.text[..newline.cursor],
            &newline.text[newline.cursor..]
        )
    }

    /// The unit follows the file: four or two spaces from the steps between
    /// lines, a tab when tabs dominate, four spaces when nothing says.
    #[test]
    fn indent_unit_follows_the_file() {
        let four = "func a() {\n    if b {\n        c()\n    }\n}\n";
        assert_eq!(indent_unit(four), "    ");
        let two = "a:\n  b:\n    c: 1\n  d: 2\n";
        assert_eq!(indent_unit(two), "  ");
        let tabs = "int main() {\n\treturn 0;\n}\n";
        assert_eq!(indent_unit(tabs), "\t");
        assert_eq!(indent_unit("flat\nfile\n"), "    ");
    }

    /// A plain line break keeps the current indentation, so code typed in a
    /// function body stays aligned with it.
    #[test]
    fn newline_keeps_the_indentation() {
        assert_eq!(
            enter("        queue.cancelAll()", "", Language::Swift),
            "        queue.cancelAll()\n        |"
        );
    }

    /// After an opening bracket the new line goes one unit deeper; between a
    /// pair, the closing bracket moves to its own line, as Xcode does. This
    /// holds in every language.
    #[test]
    fn brackets_open_blocks_in_every_language() {
        assert_eq!(
            enter("func a() {", "", Language::Swift),
            "func a() {\n    |"
        );
        assert_eq!(
            enter("func a() {", "}", Language::Swift),
            "func a() {\n    |\n"
        );
        assert_eq!(
            enter("items = [", "]", Language::Python),
            "items = [\n    |\n"
        );
        assert_eq!(enter("\tif (a) {", "", Language::C), "\tif (a) {\n\t    |");
    }

    /// A colon opens a block in Python and YAML only; a YAML block scalar
    /// opens one too.
    #[test]
    fn colon_opens_a_block_in_python_and_yaml() {
        assert_eq!(enter("def a():", "", Language::Python), "def a():\n    |");
        assert_eq!(enter("jobs:", "", Language::Yaml), "jobs:\n    |");
        assert_eq!(enter("run: |", "", Language::Yaml), "run: |\n    |");
        assert_eq!(
            enter("let a: Int =", "", Language::Swift),
            "let a: Int =\n|"
        );
    }

    /// In Python, `return` and its kind end the block: the next line goes one
    /// unit back.
    #[test]
    fn python_return_ends_the_block() {
        assert_eq!(
            enter("def a():\n    return 1", "", Language::Python),
            "    return 1\n|"
        );
    }

    /// Python's `else:` moves back to its `if` as the colon is typed, even
    /// past a deeper nested block.
    #[test]
    fn python_else_aligns_with_its_if() {
        // `with` takes no `else`, so the `if` above it is the opener.
        let before = "if a:\n    with x:\n        f(x)\n        else";
        assert_eq!(
            typed_indent(before, ':', UNIT, Some(Language::Python)).as_deref(),
            Some("")
        );
    }

    /// Ruby: `def` and `do` open blocks, not a modifier `if`; `end` moves back
    /// to its opener on Return, past nested blocks; `else` sits with its `if`
    /// and opens a block of its own.
    #[test]
    fn ruby_blocks_open_and_close_with_keywords() {
        assert_eq!(enter("def a", "", Language::Ruby), "def a\n    |");
        assert_eq!(
            enter("items.each do |item|", "", Language::Ruby),
            "items.each do |item|\n    |"
        );
        assert_eq!(
            enter("return if done", "", Language::Ruby),
            "return if done\n|"
        );
        let before = "def a\n  if b\n    c\n  end\n  d\n  end";
        assert_eq!(enter(before, "", Language::Ruby), "end\n|");
        let before = "  if b\n    c\n    else";
        assert_eq!(enter(before, "", Language::Ruby), "  else\n      |");
    }

    /// Bash: `then` and `do` open blocks, `fi` and `done` move back to their
    /// `if` and `for` on Return.
    #[test]
    fn bash_blocks_open_and_close_with_keywords() {
        assert_eq!(
            enter("if [ -f a ]; then", "", Language::Bash),
            "if [ -f a ]; then\n    |"
        );
        assert_eq!(
            enter("for f in *; do", "", Language::Bash),
            "for f in *; do\n    |"
        );
        let before = "if a; then\n    for f in *; do\n        b\n    done\n    fi";
        assert_eq!(enter(before, "", Language::Bash), "fi\n|");
    }

    /// `case` labels: aligned with `switch` in Swift and Go, one unit inside
    /// it in the C family; the label's body goes one unit deeper.
    #[test]
    fn case_labels_follow_each_languages_style() {
        let before = "switch a {\ncase .one:\n    b()\n    case .two";
        assert_eq!(
            typed_indent(before, ':', UNIT, Some(Language::Swift)).as_deref(),
            Some("")
        );
        let before = "switch (a) {\n    case 1:\n        b();\n        case 2";
        assert_eq!(
            typed_indent(before, ':', UNIT, Some(Language::Java)).as_deref(),
            Some("    ")
        );
        assert_eq!(
            enter("case .one:", "", Language::Swift),
            "case .one:\n    |"
        );
    }

    /// HTML: an opening tag opens a block, unless void or closed on the same
    /// line; a closing tag moves back to its opening tag as `>` is typed;
    /// Return between a pair opens it like braces.
    #[test]
    fn html_tags_open_and_close_blocks() {
        assert_eq!(
            enter("<div class=\"a\">", "", Language::Html),
            "<div class=\"a\">\n    |"
        );
        assert_eq!(enter("<br>", "", Language::Html), "<br>\n|");
        assert_eq!(enter("<p>text</p>", "", Language::Html), "<p>text</p>\n|");
        assert_eq!(enter("<ul>", "</ul>", Language::Html), "<ul>\n    |\n");
        let before = "<div>\n    <div>\n        <p>a</p>\n    </div>\n    </div";
        assert_eq!(
            typed_indent(before, '>', UNIT, Some(Language::Html)).as_deref(),
            Some("")
        );
    }

    /// A closing bracket typed on a blank line moves back to the line that
    /// opened its block, skipping nested pairs; with text before it it is
    /// typed as it is, and with no opener the line loses one unit.
    #[test]
    fn closing_brackets_align_with_their_opening_line() {
        let before = "func a() {\n    if b {\n        c()\n        ";
        assert_eq!(
            typed_indent(before, '}', UNIT, None).as_deref(),
            Some("    ")
        );
        let before = "  call(\n      a(b),\n      [c]\n      ";
        assert_eq!(typed_indent(before, ')', UNIT, None).as_deref(), Some("  "));
        assert_eq!(typed_indent("    let a = b", '}', UNIT, None), None);
        assert_eq!(
            typed_indent("        ", '}', UNIT, None).as_deref(),
            Some("    ")
        );
        assert_eq!(typed_indent("    a", 'x', UNIT, None), None);
    }

    /// Lines ending in a character of more than one byte (`…`, `’`, an
    /// emoji), in the current line or above it, must not crash: Return in
    /// a Ruby or Bash file with accented comments once panicked and closed
    /// the app.
    #[test]
    fn non_ascii_text_does_not_break_indentation() {
        assert_eq!(enter("# Loading…", "", Language::Ruby), "# Loading…\n|");
        let before = "# it’s ok\ndef a\n  b\n  end";
        assert_eq!(enter(before, "", Language::Ruby), "end\n|");
        assert_eq!(
            enter("if x; then # ok…", "", Language::Bash),
            "if x; then # ok…\n|"
        );
        assert_eq!(
            enter("café = 1 # 🎉", "", Language::Python),
            "café = 1 # 🎉\n|"
        );
        assert_eq!(
            typed_indent("def a\n  # é…\n  end", 'd', UNIT, Some(Language::Ruby)),
            None
        );
    }

    /// Keywords alone are not moved while typed, only on Return, so a word
    /// such as `ending` never jumps back.
    #[test]
    fn keywords_wait_for_return() {
        assert_eq!(
            typed_indent("def a\n  end", 'd', UNIT, Some(Language::Ruby)),
            None
        );
    }
}
