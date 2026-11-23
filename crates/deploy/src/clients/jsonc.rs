//! Reading and changing one member of a JSON file that belongs to another program, every other byte kept: the
//! other members, the other keys, the comments, the order, the indentation and the line endings.
//!
//! The file is read by `jsonc-parser` (comments and trailing commas tolerated, every other extension refused), for
//! its byte positions only: the change is a few splices of the original text, never a rewrite. Each result is read
//! again and compared with what was meant (same value with only our member changed, same comments outside our
//! member); any difference refuses the change, and nothing is written.

use std::panic::{AssertUnwindSafe, catch_unwind};

use jsonc_parser::ast::{Object, ObjectProp, Value as Ast};
use jsonc_parser::common::{Range, Ranged};
use jsonc_parser::tokens::{Token, TokenAndRange};
use jsonc_parser::{CollectOptions, CommentCollectionStrategy, ParseOptions, parse_to_ast};
use serde_json::{Map, Value};

use super::Unreadable;

const BOM: &str = "\u{feff}";

/// The fields of an entry, in the order they are written.
pub(crate) type Fields = [(&'static str, Value)];

/// The value the fields stand for.
pub(crate) fn fields_value(fields: &Fields) -> Value {
    Value::Object(fields.iter().map(|(k, v)| ((*k).to_owned(), v.clone())).collect())
}

/// What a file holds for one entry of one section.
#[derive(Clone, PartialEq)]
pub(crate) struct Found {
    /// The entry, numbers kept as their text (see [`to_json`]).
    pub entry: Option<Value>,
    /// The other members of the section.
    pub others: usize,
}

struct Parsed<'a> {
    text: &'a str,
    /// `None`: nothing but blanks.
    root: Option<Object<'a>>,
    /// Every token, comments included.
    tokens: Vec<TokenAndRange<'a>>,
}

fn options() -> ParseOptions {
    ParseOptions {
        allow_comments: true,
        allow_trailing_commas: true,
        allow_loose_object_property_names: false,
        allow_missing_commas: false,
        allow_single_quoted_strings: false,
        allow_hexadecimal_numbers: false,
        allow_unary_plus_numbers: false,
        allow_bare_decimal_point_numbers: false,
        allow_non_finite_numbers: false,
        allow_extended_string_escapes: false,
    }
}

fn parse(text: &str) -> Result<Parsed<'_>, Unreadable> {
    let collect = CollectOptions {
        comments: CommentCollectionStrategy::AsTokens,
        tokens: true,
    };
    // The parser is not ours: whatever it does on an unexpected input, nothing is written.
    let result = catch_unwind(AssertUnwindSafe(|| parse_to_ast(text, &collect, &options())))
        .map_err(|_| Unreadable::Unsafe)?
        .map_err(|error| Unreadable::Syntax {
            line: error.line_display(),
        })?;
    let tokens = result.tokens.unwrap_or_default();
    let root = match result.value {
        None if tokens.is_empty() => None,
        None => return Err(Unreadable::OnlyComments),
        Some(Ast::Object(object)) => Some(object),
        Some(_) => return Err(Unreadable::NotAnObject),
    };
    Ok(Parsed { text, root, tokens })
}

fn is_comment(token: &TokenAndRange<'_>) -> bool {
    matches!(token.token, Token::CommentLine(_) | Token::CommentBlock(_))
}

fn prop_name<'a>(prop: &'a ObjectProp<'_>) -> &'a str {
    prop.name.as_str()
}

/// The member `key` of `object`, refused when it appears more than once (which one would the program read?).
fn unique<'b, 'a>(object: &'b Object<'a>, key: &'static str) -> Result<Option<&'b ObjectProp<'a>>, Unreadable> {
    let mut found = object.properties.iter().filter(|p| prop_name(p) == key);
    let first = found.next();
    if found.next().is_some() {
        return Err(Unreadable::Duplicate(key));
    }
    Ok(first)
}

/// The value of the text, for comparisons: numbers are kept as their text, never parsed (a number we do not
/// touch must stay the same text).
fn to_json(value: &Ast<'_>) -> Value {
    match value {
        Ast::StringLit(s) => Value::String(s.value.to_string()),
        Ast::NumberLit(n) => Value::String(format!("\u{0}number:{}", n.value)),
        Ast::BooleanLit(b) => Value::Bool(b.value),
        Ast::NullKeyword(_) => Value::Null,
        Ast::Array(a) => Value::Array(a.elements.iter().map(to_json).collect()),
        Ast::Object(o) => Value::Object(object_json(o)),
    }
}

fn object_json(object: &Object<'_>) -> Map<String, Value> {
    object
        .properties
        .iter()
        .map(|p| (prop_name(p).to_owned(), to_json(&p.value)))
        .collect()
}

impl<'a> Parsed<'a> {
    /// The first token at or after `pos` that is not a comment.
    fn next_token(&self, pos: usize) -> Option<&TokenAndRange<'a>> {
        self.tokens.iter().find(|t| t.range.start >= pos && !is_comment(t))
    }

    /// The comma that follows `pos`, comments aside.
    fn comma_after(&self, pos: usize) -> Option<Range> {
        self.next_token(pos)
            .filter(|t| matches!(t.token, Token::Comma))
            .map(|t| t.range)
    }

    fn comment_within(&self, start: usize, end: usize, except: Range) -> bool {
        self.tokens.iter().any(|t| {
            is_comment(t)
                && t.range.start < end
                && start < t.range.end
                && !(except.start <= t.range.start && t.range.end <= except.end)
        })
    }

    /// The comments outside `except`, in order.
    fn comments_outside(&self, except: Option<Range>) -> Vec<&'a str> {
        let text = self.text;
        self.tokens
            .iter()
            .filter(|t| is_comment(t))
            .filter(|t| except.is_none_or(|e| !(e.start <= t.range.start && t.range.end <= e.end)))
            .filter_map(|t| text.get(t.range.start..t.range.end))
            .collect()
    }

    /// Past the blanks and one-line comments that end the line of `pos`, before its line break; `pos` when
    /// something else follows on that line.
    fn end_of_line(&self, pos: usize) -> usize {
        let bytes = self.text.as_bytes();
        let mut at = pos;
        loop {
            while matches!(bytes.get(at), Some(b' ' | b'\t')) {
                at += 1;
            }
            match bytes.get(at) {
                None | Some(b'\r' | b'\n') => return at,
                _ => {}
            }
            let comment = self
                .tokens
                .iter()
                .find(|t| t.range.start == at && is_comment(t))
                .filter(|t| !self.text[t.range.start..t.range.end].contains('\n'));
            match comment {
                Some(c) => at = c.range.end,
                None => return pos,
            }
        }
    }
}

fn line_start(text: &str, pos: usize) -> usize {
    text[..pos].rfind('\n').map_or(0, |i| i + 1)
}

/// Only blanks before `pos` on its line.
fn own_line(text: &str, pos: usize) -> bool {
    text[line_start(text, pos)..pos]
        .bytes()
        .all(|b| b == b' ' || b == b'\t')
}

/// The blanks that start the line of `pos`.
fn line_indent(text: &str, pos: usize) -> &str {
    let start = line_start(text, pos);
    let rest = &text[start..];
    let len = rest.bytes().take_while(|b| *b == b' ' || *b == b'\t').count();
    &rest[..len]
}

/// How the file is laid out: its line break and its indentation step.
struct Style {
    newline: &'static str,
    unit: String,
}

fn style(text: &str, root: &Object<'_>) -> Style {
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let unit = root
        .properties
        .first()
        .filter(|p| own_line(text, p.range.start))
        .map(|p| line_indent(text, p.range.start).to_owned())
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| "  ".to_owned());
    Style { newline, unit }
}

fn quoted(key: &str) -> String {
    Value::String(key.to_owned()).to_string()
}

fn is_scalar(value: &Value) -> bool {
    !matches!(value, Value::Array(_) | Value::Object(_))
}

/// `value` over several lines, its first line unindented, the others under `base`. Arrays of scalars stay on one
/// line.
fn render(value: &Value, base: &str, style: &Style) -> String {
    match value {
        Value::Object(map) if !map.is_empty() => {
            let fields: Vec<(&str, &Value)> = map.iter().map(|(k, v)| (k.as_str(), v)).collect();
            render_object(&fields, base, style)
        }
        Value::Array(items) if items.iter().all(is_scalar) => {
            let items: Vec<String> = items.iter().map(Value::to_string).collect();
            format!("[{}]", items.join(", "))
        }
        Value::Array(items) => {
            let inner = format!("{base}{}", style.unit);
            let lines: Vec<String> = items
                .iter()
                .map(|v| format!("{}{inner}{}", style.newline, render(v, &inner, style)))
                .collect();
            format!("[{}{}{base}]", lines.join(","), style.newline)
        }
        other => other.to_string(),
    }
}

fn render_object(fields: &[(&str, &Value)], base: &str, style: &Style) -> String {
    let inner = format!("{base}{}", style.unit);
    let lines: Vec<String> = fields
        .iter()
        .map(|(k, v)| format!("{}{inner}{}: {}", style.newline, quoted(k), render(v, &inner, style)))
        .collect();
    format!("{{{}{}{base}}}", lines.join(","), style.newline)
}

fn render_inline(fields: &[(&str, &Value)]) -> String {
    let parts: Vec<String> = fields.iter().map(|(k, v)| format!("{}: {}", quoted(k), v)).collect();
    format!("{{{}}}", parts.join(", "))
}

fn borrowed(fields: &Fields) -> Vec<(&str, &Value)> {
    fields.iter().map(|(k, v)| (*k, v)).collect()
}

/// What goes in as a new member: our entry, or a whole section holding it.
enum Member<'f> {
    Entry(&'f Fields),
    Section { name: &'static str, fields: &'f Fields },
}

impl Member<'_> {
    fn render(&self, base: &str, style: &Style) -> String {
        match self {
            Self::Entry(fields) => render_object(&borrowed(fields), base, style),
            Self::Section { name, fields } => {
                let inner = format!("{base}{}", style.unit);
                format!(
                    "{{{nl}{inner}{}: {}{nl}{base}}}",
                    quoted(name),
                    render_object(&borrowed(fields), &inner, style),
                    nl = style.newline
                )
            }
        }
    }

    fn render_inline(&self) -> String {
        match self {
            Self::Entry(fields) => render_inline(&borrowed(fields)),
            Self::Section { name, fields } => format!("{{{}: {}}}", quoted(name), render_inline(&borrowed(fields))),
        }
    }
}

/// `[start, end)` replaced by `text`.
struct Edit {
    start: usize,
    end: usize,
    text: String,
}

fn splice(text: &str, mut edits: Vec<Edit>) -> String {
    edits.sort_by_key(|e| std::cmp::Reverse(e.start));
    let mut out = text.to_owned();
    for edit in edits {
        out.replace_range(edit.start..edit.end, &edit.text);
    }
    out
}

/// `{ "section": { "name": { … } } }`, for a file that does not exist yet or is empty, and for the snippet to
/// paste.
pub(crate) fn fresh(section: &'static str, name: &'static str, fields: &Fields) -> String {
    let style = Style {
        newline: "\n",
        unit: "  ".to_owned(),
    };
    let member = Member::Section { name, fields };
    format!("{{\n  {}: {}\n}}\n", quoted(section), member.render("  ", &style))
}

/// `"name": { … }` as the preview shows it.
pub(crate) fn member_text(name: &str, fields: &Fields) -> String {
    let style = Style {
        newline: "\n",
        unit: "  ".to_owned(),
    };
    format!("{}: {}", quoted(name), render_object(&borrowed(fields), "", &style))
}

pub(crate) fn split_bom(text: &str) -> (&str, &str) {
    match text.strip_prefix(BOM) {
        Some(rest) => (BOM, rest),
        None => ("", text),
    }
}

/// The entry `name` of `section`.
pub(crate) fn find(text: &str, section: &'static str, name: &'static str) -> Result<Found, Unreadable> {
    let (_, body) = split_bom(text);
    let parsed = parse(body)?;
    let Some(root) = &parsed.root else {
        return Ok(Found { entry: None, others: 0 });
    };
    let Some(prop) = unique(root, section)? else {
        return Ok(Found { entry: None, others: 0 });
    };
    let object = prop.value.as_object().ok_or(Unreadable::SectionNotAnObject(section))?;
    let entry = unique(object, name)?.map(|p| to_json(&p.value));
    let others = object.properties.iter().filter(|p| prop_name(p) != name).count();
    Ok(Found { entry, others })
}

/// The text with the entry `name` of `section` set to `fields`, added (with its section when missing) or
/// replaced.
pub(crate) fn set(
    text: &str,
    section: &'static str,
    name: &'static str,
    fields: &Fields,
) -> Result<String, Unreadable> {
    let (bom, body) = split_bom(text);
    let parsed = parse(body)?;
    let Some(root) = &parsed.root else {
        return Ok(format!("{bom}{}", fresh(section, name, fields)));
    };
    let style = style(body, root);
    let edits = match unique(root, section)? {
        Some(prop) => {
            let object = prop.value.as_object().ok_or(Unreadable::SectionNotAnObject(section))?;
            match unique(object, name)? {
                Some(entry) => vec![replace_value(body, entry, fields, &style)],
                None => insert(&parsed, object, name, &Member::Entry(fields), &style)?,
            }
        }
        None => insert(&parsed, root, section, &Member::Section { name, fields }, &style)?,
    };
    let changed = splice(body, edits);
    verify(body, &changed, section, name, Some(fields_value(fields)))?;
    Ok(format!("{bom}{changed}"))
}

/// The text without the entry `name` of `section`; `None` when there is none.
pub(crate) fn remove(text: &str, section: &'static str, name: &'static str) -> Result<Option<String>, Unreadable> {
    let (bom, body) = split_bom(text);
    let parsed = parse(body)?;
    let Some(root) = &parsed.root else {
        return Ok(None);
    };
    let Some(prop) = unique(root, section)? else {
        return Ok(None);
    };
    let object = prop.value.as_object().ok_or(Unreadable::SectionNotAnObject(section))?;
    let Some(entry) = unique(object, name)? else {
        return Ok(None);
    };
    let edits = removal(&parsed, object, entry)?;
    let changed = splice(body, edits);
    verify(body, &changed, section, name, None)?;
    Ok(Some(format!("{bom}{changed}")))
}

fn replace_value(text: &str, entry: &ObjectProp<'_>, fields: &Fields, style: &Style) -> Edit {
    let range = entry.value.range();
    let rendered = if own_line(text, entry.range.start) {
        render_object(&borrowed(fields), line_indent(text, entry.range.start), style)
    } else {
        render_inline(&borrowed(fields))
    };
    Edit {
        start: range.start,
        end: range.end,
        text: rendered,
    }
}

fn insert(
    parsed: &Parsed<'_>,
    object: &Object<'_>,
    key: &'static str,
    member: &Member<'_>,
    style: &Style,
) -> Result<Vec<Edit>, Unreadable> {
    let text = parsed.text;
    let nl = style.newline;
    let key = quoted(key);
    let Some(last) = object.properties.last() else {
        // `{` and `}` are one byte each.
        let (open, close) = (object.range.start + 1, object.range.end.saturating_sub(1));
        let inner = text.get(open..close).ok_or(Unreadable::Unsafe)?;
        let blank = inner.trim().is_empty();
        if text.contains('\n') {
            let base = line_indent(text, object.range.start);
            let indent = format!("{base}{}", style.unit);
            let added = format!("{nl}{indent}{key}: {}", member.render(&indent, style));
            return Ok(vec![if blank {
                Edit {
                    start: open,
                    end: close,
                    text: format!("{added}{nl}{base}"),
                }
            } else {
                // Comments inside stay, after the new member.
                Edit {
                    start: open,
                    end: open,
                    text: added,
                }
            }]);
        }
        let added = format!("{key}: {}", member.render_inline());
        return Ok(vec![if blank {
            Edit {
                start: open,
                end: close,
                text: added,
            }
        } else {
            Edit {
                start: open,
                end: open,
                text: format!("{added} "),
            }
        }]);
    };
    let end = last.range.end;
    let comma = parsed.comma_after(end);
    let after = comma.map_or(end, |c| c.end);
    if own_line(text, last.range.start) {
        let indent = line_indent(text, last.range.start);
        // Past the comments that end the line of the last member: they stay with it.
        let at = parsed.end_of_line(after);
        let added = format!("{nl}{indent}{key}: {}", member.render(indent, style));
        return Ok(match comma {
            Some(_) => vec![Edit {
                start: at,
                end: at,
                text: format!("{added},"),
            }],
            None if at == end => vec![Edit {
                start: end,
                end,
                text: format!(",{added}"),
            }],
            None => vec![
                Edit {
                    start: end,
                    end,
                    text: ",".to_owned(),
                },
                Edit {
                    start: at,
                    end: at,
                    text: added,
                },
            ],
        });
    }
    let added = format!(" {key}: {}", member.render_inline());
    Ok(vec![match comma {
        Some(_) => Edit {
            start: after,
            end: after,
            text: format!("{added},"),
        },
        None => Edit {
            start: end,
            end,
            text: format!(",{added}"),
        },
    }])
}

fn removal(parsed: &Parsed<'_>, object: &Object<'_>, entry: &ObjectProp<'_>) -> Result<Vec<Edit>, Unreadable> {
    let text = parsed.text;
    let bytes = text.as_bytes();
    let range = entry.range;
    let index = object
        .properties
        .iter()
        .position(|p| p.range.start == range.start)
        .ok_or(Unreadable::Unsafe)?;
    let comma = parsed.comma_after(range.end);
    let previous_comma = || -> Result<Range, Unreadable> {
        let previous = index
            .checked_sub(1)
            .and_then(|i| object.properties.get(i))
            .ok_or(Unreadable::Unsafe)?;
        parsed.comma_after(previous.range.end).ok_or(Unreadable::Unsafe)
    };
    let mut edits = Vec::new();
    let mut cut = |start: usize, end: usize| -> Result<(), Unreadable> {
        // A comment of the user outside our entry is never removed.
        if parsed.comment_within(start, end, range) {
            return Err(Unreadable::Unsafe);
        }
        edits.push(Edit {
            start,
            end,
            text: String::new(),
        });
        Ok(())
    };
    let (open, close) = (object.range.start + 1, object.range.end.saturating_sub(1));
    if object.properties.len() == 1 && !parsed.comment_within(open, close, range) {
        // Our entry alone: the section goes back to `{}`.
        cut(open, close)?;
    } else if own_line(text, range.start) {
        let mut end = comma.map_or(range.end, |c| c.end);
        let mut rest = end;
        while matches!(bytes.get(rest), Some(b' ' | b'\t')) {
            rest += 1;
        }
        let start = if text[rest..].starts_with("\r\n") {
            end = rest + 2;
            line_start(text, range.start)
        } else if text[rest..].starts_with('\n') || rest == text.len() {
            end = (rest + 1).min(text.len());
            line_start(text, range.start)
        } else {
            // Something follows on the line: it joins the end of the previous token, as before our entry.
            parsed
                .tokens
                .iter()
                .map(|t| t.range.end)
                .filter(|e| *e <= range.start)
                .max()
                .unwrap_or(range.start)
        };
        cut(start, end)?;
        if comma.is_none() && index > 0 {
            let c = previous_comma()?;
            cut(c.start, c.end)?;
        }
    } else if let Some(c) = comma {
        if index > 0 {
            // From the end of the previous token: the blank before our entry goes with it.
            let start = parsed
                .tokens
                .iter()
                .map(|t| t.range.end)
                .filter(|e| *e <= range.start)
                .max()
                .unwrap_or(range.start);
            cut(start, c.end)?;
        } else {
            let mut end = c.end;
            while matches!(bytes.get(end), Some(b' ' | b'\t')) {
                end += 1;
            }
            cut(range.start, end)?;
        }
    } else if index > 0 {
        cut(previous_comma()?.start, range.end)?;
    } else {
        cut(range.start, range.end)?;
    }
    Ok(edits)
}

/// The range of the entry `name` of `section`, if there is one.
fn entry_range(parsed: &Parsed<'_>, section: &'static str, name: &'static str) -> Result<Option<Range>, Unreadable> {
    let Some(root) = &parsed.root else { return Ok(None) };
    let Some(prop) = unique(root, section)? else {
        return Ok(None);
    };
    let Some(object) = prop.value.as_object() else {
        return Ok(None);
    };
    Ok(unique(object, name)?.map(|p| p.range))
}

/// The changed text says exactly what was meant: the same value with only our entry changed (or gone), and the
/// same comments outside our entry, in the same order.
fn verify(
    before: &str,
    after: &str,
    section: &'static str,
    name: &'static str,
    entry: Option<Value>,
) -> Result<(), Unreadable> {
    let old = parse(before)?;
    let new = parse(after).map_err(|_| Unreadable::Unsafe)?;
    let mut expected = old.root.as_ref().map(object_json).unwrap_or_default();
    match entry {
        Some(value) => {
            let section = expected
                .entry(section)
                .or_insert_with(|| Value::Object(Map::new()))
                .as_object_mut()
                .ok_or(Unreadable::Unsafe)?;
            section.insert(name.to_owned(), to_json_value(value));
        }
        None => {
            if let Some(section) = expected.get_mut(section).and_then(Value::as_object_mut) {
                section.remove(name);
            }
        }
    }
    let got = new.root.as_ref().map(object_json).ok_or(Unreadable::Unsafe)?;
    if got != expected {
        return Err(Unreadable::Unsafe);
    }
    let old_comments = old.comments_outside(entry_range(&old, section, name)?);
    let new_comments = new.comments_outside(entry_range(&new, section, name).map_err(|_| Unreadable::Unsafe)?);
    if old_comments != new_comments {
        return Err(Unreadable::Unsafe);
    }
    Ok(())
}

/// `value` as [`to_json`] would read it back (numbers as their text).
fn to_json_value(value: Value) -> Value {
    match value {
        Value::Number(n) => Value::String(format!("\u{0}number:{n}")),
        Value::Array(items) => Value::Array(items.into_iter().map(to_json_value).collect()),
        Value::Object(map) => Value::Object(map.into_iter().map(|(k, v)| (k, to_json_value(v))).collect()),
        other => other,
    }
}

#[cfg(test)]
pub(crate) fn split_bom_for_tests(text: &str) -> (&str, &str) {
    split_bom(text)
}

/// The value of a readable text, numbers as their text.
#[cfg(test)]
pub(crate) fn value_for_tests(text: &str) -> Value {
    let parsed = parse(text).ok();
    Value::Object(
        parsed
            .and_then(|p| p.root.as_ref().map(object_json))
            .unwrap_or_default(),
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use serde_json::json;

    use super::*;

    fn entry() -> Vec<(&'static str, Value)> {
        vec![("command", json!(r"C:\P\mcp-proxy.exe")), ("args", json!(["stdio"]))]
    }

    #[test]
    fn a_new_member_follows_the_layout_of_the_file() {
        let text = "{\r\n\t\"mcpServers\": {\r\n\t\t\"a\": {\"command\": \"a.exe\"} // a\r\n\t}\r\n}\r\n";
        let out = set(text, "mcpServers", "mcp-proxy", &entry()).unwrap();
        assert_eq!(
            out,
            "{\r\n\t\"mcpServers\": {\r\n\t\t\"a\": {\"command\": \"a.exe\"}, // a\r\n\t\t\"mcp-proxy\": {\r\n\t\t\t\"command\": \"C:\\\\P\\\\mcp-proxy.exe\",\r\n\t\t\t\"args\": [\"stdio\"]\r\n\t\t}\r\n\t}\r\n}\r\n"
        );
    }

    #[test]
    fn a_one_line_file_stays_on_one_line() {
        let out = set(r#"{"mcpServers":{"a":{}}}"#, "mcpServers", "mcp-proxy", &entry()).unwrap();
        assert_eq!(
            out,
            r#"{"mcpServers":{"a":{}, "mcp-proxy": {"command": "C:\\P\\mcp-proxy.exe", "args": ["stdio"]}}}"#
        );
        let out = set("{}", "mcpServers", "mcp-proxy", &entry()).unwrap();
        assert_eq!(
            out,
            r#"{"mcpServers": {"mcp-proxy": {"command": "C:\\P\\mcp-proxy.exe", "args": ["stdio"]}}}"#
        );
    }

    #[test]
    fn a_byte_order_mark_is_kept() {
        let out = set("\u{feff}{\n}\n", "servers", "mcp-proxy", &entry()).unwrap();
        assert!(out.starts_with("\u{feff}{\n  \"servers\": {"), "{out}");
        assert_eq!(
            find(&out, "servers", "mcp-proxy").unwrap().entry,
            Some(fields_value(&entry()))
        );
    }

    #[test]
    fn extensions_other_than_comments_and_trailing_commas_are_refused() {
        for text in [
            "{'a': 1}",
            "{a: 1}",
            "{\"a\": 0x10}",
            "{\"a\": 1 \"b\": 2}",
            "[1]",
            "// only\n",
            "{",
        ] {
            assert!(find(text, "servers", "mcp-proxy").is_err(), "{text}");
        }
        assert!(matches!(
            find("{\n\n\"a\": }", "servers", "x"),
            Err(Unreadable::Syntax { line: 3 })
        ));
    }

    #[test]
    fn a_comment_beside_our_member_is_never_removed() {
        // The comment sits between our entry and its comma: removing the comma with the entry would take it.
        let text = "{\n  \"servers\": {\n    \"mcp-proxy\": {} /* keep */ ,\n    \"b\": {}\n  }\n}\n";
        assert_eq!(remove(text, "servers", "mcp-proxy").err(), Some(Unreadable::Unsafe));
    }
}
