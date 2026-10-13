fn main() -> Result<(), Box<dyn std::error::Error>> {
    let reference = claurst_commands::catalog::render_reference();
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../docs/slash-commands.md");
    if std::env::args().any(|argument| argument == "--check") {
        if std::fs::read_to_string(&path)? != reference {
            return Err("slash command reference is stale; regenerate it with the command_reference example".into());
        }
    } else {
        std::fs::write(&path, reference)?;
        println!("Updated {}", path.display());
    }
    Ok(())
}
