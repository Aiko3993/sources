use aidoku::{
    alloc::{String, string::ToString},
    imports::defaults::{DefaultValue, defaults_get, defaults_set},
};

// === Constants ===
const TOKEN_KEY: &str = "auth_token";
const AUTO_CHECKIN_KEY: &str = "autoCheckin";
const LAST_CHECKIN_KEY: &str = "lastCheckin";
const ENHANCED_MODE_KEY: &str = "enhancedMode";

// === Auth Token ===

/// Get auth token - returns None if token is empty or not set
pub fn get_token() -> Option<String> {
    defaults_get::<String>(TOKEN_KEY).filter(|s| !s.is_empty())
}

/// Set auth token
pub fn set_token(token: &str) {
    defaults_set(TOKEN_KEY, DefaultValue::String(token.to_string()));
}

/// Clear auth token
pub fn clear_token() {
    defaults_set(TOKEN_KEY, DefaultValue::Null);
}

// === Check-in ===

pub fn get_auto_checkin() -> bool {
    defaults_get::<bool>(AUTO_CHECKIN_KEY).unwrap_or(false)
}

pub fn has_checkin_flag() -> bool {
    defaults_get::<String>(LAST_CHECKIN_KEY)
        .filter(|s| !s.is_empty())
        .is_some()
}

pub fn set_last_checkin(date: &str) {
    defaults_set(LAST_CHECKIN_KEY, DefaultValue::String(date.into()));
}

pub fn clear_checkin_flag() {
    defaults_set(LAST_CHECKIN_KEY, DefaultValue::Null);
}

// === Enhanced Mode ===

/// Get enhanced mode setting (for accessing all content)
/// Default is false - user must explicitly enable it
pub fn get_enhanced_mode() -> bool {
    defaults_get::<bool>(ENHANCED_MODE_KEY).unwrap_or(false)
}
