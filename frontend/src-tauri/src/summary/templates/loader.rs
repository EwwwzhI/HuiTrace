use super::defaults;
use super::types::Template;
use once_cell::sync::Lazy;
use std::path::PathBuf;
use std::sync::RwLock;
use tracing::{debug, info, warn};

// Global storage for the bundled templates directory path
static BUNDLED_TEMPLATES_DIR: Lazy<RwLock<Option<PathBuf>>> = Lazy::new(|| RwLock::new(None));

/// Set the bundled templates directory path (called once at app startup)
pub fn set_bundled_templates_dir(path: PathBuf) {
    info!("Bundled templates directory set to: {:?}", path);
    if let Ok(mut dir) = BUNDLED_TEMPLATES_DIR.write() {
        *dir = Some(path);
    }
}

/// Get the user's custom templates directory path
///
/// Returns the platform-specific application data directory for custom templates:
/// - macOS: ~/Library/Application Support/Mityu/templates/
/// - Windows: %APPDATA%\Mityu\templates\
/// - Linux: ~/.config/Mityu/templates/
pub(crate) fn get_custom_templates_dir() -> Option<PathBuf> {
    let mut path = dirs::data_dir()?;
    path.push("Mityu");
    path.push("templates");
    Some(path)
}

/// Load a template from the bundled resources directory
///
/// # Arguments
/// * `template_id` - Template identifier (without .json extension)
///
/// # Returns
/// The template JSON content if found, None otherwise
fn load_bundled_template(template_id: &str) -> Option<String> {
    let bundled_dir = BUNDLED_TEMPLATES_DIR.read().ok()?.clone()?;
    let template_path = bundled_dir.join(format!("{}.json", template_id));

    debug!("Checking for bundled template at: {:?}", template_path);

    match std::fs::read_to_string(&template_path) {
        Ok(content) => {
            info!(
                "Loaded bundled template '{}' from {:?}",
                template_id, template_path
            );
            Some(content)
        }
        Err(e) => {
            debug!("No bundled template '{}' found: {}", template_id, e);
            None
        }
    }
}

/// Load a template from the user's custom templates directory
///
/// # Arguments
/// * `template_id` - Template identifier (without .json extension)
///
/// # Returns
/// The template JSON content if found, None otherwise
fn load_custom_template(template_id: &str) -> Option<String> {
    let custom_dir = get_custom_templates_dir()?;
    let template_path = custom_dir.join(format!("{}.json", template_id));

    debug!("Checking for custom template at: {:?}", template_path);

    match std::fs::read_to_string(&template_path) {
        Ok(content) => {
            info!(
                "Loaded custom template '{}' from {:?}",
                template_id, template_path
            );
            Some(content)
        }
        Err(e) => {
            debug!("No custom template '{}' found: {}", template_id, e);
            None
        }
    }
}

/// Load and parse a template by identifier
///
/// This function implements a fallback strategy:
/// 1. Check user's custom templates directory
/// 2. Check bundled resources directory (app templates)
/// 3. Fall back to built-in embedded templates
/// 4. Return error if not found in any location
///
/// # Arguments
/// * `template_id` - Template identifier (e.g., "daily_standup", "standard_meeting")
///
/// # Returns
/// Parsed and validated Template struct
pub fn get_template(template_id: &str) -> Result<Template, String> {
    validate_template_id(template_id)?;
    info!("Loading template: {}", template_id);

    // Try custom template first, then bundled, then built-in
    let json_content = if let Some(custom_content) = load_custom_template(template_id) {
        debug!("Using custom template for '{}'", template_id);
        custom_content
    } else if let Some(bundled_content) = load_bundled_template(template_id) {
        debug!("Using bundled template for '{}'", template_id);
        bundled_content
    } else if let Some(builtin_content) = defaults::get_builtin_template(template_id) {
        debug!("Using built-in template for '{}'", template_id);
        builtin_content.to_string()
    } else {
        return Err(format!(
            "Template '{}' not found. Available templates: {}",
            template_id,
            list_template_ids().join(", ")
        ));
    };

    // Parse and validate
    validate_and_parse_template(&json_content)
}

pub(crate) fn validate_template_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 100
        || !id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
    {
        return Err("Invalid template identifier".into());
    }
    Ok(())
}

pub fn save_custom_template(id: &str, template: &Template) -> Result<(), String> {
    let dir = get_custom_templates_dir().ok_or("Application data directory unavailable")?;
    save_custom_template_in(&dir, id, template)
}

fn save_custom_template_in(
    dir: &std::path::Path,
    id: &str,
    template: &Template,
) -> Result<(), String> {
    validate_template_id(id)?;
    if !id.starts_with("custom_") {
        return Err("Save built-in templates as a custom copy".into());
    }
    template.validate()?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut file = tempfile::NamedTempFile::new_in(dir).map_err(|e| e.to_string())?;
    serde_json::to_writer_pretty(&mut file, template).map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    file.persist(dir.join(format!("{id}.json")))
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Validate and parse template JSON
///
/// # Arguments
/// * `json_content` - Raw JSON string
///
/// # Returns
/// Parsed and validated Template struct
pub fn validate_and_parse_template(json_content: &str) -> Result<Template, String> {
    let template: Template = serde_json::from_str(json_content)
        .map_err(|e| format!("Failed to parse template JSON: {}", e))?;

    template.validate()?;

    Ok(template)
}

/// List all available template identifiers
///
/// Returns a combined list of:
/// - Built-in template IDs
/// - Bundled template IDs (from app resources)
/// - Custom template IDs (from user's data directory)
pub fn list_template_ids() -> Vec<String> {
    let mut ids: Vec<String> = defaults::list_builtin_template_ids()
        .into_iter()
        .map(|s| s.to_string())
        .collect();

    // Add bundled templates if directory is set
    if let Ok(bundled_dir_lock) = BUNDLED_TEMPLATES_DIR.read() {
        if let Some(bundled_dir) = bundled_dir_lock.as_ref() {
            if bundled_dir.exists() {
                match std::fs::read_dir(bundled_dir) {
                    Ok(entries) => {
                        for entry in entries.flatten() {
                            if let Some(filename) = entry.file_name().to_str() {
                                if filename.ends_with(".json") {
                                    let id = filename.trim_end_matches(".json").to_string();
                                    if !ids.contains(&id) {
                                        ids.push(id);
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Failed to read bundled templates directory: {}", e);
                    }
                }
            }
        }
    }

    // Add custom templates if directory exists
    if let Some(custom_dir) = get_custom_templates_dir() {
        if custom_dir.exists() {
            match std::fs::read_dir(&custom_dir) {
                Ok(entries) => {
                    for entry in entries.flatten() {
                        if let Some(filename) = entry.file_name().to_str() {
                            if filename.ends_with(".json") {
                                let id = filename.trim_end_matches(".json").to_string();
                                if !ids.contains(&id) {
                                    ids.push(id);
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to read custom templates directory: {}", e);
                }
            }
        }
    }

    ids.sort();
    ids
}

/// List all available templates with their metadata
///
/// Returns a list of (id, name, description, source) tuples
pub fn list_templates() -> Vec<(String, String, String, String)> {
    let mut templates = Vec::new();

    for id in list_template_ids() {
        match get_template(&id) {
            Ok(template) => {
                let source = if load_custom_template(&id).is_some() {
                    "custom"
                } else if load_bundled_template(&id).is_some() {
                    "bundled"
                } else {
                    "builtin"
                };
                templates.push((id, template.name, template.description, source.to_string()));
            }
            Err(e) => {
                warn!("Failed to load template '{}': {}", id, e);
            }
        }
    }

    templates
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_template_save_reload_and_rejected_update_preserve_content() {
        let dir = tempfile::tempdir().unwrap();
        let mut template = validate_and_parse_template(
            defaults::get_builtin_template("standard_meeting").unwrap(),
        )
        .unwrap();
        template.name = "中文模板".into();
        save_custom_template_in(dir.path(), "custom_test", &template).unwrap();
        let path = dir.path().join("custom_test.json");
        let read =
            || validate_and_parse_template(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(read().name, "中文模板");
        template.name = "修改后的模板".into();
        save_custom_template_in(dir.path(), "custom_test", &template).unwrap();
        assert_eq!(read().name, "修改后的模板");
        template.sections.push(template.sections[0].clone());
        assert!(save_custom_template_in(dir.path(), "custom_test", &template).is_err());
        assert_eq!(read().sections.len(), template.sections.len() - 1);
    }

    #[test]
    fn template_paths_and_builtin_overwrites_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let template = validate_and_parse_template(
            defaults::get_builtin_template("standard_meeting").unwrap(),
        )
        .unwrap();
        for id in [
            "../escape",
            "custom_../../escape",
            "C:\\escape",
            "",
            "standard_meeting",
        ] {
            assert!(save_custom_template_in(dir.path(), id, &template).is_err());
        }
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn test_get_builtin_template() {
        let template = get_template("daily_standup");
        assert!(template.is_ok());

        let template = template.unwrap();
        assert_eq!(template.name, "Daily Standup");
        assert!(!template.sections.is_empty());
    }

    #[test]
    fn test_get_nonexistent_template() {
        let result = get_template("nonexistent_template");
        assert!(result.is_err());
    }

    #[test]
    fn test_list_template_ids() {
        let ids = list_template_ids();
        assert!(ids.contains(&"daily_standup".to_string()));
        assert!(ids.contains(&"standard_meeting".to_string()));
    }

    #[test]
    fn test_validate_invalid_json() {
        let result = validate_and_parse_template("invalid json");
        assert!(result.is_err());
    }
}
