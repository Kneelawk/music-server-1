use actix_web::{
    dev::HttpResponseBuilder,
    http::{header, StatusCode},
    HttpResponse, ResponseError,
};
use error_chain::ChainedError;
use serde::Serialize;
use std::{fmt::Debug, result};

error_chain! {
    errors {
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
            .body(
                serde_json::to_string(&result::Result::<(), JsonError>::Err(self.handle()))
                    .unwrap(),
            )
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
        error!(
            "Internal Server Error: {}",
            self.display_chain().to_string()
        );
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub enum JsonError {
    InternalServerError,
}
