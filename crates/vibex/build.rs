// Embeds the Vibex ">_" icon (assets/vibex.ico) in vibex.exe as icon resource 1,
// so Explorer, the taskbar and Alt+Tab show it, plus version metadata. The
// window loads the same icon resource at runtime.
fn main() {
    let ico = "../../assets/vibex.ico";
    println!("cargo:rerun-if-changed={ico}");
    // HEAD only names the branch; logs/HEAD changes on every commit/checkout
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/logs/HEAD");

    // short git hash for `vibex --version` (empty outside a checkout)
    let hash = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    println!("cargo:rustc-env=VIBEX_GIT_HASH={hash}");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon_with_id(ico, "1");
        res.set("FileDescription", "Vibex Terminal");
        res.set("ProductName", "Vibex");
        res.set("CompanyName", "Kalyan Gopalam");
        res.set("LegalCopyright", "© 2026 Kalyan Gopalam");
        res.set("OriginalFilename", "vibex.exe");
        res.set("InternalName", "vibex");
        if let Err(e) = res.compile() {
            // Never fail the build over an icon; the exe just won't have one.
            println!("cargo:warning=vibex: could not embed icon: {e}");
        }
    }
}
