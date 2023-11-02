// Copyright © 2023 Dylan Baker
// SPDX-License-Identifier: MIT

use crate::manifest;
use crate::rustc::RUSTC;

use std::{env, fs, io, path};

pub fn check<W: io::Write>(writer: &mut W) {
    let root = env::var("CARGO_MANIFEST_DIR").expect("Cargo manifest environment variable unset");
    let p: path::PathBuf = [root, "Cargo.toml".to_string()].iter().collect();
    let contents = fs::read_to_string(p).expect("Could not read Cargo.toml");
    let checks = manifest::parse(&contents);

    writeln!(writer, "cargo:rerun-if-changed=Cargo.toml").unwrap();

    checks.iter().for_each(|(name, condition)| {
        if condition.check(&RUSTC) {
            writeln!(writer, "cargo:rustc-cfg={}", name).unwrap();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use temp_env;

    #[test]
    fn test_emits() {
        temp_env::with_vars(
            [
                ("CARGO_MANIFEST_DIR", Some("test_cases/basic")),
                ("CARGO_BUILD_RUSTC", Some("rustc")),
                ("TARGET", Some("x86_64-unknown-linux-gnu")),
            ],
            || {
                let mut out = Vec::new();
                check(&mut out);
                assert_eq!(
                    out,
                    b"cargo:rerun-if-changed=Cargo.toml\ncargo:rustc-cfg=foo\n"
                );
            },
        )
    }

    #[test]
    fn test_not_emits() {
        temp_env::with_vars(
            [
                ("CARGO_MANIFEST_DIR", Some("test_cases/not")),
                ("CARGO_BUILD_RUSTC", Some("rustc")),
                ("TARGET", Some("x86_64-unknown-linux-gnu")),
            ],
            || {
                let mut out = Vec::new();
                check(&mut out);
                assert_eq!(out, b"cargo:rerun-if-changed=Cargo.toml\n");
            },
        )
    }
}