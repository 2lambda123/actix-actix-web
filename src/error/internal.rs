use std::{cell::RefCell, fmt, io::Write as _};

use actix_http::{
    body::BoxBody,
    header::{self, TryIntoHeaderValue as _},
    StatusCode,
};
use bytes::{BufMut as _, BytesMut};

use crate::{Error, HttpRequest, HttpResponse, Responder, ResponseError};

/// Wraps errors to alter the generated response status code.
///
/// In following example, the `io::Error` is wrapped into `ErrorBadRequest` which will generate a
/// response with the 400 Bad Request status code instead of the usual status code generated by
/// an `io::Error`.
///
/// # Examples
/// ```
/// # use std::io;
/// # use actix_web::{error, HttpRequest};
/// async fn handler_error() -> Result<String, actix_web::Error> {
///     let err = io::Error::new(io::ErrorKind::Other, "error");
///     Err(error::ErrorBadRequest(err))
/// }
/// ```
pub struct InternalError<T> {
    cause: T,
    status: InternalErrorType,
}

enum InternalErrorType {
    Status(StatusCode),
    Response(RefCell<Option<HttpResponse>>),
}

impl<T> InternalError<T> {
    /// Constructs an `InternalError` with given status code.
    pub fn new(cause: T, status: StatusCode) -> Self {
        InternalError {
            cause,
            status: InternalErrorType::Status(status),
        }
    }

    /// Constructs an `InternalError` with pre-defined response.
    pub fn from_response(cause: T, response: HttpResponse) -> Self {
        InternalError {
            cause,
            status: InternalErrorType::Response(RefCell::new(Some(response))),
        }
    }
}

impl<T: fmt::Debug> fmt::Debug for InternalError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.cause.fmt(f)
    }
}

impl<T: fmt::Display> fmt::Display for InternalError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.cause.fmt(f)
    }
}

impl<T> ResponseError for InternalError<T>
where
    T: fmt::Debug + fmt::Display,
{
    fn status_code(&self) -> StatusCode {
        match self.status {
            InternalErrorType::Status(st) => st,
            InternalErrorType::Response(ref resp) => {
                if let Some(resp) = resp.borrow().as_ref() {
                    resp.head().status
                } else {
                    StatusCode::INTERNAL_SERVER_ERROR
                }
            }
        }
    }

    fn error_response(&self) -> HttpResponse {
        match self.status {
            InternalErrorType::Status(status) => {
                let mut res = HttpResponse::new(status);
                let mut buf = BytesMut::new().writer();
                let _ = write!(buf, "{}", self);

                let mime = mime::TEXT_PLAIN_UTF_8.try_into_value().unwrap();
                res.headers_mut().insert(header::CONTENT_TYPE, mime);

                res.set_body(BoxBody::new(buf.into_inner()))
            }

            InternalErrorType::Response(ref resp) => {
                if let Some(resp) = resp.borrow_mut().take() {
                    resp
                } else {
                    HttpResponse::new(StatusCode::INTERNAL_SERVER_ERROR)
                }
            }
        }
    }
}

impl<T> Responder for InternalError<T>
where
    T: fmt::Debug + fmt::Display + 'static,
{
    type Body = BoxBody;

    fn respond_to(self, _: &HttpRequest) -> HttpResponse<Self::Body> {
        HttpResponse::from_error(self)
    }
}

macro_rules! error_helper {
    // Workaround for 1.52.0 compat. It's not great but any use of `concat!` must be done prior
    // to insertion in a doc comment.
    ($name:ident, $status:ident) => {
        error_helper!(
            $name,
            $status,
            concat!(
                "Helper function that wraps any error and generates a `",
                stringify!($status),
                "` response."
            )
        );
    };
    ($name:ident, $status:ident, $doc:expr) => {
        #[doc = $doc]
        #[allow(non_snake_case)]
        pub fn $name<T>(err: T) -> Error
        where
            T: fmt::Debug + fmt::Display + 'static,
        {
            InternalError::new(err, StatusCode::$status).into()
        }
    };
}

error_helper!(ErrorBadRequest, BAD_REQUEST);
error_helper!(ErrorUnauthorized, UNAUTHORIZED);
error_helper!(ErrorPaymentRequired, PAYMENT_REQUIRED);
error_helper!(ErrorForbidden, FORBIDDEN);
error_helper!(ErrorNotFound, NOT_FOUND);
error_helper!(ErrorMethodNotAllowed, METHOD_NOT_ALLOWED);
error_helper!(ErrorNotAcceptable, NOT_ACCEPTABLE);
error_helper!(
    ErrorProxyAuthenticationRequired,
    PROXY_AUTHENTICATION_REQUIRED
);
error_helper!(ErrorRequestTimeout, REQUEST_TIMEOUT);
error_helper!(ErrorConflict, CONFLICT);
error_helper!(ErrorGone, GONE);
error_helper!(ErrorLengthRequired, LENGTH_REQUIRED);
error_helper!(ErrorPayloadTooLarge, PAYLOAD_TOO_LARGE);
error_helper!(ErrorUriTooLong, URI_TOO_LONG);
error_helper!(ErrorUnsupportedMediaType, UNSUPPORTED_MEDIA_TYPE);
error_helper!(ErrorRangeNotSatisfiable, RANGE_NOT_SATISFIABLE);
error_helper!(ErrorImATeapot, IM_A_TEAPOT);
error_helper!(ErrorMisdirectedRequest, MISDIRECTED_REQUEST);
error_helper!(ErrorUnprocessableEntity, UNPROCESSABLE_ENTITY);
error_helper!(ErrorLocked, LOCKED);
error_helper!(ErrorFailedDependency, FAILED_DEPENDENCY);
error_helper!(ErrorUpgradeRequired, UPGRADE_REQUIRED);
error_helper!(ErrorPreconditionFailed, PRECONDITION_FAILED);
error_helper!(ErrorPreconditionRequired, PRECONDITION_REQUIRED);
error_helper!(ErrorTooManyRequests, TOO_MANY_REQUESTS);
error_helper!(
    ErrorRequestHeaderFieldsTooLarge,
    REQUEST_HEADER_FIELDS_TOO_LARGE
);
error_helper!(
    ErrorUnavailableForLegalReasons,
    UNAVAILABLE_FOR_LEGAL_REASONS
);
error_helper!(ErrorExpectationFailed, EXPECTATION_FAILED);
error_helper!(ErrorInternalServerError, INTERNAL_SERVER_ERROR);
error_helper!(ErrorNotImplemented, NOT_IMPLEMENTED);
error_helper!(ErrorBadGateway, BAD_GATEWAY);
error_helper!(ErrorServiceUnavailable, SERVICE_UNAVAILABLE);
error_helper!(ErrorGatewayTimeout, GATEWAY_TIMEOUT);
error_helper!(ErrorHttpVersionNotSupported, HTTP_VERSION_NOT_SUPPORTED);
error_helper!(ErrorVariantAlsoNegotiates, VARIANT_ALSO_NEGOTIATES);
error_helper!(ErrorInsufficientStorage, INSUFFICIENT_STORAGE);
error_helper!(ErrorLoopDetected, LOOP_DETECTED);
error_helper!(ErrorNotExtended, NOT_EXTENDED);
error_helper!(
    ErrorNetworkAuthenticationRequired,
    NETWORK_AUTHENTICATION_REQUIRED
);

#[cfg(test)]
mod tests {
    use actix_http::error::ParseError;

    use super::*;

    #[test]
    fn test_internal_error() {
        let err = InternalError::from_response(ParseError::Method, HttpResponse::Ok().finish());
        let resp: HttpResponse = err.error_response();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn test_error_helpers() {
        let res: HttpResponse = ErrorBadRequest("err").into();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);

        let res: HttpResponse = ErrorUnauthorized("err").into();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

        let res: HttpResponse = ErrorPaymentRequired("err").into();
        assert_eq!(res.status(), StatusCode::PAYMENT_REQUIRED);

        let res: HttpResponse = ErrorForbidden("err").into();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);

        let res: HttpResponse = ErrorNotFound("err").into();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);

        let res: HttpResponse = ErrorMethodNotAllowed("err").into();
        assert_eq!(res.status(), StatusCode::METHOD_NOT_ALLOWED);

        let res: HttpResponse = ErrorNotAcceptable("err").into();
        assert_eq!(res.status(), StatusCode::NOT_ACCEPTABLE);

        let res: HttpResponse = ErrorProxyAuthenticationRequired("err").into();
        assert_eq!(res.status(), StatusCode::PROXY_AUTHENTICATION_REQUIRED);

        let res: HttpResponse = ErrorRequestTimeout("err").into();
        assert_eq!(res.status(), StatusCode::REQUEST_TIMEOUT);

        let res: HttpResponse = ErrorConflict("err").into();
        assert_eq!(res.status(), StatusCode::CONFLICT);

        let res: HttpResponse = ErrorGone("err").into();
        assert_eq!(res.status(), StatusCode::GONE);

        let res: HttpResponse = ErrorLengthRequired("err").into();
        assert_eq!(res.status(), StatusCode::LENGTH_REQUIRED);

        let res: HttpResponse = ErrorPreconditionFailed("err").into();
        assert_eq!(res.status(), StatusCode::PRECONDITION_FAILED);

        let res: HttpResponse = ErrorPayloadTooLarge("err").into();
        assert_eq!(res.status(), StatusCode::PAYLOAD_TOO_LARGE);

        let res: HttpResponse = ErrorUriTooLong("err").into();
        assert_eq!(res.status(), StatusCode::URI_TOO_LONG);

        let res: HttpResponse = ErrorUnsupportedMediaType("err").into();
        assert_eq!(res.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);

        let res: HttpResponse = ErrorRangeNotSatisfiable("err").into();
        assert_eq!(res.status(), StatusCode::RANGE_NOT_SATISFIABLE);

        let res: HttpResponse = ErrorExpectationFailed("err").into();
        assert_eq!(res.status(), StatusCode::EXPECTATION_FAILED);

        let res: HttpResponse = ErrorImATeapot("err").into();
        assert_eq!(res.status(), StatusCode::IM_A_TEAPOT);

        let res: HttpResponse = ErrorMisdirectedRequest("err").into();
        assert_eq!(res.status(), StatusCode::MISDIRECTED_REQUEST);

        let res: HttpResponse = ErrorUnprocessableEntity("err").into();
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);

        let res: HttpResponse = ErrorLocked("err").into();
        assert_eq!(res.status(), StatusCode::LOCKED);

        let res: HttpResponse = ErrorFailedDependency("err").into();
        assert_eq!(res.status(), StatusCode::FAILED_DEPENDENCY);

        let res: HttpResponse = ErrorUpgradeRequired("err").into();
        assert_eq!(res.status(), StatusCode::UPGRADE_REQUIRED);

        let res: HttpResponse = ErrorPreconditionRequired("err").into();
        assert_eq!(res.status(), StatusCode::PRECONDITION_REQUIRED);

        let res: HttpResponse = ErrorTooManyRequests("err").into();
        assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);

        let res: HttpResponse = ErrorRequestHeaderFieldsTooLarge("err").into();
        assert_eq!(res.status(), StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE);

        let res: HttpResponse = ErrorUnavailableForLegalReasons("err").into();
        assert_eq!(res.status(), StatusCode::UNAVAILABLE_FOR_LEGAL_REASONS);

        let res: HttpResponse = ErrorInternalServerError("err").into();
        assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let res: HttpResponse = ErrorNotImplemented("err").into();
        assert_eq!(res.status(), StatusCode::NOT_IMPLEMENTED);

        let res: HttpResponse = ErrorBadGateway("err").into();
        assert_eq!(res.status(), StatusCode::BAD_GATEWAY);

        let res: HttpResponse = ErrorServiceUnavailable("err").into();
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);

        let res: HttpResponse = ErrorGatewayTimeout("err").into();
        assert_eq!(res.status(), StatusCode::GATEWAY_TIMEOUT);

        let res: HttpResponse = ErrorHttpVersionNotSupported("err").into();
        assert_eq!(res.status(), StatusCode::HTTP_VERSION_NOT_SUPPORTED);

        let res: HttpResponse = ErrorVariantAlsoNegotiates("err").into();
        assert_eq!(res.status(), StatusCode::VARIANT_ALSO_NEGOTIATES);

        let res: HttpResponse = ErrorInsufficientStorage("err").into();
        assert_eq!(res.status(), StatusCode::INSUFFICIENT_STORAGE);

        let res: HttpResponse = ErrorLoopDetected("err").into();
        assert_eq!(res.status(), StatusCode::LOOP_DETECTED);

        let res: HttpResponse = ErrorNotExtended("err").into();
        assert_eq!(res.status(), StatusCode::NOT_EXTENDED);

        let res: HttpResponse = ErrorNetworkAuthenticationRequired("err").into();
        assert_eq!(res.status(), StatusCode::NETWORK_AUTHENTICATION_REQUIRED);
    }
}
