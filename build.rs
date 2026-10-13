//! Puts the icons (`wow-tools.rc`) into the Windows exe.

fn main() {
    println!("cargo:rerun-if-changed=wow-tools.rc");
    println!("cargo:rerun-if-changed=assets");
    embed_resource::compile("wow-tools.rc", embed_resource::NONE).manifest_optional().unwrap();
}
