use anyhow::Context;
use good_tools_app::gui::state::{
    Lang, LogEntry, LogSource, LogStore, NativeException, NativeMemoryOperation, TaskKind, UiError,
    UiText,
};

#[derive(Debug)]
struct ChainedError(std::io::Error);

impl std::fmt::Display for ChainedError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("outer standard error")
    }
}

impl std::error::Error for ChainedError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

#[test]
fn anyhow_chain_is_preserved_after_the_localized_hint() {
    let error = Err::<(), _>(anyhow::anyhow!("inner diagnostic marker"))
        .context("outer operation marker")
        .unwrap_err();
    let failure = UiError::from_anyhow(
        UiText::new("给用户的中文提示", "Readable English hint"),
        &error,
    );

    assert_eq!(failure.hint_text(Lang::Zh), "给用户的中文提示");
    assert_eq!(failure.hint_text(Lang::En), "Readable English hint");

    let details = failure.technical_details(Lang::En);
    assert!(details.contains("outer operation marker"));
    assert!(details.contains("inner diagnostic marker"));

    let copied = failure.copy_text(Lang::En);
    assert!(copied.starts_with("Readable English hint"));
    assert!(copied.contains("Full error details:"));
    assert!(copied.contains("outer operation marker"));
    assert!(copied.contains("inner diagnostic marker"));
}

#[test]
fn raw_inner_error_is_not_parsed_as_bilingual_text() {
    let raw = "driver failed / STATUS_DEVICE_REMOVED / retry exhausted";
    let failure = UiError::from_message(UiText::new("中文提示", "English hint"), raw);

    assert_eq!(failure.technical_details(Lang::Zh), raw);
    assert_eq!(failure.technical_details(Lang::En), raw);
}

#[test]
fn standard_error_sources_are_preserved() {
    let failure = UiError::from_error(
        UiText::new("操作失败", "The operation failed"),
        ChainedError(std::io::Error::new(
            std::io::ErrorKind::Other,
            "inner standard error",
        )),
    );

    let details = failure.technical_details(Lang::En);
    assert_eq!(
        details,
        "outer standard error\nCaused by: inner standard error"
    );
}

#[test]
fn access_violation_has_searchable_windows_diagnostics() {
    let failure = UiError::native_exception(NativeException::new(
        TaskKind::Scanner,
        Some(UiText::new("正在加载 OCR 模型", "Loading OCR models")),
        0xC0000005,
        0x1234,
        Some(NativeMemoryOperation::Read),
        Some(0xDEAD),
    ));

    let english_hint = failure.hint_text(Lang::En);
    assert!(english_hint.starts_with("The scanner stopped"));
    let english = failure.technical_details(Lang::En);
    assert!(english.contains("Task: Scanner"));
    assert!(english.contains("Step when it happened: Loading OCR models"));
    assert!(english.contains("0xC0000005 (Access Violation)"));
    assert!(english.contains("Faulting instruction address: 0x1234"));
    assert!(english.contains("Memory operation: read"));
    assert!(english.contains("Attempted memory address: 0xDEAD"));
    assert!(english.contains("not a file-permission or administrator-access error"));

    let chinese_hint = failure.hint_text(Lang::Zh);
    assert!(chinese_hint.starts_with("扫描器已停止"));
    let chinese = failure.technical_details(Lang::Zh);
    assert!(chinese.contains("Windows 异常: 0xC0000005 (Access Violation / 访问违规)"));
    assert!(chinese.contains("发生时的步骤: 正在加载 OCR 模型"));
}

#[test]
fn panic_payload_remains_available_for_copying() {
    let panic_payload = String::from("model worker invariant failed at item 42");
    let failure = UiError::from_panic(
        UiText::new(
            "后台任务意外停止。",
            "The background task stopped unexpectedly.",
        ),
        &panic_payload,
    );

    let copied = failure.copy_text(Lang::En);
    assert!(copied.starts_with("The background task stopped unexpectedly."));
    assert!(copied.contains("panic: model worker invariant failed at item 42"));
}

#[test]
fn diagnostic_log_store_keeps_pending_errors_until_the_gui_reads_them() {
    let store = LogStore::new(2);
    for message in ["first", "second", "complete inner error"] {
        store.push(LogEntry {
            level: log::Level::Error,
            message: message.to_owned(),
            timestamp: "00:00:00".to_owned(),
            source: LogSource::Scanner,
        });
    }

    let snapshot = store.snapshot();
    assert_eq!(snapshot.len(), 2);
    assert_eq!(snapshot[0].message, "second");
    assert_eq!(snapshot[1].message, "complete inner error");

    store.clear();
    assert!(store.snapshot().is_empty());
}
