// Integration test: symlink handling in fs::list_dir / fs::stat_file
//
// Verifies the fix for:
//   1. Broken symlinks were silently dropped from listings.
//   2. Navigating into a symlink-to-dir jumped to the canonical target.
//   3. Symlinks were not flagged for the UI.

use std::fs;
use std::os::unix::fs::symlink;
use tmux_mobile::fs as rfs;

fn fixture_dir(name: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!("tmm-symlink-test-{}", name));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

#[test]
fn list_dir_classifies_symlinks() {
    let root = fixture_dir("classify");
    fs::create_dir(root.join("real-dir")).unwrap();
    fs::write(root.join("real-dir/inside.txt"), "hello").unwrap();

    // symlink to dir, file, and a dangling target
    symlink(root.join("real-dir"), root.join("link-to-dir")).unwrap();
    symlink(root.join("real-dir/inside.txt"), root.join("link-to-file")).unwrap();
    symlink(root.join("does-not-exist"), root.join("broken-link")).unwrap();

    let entries = rfs::list_dir(root.to_str().unwrap(), false).unwrap();
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"link-to-dir"), "missing link-to-dir; got {:?}", names);
    assert!(names.contains(&"link-to-file"), "missing link-to-file; got {:?}", names);
    assert!(names.contains(&"broken-link"), "broken symlink not surfaced; got {:?}", names);

    let by_name = |n: &str| entries.iter().find(|e| e.name == n).unwrap();

    let ld = by_name("link-to-dir");
    assert!(ld.is_symlink, "link-to-dir should be flagged as symlink");
    assert_eq!(ld.file_type, "dir", "symlink-to-dir should report target type 'dir' for navigation");
    assert!(ld.link_target.ends_with("real-dir"));

    let lf = by_name("link-to-file");
    assert!(lf.is_symlink);
    assert_eq!(lf.file_type, "file");
    assert!(lf.link_target.ends_with("inside.txt"));

    let bl = by_name("broken-link");
    assert!(bl.is_symlink);
    assert_eq!(bl.file_type, "broken", "dangling symlinks should be marked broken");
}

#[test]
fn list_dir_preserves_symlink_path() {
    // Navigating into a symlinked dir must keep the symlink path in the
    // entry paths — not jump to the canonical target.
    let root = fixture_dir("preserve");
    fs::create_dir(root.join("real")).unwrap();
    fs::write(root.join("real/a.txt"), "x").unwrap();
    symlink(root.join("real"), root.join("via-link")).unwrap();

    let symlink_path = root.join("via-link");
    let entries = rfs::list_dir(symlink_path.to_str().unwrap(), false).unwrap();
    assert_eq!(entries.len(), 1);
    let p = &entries[0].path;
    let expected_prefix = format!("{}/", symlink_path.display());
    assert!(
        p.starts_with(&expected_prefix),
        "entry path {} should be under symlink path {}, not the canonical target",
        p, expected_prefix
    );
}

#[test]
fn stat_file_flags_symlink_and_keeps_path() {
    let root = fixture_dir("stat");
    fs::create_dir(root.join("real")).unwrap();
    symlink(root.join("real"), root.join("the-link")).unwrap();

    let link_str = root.join("the-link").to_string_lossy().to_string();
    let stat = rfs::stat_file(&link_str).unwrap();
    assert!(stat.is_symlink, "stat should flag symlink");
    assert_eq!(stat.file_type, "dir");
    assert_eq!(stat.path, link_str, "stat should preserve user-supplied path");
    assert!(stat.link_target.ends_with("real"));
}

#[test]
fn stat_file_handles_broken_symlink() {
    let root = fixture_dir("broken-stat");
    symlink(root.join("nope"), root.join("dangling")).unwrap();
    let p = root.join("dangling").to_string_lossy().to_string();
    let stat = rfs::stat_file(&p).unwrap();
    assert!(stat.is_symlink);
    assert_eq!(stat.file_type, "broken");
}
