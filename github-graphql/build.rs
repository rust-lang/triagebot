pub fn main() {
    cynic_codegen::register_schema("github")
        .from_sdl_file("src/github.graphql")
        .unwrap()
        .as_default()
        .unwrap();
    std::process::Command::new("rustfmt")
        .arg(format!(
            "{}/cynic-schemas/github.rs",
            std::env::var("OUT_DIR").unwrap()
        ))
        .status()
        .expect("failed to execute rustfmt");
}
