//! Puts the icons (`wow-tools.rc`) and the version info (product name and version, from Cargo.toml) into the
//! Windows exe.

use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=wow-tools.rc");
    println!("cargo:rerun-if-changed=assets");
    let version = env::var("CARGO_PKG_VERSION").unwrap();
    let mut nums: Vec<u16> = version.split(['.', '-', '+']).filter_map(|s| s.parse().ok()).take(3).collect();
    nums.resize(4, 0);
    let nums = nums.iter().map(u16::to_string).collect::<Vec<_>>().join(",");
    let dir = env::var("CARGO_MANIFEST_DIR").unwrap().replace('\\', "/");
    let icons = fs::read_to_string("wow-tools.rc").unwrap().replace("\"assets/", &format!("\"{dir}/assets/"));
    let rc = format!(
        r#"{icons}
1 VERSIONINFO
FILEVERSION {nums}
PRODUCTVERSION {nums}
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "CompanyName", "Aleksandr Stepanov"
      VALUE "FileDescription", "WoW Tools"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "wow-tools"
      VALUE "LegalCopyright", "Copyright (c) 2026 Aleksandr Stepanov. MIT OR Apache-2.0"
      VALUE "OriginalFilename", "wow-tools.exe"
      VALUE "ProductName", "WoW Tools"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#
    );
    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("wow-tools.rc");
    fs::write(&out, rc).unwrap();
    embed_resource::compile(&out, embed_resource::NONE).manifest_optional().unwrap();
}
