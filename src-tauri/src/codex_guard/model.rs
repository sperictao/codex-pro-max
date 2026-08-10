//! Guard 领域模型：物理文件格式与结构化校验诊断。

use serde::{Deserialize, Serialize};
use std::fmt;

/// 看守文件的物理解析格式。序列化值是稳定的 snake_case 字符串。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum GuardFileFormat {
    Toml,
    Json,
    #[serde(alias = "md")]
    Markdown,
    PlainText,
}

impl GuardFileFormat {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Toml => "toml",
            Self::Json => "json",
            Self::Markdown => "markdown",
            Self::PlainText => "plain_text",
        }
    }
}

impl fmt::Display for GuardFileFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 稳定的、不会携带解析器原文的诊断代码。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DiagnosticCode {
    InvalidUtf8,
    NulByte,
    TomlInvalid,
    JsonInvalid,
    JsonDuplicateKey,
    MarkdownMalformedMarker,
    MarkdownDuplicateMarker,
    MarkdownCrossingMarker,
    MarkdownUnmatchedMarker,
    MarkdownUnclosedMarker,
    PlanEmptyMembers,
    PlanUnknownMode,
    PlanModeIncompatible,
    PlanInvalidPath,
    PlanExpectedTypeMismatch,
    PlanConflict,
}

impl DiagnosticCode {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidUtf8 => "invalid_utf8",
            Self::NulByte => "nul_byte",
            Self::TomlInvalid => "toml_invalid",
            Self::JsonInvalid => "json_invalid",
            Self::JsonDuplicateKey => "json_duplicate_key",
            Self::MarkdownMalformedMarker => "markdown_malformed_marker",
            Self::MarkdownDuplicateMarker => "markdown_duplicate_marker",
            Self::MarkdownCrossingMarker => "markdown_crossing_marker",
            Self::MarkdownUnmatchedMarker => "markdown_unmatched_marker",
            Self::MarkdownUnclosedMarker => "markdown_unclosed_marker",
            Self::PlanEmptyMembers => "plan_empty_members",
            Self::PlanUnknownMode => "plan_unknown_mode",
            Self::PlanModeIncompatible => "plan_mode_incompatible",
            Self::PlanInvalidPath => "plan_invalid_path",
            Self::PlanExpectedTypeMismatch => "plan_expected_type_mismatch",
            Self::PlanConflict => "plan_conflict",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DiagnosticSeverity {
    Error,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiagnosticParams {
    pub expected_format: Option<GuardFileFormat>,
}

/// 传给 UI/审计层的结构化诊断；解析器错误文本和原文片段不得进入此对象。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ValidationDiagnostic {
    pub code: DiagnosticCode,
    pub severity: DiagnosticSeverity,
    pub scope_id: String,
    pub relative_file: Option<String>,
    pub field: Option<String>,
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub params: DiagnosticParams,
}

impl ValidationDiagnostic {
    pub(crate) fn new(
        scope_id: &str,
        relative_file: Option<&str>,
        code: DiagnosticCode,
        line: Option<u32>,
        column: Option<u32>,
    ) -> Self {
        Self {
            code,
            severity: DiagnosticSeverity::Error,
            scope_id: scope_id.to_string(),
            relative_file: relative_file.map(str::to_string),
            field: None,
            line,
            column,
            params: DiagnosticParams::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::GuardFileFormat;

    #[test]
    fn legacy_md_deserializes_but_serializes_canonically() {
        let format: GuardFileFormat = serde_json::from_str("\"md\"").unwrap();
        assert_eq!(format, GuardFileFormat::Markdown);
        assert_eq!(serde_json::to_string(&format).unwrap(), "\"markdown\"");
    }

    #[test]
    fn all_file_formats_have_stable_wire_names() {
        let formats = [
            (GuardFileFormat::Toml, "\"toml\""),
            (GuardFileFormat::Json, "\"json\""),
            (GuardFileFormat::Markdown, "\"markdown\""),
            (GuardFileFormat::PlainText, "\"plain_text\""),
        ];
        for (format, expected) in formats {
            assert_eq!(serde_json::to_string(&format).unwrap(), expected);
        }
    }
}
