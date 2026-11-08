// Embeds the icon and version information into the Windows .exe.
fn main() {
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "Network Tester");
        res.set("FileDescription", "Network Tester");
        res.compile().expect("could not embed the Windows resources");
    }
}
