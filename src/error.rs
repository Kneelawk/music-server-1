use actix_web::{
    dev::HttpResponseBuilder,
    http::{header, StatusCode},
    HttpResponse, ResponseError,
};
use error_chain::ChainedError;
use serde::Serialize;
use std::{borrow::Cow, fmt::Debug, result};

error_chain! {
    errors {
        IndexingError(path: Option<String>) {
            display("Indexing Error at {:?}", path)
        }
        ConfigLoadError(msg: Cow<'static, str>) {
            display("Error loading config: {}", msg)
        }
    }
}

impl ResponseError for Error {
    fn status_code(&self) -> StatusCode {
        // custom error response status codes here
        match self {
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn error_response(&self) -> HttpResponse {
        HttpResponseBuilder::new(self.status_code())
            .set_header(header::CONTENT_TYPE, "application/json; charset=utf-8")
            .json(&result::Result::<(), JsonError>::Err(self.handle()))
    }
}

impl Error {
    fn handle(&self) -> JsonError {
        // custom error response json errors here
        match self {
            _ => {
                self.log();
                JsonError::InternalServerError
            }
        }
    }

    pub fn log(&self) {
        error!("{}", self.display_chain().to_string());
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub enum JsonError {
    InternalServerError,
}
