use std::fmt::{Display, Formatter};

use crate::localization::{Language, LocalizedText};

pub type HsrResult<T> = Result<T, HsrError>;

/// Bilingual readable hint plus a complete, searchable technical chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HsrError {
    code: &'static str,
    hint: LocalizedText,
    detail: String,
}

impl HsrError {
    pub fn new(code: &'static str, hint: LocalizedText, detail: impl Into<String>) -> Self {
        Self {
            code,
            hint,
            detail: detail.into(),
        }
    }

    pub fn code(&self) -> &'static str {
        self.code
    }

    pub fn write_failed(code: &'static str, detail: impl Into<String>) -> Self {
        Self::new(code, hints::WRITE_FAILED, detail)
    }

    pub fn localized_message(&self, language: Language) -> String {
        let detail_label = match language {
            Language::ZhCn => "完整错误详情",
            Language::En => "Full error details",
        };
        format!(
            "{}\n\n{}:\n[{}] {}",
            self.hint.select(language),
            detail_label,
            self.code,
            self.detail
        )
    }
}

impl Display for HsrError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.localized_message(Language::active()))
    }
}

impl std::error::Error for HsrError {}

pub(crate) mod hints {
    use crate::localization::LocalizedText;

    pub const READ_FAILED: LocalizedText = LocalizedText::new(
        "无法读取实验性 HSR 数据文件。",
        "Could not read the experimental HSR data file.",
    );
    pub const JSON_INVALID: LocalizedText = LocalizedText::new(
        "实验性 HSR 数据文件不是有效的 JSON。",
        "The experimental HSR data file is not valid JSON.",
    );
    pub const SENSITIVE_DATA: LocalizedText = LocalizedText::new(
        "输入包含禁止保留的账号或会话字段；已在解析前拒绝。",
        "The input contains a prohibited account or session field and was rejected before parsing.",
    );
    pub const REFERENCE_INVALID: LocalizedText = LocalizedText::new(
        "HSR 参考数据缓存无效。",
        "The HSR reference-data cache is invalid.",
    );
    pub const OBSERVATION_INVALID: LocalizedText =
        LocalizedText::new("HSR 观测数据无效。", "The HSR observation data is invalid.");
    pub const REFERENCE_MISSING: LocalizedText = LocalizedText::new(
        "HSR 观测无法在当前参考数据中解析。",
        "An HSR observation could not be resolved by the current reference data.",
    );
    pub const WRITE_FAILED: LocalizedText = LocalizedText::new(
        "无法安全写入实验性 HSR 导出文件。",
        "Could not safely write the experimental HSR export file.",
    );
    pub const DEVICE_UNAVAILABLE: LocalizedText = LocalizedText::new(
        "无法连接到实验性 HSR 截图设备。",
        "Could not connect to the experimental HSR capture device.",
    );
    pub const FOCUS_REQUIRED: LocalizedText = LocalizedText::new(
        "已拒绝输入：HSR 游戏窗口必须保持在前台。",
        "Input was refused because the HSR game window must remain foreground.",
    );
    pub const SCREEN_INVALID: LocalizedText = LocalizedText::new(
        "当前画面无法通过 HSR 实验扫描器的语义验证。",
        "The current screen could not be validated by the experimental HSR scanner.",
    );
    pub const OCR_FAILED: LocalizedText = LocalizedText::new(
        "无法可靠识别当前 HSR 面板。",
        "The current HSR panel could not be recognized reliably.",
    );
    pub const CANCELLED: LocalizedText = LocalizedText::new(
        "实验性 HSR 操作已安全中止。",
        "The experimental HSR operation was safely aborted.",
    );
}
