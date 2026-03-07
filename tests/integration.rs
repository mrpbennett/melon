use std::path::PathBuf;

// We need to reference types from the main crate.
// Since termpete is a binary crate, we'll test via the specs directly.

#[test]
fn test_real_specs_deserialize() {
    let specs_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("specs");
    assert!(specs_dir.exists(), "specs/ directory should exist");

    let mut count = 0;
    let mut errors = Vec::new();
    for entry in std::fs::read_dir(&specs_dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "json") {
            let content = std::fs::read_to_string(&path).unwrap();
            let result: Result<serde_json::Value, _> = serde_json::from_str(&content);
            match result {
                Ok(val) => {
                    // Verify basic structure
                    assert!(
                        val.get("name").is_some(),
                        "{}: missing 'name' field",
                        path.display()
                    );
                    count += 1;
                }
                Err(e) => {
                    errors.push(format!("{}: {e}", path.display()));
                }
            }
        }
    }

    assert!(
        errors.is_empty(),
        "Spec parse errors:\n{}",
        errors.join("\n")
    );
    assert!(count >= 10, "Expected at least 10 specs, found {count}");
    eprintln!("Successfully validated {count} specs");
}
