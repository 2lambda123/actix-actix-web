//! Error and Result module
use std::{io, fmt, result};
use std::str::Utf8Error;
use std::string::FromUtf8Error;
use std::io::Error as IoError;

use cookie;
use httparse;
use actix::MailboxError;
use futures::Canceled;
use failure;
use failure::{Fail, Backtrace};
use http2::Error as Http2Error;
use http::{header, StatusCode, Error as HttpError};
use http::uri::InvalidUriBytes;
use http_range::HttpRangeParseError;
use serde_json::error::Error as JsonError;
pub use url::ParseError as UrlParseError;

// re-exports
pub use cookie::{ParseError as CookieParseError};

use body::Body;
use handler::Responder;
use httprequest::HttpRequest;
use httpresponse::HttpResponse;
use httpcodes::{self, HTTPBadRequest, HTTPMethodNotAllowed, HTTPExpectationFailed};

/// A specialized [`Result`](https://doc.rust-lang.org/std/result/enum.Result.html)
/// for actix web operations
///
/// This typedef is generally used to avoid writing out `actix_web::error::Error` directly and
/// is otherwise a direct mapping to `Result`.
pub type Result<T, E=Error> = result::Result<T, E>;

/// General purpose actix web error
pub struct Error {
    cause: Box<ResponseError>,
    backtrace: Option<Backtrace>,
}

impl Error {

    /// Returns a reference to the underlying cause of this Error.
    // this should return &Fail but needs this https://github.com/rust-lang/rust/issues/5665
    pub fn cause(&self) -> &ResponseError {
        self.cause.as_ref()
    }
}

/// Error that can be converted to `HttpResponse`
pub trait ResponseError: Fail {

    /// Create response for error
    ///
    /// Internal server error is generated by default.
    fn error_response(&self) -> HttpResponse {
        HttpResponse::new(StatusCode::INTERNAL_SERVER_ERROR, Body::Empty)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(&self.cause, f)
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if let Some(bt) = self.cause.backtrace() {
            write!(f, "{:?}\n\n{:?}", &self.cause, bt)
        } else {
            write!(f, "{:?}\n\n{:?}", &self.cause, self.backtrace.as_ref().unwrap())
        }
    }
}

/// `HttpResponse` for `Error`
impl From<Error> for HttpResponse {
    fn from(err: Error) -> Self {
        HttpResponse::from_error(err)
    }
}

/// `Error` for any error that implements `ResponseError`
impl<T: ResponseError> From<T> for Error {
    fn from(err: T) -> Error {
        let backtrace = if err.backtrace().is_none() {
            Some(Backtrace::new())
        } else {
            None
        };
        Error { cause: Box::new(err), backtrace: backtrace }
    }
}

/// Compatibility for `failure::Error`
impl<T> ResponseError for failure::Compat<T>
    where T: fmt::Display + fmt::Debug + Sync + Send + 'static
{ }

impl From<failure::Error> for Error {
    fn from(err: failure::Error) -> Error {
        err.compat().into()
    }
}

/// `InternalServerError` for `JsonError`
impl ResponseError for JsonError {}

/// `InternalServerError` for `UrlParseError`
impl ResponseError for UrlParseError {}

/// Return `InternalServerError` for `HttpError`,
/// Response generation can return `HttpError`, so it is internal error
impl ResponseError for HttpError {}

/// Return `InternalServerError` for `io::Error`
impl ResponseError for io::Error {

    fn error_response(&self) -> HttpResponse {
        match self.kind() {
            io::ErrorKind::NotFound =>
                HttpResponse::new(StatusCode::NOT_FOUND, Body::Empty),
            io::ErrorKind::PermissionDenied =>
                HttpResponse::new(StatusCode::FORBIDDEN, Body::Empty),
            _ =>
                HttpResponse::new(StatusCode::INTERNAL_SERVER_ERROR, Body::Empty)
        }
    }
}

/// `InternalServerError` for `InvalidHeaderValue`
impl ResponseError for header::InvalidHeaderValue {}

/// `InternalServerError` for `futures::Canceled`
impl ResponseError for Canceled {}

/// `InternalServerError` for `actix::MailboxError`
impl ResponseError for MailboxError {}

/// A set of errors that can occur during parsing HTTP streams
#[derive(Fail, Debug)]
pub enum ParseError {
    /// An invalid `Method`, such as `GE.T`.
    #[fail(display="Invalid Method specified")]
    Method,
    /// An invalid `Uri`, such as `exam ple.domain`.
    #[fail(display="Uri error: {}", _0)]
    Uri(InvalidUriBytes),
    /// An invalid `HttpVersion`, such as `HTP/1.1`
    #[fail(display="Invalid HTTP version specified")]
    Version,
    /// An invalid `Header`.
    #[fail(display="Invalid Header provided")]
    Header,
    /// A message head is too large to be reasonable.
    #[fail(display="Message head is too large")]
    TooLarge,
    /// A message reached EOF, but is not complete.
    #[fail(display="Message is incomplete")]
    Incomplete,
    /// An invalid `Status`, such as `1337 ELITE`.
    #[fail(display="Invalid Status provided")]
    Status,
    /// A timeout occurred waiting for an IO event.
    #[allow(dead_code)]
    #[fail(display="Timeout")]
    Timeout,
    /// An `io::Error` that occurred while trying to read or write to a network stream.
    #[fail(display="IO error: {}", _0)]
    Io(#[cause] IoError),
    /// Parsing a field as string failed
    #[fail(display="UTF8 error: {}", _0)]
    Utf8(#[cause] Utf8Error),
}

/// Return `BadRequest` for `ParseError`
impl ResponseError for ParseError {
    fn error_response(&self) -> HttpResponse {
        HttpResponse::new(StatusCode::BAD_REQUEST, Body::Empty)
    }
}

impl From<IoError> for ParseError {
    fn from(err: IoError) -> ParseError {
        ParseError::Io(err)
    }
}

impl From<Utf8Error> for ParseError {
    fn from(err: Utf8Error) -> ParseError {
        ParseError::Utf8(err)
    }
}

impl From<FromUtf8Error> for ParseError {
    fn from(err: FromUtf8Error) -> ParseError {
        ParseError::Utf8(err.utf8_error())
    }
}

impl From<httparse::Error> for ParseError {
    fn from(err: httparse::Error) -> ParseError {
        match err {
            httparse::Error::HeaderName | httparse::Error::HeaderValue |
                httparse::Error::NewLine | httparse::Error::Token => ParseError::Header,
            httparse::Error::Status => ParseError::Status,
            httparse::Error::TooManyHeaders => ParseError::TooLarge,
            httparse::Error::Version => ParseError::Version,
        }
    }
}

#[derive(Fail, Debug)]
/// A set of errors that can occur during payload parsing
pub enum PayloadError {
    /// A payload reached EOF, but is not complete.
    #[fail(display="A payload reached EOF, but is not complete.")]
    Incomplete,
    /// Content encoding stream corruption
    #[fail(display="Can not decode content-encoding.")]
    EncodingCorrupted,
    /// A payload reached size limit.
    #[fail(display="A payload reached size limit.")]
    Overflow,
    /// A payload length is unknown.
    #[fail(display="A payload length is unknown.")]
    UnknownLength,
    /// Parse error
    #[fail(display="{}", _0)]
    ParseError(#[cause] IoError),
    /// Http2 error
    #[fail(display="{}", _0)]
    Http2(#[cause] Http2Error),
}

impl From<IoError> for PayloadError {
    fn from(err: IoError) -> PayloadError {
        PayloadError::ParseError(err)
    }
}

/// `InternalServerError` for `PayloadError`
impl ResponseError for PayloadError {}

/// Return `BadRequest` for `cookie::ParseError`
impl ResponseError for cookie::ParseError {
    fn error_response(&self) -> HttpResponse {
        HttpResponse::new(StatusCode::BAD_REQUEST, Body::Empty)
    }
}

/// Http range header parsing error
#[derive(Fail, PartialEq, Debug)]
pub enum HttpRangeError {
    /// Returned if range is invalid.
    #[fail(display="Range header is invalid")]
    InvalidRange,
    /// Returned if first-byte-pos of all of the byte-range-spec
    /// values is greater than the content size.
    /// See `https://github.com/golang/go/commit/aa9b3d7`
    #[fail(display="First-byte-pos of all of the byte-range-spec values is greater than the content size")]
    NoOverlap,
}

/// Return `BadRequest` for `HttpRangeError`
impl ResponseError for HttpRangeError {
    fn error_response(&self) -> HttpResponse {
        HttpResponse::new(
            StatusCode::BAD_REQUEST, Body::from("Invalid Range header provided"))
    }
}

impl From<HttpRangeParseError> for HttpRangeError {
    fn from(err: HttpRangeParseError) -> HttpRangeError {
        match err {
            HttpRangeParseError::InvalidRange => HttpRangeError::InvalidRange,
            HttpRangeParseError::NoOverlap => HttpRangeError::NoOverlap,
        }
    }
}

/// A set of errors that can occur during parsing multipart streams
#[derive(Fail, Debug)]
pub enum MultipartError {
    /// Content-Type header is not found
    #[fail(display="No Content-type header found")]
    NoContentType,
    /// Can not parse Content-Type header
    #[fail(display="Can not parse Content-Type header")]
    ParseContentType,
    /// Multipart boundary is not found
    #[fail(display="Multipart boundary is not found")]
    Boundary,
    /// Error during field parsing
    #[fail(display="{}", _0)]
    Parse(#[cause] ParseError),
    /// Payload error
    #[fail(display="{}", _0)]
    Payload(#[cause] PayloadError),
}

impl From<ParseError> for MultipartError {
    fn from(err: ParseError) -> MultipartError {
        MultipartError::Parse(err)
    }
}

impl From<PayloadError> for MultipartError {
    fn from(err: PayloadError) -> MultipartError {
        MultipartError::Payload(err)
    }
}

/// Return `BadRequest` for `MultipartError`
impl ResponseError for MultipartError {

    fn error_response(&self) -> HttpResponse {
        HttpResponse::new(StatusCode::BAD_REQUEST, Body::Empty)
    }
}

/// Error during handling `Expect` header
#[derive(Fail, PartialEq, Debug)]
pub enum ExpectError {
    /// Expect header value can not be converted to utf8
    #[fail(display="Expect header value can not be converted to utf8")]
    Encoding,
    /// Unknown expect value
    #[fail(display="Unknown expect value")]
    UnknownExpect,
}

impl ResponseError for ExpectError {

    fn error_response(&self) -> HttpResponse {
        HTTPExpectationFailed.with_body("Unknown Expect")
    }
}

/// Websocket handshake errors
#[derive(Fail, PartialEq, Debug)]
pub enum WsHandshakeError {
    /// Only get method is allowed
    #[fail(display="Method not allowed")]
    GetMethodRequired,
    /// Upgrade header if not set to websocket
    #[fail(display="Websocket upgrade is expected")]
    NoWebsocketUpgrade,
    /// Connection header is not set to upgrade
    #[fail(display="Connection upgrade is expected")]
    NoConnectionUpgrade,
    /// Websocket version header is not set
    #[fail(display="Websocket version header is required")]
    NoVersionHeader,
    /// Unsupported websocket version
    #[fail(display="Unsupported version")]
    UnsupportedVersion,
    /// Websocket key is not set or wrong
    #[fail(display="Unknown websocket key")]
    BadWebsocketKey,
}

impl ResponseError for WsHandshakeError {

    fn error_response(&self) -> HttpResponse {
        match *self {
            WsHandshakeError::GetMethodRequired => {
                HTTPMethodNotAllowed
                    .build()
                    .header(header::ALLOW, "GET")
                    .finish()
                    .unwrap()
            }
            WsHandshakeError::NoWebsocketUpgrade =>
                HTTPBadRequest.with_reason("No WebSocket UPGRADE header found"),
            WsHandshakeError::NoConnectionUpgrade =>
                HTTPBadRequest.with_reason("No CONNECTION upgrade"),
            WsHandshakeError::NoVersionHeader =>
                HTTPBadRequest.with_reason("Websocket version header is required"),
            WsHandshakeError::UnsupportedVersion =>
                HTTPBadRequest.with_reason("Unsupported version"),
            WsHandshakeError::BadWebsocketKey =>
                HTTPBadRequest.with_reason("Handshake error"),
        }
    }
}

/// A set of errors that can occur during parsing urlencoded payloads
#[derive(Fail, Debug)]
pub enum UrlencodedError {
    /// Can not decode chunked transfer encoding
    #[fail(display="Can not decode chunked transfer encoding")]
    Chunked,
    /// Payload size is bigger than 256k
    #[fail(display="Payload size is bigger than 256k")]
    Overflow,
    /// Payload size is now known
    #[fail(display="Payload size is now known")]
    UnknownLength,
    /// Content type error
    #[fail(display="Content type error")]
    ContentType,
    /// Payload error
    #[fail(display="Error that occur during reading payload: {}", _0)]
    Payload(#[cause] PayloadError),
}

/// Return `BadRequest` for `UrlencodedError`
impl ResponseError for UrlencodedError {

    fn error_response(&self) -> HttpResponse {
        match *self {
            UrlencodedError::Overflow => httpcodes::HTTPPayloadTooLarge.into(),
            UrlencodedError::UnknownLength => httpcodes::HTTPLengthRequired.into(),
            _ => httpcodes::HTTPBadRequest.into(),
        }
    }
}

impl From<PayloadError> for UrlencodedError {
    fn from(err: PayloadError) -> UrlencodedError {
        UrlencodedError::Payload(err)
    }
}

/// A set of errors that can occur during parsing json payloads
#[derive(Fail, Debug)]
pub enum JsonPayloadError {
    /// Payload size is bigger than 256k
    #[fail(display="Payload size is bigger than 256k")]
    Overflow,
    /// Content type error
    #[fail(display="Content type error")]
    ContentType,
    /// Deserialize error
    #[fail(display="Json deserialize error: {}", _0)]
    Deserialize(#[cause] JsonError),
    /// Payload error
    #[fail(display="Error that occur during reading payload: {}", _0)]
    Payload(#[cause] PayloadError),
}

/// Return `BadRequest` for `UrlencodedError`
impl ResponseError for JsonPayloadError {

    fn error_response(&self) -> HttpResponse {
        match *self {
            JsonPayloadError::Overflow => httpcodes::HTTPPayloadTooLarge.into(),
            _ => httpcodes::HTTPBadRequest.into(),
        }
    }
}

impl From<PayloadError> for JsonPayloadError {
    fn from(err: PayloadError) -> JsonPayloadError {
        JsonPayloadError::Payload(err)
    }
}

impl From<JsonError> for JsonPayloadError {
    fn from(err: JsonError) -> JsonPayloadError {
        JsonPayloadError::Deserialize(err)
    }
}

/// Errors which can occur when attempting to interpret a segment string as a
/// valid path segment.
#[derive(Fail, Debug, PartialEq)]
pub enum UriSegmentError {
    /// The segment started with the wrapped invalid character.
    #[fail(display="The segment started with the wrapped invalid character")]
    BadStart(char),
    /// The segment contained the wrapped invalid character.
    #[fail(display="The segment contained the wrapped invalid character")]
    BadChar(char),
    /// The segment ended with the wrapped invalid character.
    #[fail(display="The segment ended with the wrapped invalid character")]
    BadEnd(char),
}

/// Return `BadRequest` for `UriSegmentError`
impl ResponseError for UriSegmentError {

    fn error_response(&self) -> HttpResponse {
        HttpResponse::new(StatusCode::BAD_REQUEST, Body::Empty)
    }
}

/// Errors which can occur when attempting to generate resource uri.
#[derive(Fail, Debug, PartialEq)]
pub enum UrlGenerationError {
    #[fail(display="Resource not found")]
    ResourceNotFound,
    #[fail(display="Not all path pattern covered")]
    NotEnoughElements,
    #[fail(display="Router is not available")]
    RouterNotAvailable,
    #[fail(display="{}", _0)]
    ParseError(#[cause] UrlParseError),
}

/// `InternalServerError` for `UrlGeneratorError`
impl ResponseError for UrlGenerationError {}

impl From<UrlParseError> for UrlGenerationError {
    fn from(err: UrlParseError) -> Self {
        UrlGenerationError::ParseError(err)
    }
}

/// Helper type that can wrap any error and generate custom response.
///
/// In following example any `io::Error` will be converted into "BAD REQUEST" response
/// as opposite to *INNTERNAL SERVER ERROR* which is defined by default.
///
/// ```rust
/// # extern crate actix_web;
/// # use actix_web::*;
/// use actix_web::fs::NamedFile;
///
/// fn index(req: HttpRequest) -> Result<fs::NamedFile> {
///    let f = NamedFile::open("test.txt").map_err(error::ErrorBadRequest)?;
///    Ok(f)
/// }
/// # fn main() {}
/// ```
pub struct InternalError<T> {
    cause: T,
    status: StatusCode,
    backtrace: Backtrace,
}

unsafe impl<T> Sync for InternalError<T> {}
unsafe impl<T> Send for InternalError<T> {}

impl<T> InternalError<T> {
    pub fn new(err: T, status: StatusCode) -> Self {
        InternalError {
            cause: err,
            status: status,
            backtrace: Backtrace::new(),
        }
    }
}

impl<T> Fail for InternalError<T>
    where T: Send + Sync + fmt::Debug + 'static
{
    fn backtrace(&self) -> Option<&Backtrace> {
        Some(&self.backtrace)
    }
}

impl<T> fmt::Debug for InternalError<T>
    where T: Send + Sync + fmt::Debug + 'static
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&self.cause, f)
    }
}

impl<T> fmt::Display for InternalError<T>
    where T: Send + Sync + fmt::Debug + 'static
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&self.cause, f)
    }
}

impl<T> ResponseError for InternalError<T>
    where T: Send + Sync + fmt::Debug + 'static
{
    fn error_response(&self) -> HttpResponse {
        HttpResponse::new(self.status, Body::Empty)
    }
}

impl<T> Responder for InternalError<T>
    where T: Send + Sync + fmt::Debug + 'static
{
    type Item = HttpResponse;
    type Error = Error;

    fn respond_to(self, _: HttpRequest) -> Result<HttpResponse, Error> {
        Err(self.into())
    }
}

/// Helper function that creates wrapper of any error and generate *BAD REQUEST* response.
#[allow(non_snake_case)]
pub fn ErrorBadRequest<T>(err: T) -> InternalError<T> {
    InternalError::new(err, StatusCode::BAD_REQUEST)
}

///  Helper function that creates wrapper of any error and generate *UNAUTHORIZED* response.
#[allow(non_snake_case)]
pub fn ErrorUnauthorized<T>(err: T) -> InternalError<T> {
    InternalError::new(err, StatusCode::UNAUTHORIZED)
}

///  Helper function that creates wrapper of any error and generate *FORBIDDEN* response.
#[allow(non_snake_case)]
pub fn ErrorForbidden<T>(err: T) -> InternalError<T> {
    InternalError::new(err, StatusCode::FORBIDDEN)
}

///  Helper function that creates wrapper of any error and generate *NOT FOUND* response.
#[allow(non_snake_case)]
pub fn ErrorNotFound<T>(err: T) -> InternalError<T> {
    InternalError::new(err, StatusCode::NOT_FOUND)
}

///  Helper function that creates wrapper of any error and generate *METHOD NOT ALLOWED* response.
#[allow(non_snake_case)]
pub fn ErrorMethodNotAllowed<T>(err: T) -> InternalError<T> {
    InternalError::new(err, StatusCode::METHOD_NOT_ALLOWED)
}

///  Helper function that creates wrapper of any error and generate *REQUEST TIMEOUT* response.
#[allow(non_snake_case)]
pub fn ErrorRequestTimeout<T>(err: T) -> InternalError<T> {
    InternalError::new(err, StatusCode::REQUEST_TIMEOUT)
}

///  Helper function that creates wrapper of any error and generate *CONFLICT* response.
#[allow(non_snake_case)]
pub fn ErrorConflict<T>(err: T) -> InternalError<T> {
    InternalError::new(err, StatusCode::CONFLICT)
}

///  Helper function that creates wrapper of any error and generate *GONE* response.
#[allow(non_snake_case)]
pub fn ErrorGone<T>(err: T) -> InternalError<T> {
    InternalError::new(err, StatusCode::GONE)
}

///  Helper function that creates wrapper of any error and generate *PRECONDITION FAILED* response.
#[allow(non_snake_case)]
pub fn ErrorPreconditionFailed<T>(err: T) -> InternalError<T> {
    InternalError::new(err, StatusCode::PRECONDITION_FAILED)
}

///  Helper function that creates wrapper of any error and generate *EXPECTATION FAILED* response.
#[allow(non_snake_case)]
pub fn ErrorExpectationFailed<T>(err: T) -> InternalError<T> {
    InternalError::new(err, StatusCode::EXPECTATION_FAILED)
}

///  Helper function that creates wrapper of any error and generate *INTERNAL SERVER ERROR* response.
#[allow(non_snake_case)]
pub fn ErrorInternalServerError<T>(err: T) -> InternalError<T> {
    InternalError::new(err, StatusCode::INTERNAL_SERVER_ERROR)
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::error::Error as StdError;
    use std::io;
    use httparse;
    use http::{StatusCode, Error as HttpError};
    use cookie::ParseError as CookieParseError;
    use failure;
    use super::*;

    #[test]
    #[cfg(actix_nightly)]
    fn test_nightly() {
        let resp: HttpResponse = IoError::new(io::ErrorKind::Other, "test").error_response();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn test_into_response() {
        let resp: HttpResponse = ParseError::Incomplete.error_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let resp: HttpResponse = HttpRangeError::InvalidRange.error_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let resp: HttpResponse = CookieParseError::EmptyName.error_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let resp: HttpResponse = MultipartError::Boundary.error_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let err: HttpError = StatusCode::from_u16(10000).err().unwrap().into();
        let resp: HttpResponse = err.error_response();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn test_cause() {
        let orig = io::Error::new(io::ErrorKind::Other, "other");
        let desc = orig.description().to_owned();
        let e = ParseError::Io(orig);
        assert_eq!(format!("{}", e.cause().unwrap()), desc);
    }

    #[test]
    fn test_error_cause() {
        let orig = io::Error::new(io::ErrorKind::Other, "other");
        let desc = orig.description().to_owned();
        let e = Error::from(orig);
        assert_eq!(format!("{}", e.cause()), desc);
    }

    #[test]
    fn test_error_display() {
        let orig = io::Error::new(io::ErrorKind::Other, "other");
        let desc = orig.description().to_owned();
        let e = Error::from(orig);
        assert_eq!(format!("{}", e), desc);
    }

    #[test]
    fn test_error_http_response() {
        let orig = io::Error::new(io::ErrorKind::Other, "other");
        let e = Error::from(orig);
        let resp: HttpResponse = e.into();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn test_range_error() {
        let e: HttpRangeError = HttpRangeParseError::InvalidRange.into();
        assert_eq!(e, HttpRangeError::InvalidRange);
        let e: HttpRangeError = HttpRangeParseError::NoOverlap.into();
        assert_eq!(e, HttpRangeError::NoOverlap);
    }

    #[test]
    fn test_expect_error() {
        let resp: HttpResponse = ExpectError::Encoding.error_response();
        assert_eq!(resp.status(), StatusCode::EXPECTATION_FAILED);
        let resp: HttpResponse = ExpectError::UnknownExpect.error_response();
        assert_eq!(resp.status(), StatusCode::EXPECTATION_FAILED);
    }

    #[test]
    fn test_wserror_http_response() {
        let resp: HttpResponse = WsHandshakeError::GetMethodRequired.error_response();
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
        let resp: HttpResponse = WsHandshakeError::NoWebsocketUpgrade.error_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let resp: HttpResponse = WsHandshakeError::NoConnectionUpgrade.error_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let resp: HttpResponse = WsHandshakeError::NoVersionHeader.error_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let resp: HttpResponse = WsHandshakeError::UnsupportedVersion.error_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let resp: HttpResponse = WsHandshakeError::BadWebsocketKey.error_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    macro_rules! from {
        ($from:expr => $error:pat) => {
            match ParseError::from($from) {
                e @ $error => {
                    assert!(format!("{}", e).len() >= 5);
                } ,
                e => panic!("{:?}", e)
            }
        }
    }

    macro_rules! from_and_cause {
        ($from:expr => $error:pat) => {
            match ParseError::from($from) {
                e @ $error => {
                    let desc = format!("{}", e.cause().unwrap());
                    assert_eq!(desc, $from.description().to_owned());
                },
                _ => panic!("{:?}", $from)
            }
        }
    }

    #[test]
    fn test_from() {
        from_and_cause!(io::Error::new(io::ErrorKind::Other, "other") => ParseError::Io(..));

        from!(httparse::Error::HeaderName => ParseError::Header);
        from!(httparse::Error::HeaderName => ParseError::Header);
        from!(httparse::Error::HeaderValue => ParseError::Header);
        from!(httparse::Error::NewLine => ParseError::Header);
        from!(httparse::Error::Status => ParseError::Status);
        from!(httparse::Error::Token => ParseError::Header);
        from!(httparse::Error::TooManyHeaders => ParseError::TooLarge);
        from!(httparse::Error::Version => ParseError::Version);
    }

    #[test]
    fn failure_error() {
        const NAME: &str = "RUST_BACKTRACE";
        let old_tb = env::var(NAME);
        env::set_var(NAME, "0");
        let error = failure::err_msg("Hello!");
        let resp: Error = error.into();
        assert_eq!(format!("{:?}", resp), "Compat { error: ErrorMessage { msg: \"Hello!\" } }\n\n");
        match old_tb {
            Ok(x) => env::set_var(NAME, x),
            _ => env::remove_var(NAME),
        }
    }
}
