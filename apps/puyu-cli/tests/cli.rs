// SPDX-License-Identifier: Apache-2.0

//! Process-level tests for the implemented CLI surface.

use std::process::{Command, Output};

fn puyu(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_puyu"))
        .args(args)
        .output()
        .expect("puyu should start")
}

#[test]
fn database_help_describes_the_foundation_without_promising_operations() {
    for args in [&["db", "--help"][..], &["db", "-h"], &["help", "db"]] {
        let output = puyu(args);

        assert!(output.status.success(), "{args:?}: {output:?}");
        assert!(output.stderr.is_empty(), "{args:?}: {output:?}");
        assert_eq!(output.stdout, include_bytes!("snapshots/db-help.txt"));
    }
}

#[test]
fn umbrella_help_exposes_only_the_database_namespace() {
    for args in [&["--help"][..], &["-h"], &["help"]] {
        let output = puyu(args);

        assert!(output.status.success(), "{args:?}: {output:?}");
        assert!(output.stderr.is_empty(), "{args:?}: {output:?}");
        assert_eq!(output.stdout, include_bytes!("snapshots/puyu-help.txt"));
    }
}

#[test]
fn version_matches_the_package_version() {
    for flag in ["--version", "-V"] {
        let output = puyu(&[flag]);

        assert!(output.status.success(), "{flag}: {output:?}");
        assert!(output.stderr.is_empty(), "{flag}: {output:?}");
        assert_eq!(
            output.stdout,
            format!("puyu {}\n", env!("CARGO_PKG_VERSION")).as_bytes()
        );
    }
}

#[test]
fn bare_database_namespace_is_not_a_successful_database_operation() {
    let output = puyu(&["db"]);

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(output.stderr, include_bytes!("snapshots/db-help.txt"));
}

#[test]
fn missing_namespace_reports_usage_without_success_output() {
    let output = puyu(&[]);

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(output.stderr, include_bytes!("snapshots/puyu-help.txt"));
}

#[test]
fn unimplemented_commands_and_flags_are_usage_errors() {
    for args in [
        &["db", "init"][..],
        &["db", "status"],
        &["db", "--offline"],
        &["db", "--json"],
        &["db", "--unknown"],
        &["--unknown"],
        &["server"],
    ] {
        let output = puyu(args);

        assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
        assert!(output.stdout.is_empty(), "{args:?}: {output:?}");
        let stderr = String::from_utf8(output.stderr).expect("usage should be UTF-8");
        assert!(stderr.contains("error:"), "{args:?}: {stderr}");
        assert!(stderr.contains("Usage: puyu"), "{args:?}: {stderr}");
    }
}
