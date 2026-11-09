//! `git-scry highlight`: shows the core's syntax highlighting in the
//! terminal, to check it without the app.

use std::path::Path;
use std::process::ExitCode;

use scry_core::highlight::{Highlighter, Span, SyntaxRole};
use scry_core::syntax::Language;

pub fn run(file: &Path, spans: bool) -> ExitCode {
    let text = match std::fs::read_to_string(file) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("git-scry: cannot read {}: {error}", file.display());
            return ExitCode::from(2);
        }
    };
    let Some(language) = Language::from_path(file) else {
        eprintln!("git-scry: no grammar for {}", file.display());
        return ExitCode::from(2);
    };
    let Some(highlighter) = Highlighter::new(language) else {
        eprintln!("git-scry: no highlight query for {}", language.name());
        return ExitCode::from(2);
    };
    for (line, line_spans) in text.lines().zip(highlighter.lines(&text)) {
        if spans {
            println!("{}", describe(line, &line_spans));
        } else {
            println!("{}", colour(line, &line_spans));
        }
    }
    ExitCode::SUCCESS
}

/// The line with ANSI colours. The 16 base colours follow the terminal's
/// own theme, light or dark; the app uses the design tokens instead.
fn colour(line: &str, spans: &[Span]) -> String {
    let mut out = String::new();
    let mut position = 0;
    for span in spans {
        out.push_str(&line[position..span.start]);
        out.push_str(&format!(
            "\x1b[{}m{}\x1b[0m",
            ansi(span.role),
            &line[span.start..span.end]
        ));
        position = span.end;
    }
    out.push_str(&line[position..]);
    out
}

fn ansi(role: SyntaxRole) -> &'static str {
    match role {
        SyntaxRole::Keyword => "35",
        SyntaxRole::String => "31",
        SyntaxRole::Number => "34",
        SyntaxRole::Comment => "2;3",
        SyntaxRole::Type => "36",
        SyntaxRole::Function => "94",
        SyntaxRole::Variable => "39",
        SyntaxRole::Constant => "32",
        SyntaxRole::Operator => "39",
        SyntaxRole::Punctuation => "2",
        SyntaxRole::Attribute => "33",
    }
}

/// The line's spans as text, such as `Keyword "func" · Function "upload"`.
fn describe(line: &str, spans: &[Span]) -> String {
    spans
        .iter()
        .map(|span| format!("{:?} {:?}", span.role, &line[span.start..span.end]))
        .collect::<Vec<_>>()
        .join(" · ")
}
