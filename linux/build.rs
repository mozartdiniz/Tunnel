/// Build script: compile the GResource bundle so it can be embedded
/// in the binary via `gio::resources_register_include!`.
fn main() {
    glib_build_tools::compile_resources(
        &["data/resources"],
        "data/resources/dev.tunnel.Tunnel.gresource.xml",
        "tunnel.gresource",
    );
}
