use claurst_plugins::{install_plugin_from_path, PluginManifest};

#[test]
fn manifest_path_names_are_rejected() {
    for name in [
        ".",
        "..",
        "...",
        "../outside",
        "nested/plugin",
        "/absolute",
        r"..\outside",
        r"C:\outside",
        "C:relative",
        "plugin:stream",
        "bad\0name",
    ] {
        let json = serde_json::to_vec(&serde_json::json!({ "name": name })).unwrap();
        assert!(
            PluginManifest::from_json(&json).is_err(),
            "accepted {name:?}"
        );
        let toml = toml::to_string(&PluginManifest {
            name: name.into(),
            ..Default::default()
        })
        .unwrap();
        assert!(
            PluginManifest::from_toml(toml.as_bytes()).is_err(),
            "accepted TOML {name:?}"
        );
    }
}

#[test]
fn manifest_path_normal_names_remain_valid() {
    for name in [
        "my-plugin",
        "plugin_v2",
        "plugin.v2",
        "éclair",
        ".hidden-plugin",
    ] {
        let json = serde_json::to_vec(&serde_json::json!({ "name": name })).unwrap();
        assert_eq!(PluginManifest::from_json(&json).unwrap().name, name);
        let toml = toml::to_string(&PluginManifest {
            name: name.into(),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(
            PluginManifest::from_toml(toml.as_bytes()).unwrap().name,
            name
        );
    }
}

#[test]
fn manifest_path_install_cannot_overwrite_absolute_destination() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let outside = temp.path().join("outside");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("marker"), "preserve").unwrap();
    std::fs::write(source.join("marker"), "overwritten").unwrap();
    let manifest = serde_json::json!({ "name": outside.to_str().unwrap() });
    std::fs::write(
        source.join("plugin.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();

    // An absolute name redirects the old installer solely into this temporary
    // fixture, so even the failing baseline cannot write to the real home.
    let result = install_plugin_from_path(&source);
    assert_eq!(
        std::fs::read_to_string(outside.join("marker")).unwrap(),
        "preserve"
    );
    assert!(result.is_err());
}
