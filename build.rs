//! Windows: application icon and version resource embedded in crow-browser.exe (Explorer,
//! taskbar, Alt+Tab, installer shortcuts). macOS / Linux take their icons from the .app bundle /
//! the .desktop entry; nothing is done there.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/crow.ico");
    // Build scripts run on the host: `winresource` (and this block) exist on Windows hosts only,
    // the resource goes into Windows targets only.
    #[cfg(target_os = "windows")]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/crow.ico");
        res.set("ProductName", "CrowBrowser");
        res.set("FileDescription", "CrowBrowser - Súkromný prehliadač");
        res.set("LegalCopyright", "CrowBrowser Team");
        res.compile().expect("Failed to compile Windows resource with icon");
    }
}
