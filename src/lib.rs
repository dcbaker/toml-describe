// Copyright © 2023 Dylan Baker
// SPDX-License-Identifier: MIT

use cfg_expr::targets::get_builtin_target_by_triple;
use cfg_expr::{Expression, Predicate};
use semver::{Version, VersionReq};
use serde::de::Error;
use serde::{Deserialize, Deserializer};
use std::collections::HashMap;
use std::vec::Vec;
use std::{env, fs, io, path, process};

#[derive(Deserialize, Debug)]
struct Manifest {
    package: Package,
}

#[derive(Deserialize, Debug)]
struct Package {
    metadata: Metadata,
}

#[derive(Clone, Deserialize, Debug, PartialEq)]
struct Condition {
    #[serde(deserialize_with = "to_version_req")]
    version: Option<VersionReq>,
    cfg: Option<String>,
}

impl Condition {
    fn check(&self, rust_version: &Version) -> bool {
        match &self.version {
            Some(v) => v.matches(rust_version),
            None => false,
        }
    }
}

#[derive(Deserialize, Debug, PartialEq)]
#[serde(untagged)]
enum Constraint {
    Condition(Condition),
    Cfg(HashMap<String, Condition>),
}

fn to_version_req<'de, D>(deserializer: D) -> Result<Option<VersionReq>, D::Error>
where
    D: Deserializer<'de>,
{
    let o: Option<String> = Deserialize::deserialize(deserializer)?;
    match o {
        Some(s) => VersionReq::parse(s.as_str())
            .map_err(D::Error::custom)
            .map(Some),
        None => Ok(None),
    }
}

#[derive(Deserialize, Debug)]
struct Metadata {
    compiler_support: HashMap<String, Constraint>,
}

fn parse(text: &str) -> Vec<(String, Condition)> {
    let mani: Manifest =
        toml::from_str(text).expect("Did not find a 'compiler_versions' metadata section.");
    let target = env::var("TARGET").unwrap();
    let mut ret: Vec<(String, Condition)> = vec![];

    mani.package
        .metadata
        .compiler_support
        .iter()
        .for_each(|(k, v)| {
            match v {
                Constraint::Condition(con) => ret.push((k.clone(), con.clone())),
                Constraint::Cfg(c) => {
                    let cfg = Expression::parse(k).unwrap();
                    let res = if let Some(tinfo) = get_builtin_target_by_triple(&target) {
                        cfg.eval(|p| match p {
                            Predicate::Target(tp) => tp.matches(tinfo),
                            _ => false,
                        })
                    } else {
                        false
                    };
                    if res {
                        c.iter().for_each(|(ck, cv)| {
                            ret.push((ck.clone(), cv.clone()));
                        });
                    }
                }
            };
        });

    return ret;
}

struct VersionData {
    version: Version,
}

fn get_rustc_version() -> VersionData {
    let rustc = env::var("CARGO_BUILD_RUSTC").unwrap();
    let out = process::Command::new(rustc)
        .arg("--version")
        .arg("--verbose")
        .output()
        .expect("Could not run rustc for version");

    let raw = String::from_utf8(out.stdout).expect("Did not get valid output from rustc");
    let lines = raw.split("\n").collect::<Vec<&str>>();

    let raw_version = lines[5].split(" ").collect::<Vec<&str>>()[1];
    let version = Version::parse(raw_version).expect("Invalid Rustc version");

    VersionData { version: version }
}

fn check<W: io::Write>(writer: &mut W) {
    let rustc = get_rustc_version();

    let root = env::var("CARGO_MANIFEST_DIR").unwrap();
    let p: path::PathBuf = [root, "Cargo.toml".to_string()].iter().collect();
    let contents = fs::read_to_string(p).unwrap();
    let checks = parse(&contents);

    checks.iter().for_each(|(name, condition)| {
        if condition.check(&rustc.version) {
            writeln!(
                writer,
                "cargo:rustc-cfg={}",
                condition
                    .cfg
                    .as_ref()
                    .unwrap_or(&format!("compiler_support_{}", name))
            )
            .unwrap();
        }
    });
}

pub fn evaluate() {
    check(&mut io::stdout());
}

#[cfg(test)]
mod tests {
    use super::*;
    use semver::Version;
    use temp_env;

    #[test]
    fn test_basic_read() {
        let mani: Manifest = toml::from_str(
            r#"
            [package.metadata.compiler_support]
            foo = { version = "1.0.0" }
        "#,
        )
        .unwrap();

        let v = &mani.package.metadata.compiler_support["foo"];
        let ver = match v {
            Constraint::Condition(ver) => ver.version.as_ref().unwrap(),
            _ => panic!("Did not get a Version"),
        };
        assert!(ver.matches(&Version::new(1, 0, 0)));
    }

    #[test]
    fn test_multiple_constraints() {
        let mani: Manifest = toml::from_str(
            r#"
            [package.metadata.compiler_support]
            foo = { version = ">1.0.0, <2.0.0" }
        "#,
        )
        .unwrap();

        let v = &mani.package.metadata.compiler_support["foo"];
        let ver = match v {
            Constraint::Condition(ver) => ver.version.as_ref().unwrap(),
            _ => panic!("Did not get a Version"),
        };
        assert!(ver.matches(&Version::new(1, 3, 0)));
        assert!(!ver.matches(&Version::new(0, 3, 0)));
        assert!(!ver.matches(&Version::new(2, 3, 0)));
    }

    #[test]
    fn test_cfg() {
        let mani: Manifest = toml::from_str(
            r#"
            [package.metadata.compiler_support.'cfg(target_os = "linux")']
            foo = { version = "~1.0.0" }
        "#,
        )
        .unwrap();

        let v = &mani.package.metadata.compiler_support["cfg(target_os = \"linux\")"];
        let cfg = match v {
            Constraint::Cfg(cfg) => cfg,
            _ => panic!("Did not get a Version"),
        };

        assert!(cfg.contains_key("foo"));
        assert!(cfg["foo"]
            .version
            .as_ref()
            .unwrap()
            .matches(&Version::new(1, 0, 9)));
    }

    #[test]
    fn test_parse_cfg() {
        temp_env::with_var("TARGET", Some("x86_64-unknown-linux-gnu"), || {
            let vals = parse(
                r#"
                [package.metadata.compiler_support]
                foo = { version = "1.0.0" }
                [package.metadata.compiler_support.'cfg(target_os = "linux")']
                bar = { version = "1.2.0" }
                [package.metadata.compiler_support.'cfg(target_os = "windows")']
                bad = { version = "1.2.0" }
            "#,
            );

            assert_eq!(vals.len(), 2);
        });
    }

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
                assert_eq!(out, b"cargo:rustc-cfg=compiler_support_foo\n");
            },
        )
    }

    #[test]
    fn test_emit_custom_name() {
        temp_env::with_vars(
            [
                ("CARGO_MANIFEST_DIR", Some("test_cases/custom_name")),
                ("CARGO_BUILD_RUSTC", Some("rustc")),
                ("TARGET", Some("x86_64-unknown-linux-gnu")),
            ],
            || {
                let mut out = Vec::new();
                check(&mut out);
                assert_eq!(out, b"cargo:rustc-cfg=can_do_stuff\n");
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
                assert_eq!(out.len(), 0);
            },
        )
    }
}
