mod cdn;
mod config;
mod logging;

use crate::{cdn::index::Index, config::Config};
use actix_web::{App, HttpServer};
use log::{debug, error};

mod generated_files {
    include!(concat!(env!("OUT_DIR"), "/generated.rs"));
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    logging::init();

    let config = match Config::load() {
        Ok(it) => it,
        Err(err) => {
            error!("Error loading config file: {}", err);
            debug!("Error loading config file: {:?}", err);
            return Ok(());
        }
    };

    let base_dir = config.base_dir.clone();

    let _index = Index::index(
        &base_dir,
        &config.media_include_patterns,
        &config.media_exclude_patterns,
        &config.cover_include_patterns,
        &config.cover_exclude_patterns,
    )
    .await
    .unwrap();

    let mut server = HttpServer::new(move || {
        let generated = generated_files::generate();
        App::new().service(
            actix_web_static_files::ResourceFiles::new("/", generated).resolve_not_found_to_root(),
        )
    });

    for binding in config.bindings.iter() {
        server = server.bind(binding.clone())?;
    }

    server.run().await
}
