//! Tests for the platform-independent entry points of the Swift helper module,
//! and the macOS helper cache directory.

#[cfg(not(target_os = "macos"))]
#[test]
fn precompile_entry_point_is_a_noop_off_macos() {
    super::precompile_helper_background();
}

#[cfg(target_os = "macos")]
mod cache_dir {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::os::unix::fs::PermissionsExt;

    use super::super::private_cache_dir_in;

    /// A resolved temporary directory private to its owner.
    fn private_temp() -> (tempfile::TempDir, std::path::PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let path = std::fs::canonicalize(temp.path()).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        (temp, path)
    }

    fn mode(path: &std::path::Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn a_new_cache_dir_is_created_owner_only() {
        let (_temp, root) = private_temp();
        let dir = private_cache_dir_in(&root, "helper").unwrap();
        assert_eq!(dir, root.join("helper"));
        assert_eq!(mode(&dir), 0o700);
    }

    #[test]
    fn an_existing_own_cache_dir_is_reused_and_made_owner_only() {
        let (_temp, root) = private_temp();
        let existing = root.join("helper");
        std::fs::create_dir(&existing).unwrap();
        std::fs::set_permissions(&existing, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(private_cache_dir_in(&root, "helper").unwrap(), existing);
        assert_eq!(mode(&existing), 0o700);
    }

    #[test]
    fn a_symlinked_cache_dir_is_refused() {
        let (_temp, root) = private_temp();
        let elsewhere = root.join("elsewhere");
        std::fs::create_dir(&elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, root.join("helper")).unwrap();
        assert!(private_cache_dir_in(&root, "helper").is_err());
    }

    #[test]
    fn a_temp_dir_others_can_write_is_refused() {
        let (_temp, root) = private_temp();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o770)).unwrap();
        assert!(private_cache_dir_in(&root, "helper").is_err());
        assert!(!root.join("helper").exists(), "nothing is created there");
    }
}
