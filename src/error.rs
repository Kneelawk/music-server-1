use crate::util::w_err;
use actix_web::{dev::HttpResponseBuilder, http::StatusCode, HttpResponse, ResponseError};
use error_chain::ChainedError;
use serde::Serialize;
use std::{borrow::Cow, fmt::Debug};

error_chain! {
    errors {
        IndexingError(path: Option<String>) {
            display("Indexing Error at {:?}", path)
        }
        ConfigLoadError(msg: Cow<'static, str>) {
            display("Error loading config: {}", msg)
        }
        NoSuchResource {}
        UriSegmentError {}
        FilesLimiterError {}
    }
}

impl ResponseError for Error {
    fn status_code(&self) -> StatusCode {
        // custom error response status codes here
        match self {
            Error(ErrorKind::FilesLimiterError, ..) => StatusCode::NOT_FOUND,
            Error(ErrorKind::UriSegmentError, ..) => StatusCode::BAD_REQUEST,
            Error(ErrorKind::NoSuchResource, ..) => StatusCode::NOT_FOUND,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn error_response(&self) -> HttpResponse {
        if let Some(json) = self.handle() {
            HttpResponseBuilder::new(self.status_code()).json(&w_err(json))
        } else {
            HttpResponse::new(self.status_code())
        }
    }
}

impl Error {
    fn handle(&self) -> Option<JsonError> {
        // custom error response json errors here
        match self {
            Error(ErrorKind::FilesLimiterError, ..) => None,
            Error(ErrorKind::UriSegmentError, ..) => None,
            Error(ErrorKind::NoSuchResource, ..) => Some(JsonError::NoSuchResource),
            _ => {
                self.log();
                Some(JsonError::InternalServerError)
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
    NoSuchResource,
}
