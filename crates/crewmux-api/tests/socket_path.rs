//! Long `RIG_HOME` values must not place the daemon socket directly in a shared temp directory.

use std::path::PathBuf;

#[test]
fn long_home_uses_a_private_subdirectory() {
    let home = PathBuf::from("/tmp").join("x".repeat(180));
    let sock = crewmux_api::daemon_socket(&home);
    let parent = sock.parent().expect("parent");
    let name = parent
        .file_name()
        .and_then(|name| name.to_str())
        .expect("name");
    assert!(name.starts_with("rig-"), "{}", sock.display());
    assert_ne!(parent, std::path::Path::new("/tmp"));
    assert!(
        sock.as_os_str().len() <= 100,
        "sockaddr length {}",
        sock.display()
    );
}
