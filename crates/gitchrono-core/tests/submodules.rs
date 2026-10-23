//! Submodule listing and commit signature status against a real `git`.

use gitchrono_core::{Repo, git};

fn init(dir: &std::path::Path) {
    std::fs::create_dir_all(dir).unwrap();
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["config", "user.name", "T"],
        &["config", "user.email", "t@example.com"],
        &["config", "commit.gpgsign", "false"],
    ] {
        git(dir, args).unwrap();
    }
    std::fs::write(dir.join("f"), "x").unwrap();
    git(dir, &["add", "."]).unwrap();
    git(dir, &["commit", "-qm", "init"]).unwrap();
}

#[test]
fn lists_submodules_and_reports_unsigned() {
    let base = std::env::temp_dir().join(format!("gitchrono-sub-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let (lib, app) = (base.join("lib"), base.join("app"));
    init(&lib);
    init(&app);
    git(&app, &["-c", "protocol.file.allow=always", "submodule", "add", "-q", lib.to_str().unwrap(), "vendor/lib"]).unwrap();
    git(&app, &["commit", "-qm", "add submodule"]).unwrap();

    let repo = Repo::open(&app).unwrap();
    let subs = repo.submodules().unwrap();
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].path, "vendor/lib");
    assert_eq!(subs[0].state, ' ');
    assert_eq!(subs[0].sha.len(), 40);

    assert!(repo.signature("HEAD").unwrap().is_none(), "unsigned commit");
}

#[test]
#[cfg(unix)]
fn reports_good_ssh_signature() {
    let dir = std::env::temp_dir().join(format!("gitchrono-sign-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    init(&dir);
    let key = dir.join("key");
    let ok = std::process::Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-C", "signer@example.com", "-f"])
        .arg(&key)
        .status()
        .is_ok_and(|s| s.success());
    if !ok {
        return; // no ssh-keygen on this machine
    }
    let public = std::fs::read_to_string(dir.join("key.pub")).unwrap();
    std::fs::write(dir.join("allowed"), format!("signer@example.com {public}")).unwrap();
    for args in [
        &["config", "gpg.format", "ssh"][..],
        &["config", "user.signingkey", key.to_str().unwrap()],
        &["config", "gpg.ssh.allowedSignersFile", dir.join("allowed").to_str().unwrap()],
    ] {
        git(&dir, args).unwrap();
    }
    git(&dir, &["commit", "-q", "-S", "--allow-empty", "-m", "signed"]).unwrap();

    let sig = Repo::open(&dir).unwrap().signature("HEAD").unwrap().expect("signed");
    assert!(sig.is_good(), "{sig:?}");
    assert_eq!(sig.signer, "signer@example.com");
}
