//! Git config read/write per scope and SSH key handling against real tools.

use gitchrono_core::git;
use gitchrono_core::setup::{GitSetup, Scope, free_key_path, generate_ssh_key, read_setup, ssh_keys, write_setup};

#[test]
fn local_scope_round_trips_and_unsets() {
    let dir = std::env::temp_dir().join(format!("gitchrono-setup-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q"]).unwrap();
    let scope = Scope::Local(dir.clone());

    let empty = read_setup(&scope);
    assert_eq!(empty, GitSetup::default());

    let set = GitSetup {
        name: "Ada".into(),
        email: "ada@example.com".into(),
        pull_rebase: Some(true),
        sign_commits: Some(false),
        signing_format: "ssh".into(),
        signing_key: "~/.ssh/id_ed25519.pub".into(),
        ..Default::default()
    };
    write_setup(&scope, &empty, &set).unwrap();
    assert_eq!(read_setup(&scope), set);

    // Clearing a value removes the key so the global one applies again.
    let cleared = GitSetup { name: String::new(), pull_rebase: None, ..set.clone() };
    write_setup(&scope, &set, &cleared).unwrap();
    assert_eq!(read_setup(&scope), cleared);
    assert!(git(&dir, &["config", "--local", "--get", "user.name"]).is_err());

    assert!(write_setup(&scope, &cleared, &GitSetup { email: "a b@c".into(), ..cleared.clone() }).is_err());

    // The effective view sees local values on top of everything else, and can't be written.
    let effective = Scope::Effective(dir.clone());
    assert_eq!(read_setup(&effective).email, "ada@example.com");
    assert!(write_setup(&effective, &GitSetup::default(), &cleared).is_err());
}

#[test]
fn generates_and_lists_ssh_keys() {
    let dir = std::env::temp_dir().join(format!("gitchrono-ssh-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(ssh_keys(&dir).is_empty());

    let first = free_key_path(&dir);
    assert_eq!(first.file_name().unwrap(), "id_ed25519");
    let key = generate_ssh_key(&first, "ada@example.com", "").unwrap();
    assert_eq!((key.kind.as_str(), key.comment.as_str()), ("ED25519", "ada@example.com"));
    assert!(key.fingerprint.starts_with("SHA256:"));

    let second = free_key_path(&dir);
    assert_eq!(second.file_name().unwrap(), "id_ed25519_2");
    generate_ssh_key(&second, "", "secret phrase").unwrap();
    assert!(generate_ssh_key(&first, "", "").is_err(), "never overwrites");

    let keys = ssh_keys(&dir);
    assert_eq!(keys.len(), 2);
    assert_eq!(keys[0].private, first);
    let _ = std::fs::remove_dir_all(&dir);
}
