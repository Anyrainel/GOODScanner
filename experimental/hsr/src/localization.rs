use serde::{Deserialize, Serialize};

/// Languages supported by every user-visible experimental message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    ZhCn,
    En,
}

impl Language {
    pub fn active() -> Self {
        if yas::lang::is_en() {
            Self::En
        } else {
            Self::ZhCn
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::ZhCn => "zh",
            Self::En => "en",
        }
    }
}

/// A compile-time bilingual user-facing string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalizedText {
    pub zh_cn: &'static str,
    pub en: &'static str,
}

impl LocalizedText {
    pub const fn new(zh_cn: &'static str, en: &'static str) -> Self {
        Self { zh_cn, en }
    }

    pub fn select(self, language: Language) -> &'static str {
        match language {
            Language::ZhCn => self.zh_cn,
            Language::En => self.en,
        }
    }
}

/// Bilingual game-reference text retained in exported data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BilingualName {
    pub zh_cn: String,
    pub en: String,
}

impl BilingualName {
    pub(crate) fn is_complete(&self) -> bool {
        !self.zh_cn.trim().is_empty() && !self.en.trim().is_empty()
    }
}
