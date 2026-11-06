//! Static word expansion. Walks command substitutions on the way.

use brush_parser::word::{self, Parameter, ParameterExpr, WordPiece, WordPieceWithSource};

use crate::Word;
use crate::scan::arith_targets;
use crate::walk::Walker;

/// Partial result of expanding a word.
#[derive(Default)]
pub(crate) struct Res {
    text: String,
    unknown: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// Command word or argument: globs and word splitting apply.
    Arg,
    /// Assignment value: no globbing and no splitting.
    Assign,
}

fn has_glob(s: &str) -> bool {
    if s.contains(['*', '?']) || s.contains("+(") || s.contains("@(") || s.contains("!(") {
        return true;
    }
    if let Some(i) = s.find('[')
        && s[i..].contains(']')
    {
        return true;
    }
    if let Some(i) = s.find('{') {
        let rest = &s[i + 1..];
        if let Some(j) = rest.find('}') {
            let inner = &rest[..j];
            return inner.contains(',') || inner.contains("..");
        }
    }
    false
}

impl Walker<'_> {
    /// Expand one raw word. Command substitutions inside are walked.
    pub(crate) fn expand(&mut self, raw: &str, mode: Mode) -> Word {
        let Ok(pieces) = word::parse(raw, &self.popts) else {
            self.unanalyzable(raw);
            return Word::Unknown(raw.to_string());
        };
        let mut r = Res::default();
        self.pieces(&pieces, false, mode, &mut r);
        if r.unknown {
            Word::Unknown(raw.to_string())
        } else {
            Word::Known(r.text)
        }
    }

    /// Walk substitutions in a here-doc body. Quotes in the body are literal.
    pub(crate) fn walk_heredoc(&mut self, body: &str) {
        match word::parse_heredoc(body, &self.popts) {
            Ok(pieces) => {
                let mut r = Res::default();
                self.pieces(&pieces, true, Mode::Assign, &mut r);
            }
            Err(_) => self.unanalyzable(body),
        }
    }

    fn pieces(&mut self, ps: &[WordPieceWithSource], quoted: bool, mode: Mode, r: &mut Res) {
        for p in ps {
            self.piece(&p.piece, quoted, mode, r);
        }
    }

    fn piece(&mut self, p: &WordPiece, quoted: bool, mode: Mode, r: &mut Res) {
        match p {
            WordPiece::Text(s) => {
                if !quoted && mode == Mode::Arg && has_glob(s) {
                    r.unknown = true;
                }
                r.text.push_str(s);
            }
            WordPiece::SingleQuotedText(s) => r.text.push_str(s),
            WordPiece::AnsiCQuotedText(s) => {
                if s.contains('\\') {
                    r.unknown = true;
                }
                r.text.push_str(s);
            }
            WordPiece::EscapeSequence(s) => {
                let mut cs = s.chars();
                match (cs.next(), cs.next(), cs.next()) {
                    (Some('\\'), Some(c), None) => r.text.push(c),
                    _ => r.unknown = true,
                }
            }
            WordPiece::DoubleQuotedSequence(inner)
            | WordPiece::GettextDoubleQuotedSequence(inner) => {
                self.pieces(inner, true, mode, r);
            }
            WordPiece::TildeExpansion(word::TildeExpr::Home) => match self.var_value("HOME") {
                Some(h) if h.starts_with('/') => r.text.push_str(&h),
                _ => r.unknown = true,
            },
            WordPiece::TildeExpansion(_) => r.unknown = true,
            WordPiece::ParameterExpansion(pe) => self.param(pe, quoted, mode, r),
            WordPiece::CommandSubstitution(s) | WordPiece::BackquotedCommandSubstitution(s) => {
                self.cmdsub(s);
                r.unknown = true;
            }
            WordPiece::ArithmeticExpression(e) => {
                self.arith_text(&e.value);
                r.unknown = true;
            }
        }
    }

    fn param(&mut self, pe: &ParameterExpr, quoted: bool, mode: Mode, r: &mut Res) {
        if let ParameterExpr::Parameter {
            parameter: Parameter::Named(n),
            indirect: false,
        } = pe
        {
            match self.var_value(n) {
                Some(v) => {
                    let risky = !quoted
                        && mode == Mode::Arg
                        && (v.is_empty() || v.contains(char::is_whitespace) || has_glob(&v));
                    if risky {
                        r.unknown = true;
                    }
                    r.text.push_str(&v);
                }
                None => r.unknown = true,
            }
            return;
        }
        r.unknown = true;
        let (nested, arith, assigns) = nested_text(pe);
        if let Some(n) = assigns {
            self.vars_set_unknown(n);
        }
        for t in nested {
            self.walk_nested(t);
        }
        for t in arith {
            self.arith_text(t);
        }
    }

    fn walk_nested(&mut self, s: &str) {
        match word::parse(s, &self.popts) {
            Ok(pieces) => {
                let mut r = Res::default();
                self.pieces(&pieces, false, Mode::Assign, &mut r);
            }
            Err(_) => self.unanalyzable(s),
        }
    }

    /// `$((...))` and `((...))` text: record anything hidden inside and invalidate assigned names.
    pub(crate) fn arith_text(&mut self, text: &str) {
        if text.contains("$(") || text.contains('`') {
            self.unanalyzable(text);
        }
        for n in arith_targets(text) {
            self.vars_set_unknown(&n);
        }
    }
}

fn index_text<'a>(p: &'a Parameter, out: &mut Vec<&'a str>) {
    if let Parameter::NamedWithIndex { index, .. } = p {
        out.push(index);
    }
}

/// Nested word text, arithmetic text and an assigned name inside a parameter expansion.
fn nested_text(pe: &ParameterExpr) -> (Vec<&str>, Vec<&str>, Option<&str>) {
    let mut nested: Vec<&str> = Vec::new();
    let mut arith: Vec<&str> = Vec::new();
    let mut assigns: Option<&str> = None;
    match pe {
        ParameterExpr::Parameter { parameter, .. }
        | ParameterExpr::ParameterLength { parameter, .. }
        | ParameterExpr::Transform { parameter, .. } => {
            index_text(parameter, &mut nested);
        }
        ParameterExpr::UseDefaultValues {
            parameter,
            default_value,
            ..
        } => {
            index_text(parameter, &mut nested);
            nested.extend(default_value.as_deref());
        }
        ParameterExpr::AssignDefaultValues {
            parameter,
            default_value,
            ..
        } => {
            index_text(parameter, &mut nested);
            nested.extend(default_value.as_deref());
            if let Parameter::Named(n) = parameter {
                assigns = Some(n);
            }
        }
        ParameterExpr::IndicateErrorIfNullOrUnset {
            parameter,
            error_message,
            ..
        } => {
            index_text(parameter, &mut nested);
            nested.extend(error_message.as_deref());
        }
        ParameterExpr::UseAlternativeValue {
            parameter,
            alternative_value,
            ..
        } => {
            index_text(parameter, &mut nested);
            nested.extend(alternative_value.as_deref());
        }
        ParameterExpr::RemoveSmallestSuffixPattern {
            parameter, pattern, ..
        }
        | ParameterExpr::RemoveLargestSuffixPattern {
            parameter, pattern, ..
        }
        | ParameterExpr::RemoveSmallestPrefixPattern {
            parameter, pattern, ..
        }
        | ParameterExpr::RemoveLargestPrefixPattern {
            parameter, pattern, ..
        }
        | ParameterExpr::UppercaseFirstChar {
            parameter, pattern, ..
        }
        | ParameterExpr::UppercasePattern {
            parameter, pattern, ..
        }
        | ParameterExpr::LowercaseFirstChar {
            parameter, pattern, ..
        }
        | ParameterExpr::LowercasePattern {
            parameter, pattern, ..
        } => {
            index_text(parameter, &mut nested);
            nested.extend(pattern.as_deref());
        }
        ParameterExpr::Substring {
            parameter,
            offset,
            length,
            ..
        } => {
            index_text(parameter, &mut nested);
            arith.push(&offset.value);
            arith.extend(length.as_ref().map(|l| l.value.as_str()));
        }
        ParameterExpr::ReplaceSubstring {
            parameter,
            pattern,
            replacement,
            ..
        } => {
            index_text(parameter, &mut nested);
            nested.push(pattern);
            nested.extend(replacement.as_deref());
        }
        ParameterExpr::VariableNames { .. } | ParameterExpr::MemberKeys { .. } => {}
    }
    (nested, arith, assigns)
}
