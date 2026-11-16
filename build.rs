fn main() {
    #[cfg(windows)]
    embed_resource::compile("packaging/windows.rc", embed_resource::NONE)
        .manifest_required()
        .expect("compile Windows application resources");

    // Build scripts run on the host. The GNU cross-build handles the resource
    // separately through the same windres used by the Windows linker.
    #[cfg(not(windows))]
    if std::env::var("TARGET").is_ok_and(|target| target == "x86_64-pc-windows-gnu") {
        let output =
            std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("duocle-res.o");
        let result = std::process::Command::new("x86_64-w64-mingw32-windres")
            .args(["packaging/windows.rc", "-O", "coff", "-o"])
            .arg(&output)
            .status()
            .expect("run x86_64-w64-mingw32-windres");
        assert!(result.success(), "compile Windows application resources");
        println!("cargo:rustc-link-arg-bin=duocle={}", output.display());
    }

    println!("cargo:rerun-if-changed=packaging/windows.rc");
    println!("cargo:rerun-if-changed=packaging/windows.manifest");
    println!("cargo:rerun-if-changed=assets/duocle.ico");
}
