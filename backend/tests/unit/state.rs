use super::*;

#[cfg(unix)]
#[test]
fn rejects_a_symlink_as_the_predictable_runtime_root() {
    use std::os::unix::fs::symlink;

    let parent = tempfile::tempdir().unwrap();
    let victim = parent.path().join("victim");
    fs::create_dir(&victim).unwrap();
    let base = parent.path().join("cyberscope");
    symlink(&victim, &base).unwrap();

    let error = ensure_private_runtime_root(&base).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    assert!(victim.is_dir());
}

#[test]
fn startup_cleanup_removes_all_owned_run_directories_only() {
    let base = tempfile::tempdir().unwrap();
    let old_run = base.path().join("run-old");
    let unrelated = base.path().join("keep-me");
    fs::create_dir(&old_run).unwrap();
    fs::write(old_run.join("result.csv"), b"sensitive").unwrap();
    fs::create_dir(&unrelated).unwrap();

    cleanup_stale_runs(base.path()).unwrap();
    assert!(!old_run.exists());
    assert!(unrelated.is_dir());
}

#[cfg(unix)]
#[test]
fn startup_cleanup_refuses_run_symlinks() {
    use std::os::unix::fs::symlink;

    let base = tempfile::tempdir().unwrap();
    let victim = tempfile::tempdir().unwrap();
    symlink(victim.path(), base.path().join("run-link")).unwrap();

    let error = cleanup_stale_runs(base.path()).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    assert!(victim.path().is_dir());
}
