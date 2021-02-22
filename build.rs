use actix_web_static_files::NpmBuild;
use walkdir::WalkDir;

const SERVE_DIR: &str = "web/dist/music-server";

fn main() {
    NpmBuild::new("web")
        .install()
        .unwrap()
        .run("build")
        .unwrap()
        .target(SERVE_DIR)
        .to_resource_dir()
        .build()
        .unwrap();

    for entry in WalkDir::new(SERVE_DIR) {
        println!("cargo:rerun-if-changed={}", entry.unwrap().path().display())
    }
}
