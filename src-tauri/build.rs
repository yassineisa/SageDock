fn main() {
    println!("cargo:rerun-if-changed=trusted-runtimes.json");
    println!("cargo:rerun-if-changed=runtime");
    if std::env::var("PROFILE").as_deref() == Ok("release") {
        let staged = std::fs::read_dir("runtime")
            .into_iter()
            .flatten()
            .flatten()
            .any(|e| {
                e.file_name().to_string_lossy().ends_with(".tar.xz")
                    || e.file_name().to_string_lossy().ends_with(".tar.gz")
            });
        assert!(
            staged,
            "Release requires a staged runtime: run scripts/stage-runtime.ps1 first"
        );
    }
    tauri_build::build()
}
