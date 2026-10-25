//! Embeds the app icon in the Windows executable (Explorer, taskbar, Alt-Tab).

fn main() {
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=../../packaging/windows/gitchrono.ico");
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../packaging/windows/gitchrono.ico");
        res.compile().expect("embed the Windows icon");
    }
}
