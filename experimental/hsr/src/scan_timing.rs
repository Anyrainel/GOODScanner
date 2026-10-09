use serde::{Deserialize, Serialize};

/// One setting per source of UI latency, shared by the scanner, manager and
/// drag diagnostic. Values are total waits in milliseconds, without hidden
/// per-path additions. Polling intervals are clamped to at least 1 ms at use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ScanTimings {
    pub menu_open_ms: u64,
    pub menu_close_ms: u64,
    pub input_settle_ms: u64,
    pub inventory_tab_ms: u64,
    pub panel_switch_ms: u64,
    pub traces_open_ms: u64,
    pub character_page_ms: u64,
    pub character_switch_ms: u64,
    pub capture_interval_ms: u64,
    pub key_settle_ms: u64,
    pub poll_interval_ms: u64,
    pub selection_settle_ms: u64,
    pub panel_timeout_ms: u64,
    pub menu_poll_interval_ms: u64,
    pub status_toggle_ms: u64,
}

impl Default for ScanTimings {
    fn default() -> Self {
        Self {
            menu_open_ms: 1_750,
            menu_close_ms: 1_250,
            input_settle_ms: 180,
            inventory_tab_ms: 1_750,
            panel_switch_ms: 500,
            traces_open_ms: 500,
            character_page_ms: 700,
            character_switch_ms: 200,
            capture_interval_ms: 80,
            key_settle_ms: 18,
            poll_interval_ms: 20,
            selection_settle_ms: 280,
            panel_timeout_ms: 900,
            menu_poll_interval_ms: 300,
            status_toggle_ms: 250,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_timings_keep_custom_waits_and_default_the_new_character_wait() {
        let timings: ScanTimings =
            serde_json::from_str(r#"{"panelSwitchMs":750,"tracesOpenMs":2250,"keySettleMs":18}"#)
                .unwrap();
        assert_eq!(timings.panel_switch_ms, 750);
        assert_eq!(timings.traces_open_ms, 2250);
        assert_eq!(timings.key_settle_ms, 18);
        assert_eq!(timings.character_switch_ms, 200);
        let saved = serde_json::to_value(&timings).unwrap();
        assert_eq!(saved["characterSwitchMs"], 200);
        assert_eq!(
            serde_json::from_value::<ScanTimings>(saved).unwrap(),
            timings
        );
    }
}
