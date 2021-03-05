#[macro_use]
extern crate actix_web;
// #[macro_use]
// extern crate async_stream;
#[macro_use]
extern crate error_chain;
// #[macro_use]
// extern crate futures_util;
#[macro_use]
extern crate log;

mod cdn;
mod config;
mod error;
mod logging;
mod util;

use crate::{
    cdn::index::Index,
    config::Config,
    error::{Result, ResultExt},
};
use actix_web::{web::Data, App, HttpServer};
use std::process::exit;
use actix_web::middleware::DefaultHeaders;

const FILES_URL: &str = "/cdn/files";

mod generated_files {
    include!(concat!(env!("OUT_DIR"), "/generated.rs"));
}

async fn run() -> Result<()> {
    let config = Config::load()?;

    let base_dir = config.base_dir.clone();

    let index = Index::index(
        &base_dir,
        FILES_URL,
        &config.media_include_patterns,
        &config.media_exclude_patterns,
        &config.cover_include_patterns,
        &config.cover_exclude_patterns,
    )
    .await?;
    let index_data = Data::new(index);

    let server_config = config.clone();
    let mut server = HttpServer::new(move || {
        let generated = generated_files::generate();
        let index_data = index_data.clone();
        let config = server_config.clone();

        let mut app = App::new().app_data(index_data);

        // allows CORS from development server to api server
        #[cfg(debug_assertions)]
        let mut app = app.wrap(DefaultHeaders::new().header("Access-Control-Allow-Origin", "http://localhost:4200"));

        app = app.service(cdn::apply_services(&config));
        app = app.service(
            actix_web_static_files::ResourceFiles::new("/", generated).resolve_not_found_to_root(),
        );

        app
    });

    for binding in config.bindings.iter() {
        server = server
            .bind(binding.clone())
            .chain_err(|| "Error binding the actix server")?;
    }

    server
        .run()
        .await
        .chain_err(|| "Error starting the actix server")
}

#[actix_web::main]
async fn main() {
    dotenv::dotenv().ok();
    logging::init();

    if let Err(ref e) = run().await {
        e.log();
        exit(1);
    }
}
