use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use artix::config::render_default_config_toml;
use tempfile::tempdir;

#[test]
fn help_command_lists_available_features() {
    let output = Command::new(artix_bin_path())
        .arg("help")
        .output()
        .expect("run artix help");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("USAGE:"));
    assert!(stdout.contains("artix init-config"));
    assert!(stdout.contains("--print-default-config"));
    assert!(stdout.contains("Primary path: ~/.config/artix/config.toml"));
}

#[test]
fn short_help_flag_prints_help_text() {
    let output = Command::new(artix_bin_path())
        .arg("-h")
        .output()
        .expect("run artix -h");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("COMMANDS:"));
    assert!(stdout.contains("help                    Show this help text"));
}

#[test]
fn print_default_config_outputs_rendered_toml() {
    let output = Command::new(artix_bin_path())
        .arg("--print-default-config")
        .output()
        .expect("run artix --print-default-config");

    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), render_default_config_toml());
    assert!(String::from_utf8(output.stderr).unwrap().is_empty());
}

#[test]
fn init_config_writes_default_config_to_primary_path() {
    let temp = tempdir().unwrap();
    let fake_home = temp.path().join("home");
    let expected_path = expected_primary_config_path(&fake_home);

    fs::create_dir_all(&fake_home).unwrap();

    let output = Command::new(artix_bin_path())
        .arg("init-config")
        .env("HOME", &fake_home)
        .output()
        .expect("run artix init-config");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("initialized config at {}\n", expected_path.display())
    );
    assert_eq!(
        fs::read_to_string(&expected_path).unwrap(),
        render_default_config_toml()
    );
}

#[test]
fn init_config_fails_when_primary_config_already_exists() {
    let temp = tempdir().unwrap();
    let fake_home = temp.path().join("home");
    let expected_path = expected_primary_config_path(&fake_home);

    fs::create_dir_all(expected_path.parent().unwrap()).unwrap();
    fs::write(&expected_path, "version = 1\n").unwrap();

    let output = Command::new(artix_bin_path())
        .arg("init-config")
        .env("HOME", &fake_home)
        .output()
        .expect("run artix init-config with existing file");

    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        format!("artix: config file already exists at {}\n", expected_path.display())
    );
}

fn artix_bin_path() -> PathBuf {
    std::env::var_os("CARGO_BIN_EXE_artix")
        .map(PathBuf::from)
        .expect("CARGO_BIN_EXE_artix should be set for integration tests")
}

fn expected_primary_config_path(home: &Path) -> PathBuf {
    home.join(".config/artix/config.toml")
}
