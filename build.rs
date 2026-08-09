fn main() {
    println!("cargo:rerun-if-changed=docs/assets/tasker-icon.ico");

    #[cfg(windows)]
    {
        let mut resource = winresource::WindowsResource::new();
        resource
            .set_icon("docs/assets/tasker-icon.ico")
            .set("ProductName", "Tasker")
            .set(
                "FileDescription",
                "Local-first task management for coding agents",
            )
            .set("OriginalFilename", "tasker.exe");
        resource
            .compile()
            .expect("failed to embed Tasker icon and Windows metadata");
    }
}
