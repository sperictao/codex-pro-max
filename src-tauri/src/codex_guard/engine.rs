//! 引擎：比对期望状态与实际状态，并为目标文件生成纯内存写计划。
//! 所有物理文件先经过统一格式校验；校验失败时绝不重写文件。

use crate::i18n::{tr, trf};
use sha2::{Digest, Sha256};

use super::atomic_store::{AtomicFileWriter, PlatformAtomicFileWriter};
use super::backup::legacy_write_with_backup;
use super::format::{diagnostics_message, parse_toml_document, validate_bytes};
use super::journal::{self, JournalEntry, JournalEnvelope};
use super::markdown_block::{block_begin, block_end, extract_block, upsert_block};
use super::model::{DiagnosticCode, GuardFileFormat, ValidationDiagnostic};
use super::ownership::{normalize_toml_path, validate_target_path};
use super::schema::default_for_lang;
use super::toml_ops::{
    get_toml_path, json_to_toml, remove_toml_path, render_toml_value, set_toml_path,
    toml_matches_json,
};
use super::transaction::{
    recovery_action, RecoveryAction, TransactionError, TransactionErrorCode, TransactionPhase,
    TransactionState,
};
use super::validate::validate_param_for_file;
use super::{AppPaths, GuardParam, GuardParamState};

pub struct CheckResult {
    pub status: String, // match | drift | missing | error
    pub actual: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ManagedFile {
    pub(crate) relative_file: String,
    pub(crate) format: GuardFileFormat,
    pub(crate) original_exists: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct ManagedMember {
    pub(crate) id: String,
    pub(crate) apply_mode: String,
    pub(crate) path: String,
    pub(crate) value_type: String,
    pub(crate) expected: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PostCheck {
    pub(crate) id: String,
    pub(crate) apply_mode: String,
    pub(crate) path: String,
    pub(crate) expected: serde_json::Value,
}

#[derive(Debug, Clone)]
pub(crate) struct PlannedFileWrite {
    pub(crate) relative_file: String,
    pub(crate) format: GuardFileFormat,
    pub(crate) original_exists: bool,
    pub(crate) original_sha256: String,
    pub(crate) candidate: Vec<u8>,
    pub(crate) candidate_sha256: String,
    pub(crate) changed: bool,
    pub(crate) post_checks: Vec<PostCheck>,
}

#[derive(Debug, Clone)]
struct PreparedMember {
    id: String,
    apply_mode: String,
    path: String,
    expected: serde_json::Value,
}

fn ok(status: &str, actual: Option<String>) -> CheckResult {
    CheckResult {
        status: status.to_string(),
        actual,
        error: None,
    }
}

fn err(msg: String) -> CheckResult {
    CheckResult {
        status: "error".to_string(),
        actual: None,
        error: Some(msg),
    }
}

/// 纯内存写计划：不读取路径、不写文件，也不接触 ConfigStore。
pub(crate) fn plan_file_write(
    file: &ManagedFile,
    members: &[ManagedMember],
    original: &[u8],
) -> Result<PlannedFileWrite, Vec<ValidationDiagnostic>> {
    let prepared = prepare_members(file, members)?;
    let scope_id = prepared
        .first()
        .map(|member| member.id.as_str())
        .unwrap_or("guard-plan");

    let mut toml_document = None;
    let mut markdown_content = None;
    match file.format {
        GuardFileFormat::Toml => {
            toml_document = Some(parse_toml_document(
                original,
                scope_id,
                Some(&file.relative_file),
            )?);
        }
        GuardFileFormat::Markdown => {
            if file.original_exists {
                validate_bytes(file.format, original, scope_id, Some(&file.relative_file))?;
                markdown_content = Some(String::from_utf8(original.to_vec()).map_err(|_| {
                    vec![plan_diagnostic(
                        file,
                        Some(prepared[0].id.as_str()),
                        DiagnosticCode::InvalidUtf8,
                    )]
                })?);
            } else {
                markdown_content = Some(String::new());
            }
        }
        GuardFileFormat::Json | GuardFileFormat::PlainText => {
            if file.original_exists {
                validate_bytes(file.format, original, scope_id, Some(&file.relative_file))?;
            }
        }
    }

    let candidate = if prepared.len() == 1 && prepared[0].apply_mode == "file_overwrite" {
        overwrite_candidate(file, &prepared[0])?
    } else {
        match file.format {
            GuardFileFormat::Toml => {
                let Some(mut document) = toml_document else {
                    return Err(vec![plan_diagnostic(
                        file,
                        None,
                        DiagnosticCode::PlanConflict,
                    )]);
                };
                for member in &prepared {
                    match member.apply_mode.as_str() {
                        "toml_key" => {
                            let value = json_to_toml(&member.expected).map_err(|_| {
                                vec![plan_diagnostic(
                                    file,
                                    Some(member.id.as_str()),
                                    DiagnosticCode::PlanExpectedTypeMismatch,
                                )]
                            })?;
                            set_toml_path(&mut document, &member.path, value).map_err(|_| {
                                vec![plan_diagnostic(
                                    file,
                                    Some(member.id.as_str()),
                                    DiagnosticCode::PlanConflict,
                                )]
                            })?;
                        }
                        "toml_absent" => remove_toml_path(&mut document, &member.path),
                        _ => {
                            return Err(vec![plan_diagnostic(
                                file,
                                Some(member.id.as_str()),
                                DiagnosticCode::PlanModeIncompatible,
                            )])
                        }
                    }
                }
                document.to_string().into_bytes()
            }
            GuardFileFormat::Markdown => {
                let Some(mut content) = markdown_content else {
                    return Err(vec![plan_diagnostic(
                        file,
                        None,
                        DiagnosticCode::PlanConflict,
                    )]);
                };
                for member in &prepared {
                    let expected = member.expected.as_str().ok_or_else(|| {
                        vec![plan_diagnostic(
                            file,
                            Some(member.id.as_str()),
                            DiagnosticCode::PlanExpectedTypeMismatch,
                        )]
                    })?;
                    content = upsert_block(
                        &content,
                        &block_begin(&member.id),
                        &block_end(&member.id),
                        expected,
                    );
                }
                content.into_bytes()
            }
            GuardFileFormat::Json | GuardFileFormat::PlainText => {
                return Err(vec![plan_diagnostic(
                    file,
                    Some(prepared[0].id.as_str()),
                    DiagnosticCode::PlanModeIncompatible,
                )])
            }
        }
    };

    validate_bytes(file.format, &candidate, scope_id, Some(&file.relative_file))?;
    let original_sha256 = sha256_hex(original);
    let candidate_sha256 = sha256_hex(&candidate);
    let post_checks = prepared
        .into_iter()
        .map(|member| PostCheck {
            id: member.id,
            apply_mode: member.apply_mode,
            path: member.path,
            expected: member.expected,
        })
        .collect();
    Ok(PlannedFileWrite {
        relative_file: file.relative_file.clone(),
        format: file.format,
        original_exists: file.original_exists,
        changed: original_sha256 != candidate_sha256,
        original_sha256,
        candidate,
        candidate_sha256,
        post_checks,
    })
}

fn prepare_members(
    file: &ManagedFile,
    members: &[ManagedMember],
) -> Result<Vec<PreparedMember>, Vec<ValidationDiagnostic>> {
    if members.is_empty() {
        return Err(vec![plan_diagnostic(
            file,
            None,
            DiagnosticCode::PlanEmptyMembers,
        )]);
    }
    let mut prepared = Vec::with_capacity(members.len());
    let mut diagnostics = Vec::new();
    for member in members {
        let mut path = member.path.clone();
        let mode_known = matches!(
            member.apply_mode.as_str(),
            "toml_key" | "toml_absent" | "markdown_block" | "file_overwrite"
        );
        let mode_supported = match member.apply_mode.as_str() {
            "toml_key" | "toml_absent" => file.format == GuardFileFormat::Toml,
            "markdown_block" => file.format == GuardFileFormat::Markdown,
            "file_overwrite" => true,
            _ => false,
        };
        if !mode_known {
            diagnostics.push(plan_diagnostic(
                file,
                Some(member.id.as_str()),
                DiagnosticCode::PlanUnknownMode,
            ));
        } else if !mode_supported {
            diagnostics.push(plan_diagnostic(
                file,
                Some(member.id.as_str()),
                DiagnosticCode::PlanModeIncompatible,
            ));
        }
        if matches!(member.apply_mode.as_str(), "toml_key" | "toml_absent") {
            match normalize_toml_path(&member.path) {
                Ok(normalized) => path = normalized,
                Err(_) => diagnostics.push(plan_diagnostic(
                    file,
                    Some(member.id.as_str()),
                    DiagnosticCode::PlanInvalidPath,
                )),
            }
        }
        if !valid_value_type(&member.value_type)
            || !expected_matches(
                member.apply_mode.as_str(),
                &member.value_type,
                &member.expected,
            )
        {
            diagnostics.push(plan_diagnostic(
                file,
                Some(member.id.as_str()),
                DiagnosticCode::PlanExpectedTypeMismatch,
            ));
        }
        if mode_supported {
            prepared.push(PreparedMember {
                id: member.id.clone(),
                apply_mode: member.apply_mode.clone(),
                path,
                expected: member.expected.clone(),
            });
        }
    }
    if prepared
        .iter()
        .filter(|member| member.apply_mode == "file_overwrite")
        .count()
        > 1
        || (prepared
            .iter()
            .any(|member| member.apply_mode == "file_overwrite")
            && prepared.len() > 1)
    {
        diagnostics.push(plan_diagnostic(file, None, DiagnosticCode::PlanConflict));
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }
    prepared.sort_by(|left, right| {
        (&left.path, &left.id, &left.apply_mode).cmp(&(&right.path, &right.id, &right.apply_mode))
    });
    Ok(prepared)
}

fn overwrite_candidate(
    file: &ManagedFile,
    member: &PreparedMember,
) -> Result<Vec<u8>, Vec<ValidationDiagnostic>> {
    let Some(expected) = member.expected.as_str() else {
        return Err(vec![plan_diagnostic(
            file,
            Some(member.id.as_str()),
            DiagnosticCode::PlanExpectedTypeMismatch,
        )]);
    };
    let mut candidate = expected.trim().to_string();
    candidate.push('\n');
    Ok(candidate.into_bytes())
}

fn expected_matches(apply_mode: &str, value_type: &str, expected: &serde_json::Value) -> bool {
    match apply_mode {
        "toml_absent" => value_type == "none" && expected.is_null(),
        "toml_key" => match value_type {
            "bool" => expected.is_boolean(),
            "int" => expected.as_i64().is_some(),
            "string" | "text" => expected.is_string(),
            _ => false,
        },
        "file_overwrite" | "markdown_block" => {
            matches!(value_type, "string" | "text") && expected.is_string()
        }
        _ => false,
    }
}

fn valid_value_type(value_type: &str) -> bool {
    matches!(value_type, "bool" | "int" | "string" | "text" | "none")
}

fn plan_diagnostic(
    file: &ManagedFile,
    member_id: Option<&str>,
    code: DiagnosticCode,
) -> ValidationDiagnostic {
    ValidationDiagnostic::new(
        member_id.unwrap_or(file.relative_file.as_str()),
        Some(&file.relative_file),
        code,
        None,
        None,
    )
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// 比对同一物理文件中的多个参数；文件只读取一次，格式只解析一次。
pub(crate) fn check_many(
    paths: &AppPaths,
    relative_file: &str,
    format: GuardFileFormat,
    members: &[(&GuardParam, &serde_json::Value)],
) -> Vec<CheckResult> {
    if members.is_empty() {
        return Vec::new();
    }

    let mut results = Vec::with_capacity(members.len());
    let mut valid_members = Vec::with_capacity(members.len());
    for (param, expected) in members {
        match validate_mode_format(param, format) {
            Ok(()) => {
                results.push(None);
                valid_members.push((*param, *expected));
            }
            Err(error) => {
                results.push(Some(err(error)));
            }
        }
    }
    if valid_members.is_empty() {
        return results
            .into_iter()
            .map(|result| result.expect("invalid members have a result"))
            .collect();
    }

    let file = paths.codex_file(relative_file);
    let content = match read_existing(&file) {
        Ok(Some(content)) => content,
        Ok(None) => {
            return members
                .iter()
                .zip(results)
                .map(|((param, _), result)| result.unwrap_or_else(|| missing_result(param)))
                .collect()
        }
        Err(error) => {
            return results
                .into_iter()
                .map(|result| result.unwrap_or_else(|| err(error.clone())))
                .collect()
        }
    };

    let first_scope = valid_members[0].0.id.as_str();
    let toml_document = match format {
        GuardFileFormat::Toml => {
            match parse_toml_document(&content, first_scope, Some(relative_file)) {
                Ok(document) => Some(document),
                Err(diagnostics) => {
                    let error = format_validation_error(&diagnostics);
                    return results
                        .into_iter()
                        .map(|result| result.unwrap_or_else(|| err(error.clone())))
                        .collect();
                }
            }
        }
        _ => {
            if let Err(diagnostics) =
                validate_bytes(format, &content, first_scope, Some(relative_file))
            {
                let error = format_validation_error(&diagnostics);
                return results
                    .into_iter()
                    .map(|result| result.unwrap_or_else(|| err(error.clone())))
                    .collect();
            }
            None
        }
    };

    let mut valid_results = valid_members
        .into_iter()
        .map(|(param, expected)| check_loaded(param, expected, &content, toml_document.as_ref()))
        .collect::<Vec<_>>()
        .into_iter();
    results
        .into_iter()
        .map(|result| match result {
            Some(result) => result,
            None => valid_results
                .next()
                .expect("valid members and results have the same length"),
        })
        .collect()
}

/// 比对某参数的期望状态与实际状态。格式解析失败只报结构化摘要，绝不重写文件。
pub(crate) fn check(
    paths: &AppPaths,
    param: &GuardParam,
    format: GuardFileFormat,
    expected: &serde_json::Value,
) -> CheckResult {
    let relative_file = match validate_target_path(paths, &param.file) {
        Ok(relative_file) => relative_file,
        Err(error) => return err(error.to_string()),
    };
    check_many(paths, &relative_file, format, &[(param, expected)])
        .into_iter()
        .next()
        .unwrap_or_else(|| err(tr("Guard check received no members")))
}

fn check_loaded(
    param: &GuardParam,
    expected: &serde_json::Value,
    content: &[u8],
    toml_document: Option<&toml_edit::DocumentMut>,
) -> CheckResult {
    match param.apply_mode.as_str() {
        "toml_key" => {
            let Some(doc) = toml_document else {
                return err(tr("Guard file format does not support TOML paths"));
            };
            let path = match normalize_toml_path(&param.path) {
                Ok(path) => path,
                Err(_) => return err(tr("Invalid TOML path")),
            };
            match get_toml_path(doc, &path) {
                None => ok("missing", Some(tr("(not set)"))),
                Some(item) if toml_matches_json(item, expected) => {
                    ok("match", Some(render_toml_value(item)))
                }
                Some(item) => ok("drift", Some(render_toml_value(item))),
            }
        }
        "toml_absent" => {
            let Some(doc) = toml_document else {
                return err(tr("Guard file format does not support TOML paths"));
            };
            let path = match normalize_toml_path(&param.path) {
                Ok(path) => path,
                Err(_) => return err(tr("Invalid TOML path")),
            };
            if get_toml_path(doc, &path).is_some() {
                ok("drift", Some(tr("present")))
            } else {
                ok("match", Some(tr("absent")))
            }
        }
        "file_overwrite" => {
            let expected = match expected_text(expected) {
                Ok(value) => value,
                Err(error) => return err(error),
            };
            let content = match String::from_utf8(content.to_vec()) {
                Ok(content) => content,
                Err(_) => return err(tr("Guard file is not valid UTF-8")),
            };
            let mut candidate = expected.trim().to_string();
            candidate.push('\n');
            if content.as_bytes() == candidate.as_bytes() {
                ok(
                    "match",
                    Some(trf("{n} bytes", &[("n", content.len().to_string())])),
                )
            } else {
                ok(
                    "drift",
                    Some(trf(
                        "{n} bytes, content differs",
                        &[("n", content.len().to_string())],
                    )),
                )
            }
        }
        "markdown_block" => {
            let content = match String::from_utf8(content.to_vec()) {
                Ok(content) => content,
                Err(_) => return err(tr("Guard file is not valid UTF-8")),
            };
            let expected = match expected_text(expected) {
                Ok(value) => value,
                Err(error) => return err(error),
            };
            match extract_block(&content, &block_begin(&param.id), &block_end(&param.id)) {
                None => ok("missing", Some(tr("(managed block does not exist)"))),
                Some(block) if block == expected.trim() => ok("match", Some(tr("block matches"))),
                Some(_) => ok("drift", Some(tr("block content differs"))),
            }
        }
        other => err(trf(
            "Unknown apply_mode: {mode}",
            &[("mode", other.to_string())],
        )),
    }
}

/// 通过纯计划器生成单文件候选，再交给单文件事务边界。
///
/// 目前这是单文件事务：journal/快照先 durable，写前复核原始 hash，候选通过原子
/// replace 后再做 post-check；跨文件 coordinator、启动恢复和 no-follow 句柄语义仍待
/// 后续步骤接入。
pub(crate) fn execute_single_plan(
    paths: &AppPaths,
    param: &GuardParam,
    format: GuardFileFormat,
    expected: &serde_json::Value,
) -> Result<(), String> {
    validate_mode_format(param, format)?;
    let relative_file =
        validate_target_path(paths, &param.file).map_err(|error| error.to_string())?;
    let file = paths.codex_file(&relative_file);
    let original = read_existing(&file)?;
    let managed_file = ManagedFile {
        relative_file: relative_file.clone(),
        format,
        original_exists: original.is_some(),
    };
    let member = ManagedMember {
        id: param.id.clone(),
        apply_mode: param.apply_mode.clone(),
        path: param.path.clone(),
        value_type: param.value_type.clone(),
        expected: expected.clone(),
    };
    let plan = plan_file_write(
        &managed_file,
        std::slice::from_ref(&member),
        original.as_deref().unwrap_or_default(),
    )
    .map_err(|diagnostics| format_validation_error(&diagnostics))?;
    let original_bytes = original.as_deref().unwrap_or_default();
    let plan_is_consistent = plan.relative_file == relative_file
        && plan.format == format
        && plan.original_exists == original.is_some()
        && plan.original_sha256 == sha256_hex(original_bytes)
        && plan.candidate_sha256 == sha256_hex(&plan.candidate)
        && plan.post_checks.len() == 1
        && plan.post_checks[0].id == param.id;
    if !plan_is_consistent {
        return Err(tr("Guard write plan is inconsistent"));
    }
    if !plan.changed {
        return Ok(());
    }

    let batch_id = journal::new_batch_id();
    let snapshot_ref = "snapshots/entry-0.bin";
    let mut journal = JournalEnvelope::new(
        batch_id.clone(),
        vec![JournalEntry {
            relative_file: relative_file.clone(),
            original_exists: plan.original_exists,
            original_sha256: plan.original_sha256.clone(),
            candidate_sha256: plan.candidate_sha256.clone(),
            snapshot_ref: snapshot_ref.to_string(),
            completed: false,
            post_checked: false,
            restored: false,
        }],
    );
    let transaction_root = paths.transaction_root();
    journal::write_journal(transaction_root, &journal).map_err(transaction_error_message)?;

    if let Err(error) =
        journal::write_snapshot(transaction_root, &batch_id, snapshot_ref, original_bytes)
    {
        return Err(mark_transaction_critical(
            transaction_root,
            &mut journal,
            &mut TransactionState::new(),
            error.code,
        ));
    }

    let mut state = TransactionState::new();
    transition_and_persist(
        transaction_root,
        &mut journal,
        &mut state,
        TransactionPhase::Snapshot,
    )
    .map_err(transaction_error_message)?;

    let precondition_matches =
        match target_matches(&file, plan.original_exists, &plan.original_sha256) {
            Ok(matches) => matches,
            Err(_) => {
                return Err(mark_transaction_critical(
                    transaction_root,
                    &mut journal,
                    &mut state,
                    TransactionErrorCode::ReadFailed,
                ))
            }
        };
    if !precondition_matches {
        return Err(mark_transaction_critical(
            transaction_root,
            &mut journal,
            &mut state,
            TransactionErrorCode::IdentityChanged,
        ));
    }

    transition_and_persist(
        transaction_root,
        &mut journal,
        &mut state,
        TransactionPhase::Writing,
    )
    .map_err(transaction_error_message)?;

    if let Err(_error) = legacy_write_with_backup(paths, &relative_file, &file, &plan.candidate) {
        return restore_after_failure(
            paths,
            &file,
            transaction_root,
            &mut journal,
            &mut state,
            TransactionErrorCode::ReplaceFailed,
        );
    }
    journal.entries[0].completed = true;
    transition_and_persist(
        transaction_root,
        &mut journal,
        &mut state,
        TransactionPhase::PostCheck,
    )
    .map_err(transaction_error_message)?;

    let postcondition_matches = match target_matches(&file, true, &plan.candidate_sha256) {
        Ok(matches) => matches,
        Err(_) => {
            return restore_after_failure(
                paths,
                &file,
                transaction_root,
                &mut journal,
                &mut state,
                TransactionErrorCode::ReadFailed,
            )
        }
    };
    if !postcondition_matches {
        return restore_after_failure(
            paths,
            &file,
            transaction_root,
            &mut journal,
            &mut state,
            TransactionErrorCode::PostCheckFailed,
        );
    }
    journal.entries[0].post_checked = true;
    transition_and_persist(
        transaction_root,
        &mut journal,
        &mut state,
        TransactionPhase::Completed,
    )
    .map_err(transaction_error_message)?;
    journal.commit_marker = true;
    journal::write_journal(transaction_root, &journal).map_err(transaction_error_message)?;
    journal::cleanup_batch(transaction_root, &batch_id).map_err(transaction_error_message)?;
    Ok(())
}

fn transition_and_persist(
    root: &std::path::Path,
    journal: &mut JournalEnvelope,
    state: &mut TransactionState,
    next: TransactionPhase,
) -> Result<(), TransactionError> {
    state.transition(next)?;
    journal.phase = next;
    journal::write_journal(root, journal).map(|_| ())
}

fn target_matches(
    target: &std::path::Path,
    expected_exists: bool,
    expected_sha256: &str,
) -> Result<bool, String> {
    let current = read_existing(target)?;
    Ok(content_matches(
        current.as_deref(),
        expected_exists,
        expected_sha256,
    ))
}

fn restore_after_failure(
    paths: &AppPaths,
    target: &std::path::Path,
    root: &std::path::Path,
    journal: &mut JournalEnvelope,
    state: &mut TransactionState,
    failure: TransactionErrorCode,
) -> Result<(), String> {
    transition_and_persist(root, journal, state, TransactionPhase::Restoring)
        .map_err(transaction_error_message)?;
    let entry = &journal.entries[0];
    let snapshot = match journal::read_snapshot(root, &journal.batch_id, &entry.snapshot_ref) {
        Ok(snapshot) => snapshot,
        Err(error) => return Err(mark_transaction_critical(root, journal, state, error.code)),
    };
    let restore_result = restore_entry_snapshot(target, entry.original_exists, &snapshot);
    if let Err(error) = restore_result {
        return Err(mark_transaction_critical(root, journal, state, error.code));
    }
    journal.entries[0].restored = true;
    transition_and_persist(root, journal, state, TransactionPhase::Completed)
        .map_err(transaction_error_message)?;
    journal.commit_marker = true;
    journal::write_journal(root, journal).map_err(transaction_error_message)?;
    journal::cleanup_batch(root, &journal.batch_id).map_err(transaction_error_message)?;
    let _ = paths;
    Err(transaction_error_message(TransactionError::new(failure)))
}

/// 启动时处理上一次单文件事务留下的 durable journal。若当前内容既不是原始值也
/// 不是候选值，则 fail closed，保留 journal 供人工/后续 recovery 处理，不覆盖外部修改。
pub(crate) fn recover_pending_transactions(paths: &AppPaths) -> Result<(), String> {
    let root = paths.transaction_root();
    let batches = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => {
            return Err(transaction_error_message(TransactionError::new(
                TransactionErrorCode::JournalIo,
            )))
        }
    };
    for batch in batches {
        let batch = batch.map_err(|_| {
            transaction_error_message(TransactionError::new(TransactionErrorCode::JournalIo))
        })?;
        if !batch
            .file_type()
            .map_err(|_| {
                transaction_error_message(TransactionError::new(TransactionErrorCode::JournalIo))
            })?
            .is_dir()
        {
            continue;
        }
        let batch_id = batch.file_name();
        let Some(batch_id) = batch_id.to_str() else {
            return Err(transaction_error_message(TransactionError::new(
                TransactionErrorCode::JournalSchema,
            )));
        };
        let journal_path = batch.path().join("journal.json");
        let mut journal =
            journal::read_journal(&journal_path).map_err(transaction_error_message)?;
        if journal.batch_id != batch_id {
            return Err(transaction_error_message(TransactionError::new(
                TransactionErrorCode::JournalSchema,
            )));
        }
        match recovery_action(&journal) {
            RecoveryAction::CleanupCompleted => {
                journal::cleanup_batch(root, batch_id).map_err(transaction_error_message)?;
            }
            RecoveryAction::Critical => {
                return Err(transaction_error_message(TransactionError::new(
                    TransactionErrorCode::RestoreFailedCritical,
                )));
            }
            RecoveryAction::RestorePending => {
                recover_one_batch(paths, &mut journal)?;
            }
        }
    }
    Ok(())
}

fn recover_one_batch(paths: &AppPaths, journal: &mut JournalEnvelope) -> Result<(), String> {
    let root = paths.transaction_root();
    journal.phase = TransactionPhase::Restoring;
    journal.critical = false;
    journal::write_journal(root, journal).map_err(transaction_error_message)?;
    let batch_id = journal.batch_id.clone();
    for index in 0..journal.entries.len() {
        let (relative_file, original_exists, original_sha256, candidate_sha256, snapshot_ref) = {
            let entry = &journal.entries[index];
            (
                entry.relative_file.clone(),
                entry.original_exists,
                entry.original_sha256.clone(),
                entry.candidate_sha256.clone(),
                entry.snapshot_ref.clone(),
            )
        };
        let target = paths.codex_file(&relative_file);
        let current = match read_existing(&target) {
            Ok(current) => current,
            Err(_) => {
                journal.phase = TransactionPhase::Critical;
                journal.critical = true;
                let _ = journal::write_journal(root, journal);
                return Err(transaction_error_message(TransactionError::new(
                    TransactionErrorCode::ReadFailed,
                )));
            }
        };
        let is_original = content_matches(current.as_deref(), original_exists, &original_sha256);
        let is_candidate = content_matches(current.as_deref(), true, &candidate_sha256);
        if !is_original && !is_candidate {
            journal.phase = TransactionPhase::Critical;
            journal.critical = true;
            let _ = journal::write_journal(root, journal);
            return Err(transaction_error_message(TransactionError::new(
                TransactionErrorCode::IdentityChanged,
            )));
        }
        if !is_original {
            let snapshot = journal::read_snapshot(root, &batch_id, &snapshot_ref)
                .map_err(transaction_error_message)?;
            if let Err(error) = restore_entry_snapshot(&target, original_exists, &snapshot) {
                journal.phase = TransactionPhase::Critical;
                journal.critical = true;
                let _ = journal::write_journal(root, journal);
                return Err(transaction_error_message(error));
            }
        }
        journal.entries[index].restored = true;
    }
    journal.phase = TransactionPhase::Completed;
    journal.commit_marker = true;
    journal::write_journal(root, journal).map_err(transaction_error_message)?;
    journal::cleanup_batch(root, &batch_id).map_err(transaction_error_message)
}

fn restore_entry_snapshot(
    target: &std::path::Path,
    original_exists: bool,
    snapshot: &[u8],
) -> Result<(), TransactionError> {
    if original_exists {
        PlatformAtomicFileWriter
            .replace(target, snapshot)
            .map_err(|_| TransactionError::new(TransactionErrorCode::RestoreFailedCritical))
    } else {
        remove_new_target(target)
    }
}

fn content_matches(current: Option<&[u8]>, expected_exists: bool, expected_sha256: &str) -> bool {
    match (expected_exists, current) {
        (false, None) => true,
        (true, Some(bytes)) => sha256_hex(bytes) == expected_sha256,
        _ => false,
    }
}

fn remove_new_target(target: &std::path::Path) -> Result<(), TransactionError> {
    match std::fs::symlink_metadata(target) {
        Ok(metadata) if metadata.file_type().is_symlink() || metadata.is_dir() => Err(
            TransactionError::new(TransactionErrorCode::RestoreFailedCritical),
        ),
        Ok(_) => std::fs::remove_file(target)
            .map_err(|_| TransactionError::new(TransactionErrorCode::RestoreFailedCritical)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(TransactionError::new(
            TransactionErrorCode::RestoreFailedCritical,
        )),
    }
}

fn mark_transaction_critical(
    root: &std::path::Path,
    journal: &mut JournalEnvelope,
    state: &mut TransactionState,
    code: TransactionErrorCode,
) -> String {
    let _ = state.mark_critical();
    journal.phase = TransactionPhase::Critical;
    journal.critical = true;
    let _ = journal::write_journal(root, journal);
    transaction_error_message(TransactionError::new(code))
}

fn transaction_error_message(error: TransactionError) -> String {
    format!("guard transaction failed: {}", error.code.as_str())
}

fn read_existing(file: &std::path::Path) -> Result<Option<Vec<u8>>, String> {
    match std::fs::read(file) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(trf("Read failed: {error}", &[("error", error.to_string())])),
    }
}

fn validate_mode_format(param: &GuardParam, format: GuardFileFormat) -> Result<(), String> {
    validate_param_for_file(param, format)
}

fn expected_text(expected: &serde_json::Value) -> Result<&str, String> {
    expected
        .as_str()
        .ok_or_else(|| tr("Expected value must be text for this apply mode"))
}

fn format_validation_error(diagnostics: &[ValidationDiagnostic]) -> String {
    trf(
        "Guard file validation failed: {diagnostic}",
        &[("diagnostic", diagnostics_message(diagnostics))],
    )
}

fn missing_result(param: &GuardParam) -> CheckResult {
    match param.apply_mode.as_str() {
        "toml_absent" => ok("match", Some(tr("absent"))),
        "toml_key" | "file_overwrite" | "markdown_block" => {
            ok("missing", Some(tr("(file does not exist)")))
        }
        _ => err(trf(
            "Unknown apply_mode: {mode}",
            &[("mode", param.apply_mode.clone())],
        )),
    }
}

/// 期望值计算：用户改过的值永远优先；否则期望值随界面语言（带 default_en 的参数）。
pub(crate) fn expected_of(
    param: &GuardParam,
    state: Option<&GuardParamState>,
) -> serde_json::Value {
    state
        .and_then(|s| s.value.clone())
        .unwrap_or_else(|| default_for_lang(param, crate::i18n::current()).clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn param(file: &str, apply_mode: &str, path: &str, value_type: &str) -> GuardParam {
        GuardParam {
            id: "custom.test".into(),
            label: "测试".into(),
            label_en: String::new(),
            description: String::new(),
            description_en: String::new(),
            file: file.into(),
            apply_mode: apply_mode.into(),
            path: path.into(),
            value_type: value_type.into(),
            default: serde_json::Value::Null,
            default_en: serde_json::Value::Null,
            custom: true,
        }
    }

    #[test]
    fn execute_plan_rejects_directory_without_treating_it_as_an_empty_document() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());
        let target = paths.codex_file("config.toml");
        std::fs::create_dir_all(&target).unwrap();
        let result = execute_single_plan(
            &paths,
            &param("config.toml", "toml_key", "features.enabled", "bool"),
            GuardFileFormat::Toml,
            &serde_json::json!(true),
        );
        assert!(result.is_err());
        assert!(target.is_dir());
    }

    #[test]
    fn execute_plan_rejects_invalid_toml_before_writing() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());
        let target = paths.codex_file("config.toml");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        let original = b"[features\nthis is invalid\n";
        std::fs::write(&target, original).unwrap();
        let result = execute_single_plan(
            &paths,
            &param("config.toml", "toml_key", "features.enabled", "bool"),
            GuardFileFormat::Toml,
            &serde_json::json!(true),
        );
        assert!(result.is_err());
        assert_eq!(std::fs::read(&target).unwrap(), original);
    }

    #[test]
    fn execute_plan_allows_missing_toml_only_as_an_explicit_empty_document() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());
        let param = param("config.toml", "toml_key", "features.enabled", "bool");
        execute_single_plan(
            &paths,
            &param,
            GuardFileFormat::Toml,
            &serde_json::json!(true),
        )
        .unwrap();
        let content = std::fs::read_to_string(paths.codex_file("config.toml")).unwrap();
        let document = content.parse::<toml_edit::DocumentMut>().unwrap();
        assert!(toml_matches_json(
            get_toml_path(&document, "features.enabled").unwrap(),
            &serde_json::json!(true),
        ));
    }

    #[test]
    fn check_reports_format_code_without_echoing_json_content() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());
        let target = paths.codex_file("settings.json");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, br#"{"TOP_SECRET":1,"TOP_SECRET":2}"#).unwrap();
        let result = check(
            &paths,
            &param("settings.json", "file_overwrite", "", "text"),
            GuardFileFormat::Json,
            &serde_json::json!("{}"),
        );
        let error = result.error.unwrap();
        assert!(error.contains("json_duplicate_key"));
        assert!(!error.contains("TOP_SECRET"));
    }

    #[test]
    fn check_many_preserves_member_order_for_one_toml_parse() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());
        let target = paths.codex_file("config.toml");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, "[features]\nalpha = true\nbeta = false\n").unwrap();
        let mut alpha = param("config.toml", "toml_key", "features.alpha", "bool");
        alpha.id = "alpha".into();
        let mut beta = param("config.toml", "toml_key", "features.beta", "bool");
        beta.id = "beta".into();
        let expected_alpha = serde_json::json!(true);
        let expected_beta = serde_json::json!(true);
        let results = check_many(
            &paths,
            "config.toml",
            GuardFileFormat::Toml,
            &[(&alpha, &expected_alpha), (&beta, &expected_beta)],
        );
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].status, "match");
        assert_eq!(results[1].status, "drift");
    }

    fn member(
        id: &str,
        apply_mode: &str,
        path: &str,
        value_type: &str,
        expected: serde_json::Value,
    ) -> ManagedMember {
        ManagedMember {
            id: id.into(),
            apply_mode: apply_mode.into(),
            path: path.into(),
            value_type: value_type.into(),
            expected,
        }
    }

    fn file(relative_file: &str, format: GuardFileFormat, original_exists: bool) -> ManagedFile {
        ManagedFile {
            relative_file: relative_file.into(),
            format,
            original_exists,
        }
    }

    #[test]
    fn plan_merges_toml_members_once_and_is_order_independent() {
        let managed = file("config.toml", GuardFileFormat::Toml, true);
        let members = vec![
            member(
                "features.multi_agent_v2.enabled",
                "toml_key",
                "features.multi_agent_v2.enabled",
                "bool",
                serde_json::json!(true),
            ),
            member(
                "features.multi_agent_v2.hide_spawn_agent_metadata",
                "toml_key",
                "features.multi_agent_v2.hide_spawn_agent_metadata",
                "bool",
                serde_json::json!(true),
            ),
            member(
                "features.multi_agent_v2.tool_namespace",
                "toml_key",
                "features.multi_agent_v2.tool_namespace",
                "string",
                serde_json::json!("agents"),
            ),
            member(
                "features.multi_agent_v2.max_concurrent_threads_per_session",
                "toml_key",
                "features.multi_agent_v2.max_concurrent_threads_per_session",
                "int",
                serde_json::json!(7),
            ),
            member(
                "features.multi_agent_v2.min_wait_timeout_ms",
                "toml_key",
                "features.multi_agent_v2.min_wait_timeout_ms",
                "int",
                serde_json::json!(10000),
            ),
            member(
                "features.multi_agent_v2.default_wait_timeout_ms",
                "toml_key",
                "features.multi_agent_v2.default_wait_timeout_ms",
                "int",
                serde_json::json!(30000),
            ),
            member(
                "features.multi_agent_v2.max_wait_timeout_ms",
                "toml_key",
                "features.multi_agent_v2.max_wait_timeout_ms",
                "int",
                serde_json::json!(120000),
            ),
            member(
                "features.image_generation",
                "toml_key",
                "features.image_generation",
                "bool",
                serde_json::json!(false),
            ),
        ];
        let reversed = members.iter().cloned().rev().collect::<Vec<_>>();
        let original = b"# keep this comment\n";
        let plan = plan_file_write(&managed, &members, original).unwrap();
        let reversed_plan = plan_file_write(&managed, &reversed, original).unwrap();

        assert_eq!(plan.candidate, reversed_plan.candidate);
        assert_eq!(plan.candidate_sha256, reversed_plan.candidate_sha256);
        assert_eq!(plan.post_checks.len(), 8);
        assert_eq!(plan.post_checks, reversed_plan.post_checks);
        let candidate_document = String::from_utf8(plan.candidate.clone())
            .unwrap()
            .parse::<toml_edit::DocumentMut>()
            .unwrap();
        for member in &members {
            assert!(toml_matches_json(
                get_toml_path(&candidate_document, &member.path).unwrap(),
                &member.expected,
            ));
        }
        assert!(plan.changed);
        assert!(String::from_utf8(plan.candidate)
            .unwrap()
            .contains("# keep this comment"));
    }

    #[test]
    fn plan_merges_markdown_blocks_and_preserves_unmanaged_text() {
        let managed = file("AGENTS.md", GuardFileFormat::Markdown, true);
        let members = vec![
            member("zeta", "markdown_block", "", "text", serde_json::json!("Z")),
            member(
                "alpha",
                "markdown_block",
                "",
                "text",
                serde_json::json!("A"),
            ),
        ];
        let original = b"# User notes\n\nKeep this paragraph.\n";
        let plan = plan_file_write(&managed, &members, original).unwrap();
        let candidate = String::from_utf8(plan.candidate).unwrap();

        assert!(candidate.contains("Keep this paragraph."));
        assert!(candidate.contains("<!-- dashi:begin alpha -->\nA\n<!-- dashi:end alpha -->"));
        assert!(candidate.contains("<!-- dashi:begin zeta -->\nZ\n<!-- dashi:end zeta -->"));
        assert!(candidate.find("begin alpha").unwrap() < candidate.find("begin zeta").unwrap());
        assert_eq!(plan.post_checks.len(), 2);
    }

    #[test]
    fn plan_rejects_semantic_errors_before_mutating_candidate() {
        let managed = file("config.toml", GuardFileFormat::Toml, true);
        let invalid_mode = vec![member(
            "bad-mode",
            "unknown",
            "features.enabled",
            "bool",
            serde_json::json!(true),
        )];
        let diagnostics = plan_file_write(&managed, &invalid_mode, b"").unwrap_err();
        assert_eq!(diagnostics[0].code, DiagnosticCode::PlanUnknownMode);

        let invalid_value = vec![member(
            "bad-value",
            "toml_key",
            "features.enabled",
            "bool",
            serde_json::json!("true"),
        )];
        let diagnostics = plan_file_write(&managed, &invalid_value, b"").unwrap_err();
        assert_eq!(
            diagnostics[0].code,
            DiagnosticCode::PlanExpectedTypeMismatch
        );

        let invalid_absent = vec![member(
            "bad-absent",
            "toml_absent",
            "agents",
            "bool",
            serde_json::json!(true),
        )];
        let diagnostics = plan_file_write(&managed, &invalid_absent, b"").unwrap_err();
        assert_eq!(
            diagnostics[0].code,
            DiagnosticCode::PlanExpectedTypeMismatch
        );

        let invalid_path = vec![member(
            "bad-path",
            "toml_key",
            "features..enabled",
            "bool",
            serde_json::json!(true),
        )];
        let diagnostics = plan_file_write(&managed, &invalid_path, b"").unwrap_err();
        assert_eq!(diagnostics[0].code, DiagnosticCode::PlanInvalidPath);
    }

    #[test]
    fn plan_validates_original_and_candidate_without_writing() {
        let managed = file("config.toml", GuardFileFormat::Toml, true);
        let member = member(
            "enabled",
            "toml_key",
            "features.enabled",
            "bool",
            serde_json::json!(true),
        );
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("config.toml");
        let original = b"[features]\nenabled = false\n";
        std::fs::write(&target, original).unwrap();
        let plan = plan_file_write(&managed, &[member], original).unwrap();

        assert_ne!(plan.candidate, original);
        assert_eq!(std::fs::read(&target).unwrap(), original);
    }

    #[test]
    fn plan_missing_toml_can_create_and_missing_absent_is_noop() {
        let key_file = file("config.toml", GuardFileFormat::Toml, false);
        let key = member(
            "enabled",
            "toml_key",
            "features.enabled",
            "bool",
            serde_json::json!(true),
        );
        let key_plan = plan_file_write(&key_file, &[key], b"").unwrap();
        assert!(key_plan.changed);
        assert!(key_plan.original_sha256 == sha256_hex(b""));

        let absent = member(
            "agents",
            "toml_absent",
            "agents",
            "none",
            serde_json::Value::Null,
        );
        let absent_plan = plan_file_write(&key_file, &[absent], b"").unwrap();
        assert!(!absent_plan.changed);
        assert!(absent_plan.candidate.is_empty());
    }

    #[test]
    fn recovery_restores_candidate_after_interrupted_single_file_write() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());
        let target = paths.codex_file("config.toml");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        let original = b"[features]\nenabled = false\n";
        let candidate = b"[features]\nenabled = true\n";
        std::fs::write(&target, candidate).unwrap();
        let batch_id = "batch-recovery";
        let snapshot_ref = "snapshots/entry-0.bin";
        let mut journal = JournalEnvelope::new(
            batch_id.into(),
            vec![JournalEntry {
                relative_file: "config.toml".into(),
                original_exists: true,
                original_sha256: sha256_hex(original),
                candidate_sha256: sha256_hex(candidate),
                snapshot_ref: snapshot_ref.into(),
                completed: true,
                post_checked: false,
                restored: false,
            }],
        );
        journal.phase = TransactionPhase::Writing;
        journal::write_journal(paths.transaction_root(), &journal).unwrap();
        journal::write_snapshot(paths.transaction_root(), batch_id, snapshot_ref, original)
            .unwrap();

        recover_pending_transactions(&paths).unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), original);
        assert!(!paths.transaction_root().join(batch_id).exists());
    }

    #[test]
    fn recovery_refuses_unknown_external_content_and_keeps_journal_critical() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());
        let target = paths.codex_file("config.toml");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        let original = b"original";
        let candidate = b"candidate";
        let external = b"external edit";
        std::fs::write(&target, external).unwrap();
        let batch_id = "batch-critical";
        let snapshot_ref = "snapshots/entry-0.bin";
        let mut journal = JournalEnvelope::new(
            batch_id.into(),
            vec![JournalEntry {
                relative_file: "config.toml".into(),
                original_exists: true,
                original_sha256: sha256_hex(original),
                candidate_sha256: sha256_hex(candidate),
                snapshot_ref: snapshot_ref.into(),
                completed: true,
                post_checked: false,
                restored: false,
            }],
        );
        journal.phase = TransactionPhase::Writing;
        journal::write_journal(paths.transaction_root(), &journal).unwrap();
        journal::write_snapshot(paths.transaction_root(), batch_id, snapshot_ref, original)
            .unwrap();

        let error = recover_pending_transactions(&paths).unwrap_err();

        assert!(error.contains("identity_changed"));
        assert_eq!(std::fs::read(&target).unwrap(), external);
        let saved =
            journal::read_journal(&paths.transaction_root().join(batch_id).join("journal.json"))
                .unwrap();
        assert!(saved.critical);
        assert_eq!(saved.phase, TransactionPhase::Critical);
    }
}
