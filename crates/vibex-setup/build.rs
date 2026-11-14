// Embeds the release vibex.exe as the installer's payload.
//
// The payload is `<target>/<profile>/vibex.exe` from the same target dir,
// so build vibex first (tools/package.ps1 does both). Without it the setup
// still compiles (so `cargo test --workspace` works) but refuses to install.
use std::path::PathBuf;

/// `VIBEX_*_PAYLOAD` names a file to embed. Set but missing (a typo, or a
/// relative path: build scripts run in the crate folder) is an error, not
/// a setup that quietly lacks it.
fn payload_var(name: &str) -> Option<PathBuf> {
    println!("cargo:rerun-if-env-changed={name}");
    let p = PathBuf::from(std::env::var_os(name).filter(|v| !v.is_empty())?);
    if !p.is_file() {
        panic!("{name} = {} is not a file (use an absolute path)", p.display());
    }
    Some(p)
}

fn main() {
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    // OUT_DIR = <target>/<profile>/build/vibex-setup-<hash>/out
    let profile_dir = out.ancestors().nth(3).map(PathBuf::from);
    let payload = payload_var("VIBEX_PAYLOAD").or_else(|| profile_dir.map(|d| d.join("vibex.exe")).filter(|p| p.is_file()));

    let dest = out.join("payload.bin");
    match &payload {
        Some(p) => {
            println!("cargo:rerun-if-changed={}", p.display());
            std::fs::copy(p, &dest).expect("copy payload");
            println!("cargo:rustc-env=VIBEX_HAS_PAYLOAD=1");
        }
        None => {
            std::fs::write(&dest, b"").unwrap();
            println!("cargo:rustc-env=VIBEX_HAS_PAYLOAD=0");
            println!("cargo:warning=vibex-setup: no vibex.exe payload found; build vibex --release first");
        }
    }

    // Optional: OpenConsole.exe (newer console host, passes images through)
    let oc = payload_var("VIBEX_OPENCONSOLE_PAYLOAD");
    let oc_dest = out.join("openconsole.bin");
    match &oc {
        Some(p) => {
            println!("cargo:rerun-if-changed={}", p.display());
            std::fs::copy(p, &oc_dest).expect("copy OpenConsole");
        }
        None => std::fs::write(&oc_dest, b"").unwrap(),
    }

    // Optional: vibex.com, the console-side `vibex` (shells wait for it)
    let launcher = payload_var("VIBEX_LAUNCHER_PAYLOAD");
    let launcher_dest = out.join("launcher.bin");
    match &launcher {
        Some(p) => {
            println!("cargo:rerun-if-changed={}", p.display());
            std::fs::copy(p, &launcher_dest).expect("copy vibex.com");
        }
        None => std::fs::write(&launcher_dest, b"").unwrap(),
    }

    // Optional: the bundled editors (nano, Neovim) as one zip of vendor\tools
    let tools = payload_var("VIBEX_TOOLS_PAYLOAD");
    let tools_dest = out.join("tools.bin");
    match &tools {
        Some(p) => {
            println!("cargo:rerun-if-changed={}", p.display());
            std::fs::copy(p, &tools_dest).expect("copy the editors");
        }
        None => std::fs::write(&tools_dest, b"").unwrap(),
    }

    // The setup usually runs from Downloads: load the DLLs it imports from
    // System32 only, never from the folder it sits in.
    // (LOAD_LIBRARY_SEARCH_SYSTEM32; MSVC link.exe)
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg-bins=/DEPENDENTLOADFLAG:0x800");
    }

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon_with_id("../../assets/vibex.ico", "1");
        res.set("FileDescription", "Vibex Terminal Setup");
        res.set("ProductName", "Vibex");
        res.set("CompanyName", "Kalyan Gopalam");
        res.set("LegalCopyright", "© 2026 Kalyan Gopalam");
        res.set("OriginalFilename", "vibex-setup.exe");
        if let Err(e) = res.compile() {
            println!("cargo:warning=vibex-setup: could not embed resources: {e}");
        }
    }
}
