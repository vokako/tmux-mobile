fn main() {
    // With `gui` on, tauri-build does everything: ACL/permission codegen, the
    // mobile project glue, and the `desktop` / `mobile` cfg aliases.
    #[cfg(feature = "gui")]
    tauri_build::build();

    // With `gui` off there is no `tauri` dependency, so tauri_build::build()
    // panics on the `cargo:dev` instruction that tauri's own build script would
    // have emitted. Nothing it generates is needed for the headless binaries —
    // except the two cfg aliases, which src/lib.rs gates modules on. Emit them
    // here with the same rule tauri-build uses (mobile == android | ios).
    #[cfg(not(feature = "gui"))]
    {
        println!("cargo:rustc-check-cfg=cfg(desktop)");
        println!("cargo:rustc-check-cfg=cfg(mobile)");
        let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
        if target_os == "android" || target_os == "ios" {
            println!("cargo:rustc-cfg=mobile");
        } else {
            println!("cargo:rustc-cfg=desktop");
        }
    }
}
