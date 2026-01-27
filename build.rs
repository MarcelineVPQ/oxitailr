fn main() {
    // Embed Windows resources (icon) when targeting Windows
    embed_resource::compile("assets/oxitailr.rc", embed_resource::NONE);
}
