//! Links the app so that, in a bundle that carries our `WebKit` fork, every load of a `WebKit`
//! framework takes the bundle's copy: `DYLD_FRAMEWORK_PATH` and `DYLD_LIBRARY_PATH` set to
//! `Contents/Frameworks` in the binary's own `LC_DYLD_ENVIRONMENT`, which dyld honors under the
//! hardened runtime, as it is part of the signed binary.
//!
//! The bundle's install names already point at its copies (`cargo xtask bundle --webkit`), but
//! system frameworks the app loads deep down link the system `WebKit` by its absolute path;
//! without this, both copies would be loaded into one process. Where `Contents/Frameworks` holds
//! no `WebKit`, as in a development build or a bundle on the system `WebKit`, dyld finds nothing
//! there and loads the system's.

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        for variable in ["DYLD_FRAMEWORK_PATH", "DYLD_LIBRARY_PATH"] {
            println!(
                "cargo:rustc-link-arg-bins=-Wl,-dyld_env,{variable}=@executable_path/../Frameworks"
            );
        }
    }
    println!("cargo:rerun-if-changed=build.rs");
}
