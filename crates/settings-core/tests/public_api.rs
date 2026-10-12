//! Consumer-level checks ensure the core works without app/UI/storage adapters.
use lumen_settings_core::{
    Layer, Value, catalog, parse, resolve, serialize, setting, validate_layers,
};

#[test]
fn public_api_preserves_precedence_sparse_roundtrips_and_search_rules() {
    let (user, _) = parse(Some("[editor]\nfont_size = 16\n")).unwrap();
    let source = "# project\n[editor]\nfont_size = 14 # explicit default\nfuture = 'keep'\n";
    let (mut project, warnings) = parse(Some(source)).unwrap();
    assert_eq!(warnings.len(), 1);
    validate_layers(&user, Some(&project)).unwrap();
    let font_size = setting("editor.font_size");
    assert_eq!(
        resolve(font_size, &user, Some(&project)),
        (Value::Number(14.0), "Project")
    );
    assert!(font_size.matches("text size @modified", &project, false));
    assert!(!font_size.matches("@pending", &project, false));
    project.remove(font_size.id);
    assert_eq!(
        resolve(font_size, &user, Some(&project)),
        (Value::Number(16.0), "User")
    );
    let encoded = serialize(Some(source), &project).unwrap();
    assert!(encoded.contains("# project") && encoded.contains("future = 'keep'"));
    assert!(!encoded.contains("font_size"));
    let (roundtrip, _) = parse(Some(&encoded)).unwrap();
    assert_eq!(roundtrip, project);
    assert!(catalog().count() >= 9);
    assert_eq!(
        setting("files.auto_save").default_value(),
        Value::Choice("off".into())
    );
    assert!(validate_layers(&Layer::new(), None).is_ok());
}

#[test]
fn public_api_rejects_bad_known_settings_and_unsupported_schemas() {
    assert!(parse(Some("schema_version = 2")).is_err());
    assert!(parse(Some("[editor]\nfont_size = true")).is_err());
    let (user, _) = parse(Some("[editor]\nfont_size = 24\n")).unwrap();
    assert!(validate_layers(&user, None).is_err());
    let (project, _) = parse(Some("[editor]\nline_height = 30\n")).unwrap();
    validate_layers(&user, Some(&project)).unwrap();
}
