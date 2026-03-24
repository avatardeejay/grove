fn main() {
    #[cfg(target_os = "windows")]
    embed_resource::compile("grove_installer.rc", embed_resource::NONE);
}