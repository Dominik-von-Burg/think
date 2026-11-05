fn main() {
    println!("cargo:rerun-if-changed=assets/think.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resource = winres::WindowsResource::new();
        // Cargo build scripts run on the Linux host during a WSL GNU cross-build.
        if cfg!(not(target_os = "windows"))
            && std::env::var("TARGET") == Ok("x86_64-pc-windows-gnu".into())
        {
            resource.set_windres_path("x86_64-w64-mingw32-windres");
        }
        resource.set_icon("assets/think.ico");
        resource
            .compile()
            .expect("could not embed the Windows icon");
        // MinGW only extracts archive members that satisfy a symbol reference.
        // A resource object has no symbols, so link it directly instead.
        let resource_object =
            std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("resource.o");
        println!("cargo:rustc-link-arg={}", resource_object.display());
    }
}
