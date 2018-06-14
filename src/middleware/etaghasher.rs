//! ETag header and `304 Not Modified` support for HTTP responses
///
/// The `EtagHasher` middleware generates [RFC
/// 7232](https://tools.ietf.org/html/rfc7232) ETag headers for `200 OK`
/// responses to HTTP `GET` requests, and checks the ETag for a response
/// against those provided in the `If-None-Match` header of the request,
/// if present.  In the event of a match, instead of returning the
/// original response, an HTTP `304 Not Modified` response with no
/// content is returned instead.  Only response [Body](enum.Body.html)s
/// of type `Binary` are supported; responses with other body types will
/// be left unchanged.
///
/// ETag values are generated by computing a hash function over the
/// bytes of the body of the original response. Thus, using this
/// middleware amounts to trading CPU resources for bandwidth. Some CPU
/// overhead is incurred by having to compute a hash for each response
/// body, but in return one avoids sending response bodies to requesters
/// that already have the body content cached.
///
/// This approach is most useful for dynamically generated responses
/// that don't correspond to a specific external resource (e.g. a
/// file). For such external resources, it's better to generate ETags
/// based on the inherent properties of the resource rather than by
/// hashing the bytes of an HTTP response corresponding to its
/// serialized representation as this middleware does.
///
/// An `EtagHasher` instance makes use of two functions, `hash` and
/// `filter`. The `hash` function takes the bytes of the original
/// response body as input and produces an ETag value. The `filter`
/// function takes the original HTTP request and response, and returns
/// `true` if ETag processing should be applied to this response and
/// `false` otherwise. These functions are supplied by the user when the
/// instance is created; the `DefaultHasher` and `DefaultFilter` can be
/// used if desired. Currently `DefaultHasher` computes an SHA-1 hash,
/// but this should not be relied upon. The `DefaultFilter` returns
/// `true` for all `(request, response)` pairs.
///
/// ```rust
/// # extern crate actix_web;
/// use actix_web::{http, middleware, App, HttpResponse};
/// use middleware::etaghasher::{EtagHasher, DefaultHasher, DefaultFilter};
///
/// fn main() {
///     let eh = EtagHasher::new(DefaultHasher::new(), DefaultFilter);
///     let app = App::new()
///         .middleware(eh)
///         .resource("/test", |r| {
///             r.method(http::Method::GET).f(|_| HttpResponse::Ok());
///         })
///         .finish();
/// }
/// ```
///
/// With custom `hash` and `filter` functions:
///
/// ```rust
/// # extern crate actix_web;
/// use actix_web::{http, middleware, App, HttpRequest, HttpResponse};
/// use middleware::etaghasher::EtagHasher;
///
/// fn main() {
///     let eh = EtagHasher::new(
///         |_input: &[u8]| "static".to_string(),
///         |_req: &HttpRequest<()>, _res: &HttpResponse| true,
///     );
///     let app = App::new()
///         .middleware(eh)
///         .resource("/test", |r| {
///             r.method(http::Method::GET).f(|_| HttpResponse::Ok());
///         })
///         .finish();
/// }
/// ```

use error::Result;
use header::EntityTag;
use httprequest::HttpRequest;
use httpresponse::HttpResponse;
use middleware;

use std::marker::PhantomData;

/// Can produce an ETag value from a byte slice. Per RFC 7232, **must only
/// produce** bytes with hex values `21`, `23-7E`, or greater than or equal
/// to `80`. Producing invalid bytes will result in a panic when the output
/// is converted to an ETag.
pub trait Hasher {
    /// Produce an ETag value given a byte slice.
    fn hash(&mut self, input: &[u8]) -> String;
}
/// Can test a (request, response) pair and return `true` or `false`
pub trait Filter<S> {
    /// Return `true` if ETag processing should be applied to this
    /// `(request, response)` pair and `false` otherwise. A `false` return
    /// value will immediately return the original response unchanged.
    fn filter(&self, req: &HttpRequest<S>, res: &HttpResponse) -> bool;
}

// Closure implementations
impl<F: FnMut(&[u8]) -> String> Hasher for F {
    fn hash(&mut self, input: &[u8]) -> String {
        self(input)
    }
}
impl<S, F: Fn(&HttpRequest<S>, &HttpResponse) -> bool> Filter<S> for F {
    fn filter(&self, req: &HttpRequest<S>, res: &HttpResponse) -> bool {
        self(req, res)
    }
}

// Defaults
/// Computes an ETag value from a byte slice using a default cryptographic hash
/// function.
pub struct DefaultHasher {
    hashstate: ::sha1::Sha1,
}
impl DefaultHasher {
    /// Create a new instance.
    pub fn new() -> Self {
        DefaultHasher {
            hashstate: ::sha1::Sha1::new()
        }
    }
}
impl Hasher for DefaultHasher {
    fn hash(&mut self, input: &[u8]) -> String {
        self.hashstate.reset();
        self.hashstate.update(input);
        self.hashstate.digest().to_string()
    }
}

/// Returns `true` for every `(request, response)` pair.
pub struct DefaultFilter;
impl<S> Filter<S> for DefaultFilter {
    fn filter(&self, _req: &HttpRequest<S>, _res: &HttpResponse) -> bool {
        true
    }
}

/// Middleware for [RFC 7232](https://tools.ietf.org/html/rfc7232) ETag
/// generation and comparison.
///
/// The `EtagHasher` struct contains a Hasher to compute ETag values for
/// byte slices and a Filter to determine whether ETag computation and
/// checking should be applied to a particular (request, response)
/// pair.
///
/// Middleware processing will be performed only if the following
/// conditions hold:
///
/// * The request method is `GET`
/// * The status of the original response is `200 OK`
/// * The type of the original response [Body](enum.Body.html) is `Binary`
///
/// If any of these conditions is false, the original response will be
/// passed through unmodified.
pub struct EtagHasher<S, H, F>
where
    S: 'static,
    H: Hasher + 'static,
    F: Filter<S> + 'static,
{
    hasher: H,
    filter: F,
    _phantom: PhantomData<S>,
}

impl<S, H, F> EtagHasher<S, H, F>
where
    S: 'static,
    H: Hasher + 'static,
    F: Filter<S> + 'static,
{
    /// Create a new middleware struct with the given Hasher and Filter.
    pub fn new(hasher: H, filter: F) -> Self {
        EtagHasher {
            hasher,
            filter,
            _phantom: PhantomData,
        }
    }
}

impl<S, H, F> middleware::Middleware<S> for EtagHasher<S, H, F>
where
    S: 'static,
    H: Hasher + 'static,
    F: Filter<S> + 'static,
{
    fn response(
        &mut self, req: &mut HttpRequest<S>, mut res: HttpResponse,
    ) -> Result<middleware::Response> {
        use http::{Method, StatusCode};
        use header;
        use Body;

        let valid = *req.method() == Method::GET && res.status() == StatusCode::OK;
        if !(valid && self.filter.filter(req, &res)) {
            return Ok(middleware::Response::Done(res));
        }

        let e = if let Body::Binary(b) = res.body() {
            Some(EntityTag::strong(self.hasher.hash(b.as_ref())))
        } else {
            None
        };

        if let Some(etag) = e {
            if !none_match(&etag, req) {
                let mut not_modified =
                    HttpResponse::NotModified().set(header::ETag(etag)).finish();

                // RFC 7232 requires copying over these headers:
                copy_header(header::CACHE_CONTROL, &res, &mut not_modified);
                copy_header(header::CONTENT_LOCATION, &res, &mut not_modified);
                copy_header(header::DATE, &res, &mut not_modified);
                copy_header(header::EXPIRES, &res, &mut not_modified);
                copy_header(header::VARY, &res, &mut not_modified);

                return Ok(middleware::Response::Done(not_modified));
            }
            etag.to_string()
                .parse::<header::HeaderValue>()
                .map(|v| {
                    res.headers_mut().insert(header::ETAG, v);
                })
                .unwrap_or(());
        }
        Ok(middleware::Response::Done(res))
    }
}

#[inline]
fn copy_header(h: ::header::HeaderName, src: &HttpResponse, dst: &mut HttpResponse) {
    if let Some(val) = src.headers().get(&h) {
        dst.headers_mut().insert(h, val.clone());
    }
}

// Returns true if `req` doesn't have an `If-None-Match` header matching `req`.
#[inline]
fn none_match<S>(etag: &EntityTag, req: &HttpRequest<S>) -> bool {
    use header::IfNoneMatch;
    use httpmessage::HttpMessage;
    match req.get_header::<IfNoneMatch>() {
        Some(IfNoneMatch::Items(ref items)) => {
            for item in items {
                if item.weak_eq(etag) {
                    return false;
                }
            }
            true
        }
        Some(IfNoneMatch::Any) => false,
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use header::ETAG;
    use http::StatusCode;
    use httpmessage::HttpMessage;
    use middleware::Middleware;
    use test::{TestRequest, TestServer};

    const TEST_BODY: &'static str = "test";
    const TEST_ETAG: &'static str = "\"a94a8fe5ccb19ba61c4c0873d391e987982fbbd3\"";
    struct TestState {
        _state: u32,
    }
    fn test_index<S>(_req: HttpRequest<S>) -> &'static str {
        TEST_BODY
    }

    fn mwres(r: Result<middleware::Response>) -> HttpResponse {
        match r {
            Ok(middleware::Response::Done(hr)) => hr,
            _ => panic!(),
        }
    }

    #[test]
    fn test_default_create_etag() {
        let mut eh = EtagHasher::new(DefaultHasher::new(), DefaultFilter);
        let mut req = TestRequest::default().finish();
        let res = HttpResponse::Ok().body(TEST_BODY);
        let res = mwres(eh.response(&mut req, res));
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.headers().get(ETAG).unwrap(), TEST_ETAG);
    }
    #[test]
    fn test_default_with_state_create_etag() {
        let state = TestState { _state: 0 };
        let mut eh = EtagHasher::new(DefaultHasher::new(), DefaultFilter);
        let mut req = TestRequest::with_state(state).finish();
        let res = HttpResponse::Ok().body(TEST_BODY);
        let res = mwres(eh.response(&mut req, res));
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.headers().get(ETAG).unwrap(), TEST_ETAG);
    }
    #[test]
    fn test_default_none_match() {
        let mut eh = EtagHasher::new(DefaultHasher::new(), DefaultFilter);
        let mut req = TestRequest::with_header("If-None-Match", "_").finish();
        let res = HttpResponse::Ok().body(TEST_BODY);
        let res = mwres(eh.response(&mut req, res));
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.headers().get(ETAG).unwrap(), TEST_ETAG);
    }
    #[test]
    fn test_default_match() {
        let mut eh = EtagHasher::new(DefaultHasher::new(), DefaultFilter);
        let mut req = TestRequest::with_header("If-None-Match", TEST_ETAG).finish();
        let res = HttpResponse::Ok().body(TEST_BODY);
        let res = mwres(eh.response(&mut req, res));
        assert_eq!(res.status(), StatusCode::NOT_MODIFIED);
    }
    #[test]
    fn test_custom_match() {
        let mut eh = EtagHasher::new(
            |_input: &[u8]| "static".to_string(),
            |_req: &HttpRequest<()>, _res: &HttpResponse| true,
        );
        let mut req = TestRequest::with_header("If-None-Match", "\"static\"").finish();
        let res = HttpResponse::Ok().body(TEST_BODY);
        let res = mwres(eh.response(&mut req, res));
        assert_eq!(res.status(), StatusCode::NOT_MODIFIED);
    }
    #[test]
    fn test_srv_default_create_etag() {
        let mut srv =
            TestServer::build_with_state(|| TestState { _state: 0 }).start(|app| {
                let eh = EtagHasher::new(DefaultHasher::new(), DefaultFilter);
                app.middleware(eh).handler(test_index)
            });

        let req = srv.get().finish().unwrap();
        let response = srv.execute(req.send()).unwrap();
        assert!(response.status().is_success());
        assert_eq!(response.headers().get(ETAG).unwrap(), TEST_ETAG);
    }
}
