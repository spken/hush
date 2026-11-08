fn main() -> Result<(), embed_resource::CompilationResult> {
    embed_resource::compile("hush.rc", embed_resource::NONE).manifest_optional()
}
