use std::fmt;

use actix_http::Request;
use actix_service::IntoServiceFactory;
use serde::de::DeserializeOwned;

use crate::{
    body::{self, MessageBody},
    config::AppConfig,
    dev::{Service, ServiceFactory},
    service::ServiceResponse,
    web::Bytes,
    Error,
};

/// Initialize service from application builder instance.
///
/// # Examples
/// ```
/// use actix_service::Service;
/// use actix_web::{test, web, App, HttpResponse, http::StatusCode};
///
/// #[actix_web::test]
/// async fn test_init_service() {
///     let app = test::init_service(
///         App::new()
///             .service(web::resource("/test").to(|| async { "OK" }))
///     ).await;
///
///     // Create request object
///     let req = test::TestRequest::with_uri("/test").to_request();
///
///     // Execute application
///     let res = app.call(req).await.unwrap();
///     assert_eq!(res.status(), StatusCode::OK);
/// }
/// ```
///
/// # Panics
/// Panics if service initialization returns an error.
pub async fn init_service<R, S, B, E>(
    app: R,
) -> impl Service<Request, Response = ServiceResponse<B>, Error = E>
where
    R: IntoServiceFactory<S, Request>,
    S: ServiceFactory<Request, Config = AppConfig, Response = ServiceResponse<B>, Error = E>,
    S::InitError: std::fmt::Debug,
{
    try_init_service(app)
        .await
        .expect("service initialization failed")
}

/// Fallible version of [`init_service`] that allows testing initialization errors.
pub(crate) async fn try_init_service<R, S, B, E>(
    app: R,
) -> Result<impl Service<Request, Response = ServiceResponse<B>, Error = E>, S::InitError>
where
    R: IntoServiceFactory<S, Request>,
    S: ServiceFactory<Request, Config = AppConfig, Response = ServiceResponse<B>, Error = E>,
    S::InitError: std::fmt::Debug,
{
    let srv = app.into_factory();
    srv.new_service(AppConfig::default()).await
}

/// Calls service and waits for response future completion.
///
/// # Examples
/// ```
/// use actix_web::{test, web, App, HttpResponse, http::StatusCode};
///
/// #[actix_web::test]
/// async fn test_response() {
///     let app = test::init_service(
///         App::new()
///             .service(web::resource("/test").to(|| async {
///                 HttpResponse::Ok()
///             }))
///     ).await;
///
///     // Create request object
///     let req = test::TestRequest::with_uri("/test").to_request();
///
///     // Call application
///     let res = test::call_service(&app, req).await;
///     assert_eq!(res.status(), StatusCode::OK);
/// }
/// ```
///
/// # Panics
/// Panics if service call returns error.
pub async fn call_service<S, R, B, E>(app: &S, req: R) -> S::Response
where
    S: Service<R, Response = ServiceResponse<B>, Error = E>,
    E: std::fmt::Debug,
{
    app.call(req)
        .await
        .expect("test service call returned error")
}

/// Helper function that returns a response body of a TestRequest
///
/// # Examples
/// ```
/// use actix_web::{test, web, App, HttpResponse, http::header};
/// use bytes::Bytes;
///
/// #[actix_web::test]
/// async fn test_index() {
///     let app = test::init_service(
///         App::new().service(
///             web::resource("/index.html")
///                 .route(web::post().to(|| async {
///                     HttpResponse::Ok().body("welcome!")
///                 })))
///     ).await;
///
///     let req = test::TestRequest::post()
///         .uri("/index.html")
///         .header(header::CONTENT_TYPE, "application/json")
///         .to_request();
///
///     let result = test::call_and_read_body(&app, req).await;
///     assert_eq!(result, Bytes::from_static(b"welcome!"));
/// }
/// ```
///
/// # Panics
/// Panics if:
/// - service call returns error;
/// - body yields an error while it is being read.
pub async fn call_and_read_body<S, B>(app: &S, req: Request) -> Bytes
where
    S: Service<Request, Response = ServiceResponse<B>, Error = Error>,
    B: MessageBody,
    B::Error: fmt::Debug,
{
    let res = call_service(app, req).await;
    read_body(res).await
}

#[doc(hidden)]
#[deprecated(since = "4.0.0", note = "Renamed to `call_and_read_body`.")]
pub async fn read_response<S, B>(app: &S, req: Request) -> Bytes
where
    S: Service<Request, Response = ServiceResponse<B>, Error = Error>,
    B: MessageBody,
    B::Error: fmt::Debug,
{
    let res = call_service(app, req).await;
    read_body(res).await
}

/// Helper function that returns a response body of a ServiceResponse.
///
/// # Examples
/// ```
/// use actix_web::{test, web, App, HttpResponse, http::header};
/// use bytes::Bytes;
///
/// #[actix_web::test]
/// async fn test_index() {
///     let app = test::init_service(
///         App::new().service(
///             web::resource("/index.html")
///                 .route(web::post().to(|| async {
///                     HttpResponse::Ok().body("welcome!")
///                 })))
///     ).await;
///
///     let req = test::TestRequest::post()
///         .uri("/index.html")
///         .header(header::CONTENT_TYPE, "application/json")
///         .to_request();
///
///     let res = test::call_service(&app, req).await;
///     let result = test::read_body(res).await;
///     assert_eq!(result, Bytes::from_static(b"welcome!"));
/// }
/// ```
///
/// # Panics
/// Panics if body yields an error while it is being read.
pub async fn read_body<B>(res: ServiceResponse<B>) -> Bytes
where
    B: MessageBody,
    B::Error: fmt::Debug,
{
    let body = res.into_body();
    body::to_bytes(body)
        .await
        .expect("error reading test response body")
}

/// Helper function that returns a deserialized response body of a ServiceResponse.
///
/// # Examples
/// ```
/// use actix_web::{App, test, web, HttpResponse, http::header};
/// use serde::{Serialize, Deserialize};
///
/// #[derive(Serialize, Deserialize)]
/// pub struct Person {
///     id: String,
///     name: String,
/// }
///
/// #[actix_web::test]
/// async fn test_post_person() {
///     let app = test::init_service(
///         App::new().service(
///             web::resource("/people")
///                 .route(web::post().to(|person: web::Json<Person>| async {
///                     HttpResponse::Ok()
///                         .json(person)})
///                     ))
///     ).await;
///
///     let payload = r#"{"id":"12345","name":"User name"}"#.as_bytes();
///
///     let res = test::TestRequest::post()
///         .uri("/people")
///         .header(header::CONTENT_TYPE, "application/json")
///         .set_payload(payload)
///         .send_request(&mut app)
///         .await;
///
///     assert!(res.status().is_success());
///
///     let result: Person = test::read_body_json(res).await;
/// }
/// ```
///
/// # Panics
/// Panics if:
/// - body yields an error while it is being read;
/// - received body is not a valid JSON representation of `T`.
pub async fn read_body_json<T, B>(res: ServiceResponse<B>) -> T
where
    B: MessageBody,
    B::Error: fmt::Debug,
    T: DeserializeOwned,
{
    let body = read_body(res).await;

    serde_json::from_slice(&body).unwrap_or_else(|err| {
        panic!(
            "could not deserialize body into a {}\nerr: {}\nbody: {:?}",
            std::any::type_name::<T>(),
            err,
            body,
        )
    })
}

/// Helper function that returns a deserialized response body of a TestRequest
///
/// # Examples
/// ```
/// use actix_web::{App, test, web, HttpResponse, http::header};
/// use serde::{Serialize, Deserialize};
///
/// #[derive(Serialize, Deserialize)]
/// pub struct Person {
///     id: String,
///     name: String
/// }
///
/// #[actix_web::test]
/// async fn test_add_person() {
///     let app = test::init_service(
///         App::new().service(
///             web::resource("/people")
///                 .route(web::post().to(|person: web::Json<Person>| async {
///                     HttpResponse::Ok()
///                         .json(person)})
///                     ))
///     ).await;
///
///     let payload = r#"{"id":"12345","name":"User name"}"#.as_bytes();
///
///     let req = test::TestRequest::post()
///         .uri("/people")
///         .header(header::CONTENT_TYPE, "application/json")
///         .set_payload(payload)
///         .to_request();
///
///     let result: Person = test::call_and_read_body_json(&mut app, req).await;
/// }
/// ```
///
/// # Panics
/// Panics if:
/// - service call returns an error body yields an error while it is being read;
/// - body yields an error while it is being read;
/// - received body is not a valid JSON representation of `T`.
pub async fn call_and_read_body_json<S, B, T>(app: &S, req: Request) -> T
where
    S: Service<Request, Response = ServiceResponse<B>, Error = Error>,
    B: MessageBody,
    B::Error: fmt::Debug,
    T: DeserializeOwned,
{
    let res = call_service(app, req).await;
    read_body_json(res).await
}

#[doc(hidden)]
#[deprecated(since = "4.0.0", note = "Renamed to `call_and_read_body_json`.")]
pub async fn read_response_json<S, B, T>(app: &S, req: Request) -> T
where
    S: Service<Request, Response = ServiceResponse<B>, Error = Error>,
    B: MessageBody,
    B::Error: fmt::Debug,
    T: DeserializeOwned,
{
    call_and_read_body_json(app, req).await
}

#[cfg(test)]
mod tests {

    use serde::{Deserialize, Serialize};

    use super::*;
    use crate::{http::header, test::TestRequest, web, App, HttpMessage, HttpResponse};

    #[actix_rt::test]
    async fn test_request_methods() {
        let app = init_service(
            App::new().service(
                web::resource("/index.html")
                    .route(web::put().to(|| HttpResponse::Ok().body("put!")))
                    .route(web::patch().to(|| HttpResponse::Ok().body("patch!")))
                    .route(web::delete().to(|| HttpResponse::Ok().body("delete!"))),
            ),
        )
        .await;

        let put_req = TestRequest::put()
            .uri("/index.html")
            .insert_header((header::CONTENT_TYPE, "application/json"))
            .to_request();

        let result = read_response(&app, put_req).await;
        assert_eq!(result, Bytes::from_static(b"put!"));

        let patch_req = TestRequest::patch()
            .uri("/index.html")
            .insert_header((header::CONTENT_TYPE, "application/json"))
            .to_request();

        let result = read_response(&app, patch_req).await;
        assert_eq!(result, Bytes::from_static(b"patch!"));

        let delete_req = TestRequest::delete().uri("/index.html").to_request();
        let result = read_response(&app, delete_req).await;
        assert_eq!(result, Bytes::from_static(b"delete!"));
    }

    #[derive(Serialize, Deserialize)]
    pub struct Person {
        id: String,
        name: String,
    }

    #[actix_rt::test]
    async fn test_response_json() {
        let app = init_service(App::new().service(web::resource("/people").route(
            web::post().to(|person: web::Json<Person>| HttpResponse::Ok().json(person)),
        )))
        .await;

        let payload = r#"{"id":"12345","name":"User name"}"#.as_bytes();

        let req = TestRequest::post()
            .uri("/people")
            .insert_header((header::CONTENT_TYPE, "application/json"))
            .set_payload(payload)
            .to_request();

        let result: Person = read_response_json(&app, req).await;
        assert_eq!(&result.id, "12345");
    }

    #[actix_rt::test]
    async fn test_body_json() {
        let app = init_service(App::new().service(web::resource("/people").route(
            web::post().to(|person: web::Json<Person>| HttpResponse::Ok().json(person)),
        )))
        .await;

        let payload = r#"{"id":"12345","name":"User name"}"#.as_bytes();

        let res = TestRequest::post()
            .uri("/people")
            .insert_header((header::CONTENT_TYPE, "application/json"))
            .set_payload(payload)
            .send_request(&app)
            .await;

        let result: Person = read_body_json(res).await;
        assert_eq!(&result.name, "User name");
    }

    #[actix_rt::test]
    async fn test_request_response_form() {
        let app = init_service(App::new().service(web::resource("/people").route(
            web::post().to(|person: web::Form<Person>| HttpResponse::Ok().json(person)),
        )))
        .await;

        let payload = Person {
            id: "12345".to_string(),
            name: "User name".to_string(),
        };

        let req = TestRequest::post()
            .uri("/people")
            .set_form(&payload)
            .to_request();

        assert_eq!(req.content_type(), "application/x-www-form-urlencoded");

        let result: Person = read_response_json(&app, req).await;
        assert_eq!(&result.id, "12345");
        assert_eq!(&result.name, "User name");
    }

    #[actix_rt::test]
    async fn test_response() {
        let app = init_service(
            App::new().service(
                web::resource("/index.html")
                    .route(web::post().to(|| HttpResponse::Ok().body("welcome!"))),
            ),
        )
        .await;

        let req = TestRequest::post()
            .uri("/index.html")
            .insert_header((header::CONTENT_TYPE, "application/json"))
            .to_request();

        let result = read_response(&app, req).await;
        assert_eq!(result, Bytes::from_static(b"welcome!"));
    }

    #[actix_rt::test]
    async fn test_request_response_json() {
        let app = init_service(App::new().service(web::resource("/people").route(
            web::post().to(|person: web::Json<Person>| HttpResponse::Ok().json(person)),
        )))
        .await;

        let payload = Person {
            id: "12345".to_string(),
            name: "User name".to_string(),
        };

        let req = TestRequest::post()
            .uri("/people")
            .set_json(&payload)
            .to_request();

        assert_eq!(req.content_type(), "application/json");

        let result: Person = read_response_json(&app, req).await;
        assert_eq!(&result.id, "12345");
        assert_eq!(&result.name, "User name");
    }
}
