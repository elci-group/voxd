use std::fs;

use voxd::project::{resolve, short_hash};

#[test]
fn resolves_git_root_from_nested_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fs::create_dir(root.join(".git")).unwrap();
    let nested = root.join("src").join("deep");
    fs::create_dir_all(&nested).unwrap();

    let pref = resolve(&nested).unwrap();
    assert_eq!(
        pref.root_path,
        fs::canonicalize(root).unwrap().display().to_string()
    );
    assert_eq!(pref.name, root.file_name().unwrap().to_str().unwrap());
    assert_eq!(pref.id, short_hash(&pref.root_path));
}

#[test]
fn falls_back_to_path_without_git() {
    let tmp = tempfile::tempdir().unwrap();
    let pref = resolve(tmp.path()).unwrap();
    assert_eq!(
        pref.root_path,
        fs::canonicalize(tmp.path()).unwrap().display().to_string()
    );
    assert_eq!(pref.id, short_hash(&pref.root_path));
}

#[test]
fn id_is_stable_for_same_root() {
    let tmp = tempfile::tempdir().unwrap();
    fs::create_dir(tmp.path().join(".git")).unwrap();
    let a = resolve(&tmp.path().join("a")).unwrap();
    let b = resolve(&tmp.path().join("b").join("c")).unwrap();
    assert_eq!(a.id, b.id);
    assert_eq!(a.root_path, b.root_path);
}
