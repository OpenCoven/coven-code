use claurst_plugins::loader::{collect_command_defs, try_load_from_path};
use claurst_plugins::{CommandRunAction, PluginRegistry, PluginSource};

#[test]
fn collected_commands_keep_their_owning_plugin_metadata() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("fixture");
    for directory in ["commands/nested", "commands/skill", "extras/nested"] {
        std::fs::create_dir_all(root.join(directory)).unwrap();
    }
    std::fs::write(
        root.join("plugin.json"),
        br#"{"name":"fixture","commands":["extras","direct.md"],"capabilities":["read_files"]}"#,
    )
    .unwrap();
    let files = [
        ("commands/nested/build.md", "fixture:build"),
        ("commands/skill/SKILL.md", "fixture:skill"),
        ("extras/nested/deploy.md", "fixture:deploy"),
        ("direct.md", "fixture:direct"),
    ];
    for (file, _) in files {
        std::fs::write(root.join(file), "# Fixture command\nPrompt body").unwrap();
    }
    let plugin = try_load_from_path(&root, PluginSource::Extra("fixture-source".into()))
        .unwrap()
        .unwrap();
    let defs = collect_command_defs(&plugin);
    assert_eq!(defs.len(), files.len());
    for (file, name) in files {
        let definition = defs.iter().find(|def| def.name == name).unwrap();
        assert_eq!(definition.plugin_name, "fixture");
        assert_eq!(definition.description, "Fixture command");
        assert_eq!(
            definition.plugin_capabilities,
            Some(vec!["read_files".to_string()])
        );
        assert_eq!(
            definition.plugin_source_id, "fixture@fixture-source",
            "{file}"
        );
        match &definition.run_action {
            CommandRunAction::MarkdownPrompt {
                file_path,
                plugin_root,
            } => {
                assert_eq!(std::path::Path::new(file_path), root.join(file));
                assert_eq!(std::path::Path::new(plugin_root), root, "{file}");
            }
            other => panic!("unexpected command action: {other:?}"),
        }
    }
    let mut registry = PluginRegistry::new();
    registry.insert(plugin);
    let registered = registry.all_command_defs();
    assert_eq!(registered.len(), defs.len());
    for definition in &defs {
        let actual = registered
            .iter()
            .find(|def| def.name == definition.name)
            .unwrap();
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(definition).unwrap()
        );
    }
    assert!(registry.disable("fixture"));
    assert!(registry.all_command_defs().is_empty());
    assert!(registry.enable("fixture"));
    assert_eq!(registry.all_command_defs().len(), files.len());
}
