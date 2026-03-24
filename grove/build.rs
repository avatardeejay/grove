fn main() {
    #[cfg(target_os = "windows")]
    embed_resource::compile("grove.rc", embed_resource::NONE);
}