//! Embeds the Sage crest as the icon of `sage.exe`, for Explorer and shortcuts.

fn main() {
    let icon = "../../assets/icons/sage.ico";
    println!("cargo:rerun-if-changed={icon}");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon(icon)
            .compile()
            .expect("embedding the icon needs the Windows SDK's rc.exe");
    }
}
