//! 看守文件的统一 bytes → 格式校验 seam。

use std::cell::Cell;
use std::collections::BTreeSet;

use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};
use toml_edit::DocumentMut;

use super::model::{DiagnosticCode, GuardFileFormat, ValidationDiagnostic};

pub(crate) const MAX_DIAGNOSTICS: usize = 64;

pub(crate) fn validate_bytes(
    format: GuardFileFormat,
    bytes: &[u8],
    scope_id: &str,
    relative_file: Option<&str>,
) -> Result<(), Vec<ValidationDiagnostic>> {
    match format {
        GuardFileFormat::Toml => parse_toml_document(bytes, scope_id, relative_file).map(|_| ()),
        GuardFileFormat::Json => parse_json(bytes, scope_id, relative_file).map(|_| ()),
        GuardFileFormat::Markdown => validate_markdown(bytes, scope_id, relative_file),
        GuardFileFormat::PlainText => validate_plain_text(bytes, scope_id, relative_file),
    }
}

pub(crate) fn parse_toml_document(
    bytes: &[u8],
    scope_id: &str,
    relative_file: Option<&str>,
) -> Result<DocumentMut, Vec<ValidationDiagnostic>> {
    let text = match std::str::from_utf8(bytes) {
        Ok(text) => text,
        Err(error) => {
            return Err(vec![diagnostic_at(
                bytes,
                error.valid_up_to(),
                scope_id,
                relative_file,
                DiagnosticCode::InvalidUtf8,
            )]);
        }
    };
    text.parse::<DocumentMut>().map_err(|error| {
        let offset = error.span().map(|span| span.start);
        vec![diagnostic_at(
            bytes,
            offset.unwrap_or(0),
            scope_id,
            relative_file,
            DiagnosticCode::TomlInvalid,
        )]
    })
}

pub(crate) fn diagnostics_message(diagnostics: &[ValidationDiagnostic]) -> String {
    let Some(first) = diagnostics.first() else {
        return "format validation failed".to_string();
    };
    match (first.line, first.column) {
        (Some(line), Some(column)) => format!("{} at {}:{}", first.code.as_str(), line, column),
        _ => first.code.as_str().to_string(),
    }
}

fn validate_plain_text(
    bytes: &[u8],
    scope_id: &str,
    relative_file: Option<&str>,
) -> Result<(), Vec<ValidationDiagnostic>> {
    let text = match std::str::from_utf8(bytes) {
        Ok(text) => text,
        Err(error) => {
            return Err(vec![diagnostic_at(
                bytes,
                error.valid_up_to(),
                scope_id,
                relative_file,
                DiagnosticCode::InvalidUtf8,
            )]);
        }
    };
    if let Some(offset) = text.as_bytes().iter().position(|byte| *byte == 0) {
        return Err(vec![diagnostic_at(
            bytes,
            offset,
            scope_id,
            relative_file,
            DiagnosticCode::NulByte,
        )]);
    }
    Ok(())
}

fn parse_json(
    bytes: &[u8],
    scope_id: &str,
    relative_file: Option<&str>,
) -> Result<serde_json::Value, Vec<ValidationDiagnostic>> {
    let text = match std::str::from_utf8(bytes) {
        Ok(text) => text,
        Err(error) => {
            return Err(vec![diagnostic_at(
                bytes,
                error.valid_up_to(),
                scope_id,
                relative_file,
                DiagnosticCode::InvalidUtf8,
            )]);
        }
    };

    let duplicate = Cell::new(false);
    let mut deserializer = serde_json::Deserializer::from_str(text);
    let parsed = StrictJsonSeed {
        duplicate: &duplicate,
    }
    .deserialize(&mut deserializer);
    let parsed = parsed.and_then(|value| deserializer.end().map(|_| value));
    parsed.map_err(|error| {
        let code = if duplicate.get() {
            DiagnosticCode::JsonDuplicateKey
        } else {
            DiagnosticCode::JsonInvalid
        };
        vec![ValidationDiagnostic::new(
            scope_id,
            relative_file,
            code,
            Some(error.line() as u32),
            Some(error.column() as u32),
        )]
    })
}

struct StrictJsonSeed<'a> {
    duplicate: &'a Cell<bool>,
}

impl<'de, 'a> DeserializeSeed<'de> for StrictJsonSeed<'a> {
    type Value = serde_json::Value;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(StrictJsonVisitor {
            duplicate: self.duplicate,
        })
    }
}

struct StrictJsonVisitor<'a> {
    duplicate: &'a Cell<bool>,
}

impl<'de, 'a> Visitor<'de> for StrictJsonVisitor<'a> {
    type Value = serde_json::Value;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(serde_json::Value::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(serde_json::Value::Number(value.into()))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(serde_json::Value::Number(value.into()))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        serde_json::Number::from_f64(value)
            .map(serde_json::Value::Number)
            .ok_or_else(|| E::custom("invalid JSON number"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(serde_json::Value::String(value.to_string()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(serde_json::Value::String(value))
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(serde_json::Value::Null)
    }

    fn visit_none<E>(self) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(serde_json::Value::Null)
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        StrictJsonSeed {
            duplicate: self.duplicate,
        }
        .deserialize(deserializer)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element_seed(StrictJsonSeed {
            duplicate: self.duplicate,
        })? {
            values.push(value);
        }
        Ok(serde_json::Value::Array(values))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut keys = BTreeSet::new();
        let mut object = serde_json::Map::new();
        while let Some(key) = map.next_key::<String>()? {
            if !keys.insert(key.clone()) {
                self.duplicate.set(true);
                return Err(serde::de::Error::custom("duplicate JSON object key"));
            }
            let value = map.next_value_seed(StrictJsonSeed {
                duplicate: self.duplicate,
            })?;
            object.insert(key, value);
        }
        Ok(serde_json::Value::Object(object))
    }
}

fn validate_markdown(
    bytes: &[u8],
    scope_id: &str,
    relative_file: Option<&str>,
) -> Result<(), Vec<ValidationDiagnostic>> {
    let text = match std::str::from_utf8(bytes) {
        Ok(text) => text,
        Err(error) => {
            return Err(vec![diagnostic_at(
                bytes,
                error.valid_up_to(),
                scope_id,
                relative_file,
                DiagnosticCode::InvalidUtf8,
            )]);
        }
    };

    let mut diagnostics = Vec::new();
    let mut active: Vec<(String, usize)> = Vec::new();
    let mut seen = BTreeSet::new();
    let mut line_offset = 0;

    for segment in text.split_inclusive('\n') {
        let line = segment.strip_suffix('\n').unwrap_or(segment);
        let line = line.strip_suffix('\r').unwrap_or(line);
        let mut search = 0;
        while search < line.len() {
            let Some(relative_start) = line[search..].find("<!-- dashi:") else {
                break;
            };
            let start = search + relative_start;
            let rest = &line[start..];
            let Some(end_relative) = rest.find("-->") else {
                push_diagnostic(
                    &mut diagnostics,
                    bytes,
                    line_offset + start,
                    scope_id,
                    relative_file,
                    DiagnosticCode::MarkdownMalformedMarker,
                );
                break;
            };
            let end = start + end_relative + 3;
            let marker = &line[start..end];
            match parse_marker(marker) {
                Some((MarkerKind::Begin, id)) => {
                    if seen.contains(&id) || active.iter().any(|(active_id, _)| active_id == &id) {
                        push_diagnostic(
                            &mut diagnostics,
                            bytes,
                            line_offset + start,
                            scope_id,
                            relative_file,
                            DiagnosticCode::MarkdownDuplicateMarker,
                        );
                    } else {
                        active.push((id, line_offset + start));
                        seen.insert(active.last().unwrap().0.clone());
                    }
                }
                Some((MarkerKind::End, id)) => {
                    let Some(position) = active.iter().position(|(active_id, _)| active_id == &id)
                    else {
                        push_diagnostic(
                            &mut diagnostics,
                            bytes,
                            line_offset + start,
                            scope_id,
                            relative_file,
                            DiagnosticCode::MarkdownUnmatchedMarker,
                        );
                        search = end;
                        continue;
                    };
                    if position + 1 != active.len() {
                        push_diagnostic(
                            &mut diagnostics,
                            bytes,
                            line_offset + start,
                            scope_id,
                            relative_file,
                            DiagnosticCode::MarkdownCrossingMarker,
                        );
                    } else {
                        active.pop();
                    }
                }
                None => push_diagnostic(
                    &mut diagnostics,
                    bytes,
                    line_offset + start,
                    scope_id,
                    relative_file,
                    DiagnosticCode::MarkdownMalformedMarker,
                ),
            }
            search = end;
        }
        line_offset += segment.len();
    }

    for (_, offset) in active {
        push_diagnostic(
            &mut diagnostics,
            bytes,
            offset,
            scope_id,
            relative_file,
            DiagnosticCode::MarkdownUnclosedMarker,
        );
    }

    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(diagnostics)
    }
}

#[derive(Debug, Clone, Copy)]
enum MarkerKind {
    Begin,
    End,
}

fn parse_marker(marker: &str) -> Option<(MarkerKind, String)> {
    let body = marker
        .strip_prefix("<!-- dashi:")?
        .strip_suffix("-->")?
        .trim_end();
    let (kind, id) = body.split_once(' ')?;
    if id.is_empty() || id.chars().any(char::is_whitespace) || id.contains('<') || id.contains('>')
    {
        return None;
    }
    let kind = match kind {
        "begin" => MarkerKind::Begin,
        "end" => MarkerKind::End,
        _ => return None,
    };
    Some((kind, id.to_string()))
}

fn push_diagnostic(
    diagnostics: &mut Vec<ValidationDiagnostic>,
    bytes: &[u8],
    offset: usize,
    scope_id: &str,
    relative_file: Option<&str>,
    code: DiagnosticCode,
) {
    if diagnostics.len() < MAX_DIAGNOSTICS {
        let (line, column) = line_col(bytes, offset);
        diagnostics.push(ValidationDiagnostic::new(
            scope_id,
            relative_file,
            code,
            Some(line),
            Some(column),
        ));
    }
}

fn diagnostic_at(
    bytes: &[u8],
    offset: usize,
    scope_id: &str,
    relative_file: Option<&str>,
    code: DiagnosticCode,
) -> ValidationDiagnostic {
    let (line, column) = line_col(bytes, offset);
    ValidationDiagnostic::new(scope_id, relative_file, code, Some(line), Some(column))
}

fn line_col(bytes: &[u8], offset: usize) -> (u32, u32) {
    let offset = offset.min(bytes.len());
    let line = bytes[..offset]
        .iter()
        .filter(|byte| **byte == b'\n')
        .count() as u32
        + 1;
    let line_start = bytes[..offset]
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |position| position + 1);
    let column = std::str::from_utf8(&bytes[line_start..offset])
        .map(|line| line.chars().count() as u32 + 1)
        .unwrap_or((offset - line_start) as u32 + 1);
    (line, column)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_valid(format: GuardFileFormat, bytes: &[u8]) {
        assert!(validate_bytes(format, bytes, "scope", Some("config.toml")).is_ok());
    }

    fn assert_code(format: GuardFileFormat, bytes: &[u8], expected: DiagnosticCode) {
        let diagnostics = validate_bytes(format, bytes, "scope", Some("safe.toml")).unwrap_err();
        assert_eq!(diagnostics[0].code, expected);
    }

    #[test]
    fn valid_toml_is_accepted() {
        assert_valid(GuardFileFormat::Toml, b"[features]\nenabled = true\n");
    }

    #[test]
    fn valid_json_is_accepted() {
        assert_valid(
            GuardFileFormat::Json,
            br#"{"features":{"enabled":true},"items":[1,"two",null]}"#,
        );
    }

    #[test]
    fn toml_duplicate_key_reports_line_and_column() {
        let diagnostics = validate_bytes(
            GuardFileFormat::Toml,
            b"[features]\nenabled = true\nenabled = false\n",
            "scope",
            Some("config.toml"),
        )
        .unwrap_err();
        assert_eq!(diagnostics[0].code, DiagnosticCode::TomlInvalid);
        assert_eq!(diagnostics[0].line, Some(3));
        assert!(diagnostics[0].column.is_some());
    }

    #[test]
    fn json_nested_duplicate_key_is_rejected() {
        assert_code(
            GuardFileFormat::Json,
            br#"{"outer":[{"secret":1,"secret":2}]}"#,
            DiagnosticCode::JsonDuplicateKey,
        );
    }

    #[test]
    fn json_root_duplicate_and_equivalent_escaped_keys_are_rejected() {
        assert_code(
            GuardFileFormat::Json,
            br#"{"a":1,"a":2}"#,
            DiagnosticCode::JsonDuplicateKey,
        );
        assert_code(
            GuardFileFormat::Json,
            br#"{"a":1,"\u0061":2}"#,
            DiagnosticCode::JsonDuplicateKey,
        );
    }

    #[test]
    fn json_invalid_syntax_and_trailing_values_are_rejected() {
        assert_code(
            GuardFileFormat::Json,
            br#"{"a":1} trailing"#,
            DiagnosticCode::JsonInvalid,
        );
        assert_code(
            GuardFileFormat::Json,
            b"{\"a\":}",
            DiagnosticCode::JsonInvalid,
        );
        assert_code(GuardFileFormat::Json, &[0xff], DiagnosticCode::InvalidUtf8);
    }

    #[test]
    fn markdown_crossing_markers_are_rejected() {
        assert_code(
            GuardFileFormat::Markdown,
            b"<!-- dashi:begin a -->\n<!-- dashi:begin b -->\n<!-- dashi:end a -->\n<!-- dashi:end b -->\n",
            DiagnosticCode::MarkdownCrossingMarker,
        );
    }

    #[test]
    fn markdown_duplicate_unmatched_unclosed_and_malformed_markers_are_rejected() {
        assert_code(
            GuardFileFormat::Markdown,
            b"<!-- dashi:begin a -->\n<!-- dashi:end a -->\n<!-- dashi:begin a -->\n",
            DiagnosticCode::MarkdownDuplicateMarker,
        );
        assert_code(
            GuardFileFormat::Markdown,
            b"<!-- dashi:end missing -->\n",
            DiagnosticCode::MarkdownUnmatchedMarker,
        );
        assert_code(
            GuardFileFormat::Markdown,
            b"<!-- dashi:begin open -->\n",
            DiagnosticCode::MarkdownUnclosedMarker,
        );
        assert_code(
            GuardFileFormat::Markdown,
            b"<!-- dashi:unknown bad -->\n",
            DiagnosticCode::MarkdownMalformedMarker,
        );
    }

    #[test]
    fn markdown_without_managed_markers_is_valid() {
        assert_valid(GuardFileFormat::Markdown, b"# Notes\nplain text\n");
    }

    #[test]
    fn markdown_diagnostics_are_bounded() {
        let source = (0..(MAX_DIAGNOSTICS + 10))
            .map(|_| "<!-- dashi:end missing -->\n")
            .collect::<String>();
        let diagnostics = validate_bytes(
            GuardFileFormat::Markdown,
            source.as_bytes(),
            "scope",
            Some("notes.md"),
        )
        .unwrap_err();
        assert_eq!(diagnostics.len(), MAX_DIAGNOSTICS);
    }

    #[test]
    fn plain_text_requires_utf8_and_no_nul() {
        assert_valid(GuardFileFormat::PlainText, b"hello\n");
        assert_code(
            GuardFileFormat::PlainText,
            &[0xff, 0xfe],
            DiagnosticCode::InvalidUtf8,
        );
        assert_code(
            GuardFileFormat::PlainText,
            b"hello\0",
            DiagnosticCode::NulByte,
        );
    }

    #[test]
    fn diagnostics_do_not_echo_source_or_parser_text() {
        let diagnostics = validate_bytes(
            GuardFileFormat::Json,
            br#"{"TOP_SECRET":"/Users/eric/private"}"oops"#,
            "scope",
            Some("safe.json"),
        )
        .unwrap_err();
        let serialized = serde_json::to_string(&diagnostics).unwrap();
        assert!(!serialized.contains("TOP_SECRET"));
        assert!(!serialized.contains("/Users/eric/private"));
        assert!(!serialized.contains("oops"));
    }
}
