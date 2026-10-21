// Embed the app icon and a manifest enabling Common Controls v6 (modern visual styles).
fn main() {
    embed_resource::compile("res/minedit.rc", embed_resource::NONE).manifest_optional().unwrap();
}
