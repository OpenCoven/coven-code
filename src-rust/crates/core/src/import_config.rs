use crate::config::{Settings, Theme};
use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportSelection {
    ClaudeMd,
    Settings,
    Both,
}

impl ImportSelection {
    pub fn include_claude_md(self) -> bool {
        matches!(self, Self::ClaudeMd | Self::Both)
    }

    pub fn include_settings(self) -> bool {
        matches!(self, Self::Settings | Self::Both)
    }
}

#[derive(Debug, Clone)]
pub struct ImportPaths {
    pub source_claude_md: PathBuf,
    pub source_settings_json: PathBuf,
    pub target_claude_md: PathBuf,
    pub target_settings_json: PathBuf,
}

impl ImportPaths {
    pub fn detect() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let claude_dir = home.join(".claude");
        let claurst_dir = Settings::config_dir();
        Self {
            source_claude_md: claude_dir.join("CLAUDE.md"),
            source_settings_json: claude_dir.join("settings.json"),
            target_claude_md: claurst_dir.join("CLAUDE.md"),
            target_settings_json: claurst_dir.join("settings.json"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct FilePlan {
    pub source_path: PathBuf,
    pub target_path: PathBuf,
    pub source_exists: bool,
    pub target_exists: bool,
    pub will_write: bool,
}

#[derive(Debug, Clone)]
pub struct PreviewField {
    pub name: String,
    pub action: PreviewAction,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewAction {
    Import,
    Replace,
    Keep,
    Skip,
}

impl PreviewAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::Import => "Import",
            Self::Replace => "Replace",
            Self::Keep => "Keep",
            Self::Skip => "Skip",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ClaudeMdPreview {
    pub plan: FilePlan,
    pub line_count: usize,
    pub char_count: usize,
    pub excerpt: String,
}

#[derive(Debug, Clone)]
pub struct SettingsPreview {
    pub plan: FilePlan,
    pub fields: Vec<PreviewField>,
    pub imported_count: usize,
    pub replaced_count: usize,
    pub kept_count: usize,
    pub skipped_count: usize,
}

#[derive(Debug, Clone)]
pub struct ImportPreview {
    pub selection: ImportSelection,
    pub claude_md: Option<ClaudeMdPreview>,
    pub settings: Option<SettingsPreview>,
}

#[derive(Debug, Clone)]
pub struct ImportExecutionResult {
    pub wrote_claude_md: bool,
    pub wrote_settings: bool,
    pub imported_fields: Vec<String>,
    pub skipped_fields: Vec<String>,
}

#[derive(Debug, Clone)]
struct PreparedImport {
    preview: ImportPreview,
    claude_md_content: Option<String>,
    merged_settings: Option<Settings>,
    imported_fields: Vec<String>,
    skipped_fields: Vec<String>,
}

pub fn build_import_preview(selection: ImportSelection) -> Result<ImportPreview> {
    Ok(prepare_import(selection)?.preview)
}

pub fn execute_import(selection: ImportSelection) -> Result<ImportExecutionResult> {
    let prepared = prepare_import(selection)?;
    let paths = ImportPaths::detect();

    let original_claude_md = if prepared.claude_md_content.is_some() {
        std::fs::read_to_string(&paths.target_claude_md).ok()
    } else {
        None
    };
    let original_settings = if prepared.merged_settings.is_some() {
        std::fs::read_to_string(&paths.target_settings_json).ok()
    } else {
        None
    };

    let mut wrote_claude_md = false;
    let mut wrote_settings = false;

    let result = (|| -> Result<()> {
        if let Some(content) = &prepared.claude_md_content {
            atomic_write_text(&paths.target_claude_md, content)?;
            wrote_claude_md = true;
        }
        if let Some(settings) = &prepared.merged_settings {
            let content = serde_json::to_string_pretty(settings)?;
            atomic_write_text(&paths.target_settings_json, &content)?;
            wrote_settings = true;
        }
        Ok(())
    })();

    if let Err(err) = result {
        if wrote_settings {
            restore_original(&paths.target_settings_json, original_settings.as_deref())?;
        }
        if wrote_claude_md {
            restore_original(&paths.target_claude_md, original_claude_md.as_deref())?;
        }
        return Err(err);
    }

    Ok(ImportExecutionResult {
        wrote_claude_md,
        wrote_settings,
        imported_fields: prepared.imported_fields,
        skipped_fields: prepared.skipped_fields,
    })
}

pub fn summarize_import_result(result: &ImportExecutionResult, paths: &ImportPaths) -> String {
    let mut lines = vec!["Config import completed.".to_string()];

    if result.wrote_claude_md {
        lines.push(format!(
            "- Wrote CLAUDE.md: {}",
            paths.target_claude_md.display()
        ));
    }
    if result.wrote_settings {
        lines.push(format!(
            "- Wrote settings.json: {}",
            paths.target_settings_json.display()
        ));
    }
    if !result.imported_fields.is_empty() {
        lines.push(format!(
            "- Imported fields: {}",
            result.imported_fields.join(", ")
        ));
    }
    if !result.skipped_fields.is_empty() {
        lines.push(format!(
            "- Skipped fields: {}",
            result.skipped_fields.join(", ")
        ));
    }
    if result.wrote_settings {
        lines.push("Reopen settings to review changes.".to_string());
    }
    if result.wrote_claude_md {
        lines.push("Review CLAUDE.md changes in a new session.".to_string());
    }
    lines.join("\n")
}

fn prepare_import(selection: ImportSelection) -> Result<PreparedImport> {
    let paths = ImportPaths::detect();
    let mut preview = ImportPreview {
        selection,
        claude_md: None,
        settings: None,
    };
    let mut claude_md_content = None;
    let mut merged_settings = None;
    let mut imported_fields = Vec::new();
    let mut skipped_fields = Vec::new();

    if selection.include_claude_md() {
        let content = std::fs::read_to_string(&paths.source_claude_md).with_context(|| {
            format!(
                "Failed to read source CLAUDE.md: {}",
                paths.source_claude_md.display()
            )
        })?;
        let excerpt = build_excerpt(&content, 8, 500);
        preview.claude_md = Some(ClaudeMdPreview {
            plan: FilePlan {
                source_path: paths.source_claude_md.clone(),
                target_path: paths.target_claude_md.clone(),
                source_exists: true,
                target_exists: paths.target_claude_md.exists(),
                will_write: true,
            },
            line_count: content.lines().count(),
            char_count: content.chars().count(),
            excerpt,
        });
        claude_md_content = Some(content);
    }

    if selection.include_settings() {
        let source_text =
            std::fs::read_to_string(&paths.source_settings_json).with_context(|| {
                format!(
                    "Failed to read source settings.json: {}",
                    paths.source_settings_json.display()
                )
            })?;
        let source_value: Value = serde_json::from_str(&source_text).with_context(|| {
            format!(
                "Failed to parse source settings.json: {}",
                paths.source_settings_json.display()
            )
        })?;

        let mut current_settings = Settings::load_sync().unwrap_or_default();
        let current_value = serde_json::to_value(&current_settings).unwrap_or(Value::Null);
        let settings_outcome =
            map_settings_preview(&source_value, &current_value, &mut current_settings)?;
        imported_fields.extend(settings_outcome.imported_fields.iter().cloned());
        skipped_fields.extend(settings_outcome.skipped_fields.iter().cloned());
        preview.settings = Some(SettingsPreview {
            plan: FilePlan {
                source_path: paths.source_settings_json.clone(),
                target_path: paths.target_settings_json.clone(),
                source_exists: true,
                target_exists: paths.target_settings_json.exists(),
                will_write: true,
            },
            fields: settings_outcome.preview_fields,
            imported_count: settings_outcome.imported_count,
            replaced_count: settings_outcome.replaced_count,
            kept_count: settings_outcome.kept_count,
            skipped_count: settings_outcome.skipped_count,
        });
        merged_settings = Some(current_settings);
    }

    Ok(PreparedImport {
        preview,
        claude_md_content,
        merged_settings,
        imported_fields,
        skipped_fields,
    })
}

struct SettingsPreviewOutcome {
    preview_fields: Vec<PreviewField>,
    imported_fields: Vec<String>,
    skipped_fields: Vec<String>,
    imported_count: usize,
    replaced_count: usize,
    kept_count: usize,
    skipped_count: usize,
}

fn map_settings_preview(
    source: &Value,
    current: &Value,
    target: &mut Settings,
) -> Result<SettingsPreviewOutcome> {
    let source_obj = source
        .as_object()
        .ok_or_else(|| anyhow!("source settings.json must be a JSON object"))?;

    let mut preview_fields = Vec::new();
    let mut imported_fields = Vec::new();
    let mut skipped_fields = Vec::new();
    let mut imported_count = 0;
    let mut replaced_count = 0;
    let mut kept_count = 0;
    let mut skipped_count = 0;

    if source_obj.contains_key("model") {
        preview_fields.push(PreviewField {
            name: "model".to_string(),
            action: PreviewAction::Skip,
            reason: Some(
                "model is not imported to keep the current session and default model unchanged"
                    .to_string(),
            ),
        });
        skipped_fields.push("model".to_string());
        skipped_count += 1;
    } else {
        preview_fields.push(PreviewField {
            name: "model".to_string(),
            action: PreviewAction::Keep,
            reason: Some("source file does not provide this field".to_string()),
        });
        kept_count += 1;
    }

    map_theme_field(
        source_obj.get("theme"),
        current.pointer("/config/theme"),
        &mut preview_fields,
        &mut imported_fields,
        &mut imported_count,
        &mut replaced_count,
        &mut skipped_fields,
        &mut skipped_count,
        target,
    );

    let output_style_value = source_obj
        .get("outputStyle")
        .or_else(|| source_obj.get("output_style"));
    map_scalar_field(
        output_style_value,
        current.pointer("/config/output_style"),
        "output_style",
        &mut preview_fields,
        &mut imported_fields,
        &mut imported_count,
        &mut replaced_count,
        || {
            if let Some(style) = output_style_value.and_then(Value::as_str) {
                target.config.output_style = Some(style.to_string());
            }
        },
    );

    map_executable_config_field(
        source_obj.get("mcpServers"),
        "mcpServers",
        &mut preview_fields,
        &mut skipped_fields,
        &mut skipped_count,
    );

    map_executable_config_field(
        source_obj.get("hooks"),
        "hooks",
        &mut preview_fields,
        &mut skipped_fields,
        &mut skipped_count,
    );

    for key in [
        "env",
        "ANTHROPIC_AUTH_TOKEN",
        "apiKey",
        "providers",
        "enabledPlugins",
        "disabledMcpServers",
        "extraKnownMarketplaces",
        "skipAutoPermissionPrompt",
        "autoDreamEnabled",
        "codemossProviderId",
        "effortLevel",
    ] {
        if source_obj.contains_key(key) {
            preview_fields.push(PreviewField {
                name: key.to_string(),
                action: PreviewAction::Skip,
                reason: Some(skip_reason_for_key(key).to_string()),
            });
            skipped_fields.push(key.to_string());
            skipped_count += 1;
        }
    }

    for field in &mut preview_fields {
        if field.action == PreviewAction::Skip {
            continue;
        }
        if let Some(reason) = &field.reason {
            if reason == "source file does not provide this field" {
                kept_count += 1;
            }
        }
    }

    Ok(SettingsPreviewOutcome {
        preview_fields,
        imported_fields,
        skipped_fields,
        imported_count,
        replaced_count,
        kept_count,
        skipped_count,
    })
}

fn map_scalar_field<F>(
    source_value: Option<&Value>,
    current_value: Option<&Value>,
    name: &str,
    preview_fields: &mut Vec<PreviewField>,
    imported_fields: &mut Vec<String>,
    imported_count: &mut usize,
    replaced_count: &mut usize,
    apply: F,
) where
    F: FnOnce(),
{
    match source_value.and_then(Value::as_str) {
        Some(source_text) => {
            let action = match current_value.and_then(Value::as_str) {
                Some(current_text) if current_text == source_text => PreviewAction::Import,
                Some(_) => PreviewAction::Replace,
                None => PreviewAction::Import,
            };
            preview_fields.push(PreviewField {
                name: name.to_string(),
                action,
                reason: None,
            });
            imported_fields.push(name.to_string());
            if action == PreviewAction::Replace {
                *replaced_count += 1;
            } else {
                *imported_count += 1;
            }
            apply();
        }
        None => preview_fields.push(PreviewField {
            name: name.to_string(),
            action: PreviewAction::Keep,
            reason: Some("source file does not provide this field".to_string()),
        }),
    }
}

fn map_theme_field(
    source_value: Option<&Value>,
    current_value: Option<&Value>,
    preview_fields: &mut Vec<PreviewField>,
    imported_fields: &mut Vec<String>,
    imported_count: &mut usize,
    replaced_count: &mut usize,
    skipped_fields: &mut Vec<String>,
    skipped_count: &mut usize,
    target: &mut Settings,
) {
    match source_value.and_then(Value::as_str) {
        Some(raw) => {
            let parsed = match raw.to_lowercase().as_str() {
                "default" => Some(Theme::Default),
                "dark" => Some(Theme::Dark),
                "light" => Some(Theme::Light),
                "deuteranopia" => Some(Theme::Deuteranopia),
                other if !other.is_empty() => Some(Theme::Custom(raw.to_string())),
                _ => None,
            };
            if let Some(theme) = parsed {
                let action = match current_value.and_then(Value::as_str) {
                    Some(current_text) if current_text.eq_ignore_ascii_case(raw) => {
                        PreviewAction::Import
                    }
                    Some(_) => PreviewAction::Replace,
                    None => PreviewAction::Import,
                };
                preview_fields.push(PreviewField {
                    name: "theme".to_string(),
                    action,
                    reason: None,
                });
                target.config.theme = theme;
                imported_fields.push("theme".to_string());
                if action == PreviewAction::Replace {
                    *replaced_count += 1;
                } else {
                    *imported_count += 1;
                }
            } else {
                preview_fields.push(PreviewField {
                    name: "theme".to_string(),
                    action: PreviewAction::Skip,
                    reason: Some("theme value cannot be mapped to the current program".to_string()),
                });
                skipped_fields.push("theme".to_string());
                *skipped_count += 1;
            }
        }
        None => preview_fields.push(PreviewField {
            name: "theme".to_string(),
            action: PreviewAction::Keep,
            reason: Some("source file does not provide this field".to_string()),
        }),
    }
}

fn map_executable_config_field(
    source_value: Option<&Value>,
    name: &str,
    preview_fields: &mut Vec<PreviewField>,
    skipped_fields: &mut Vec<String>,
    skipped_count: &mut usize,
) {
    if source_value.is_some() {
        preview_fields.push(PreviewField {
            name: name.to_string(),
            action: PreviewAction::Skip,
            reason: Some("executable configuration is not imported automatically; configure it manually after review".to_string()),
        });
        skipped_fields.push(name.to_string());
        *skipped_count += 1;
    } else {
        preview_fields.push(PreviewField {
            name: name.to_string(),
            action: PreviewAction::Keep,
            reason: Some("source file does not provide this field".to_string()),
        });
    }
}

fn skip_reason_for_key(key: &str) -> &'static str {
    match key {
        "env" => "contains sensitive environment variables and is not imported automatically",
        "ANTHROPIC_AUTH_TOKEN" | "apiKey" | "providers" => {
            "auth and provider credentials are not migrated automatically"
        }
        "enabledPlugins" => "plugin config structure differs from the current program",
        "disabledMcpServers" => "the current program has no matching field",
        "extraKnownMarketplaces" => "the current program has no matching field",
        "skipAutoPermissionPrompt" => "the current program has no matching field",
        "autoDreamEnabled" => "the current program has no matching field",
        "codemossProviderId" => "the current program has no matching field",
        "effortLevel" => "the current program has no stable persistence mapping",
        _ => "the current program does not support this field",
    }
}

fn build_excerpt(content: &str, max_lines: usize, max_chars: usize) -> String {
    let mut excerpt = content
        .lines()
        .take(max_lines)
        .collect::<Vec<_>>()
        .join("\n");
    if excerpt.chars().count() > max_chars {
        excerpt = excerpt.chars().take(max_chars).collect::<String>();
    }
    if content.chars().count() > excerpt.chars().count() {
        excerpt.push_str("\n...");
    }
    excerpt
}

fn atomic_write_text(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("import-target");
    let tmp_path = path.with_file_name(format!("{}.tmp-import", file_name));
    std::fs::write(&tmp_path, content)?;
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    std::fs::rename(&tmp_path, path)?;
    Ok(())
}

fn restore_original(path: &Path, original: Option<&str>) -> Result<()> {
    match original {
        Some(content) => atomic_write_text(path, content),
        None => {
            if path.exists() {
                std::fs::remove_file(path)?;
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn build_excerpt_truncates() {
        let excerpt = build_excerpt("a\nb\nc\nd", 2, 10);
        assert!(excerpt.contains("a\nb"));
        assert!(excerpt.contains("..."));
    }

    #[test]
    fn atomic_write_text_replaces_existing_file() {
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("settings.json");
        std::fs::write(&target, "old").unwrap();
        atomic_write_text(&target, "new").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new");
    }

    #[test]
    fn build_import_preview_maps_settings_and_doc() {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path();
        let claude_dir = home.join(".claude");
        let claurst_dir = home.join(".coven-code");
        std::fs::create_dir_all(&claude_dir).unwrap();
        std::fs::create_dir_all(&claurst_dir).unwrap();
        std::fs::write(claude_dir.join("CLAUDE.md"), "hello\nworld").unwrap();
        std::fs::write(
            claude_dir.join("settings.json"),
            serde_json::json!({
                "model": "openai/gpt-4o",
                "theme": "dark",
                "hooks": {
                    "PreToolUse": [
                        {"matcher": "Bash", "hooks": [{"type": "command", "command": "echo hi"}]}
                    ]
                },
                "mcpServers": {
                    "demo": {"command": "npx", "args": ["demo"]}
                },
                "env": {"SECRET": "x"}
            })
            .to_string(),
        )
        .unwrap();

        let old_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", home);

        let preview = build_import_preview(ImportSelection::Both).unwrap();
        assert!(preview.claude_md.is_some());
        let settings = preview.settings.unwrap();
        assert!(settings
            .fields
            .iter()
            .any(|f| f.name == "model" && f.action == PreviewAction::Skip));
        assert!(settings.fields.iter().any(|f| f.name == "theme"));
        assert!(settings
            .fields
            .iter()
            .any(|f| f.name == "hooks" && f.action == PreviewAction::Skip));
        assert!(settings
            .fields
            .iter()
            .any(|f| f.name == "mcpServers" && f.action == PreviewAction::Skip));
        assert!(settings
            .fields
            .iter()
            .any(|f| f.name == "env" && f.action == PreviewAction::Skip));

        if let Some(old) = old_home {
            std::env::set_var("HOME", old);
        } else {
            std::env::remove_var("HOME");
        }
    }
}
