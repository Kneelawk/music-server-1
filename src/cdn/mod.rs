pub mod files;
pub mod index;

use crate::config::Config;
use actix_web::{web, Scope};

pub fn apply_services(config: &Config) -> Scope {
    web::scope("/cdn")
        .service(index::apply_services())
        .service(files::apply_services(config))
}
