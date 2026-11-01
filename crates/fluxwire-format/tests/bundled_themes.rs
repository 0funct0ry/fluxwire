//! Every theme file shipped in `fluxwire-ui/assets/themes` (SPEC §17.7).

use fluxwire_format::theme::{PERMISSIVE_LICENSES, ThemeRegistry};

fn files() -> Vec<(String, String)> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../fluxwire-ui/assets/themes");
    let mut v: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json" || x == "toml"))
        .map(|p| {
            (
                p.file_name().unwrap().to_string_lossy().into_owned(),
                std::fs::read_to_string(&p).unwrap(),
            )
        })
        .collect();
    v.sort();
    v
}

fn registry() -> ThemeRegistry {
    let f = files();
    let pairs: Vec<(&str, &str)> = f.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    ThemeRegistry::bundled(&pairs)
}

#[test]
fn fifty_three_unique_themes_parse() {
    assert_eq!(files().len(), 53);
    let r = registry();
    assert_eq!(r.len(), 53);
    for must in [
        "One Dark",
        "One Light",
        "Nord",
        "Nordfox",
        "Poimandres",
        "Rosé Pine",
    ] {
        assert!(r.get(must).is_some(), "{must}");
    }
}

#[test]
fn appearance_matches_background_lightness() {
    for t in registry().all() {
        assert_eq!(!t.is_dark(), t.editor.to_oklch().l > 0.5, "{}", t.name);
    }
}

#[test]
fn toml_themes_have_author_and_permissive_license() {
    for t in registry().all().iter().filter(|t| t.terminal.is_some()) {
        assert!(!t.author.trim().is_empty(), "{}", t.name);
        assert!(
            PERMISSIVE_LICENSES.contains(&t.license.as_str()),
            "{}",
            t.name
        );
    }
}

#[test]
fn rose_pine_sorts_under_r() {
    let r = registry();
    let names: Vec<_> = r.all().iter().map(|t| t.name.as_str()).collect();
    let i = names.iter().position(|n| n.starts_with("Ros")).unwrap();
    assert!(names[i - 1] < "Ros" && names[i - 1].to_lowercase().starts_with(|c: char| c <= 'r'));
}
