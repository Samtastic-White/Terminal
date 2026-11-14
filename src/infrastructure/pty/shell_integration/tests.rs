//! Shell integration tests: wrapper permissions, sourcing order, hook
//! registration guards, and cleanup.

use super::*;
use std::process::Command;

fn scratch_user_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "nt-test-{label}-{}-{}",
        std::process::id(),
        NEXT_WRAPPER_ID.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir).expect("scratch directory");
    dir
}

fn mode_of(path: &Path) -> u32 {
    fs::metadata(path).expect("metadata").permissions().mode() & 0o777
}

fn zsh_probe(script: &Path, interactive: bool, native: bool, active: Option<&str>) -> String {
    let mut command = Command::new("/bin/zsh");
    command.env("NT_SCRIPT", script);
    if native {
        command.env("NATIVE_TERMINAL", "1");
    } else {
        command.env_remove("NATIVE_TERMINAL");
    }
    match active {
        Some(value) => command.env("NATIVE_TERMINAL_ACTIVE", value),
        None => command.env_remove("NATIVE_TERMINAL_ACTIVE"),
    };
    let probe = "source \"$NT_SCRIPT\"; \
                 printf '%s %s %s\\n' \"${#precmd_functions}\" \
                 \"${#preexec_functions}\" \"${NATIVE_TERMINAL_ACTIVE:-unset}\"";
    if interactive {
        command.args(["-fic", probe]);
    } else {
        command.args(["-fc", probe]);
    }
    let output = command.output().expect("zsh probe runs");
    assert!(
        output.status.success(),
        "zsh failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("zsh output is utf-8")
        .trim()
        .to_string()
}

#[test]
fn wrapper_directory_and_files_are_private() {
    let user = scratch_user_dir("modes");
    let integration = ShellIntegration::prepare(&user).expect("wrapper prepared");

    assert_eq!(mode_of(integration.dir()), 0o700);
    for (name, _) in WRAPPER_FILES {
        assert_eq!(mode_of(&integration.dir().join(name)), 0o600, "{name}");
    }
    assert_eq!(
        mode_of(&integration.dir().join(INTEGRATION_FILE)),
        0o600,
        "integration script"
    );

    fs::remove_dir_all(user).expect("scratch cleanup");
}

#[test]
fn zshrc_sources_the_user_file_before_the_integration() {
    let user = scratch_user_dir("order");
    let integration = ShellIntegration::prepare(&user).expect("wrapper prepared");
    let zshrc = fs::read_to_string(integration.dir().join(".zshrc")).expect("zshrc");

    let user_source = zshrc.find(".zshrc").expect("user file is sourced");
    let integration_source = zshrc
        .find("integration.zsh")
        .expect("integration is sourced");
    assert!(user_source < integration_source);
    assert!(zshrc.contains("NT_USER_ZDOTDIR"));

    fs::remove_dir_all(user).expect("scratch cleanup");
}

#[test]
fn integration_script_avoids_print_payload_expansion() {
    assert!(!INTEGRATION_SCRIPT.contains("print -P"));
    assert!(!INTEGRATION_SCRIPT.contains("print -rn"));
    assert!(INTEGRATION_SCRIPT.contains("printf '\\033]133;D;%s\\033\\\\'"));
    assert!(INTEGRATION_SCRIPT.contains("printf '\\033]633;E;%s\\033\\\\'"));
}

#[test]
fn interactive_gated_shell_registers_both_hooks_once() {
    let user = scratch_user_dir("hooks");
    let integration = ShellIntegration::prepare(&user).expect("wrapper prepared");
    let script = integration.dir().join(INTEGRATION_FILE);

    assert_eq!(zsh_probe(&script, true, true, None), "1 1 1");

    fs::remove_dir_all(user).expect("scratch cleanup");
}

#[test]
fn inactive_or_ungated_shells_register_nothing() {
    let user = scratch_user_dir("guards");
    let integration = ShellIntegration::prepare(&user).expect("wrapper prepared");
    let script = integration.dir().join(INTEGRATION_FILE);

    assert_eq!(zsh_probe(&script, false, true, None), "0 0 unset");
    assert_eq!(zsh_probe(&script, true, false, None), "0 0 unset");
    assert_eq!(
        zsh_probe(&script, true, true, Some("preexisting")),
        "0 0 preexisting"
    );

    fs::remove_dir_all(user).expect("scratch cleanup");
}

#[test]
fn dropping_the_integration_removes_the_wrapper_directory() {
    let user = scratch_user_dir("cleanup");
    let integration = ShellIntegration::prepare(&user).expect("wrapper prepared");
    let wrapper = integration.dir().to_path_buf();

    drop(integration);

    assert!(!wrapper.exists());
    fs::remove_dir_all(user).expect("scratch cleanup");
}
