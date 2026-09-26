use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=src/apple_translation.swift");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }

    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR is set"));
    let library = out.join("libCockpitTranslation.dylib");
    // 最低 OS は束と同じ 15.0。指定しないと組んだ機体の OS 版が最低になり、
    // 古い macOS で読み込めない橋ができる。
    let arch = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("aarch64") => "arm64",
        _ => "x86_64",
    };
    let target = format!("{arch}-apple-macos15.0");
    let status = Command::new("xcrun")
        .args([
            "swiftc",
            "-target",
            &target,
            "-O",
            "-parse-as-library",
            "-emit-library",
            "-module-name",
            "CockpitTranslation",
            "src/apple_translation.swift",
            "-o",
        ])
        .arg(&library)
        .status()
        .expect("xcrun swiftc starts");
    assert!(status.success(), "Apple翻訳の橋を組み立てられない");

    let status = Command::new("install_name_tool")
        .args(["-id", "@rpath/libCockpitTranslation.dylib"])
        .arg(&library)
        .status()
        .expect("install_name_tool starts");
    assert!(status.success(), "Apple翻訳のinstall nameを固定できない");
    let linked = Command::new("otool")
        .arg("-L")
        .arg(&library)
        .output()
        .expect("otool starts");
    for dependency in String::from_utf8_lossy(&linked.stdout)
        .lines()
        .map(str::trim)
        .filter_map(|line| line.split_whitespace().next())
        .filter(|dependency| dependency.starts_with("@rpath/libswift_"))
    {
        let name = dependency.trim_start_matches("@rpath/");
        let status = Command::new("install_name_tool")
            .args(["-change", dependency])
            .arg(format!("/usr/lib/swift/{name}"))
            .arg(&library)
            .status()
            .expect("install_name_tool starts");
        assert!(status.success(), "Swift runtimeの参照をOSへ固定できない");
    }

    let profile_dir = out
        .parent()
        .and_then(|out| out.parent())
        .and_then(|build| build.parent())
        .expect("Cargo target profile layout");
    std::fs::copy(&library, profile_dir.join("libCockpitTranslation.dylib"))
        .expect("Apple翻訳の橋をprofileへ置ける");

    // リンクはしない。アプリが起動後に `dlopen` で読む(`apple_translation.rs`)——リンクで
    // 結ぶと、Armature を依存に取った crate の実体が rpath を持たずに起動で落ちる。
}
