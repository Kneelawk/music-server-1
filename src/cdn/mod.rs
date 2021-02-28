pub mod files;
pub mod index;

use crate::config::Config;
use actix_files::Files;
use actix_service::{Service, ServiceFactory, Transform};
use actix_web::{
    body::MessageBody,
    dev::{ServiceRequest, ServiceResponse},
    web, App, Scope,
};
use futures::{future, future::Ready, Future};
use std::{
    pin::Pin,
    result,
    task::{Context, Poll},
};

pub fn apply_services(config: &Config) -> Scope {
    web::scope("/cdn")
        .service(index::apply_services())
        .service(files::apply_services(config))
}
