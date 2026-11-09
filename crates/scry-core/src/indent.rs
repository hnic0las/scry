//! Indentation for text typed by hand: the file's indent unit, and what a
//! line break inserts so the new line starts where the code expects it.

use crate::syntax::Language;

/// What pressing Return inserts in place of the line break, and where the
/// cursor goes, in UTF-8 bytes from the start of `text`.
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

/// What Return inserts when the cursor has `before` on its left and
/// `after` on its right, on the same line.
///
/// The new line keeps the current line's indentation, one unit deeper after
/// an opening bracket (or a colon in Python and YAML). Between an opening
/// bracket and its closing one, the closing bracket moves to a line of its
/// own at the current indentation and the cursor sits on the indented line
/// between them, as Xcode does.
pub fn newline(before: &str, after: &str, unit: &str, language: Option<Language>) -> Newline {
    let indentation: String = before
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect();
    let last = before.trim_end().chars().last();
    let opens = match last {
        Some('{' | '(' | '[') => true,
        Some(':') => matches!(language, Some(Language::Python | Language::Yaml)),
        _ => false,
    };
    if !opens {
        let text = format!("\n{indentation}");
        return Newline {
            cursor: text.len(),
            text,
        };
    }
    let inner = format!("\n{indentation}{unit}");
    let closes = matches!(
        (last, after.trim_start().chars().next()),
        (Some('{'), Some('}')) | (Some('('), Some(')')) | (Some('['), Some(']'))
    );
    if closes {
        Newline {
            cursor: inner.len(),
            text: format!("{inner}\n{indentation}"),
        }
    } else {
        Newline {
            cursor: inner.len(),
            text: inner,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let newline = newline(
            "        queue.cancelAll()",
            "",
            "    ",
            Some(Language::Swift),
        );
        assert_eq!(newline.text, "\n        ");
        assert_eq!(newline.cursor, newline.text.len());
    }

    /// After an opening brace the new line goes one unit deeper.
    #[test]
    fn newline_after_an_opening_brace_indents() {
        let newline = newline("    func pause() {", "", "    ", Some(Language::Swift));
        assert_eq!(newline.text, "\n        ");
    }

    /// Between braces, the closing brace moves to its own line at the current
    /// indentation and the cursor sits on the indented line between them.
    #[test]
    fn newline_between_braces_opens_a_block() {
        let newline = newline("    func pause() {", "}", "    ", Some(Language::Swift));
        assert_eq!(newline.text, "\n        \n    ");
        assert_eq!(newline.cursor, "\n        ".len());
    }

    /// A colon indents only in languages where it opens a block.
    #[test]
    fn colon_opens_a_block_only_in_python_and_yaml() {
        assert_eq!(
            newline("def a():", "", "    ", Some(Language::Python)).text,
            "\n    "
        );
        assert_eq!(
            newline("jobs:", "", "  ", Some(Language::Yaml)).text,
            "\n  "
        );
        assert_eq!(
            newline("let a: Int =", "", "    ", Some(Language::Swift)).text,
            "\n"
        );
    }

    /// Tab-indented code continues with tabs.
    #[test]
    fn tabs_stay_tabs() {
        assert_eq!(
            newline("\tif (a) {", "", "\t", Some(Language::C)).text,
            "\n\t\t"
        );
    }
}
