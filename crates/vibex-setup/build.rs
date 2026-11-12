// Embeds the release vibex.exe as the installer's payload.
//
// The payload is `<target>/<profile>/vibex.exe` from the same target dir,
// so build vibex first (tools/package.ps1 does both). Without it the setup
// still compiles (so `cargo test --workspace` works) but refuses to install.
use std::path::PathBuf;

fn main() {
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    // OUT_DIR = <target>/<profile>/build/vibex-setup-<hash>/out
    let profile_dir = out.ancestors().nth(3).map(PathBuf::from);
    let payload = std::env::var("VIBEX_PAYLOAD").map(PathBuf::from).ok().or_else(|| profile_dir.map(|d| d.join("vibex.exe"))).filter(|p| p.is_file());

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
    println!("cargo:rerun-if-env-changed=VIBEX_PAYLOAD");

    // Optional: OpenConsole.exe (newer console host, passes images through)
    let oc = std::env::var("VIBEX_OPENCONSOLE_PAYLOAD").map(PathBuf::from).ok().filter(|p| p.is_file());
    let oc_dest = out.join("openconsole.bin");
    match &oc {
        Some(p) => {
            println!("cargo:rerun-if-changed={}", p.display());
            std::fs::copy(p, &oc_dest).expect("copy OpenConsole");
        }
        None => std::fs::write(&oc_dest, b"").unwrap(),
    }
    println!("cargo:rerun-if-env-changed=VIBEX_OPENCONSOLE_PAYLOAD");

    // Optional: vibex.com, the console-side `vibex` (shells wait for it)
    let launcher = std::env::var("VIBEX_LAUNCHER_PAYLOAD").map(PathBuf::from).ok().filter(|p| p.is_file());
    let launcher_dest = out.join("launcher.bin");
    match &launcher {
        Some(p) => {
            println!("cargo:rerun-if-changed={}", p.display());
            std::fs::copy(p, &launcher_dest).expect("copy vibex.com");
        }
        None => std::fs::write(&launcher_dest, b"").unwrap(),
    }
    println!("cargo:rerun-if-env-changed=VIBEX_LAUNCHER_PAYLOAD");

    // Optional: the bundled editors (nano, Neovim) as one zip of vendor\tools
    let tools = std::env::var("VIBEX_TOOLS_PAYLOAD").map(PathBuf::from).ok().filter(|p| p.is_file());
    let tools_dest = out.join("tools.bin");
    match &tools {
        Some(p) => {
            println!("cargo:rerun-if-changed={}", p.display());
            std::fs::copy(p, &tools_dest).expect("copy the editors");
        }
        None => std::fs::write(&tools_dest, b"").unwrap(),
    }
    println!("cargo:rerun-if-env-changed=VIBEX_TOOLS_PAYLOAD");

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
