//! Interface language is independent of ASR and summary language preferences.
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Runtime};
static CHINESE: AtomicBool = AtomicBool::new(false);

pub fn text(en: &'static str, zh: &'static str) -> &'static str {
    if CHINESE.load(Ordering::Relaxed) {
        zh
    } else {
        en
    }
}

pub fn is_chinese() -> bool {
    CHINESE.load(Ordering::Relaxed)
}

#[tauri::command]
pub fn set_ui_language<R: Runtime>(app: AppHandle<R>, language: String) -> Result<(), String> {
    match language.as_str() {
        "zh-CN" => CHINESE.store(true, Ordering::Relaxed),
        "en" => CHINESE.store(false, Ordering::Relaxed),
        _ => return Err("Unsupported interface language".to_string()),
    }
    crate::tray::update_tray_menu(&app);
    Ok(())
}
