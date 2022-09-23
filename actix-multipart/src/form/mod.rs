//! Process and extract typed data from a multipart stream.
pub mod bytes;
pub mod json;
#[cfg(feature = "tempfile")]
pub mod tempfile;
pub mod text;

use crate::{Field, Multipart, MultipartError};
use actix_http::error::PayloadError;
use actix_web::dev::Payload;
use actix_web::{web, Error, FromRequest, HttpRequest};
use derive_more::{Deref, DerefMut};
use futures_core::future::LocalBoxFuture;
use futures_util::TryFutureExt;
use futures_util::{FutureExt, TryStreamExt};
use std::any::Any;
use std::collections::HashMap;
use std::future::{ready, Future};
use std::sync::Arc;

/// Implements the [`MultipartFormTrait`] for a struct so that it can be used with the
/// [`struct@MultipartForm`] extractor.
///
/// ## Simple Example
///
/// Each field type should implement the [`FieldReader`] trait:
///
/// ```
/// # use actix_multipart::form::tempfile::Tempfile;
/// # use actix_multipart::form::text::Text;
/// # use actix_multipart::form::MultipartForm;
/// #[derive(MultipartForm)]
/// struct ImageUpload {
///     description: Text<String>,
///     timestamp: Text<i64>,
///     image: Tempfile,
/// }
/// ```
///
/// ## Optional and List Fields
///
/// You can also use `Vec<T>` and `Option<T>` provided that `T: FieldReader`.
///
/// A [`Vec`] field corresponds to an upload with multiple parts under the
/// [same field name](https://www.rfc-editor.org/rfc/rfc7578#section-4.3).
///
/// ```
/// # use actix_multipart::form::tempfile::Tempfile;
/// # use actix_multipart::form::text::Text;
/// # use actix_multipart::form::MultipartForm;
/// #[derive(MultipartForm)]
/// struct Form {
///     category: Option<Text<String>>,
///     files: Vec<Tempfile>,
/// }
/// ```
///
/// ## Field Renaming
///
/// You can use the `#[multipart(rename="")]` attribute to receive a field by a different name.
///
/// ```
/// # use actix_multipart::form::tempfile::Tempfile;
/// # use actix_multipart::form::MultipartForm;
/// #[derive(MultipartForm)]
/// struct Form {
///     #[multipart(rename="files[]")]
///     files: Vec<Tempfile>,
/// }
/// ```
///
/// ## Field Limits
///
/// You can use the `#[multipart(limit="")]` attribute to set field level limits. The limit
/// string is parsed using [parse_size](https://docs.rs/parse-size/1.0.0/parse_size/).
///
/// Note: the form is also subject to the global limits configured using the
/// [`MultipartFormConfig`].
///
/// ```
/// # use actix_multipart::form::tempfile::Tempfile;
/// # use actix_multipart::form::text::Text;
/// # use actix_multipart::form::MultipartForm;
/// #[derive(MultipartForm)]
/// struct Form {
///     #[multipart(limit="2KiB")]
///     description: Text<String>,
///     #[multipart(limit="512MiB")]
///     files: Vec<Tempfile>,
/// }
/// ```
///
/// ## Unknown Fields
///
/// By default fields with an unknown name are ignored. You can change this using the
/// `#[multipart(deny_unknown_fields)]` attribute:
///
/// ```
/// # use actix_multipart::form::MultipartForm;
/// #[derive(MultipartForm)]
/// #[multipart(deny_unknown_fields)]
/// struct Form { }
/// ```
///
/// ## Duplicate Fields
///
/// You can change the behaviour for when multiple fields are received with the same name using the
/// `#[multipart(duplicate_action = "")]` attribute:
///
/// - "ignore": Extra fields are ignored (default).
/// - "replace": Each field is processed, but only the last one is persisted.
/// - "deny": A [MultipartError::UnsupportedField] error is returned.
///
/// (Note this option does not apply to `Vec` fields)
///
/// ```
/// # use actix_multipart::form::MultipartForm;
/// #[derive(MultipartForm)]
/// #[multipart(duplicate_action = "deny")]
/// struct Form { }
/// ```
pub use actix_multipart_derive::MultipartForm;

/// Trait that data types to be used in a multipart form struct should implement.
///
/// It represents an asynchronous handler that processes a multipart field to produce `Self`.
pub trait FieldReader<'t>: Sized + Any {
    /// Future that resolves to a `Self`.
    type Future: Future<Output = Result<Self, MultipartError>>;

    /// The form will call this function to handle the field.
    fn read_field(req: &'t HttpRequest, field: Field, limits: &'t mut Limits) -> Self::Future;
}

/// Used to accumulate the state of the loaded fields.
#[doc(hidden)]
#[derive(Default, Deref, DerefMut)]
pub struct State(pub HashMap<String, Box<dyn Any>>);

// Trait that the field collection types implement, i.e. `Vec<T>`, `Option<T>`, or `T` itself.
#[doc(hidden)]
pub trait FieldGroupReader<'t>: Sized + Any {
    type Future: Future<Output = Result<(), MultipartError>>;

    /// The form will call this function for each matching field
    fn handle_field(
        req: &'t HttpRequest,
        field: Field,
        limits: &'t mut Limits,
        state: &'t mut State,
        duplicate_action: DuplicateAction,
    ) -> Self::Future;

    /// Create `Self` from the group of processed fields
    fn from_state(name: &str, state: &'t mut State) -> Result<Self, MultipartError>;
}

impl<'t, T> FieldGroupReader<'t> for Option<T>
where
    T: FieldReader<'t>,
{
    type Future = LocalBoxFuture<'t, Result<(), MultipartError>>;

    fn handle_field(
        req: &'t HttpRequest,
        field: Field,
        limits: &'t mut Limits,
        state: &'t mut State,
        duplicate_action: DuplicateAction,
    ) -> Self::Future {
        if state.contains_key(field.name()) {
            match duplicate_action {
                DuplicateAction::Ignore => return ready(Ok(())).boxed_local(),
                DuplicateAction::Deny => {
                    return ready(Err(MultipartError::DuplicateField(
                        field.name().to_string(),
                    )))
                    .boxed_local()
                }
                DuplicateAction::Replace => {}
            }
        }
        async move {
            let field_name = field.name().to_string();
            let t = T::read_field(req, field, limits).await?;
            state.insert(field_name, Box::new(t));
            Ok(())
        }
        .boxed_local()
    }

    fn from_state(name: &str, state: &'t mut State) -> Result<Self, MultipartError> {
        Ok(state.remove(name).map(|m| *m.downcast::<T>().unwrap()))
    }
}

impl<'t, T> FieldGroupReader<'t> for Vec<T>
where
    T: FieldReader<'t>,
{
    type Future = LocalBoxFuture<'t, Result<(), MultipartError>>;

    fn handle_field(
        req: &'t HttpRequest,
        field: Field,
        limits: &'t mut Limits,
        state: &'t mut State,
        _duplicate_action: DuplicateAction,
    ) -> Self::Future {
        // Vec GroupReader always allows duplicates!
        async move {
            let field_name = field.name().to_string();
            let vec = state
                .entry(field_name)
                .or_insert_with(|| Box::new(Vec::<T>::new()))
                .downcast_mut::<Vec<T>>()
                .unwrap();
            let item = T::read_field(req, field, limits).await?;
            vec.push(item);
            Ok(())
        }
        .boxed_local()
    }

    fn from_state(name: &str, state: &'t mut State) -> Result<Self, MultipartError> {
        Ok(state
            .remove(name)
            .map(|m| *m.downcast::<Vec<T>>().unwrap())
            .unwrap_or_default())
    }
}

impl<'t, T> FieldGroupReader<'t> for T
where
    T: FieldReader<'t>,
{
    type Future = LocalBoxFuture<'t, Result<(), MultipartError>>;

    fn handle_field(
        req: &'t HttpRequest,
        field: Field,
        limits: &'t mut Limits,
        state: &'t mut State,
        duplicate_action: DuplicateAction,
    ) -> Self::Future {
        if state.contains_key(field.name()) {
            match duplicate_action {
                DuplicateAction::Ignore => return ready(Ok(())).boxed_local(),
                DuplicateAction::Deny => {
                    return ready(Err(MultipartError::DuplicateField(
                        field.name().to_string(),
                    )))
                    .boxed_local()
                }
                DuplicateAction::Replace => {}
            }
        }
        async move {
            let field_name = field.name().to_string();
            let t = T::read_field(req, field, limits).await?;
            state.insert(field_name, Box::new(t));
            Ok(())
        }
        .boxed_local()
    }

    fn from_state(name: &str, state: &'t mut State) -> Result<Self, MultipartError> {
        state
            .remove(name)
            .map(|m| *m.downcast::<T>().unwrap())
            .ok_or_else(|| MultipartError::MissingField(name.to_owned()))
    }
}

/// Trait that allows a type to be used in the [`struct@MultipartForm`] extractor. You should use
/// the [`macro@MultipartForm`] to implement this for your struct.
pub trait MultipartFormTrait: Sized {
    /// An optional limit in bytes to be applied a given field name. Note this limit will be shared
    /// across all fields sharing the same name.
    fn limit(field_name: &str) -> Option<usize>;

    /// The extractor will call this function for each incoming field, the state can be updated
    /// with the processed field data.
    fn handle_field<'t>(
        req: &'t HttpRequest,
        field: Field,
        limits: &'t mut Limits,
        state: &'t mut State,
    ) -> LocalBoxFuture<'t, Result<(), MultipartError>>;

    /// Once all the fields have been processed and stored in the state, this is called
    /// to convert into the struct representation.
    fn from_state(state: State) -> Result<Self, MultipartError>;
}

#[doc(hidden)]
pub enum DuplicateAction {
    /// Additional fields are not processed
    Ignore,
    /// An error will be raised
    Deny,
    /// All fields will be processed, the last one will replace all previous
    Replace,
}

/// Used to keep track of the remaining limits for the form and current field.
pub struct Limits {
    pub total_limit_remaining: usize,
    pub memory_limit_remaining: usize,
    pub field_limit_remaining: Option<usize>,
}

impl Limits {
    pub fn new(total_limit: usize, memory_limit: usize) -> Self {
        Self {
            total_limit_remaining: total_limit,
            memory_limit_remaining: memory_limit,
            field_limit_remaining: None,
        }
    }

    /// This function should be called within a [`FieldReader`] when reading each chunk of a field
    /// to ensure that the form limits are not exceeded.
    ///
    /// # Arguments
    ///
    /// * `bytes` - The number of bytes being read from this chunk
    /// * `in_memory` - Whether to consume from the memory limits
    pub fn try_consume_limits(
        &mut self,
        bytes: usize,
        in_memory: bool,
    ) -> Result<(), MultipartError> {
        self.total_limit_remaining = self
            .total_limit_remaining
            .checked_sub(bytes)
            .ok_or(MultipartError::Payload(PayloadError::Overflow))?;
        if in_memory {
            self.memory_limit_remaining = self
                .memory_limit_remaining
                .checked_sub(bytes)
                .ok_or(MultipartError::Payload(PayloadError::Overflow))?;
        }
        if let Some(field_limit) = self.field_limit_remaining {
            self.field_limit_remaining = Some(
                field_limit
                    .checked_sub(bytes)
                    .ok_or(MultipartError::Payload(PayloadError::Overflow))?,
            );
        }
        Ok(())
    }
}

/// Typed `multipart/form-data` extractor.
///
/// To extract typed data from a multipart stream, the inner type `T` must implement the
/// [`MultipartFormTrait`] trait, you should use the [`macro@MultipartForm`] macro to derive this for
/// your struct.
///
/// Use [`MultipartFormConfig`] to configure extraction options.
#[derive(Deref, DerefMut)]
pub struct MultipartForm<T: MultipartFormTrait>(pub T);

impl<T: MultipartFormTrait> MultipartForm<T> {
    /// Unwrap into inner `T` value.
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> FromRequest for MultipartForm<T>
where
    T: MultipartFormTrait,
{
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self, Self::Error>>;

    #[inline]
    fn from_request(req: &HttpRequest, payload: &mut Payload) -> Self::Future {
        let mut payload = Multipart::new(req.headers(), payload.take());
        let config = MultipartFormConfig::from_req(req);
        let mut limits = Limits::new(config.total_limit, config.memory_limit);
        let req = req.clone();
        let req2 = req.clone();
        let err_handler = config.err_handler.clone();

        async move {
            let mut state = State::default();
            // We need to ensure field limits are shared for all instances of this field name
            let mut field_limits = HashMap::<String, Option<usize>>::new();

            while let Some(field) = payload.try_next().await? {
                // Retrieve the limit for this field
                let entry = field_limits
                    .entry(field.name().to_owned())
                    .or_insert_with(|| T::limit(field.name()));
                limits.field_limit_remaining = entry.to_owned();

                T::handle_field(&req, field, &mut limits, &mut state).await?;

                // Update the stored limit
                *entry = limits.field_limit_remaining;
            }
            let inner = T::from_state(state)?;
            Ok(MultipartForm(inner))
        }
        .map_err(move |e| {
            if let Some(handler) = err_handler {
                (*handler)(e, &req2)
            } else {
                e.into()
            }
        })
        .boxed_local()
    }
}

type MultipartFormErrorHandler =
    Option<Arc<dyn Fn(MultipartError, &HttpRequest) -> Error + Send + Sync>>;

/// [`struct@MultipartForm`] extractor configuration.
#[derive(Clone)]
pub struct MultipartFormConfig {
    total_limit: usize,
    memory_limit: usize,
    err_handler: MultipartFormErrorHandler,
}

impl MultipartFormConfig {
    /// Set maximum accepted payload size for the entire form. By default this limit is 50MiB.
    pub fn total_limit(mut self, total_limit: usize) -> Self {
        self.total_limit = total_limit;
        self
    }

    /// Set maximum accepted data that will be read into memory. By default this limit is 2MiB.
    pub fn memory_limit(mut self, memory_limit: usize) -> Self {
        self.memory_limit = memory_limit;
        self
    }

    /// Set custom error handler.
    pub fn error_handler<F>(mut self, f: F) -> Self
    where
        F: Fn(MultipartError, &HttpRequest) -> Error + Send + Sync + 'static,
    {
        self.err_handler = Some(Arc::new(f));
        self
    }

    /// Extract payload config from app data. Check both `T` and `Data<T>`, in that order, and fall
    /// back to the default payload config.
    fn from_req(req: &HttpRequest) -> &Self {
        req.app_data::<Self>()
            .or_else(|| req.app_data::<web::Data<Self>>().map(|d| d.as_ref()))
            .unwrap_or(&DEFAULT_CONFIG)
    }
}

const DEFAULT_CONFIG: MultipartFormConfig = MultipartFormConfig {
    total_limit: 52_428_800, // 50 MiB
    memory_limit: 2_097_152, // 2 MiB
    err_handler: None,
};

impl Default for MultipartFormConfig {
    fn default() -> Self {
        DEFAULT_CONFIG.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::MultipartForm;
    use crate::form::bytes::Bytes;
    use crate::form::tempfile::Tempfile;
    use crate::form::text::Text;
    use crate::form::MultipartFormConfig;
    use actix_http::encoding::Decoder;
    use actix_http::Payload;
    use actix_multipart_rfc7578::client::multipart;
    use actix_test::TestServer;
    use actix_web::http::StatusCode;
    use actix_web::{web, App, HttpResponse, Responder};
    use awc::{Client, ClientResponse};

    pub async fn send_form(
        srv: &TestServer,
        form: multipart::Form<'static>,
        uri: &'static str,
    ) -> ClientResponse<Decoder<Payload>> {
        Client::default()
            .post(srv.url(uri))
            .content_type(form.content_type())
            .send_body(multipart::Body::from(form))
            .await
            .unwrap()
    }

    /// Test `Option` fields

    #[derive(MultipartForm)]
    struct TestOptions {
        field1: Option<Text<String>>,
        field2: Option<Text<String>>,
    }

    async fn test_options_route(form: MultipartForm<TestOptions>) -> impl Responder {
        assert!(form.field1.is_some());
        assert!(form.field2.is_none());
        HttpResponse::Ok().finish()
    }

    #[actix_rt::test]
    async fn test_options() {
        let srv =
            actix_test::start(|| App::new().route("/", web::post().to(test_options_route)));

        let mut form = multipart::Form::default();
        form.add_text("field1", "value");

        let response = send_form(&srv, form, "/").await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    /// Test `Vec` fields

    #[derive(MultipartForm)]
    struct TestVec {
        list1: Vec<Text<String>>,
        list2: Vec<Text<String>>,
    }

    async fn test_vec_route(form: MultipartForm<TestVec>) -> impl Responder {
        let form = form.into_inner();
        let strings = form
            .list1
            .into_iter()
            .map(|s| s.into_inner())
            .collect::<Vec<_>>();
        assert_eq!(strings, vec!["value1", "value2", "value3"]);
        assert_eq!(form.list2.len(), 0);
        HttpResponse::Ok().finish()
    }

    #[actix_rt::test]
    async fn test_vec() {
        let srv = actix_test::start(|| App::new().route("/", web::post().to(test_vec_route)));

        let mut form = multipart::Form::default();
        form.add_text("list1", "value1");
        form.add_text("list1", "value2");
        form.add_text("list1", "value3");

        let response = send_form(&srv, form, "/").await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    /// Test the `rename` field attribute

    #[derive(MultipartForm)]
    struct TestFieldRenaming {
        #[multipart(rename = "renamed")]
        field1: Text<String>,
        #[multipart(rename = "field1")]
        field2: Text<String>,
        field3: Text<String>,
    }

    async fn test_field_renaming_route(
        form: MultipartForm<TestFieldRenaming>,
    ) -> impl Responder {
        assert_eq!(&*form.field1, "renamed");
        assert_eq!(&*form.field2, "field1");
        assert_eq!(&*form.field3, "field3");
        HttpResponse::Ok().finish()
    }

    #[actix_rt::test]
    async fn test_field_renaming() {
        let srv = actix_test::start(|| {
            App::new().route("/", web::post().to(test_field_renaming_route))
        });

        let mut form = multipart::Form::default();
        form.add_text("renamed", "renamed");
        form.add_text("field1", "field1");
        form.add_text("field3", "field3");

        let response = send_form(&srv, form, "/").await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    /// Test the `deny_unknown_fields` struct attribute

    #[derive(MultipartForm)]
    #[multipart(deny_unknown_fields)]
    struct TestDenyUnknown {}

    #[derive(MultipartForm)]
    struct TestAllowUnknown {}

    async fn test_deny_unknown_route(_: MultipartForm<TestDenyUnknown>) -> impl Responder {
        HttpResponse::Ok().finish()
    }

    async fn test_allow_unknown_route(_: MultipartForm<TestAllowUnknown>) -> impl Responder {
        HttpResponse::Ok().finish()
    }

    #[actix_rt::test]
    async fn test_deny_unknown() {
        let srv = actix_test::start(|| {
            App::new()
                .route("/deny", web::post().to(test_deny_unknown_route))
                .route("/allow", web::post().to(test_allow_unknown_route))
        });

        let mut form = multipart::Form::default();
        form.add_text("unknown", "value");
        let response = send_form(&srv, form, "/deny").await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let mut form = multipart::Form::default();
        form.add_text("unknown", "value");
        let response = send_form(&srv, form, "/allow").await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    /// Test the `duplicate_action` struct attribute

    #[derive(MultipartForm)]
    #[multipart(duplicate_action = "deny")]
    struct TestDuplicateDeny {
        _field: Text<String>,
    }

    #[derive(MultipartForm)]
    #[multipart(duplicate_action = "replace")]
    struct TestDuplicateReplace {
        field: Text<String>,
    }

    #[derive(MultipartForm)]
    #[multipart(duplicate_action = "ignore")]
    struct TestDuplicateIgnore {
        field: Text<String>,
    }

    async fn test_duplicate_deny_route(_: MultipartForm<TestDuplicateDeny>) -> impl Responder {
        HttpResponse::Ok().finish()
    }

    async fn test_duplicate_replace_route(
        form: MultipartForm<TestDuplicateReplace>,
    ) -> impl Responder {
        assert_eq!(&*form.field, "second_value");
        HttpResponse::Ok().finish()
    }

    async fn test_duplicate_ignore_route(
        form: MultipartForm<TestDuplicateIgnore>,
    ) -> impl Responder {
        assert_eq!(&*form.field, "first_value");
        HttpResponse::Ok().finish()
    }

    #[actix_rt::test]
    async fn test_duplicate_action() {
        let srv = actix_test::start(|| {
            App::new()
                .route("/deny", web::post().to(test_duplicate_deny_route))
                .route("/replace", web::post().to(test_duplicate_replace_route))
                .route("/ignore", web::post().to(test_duplicate_ignore_route))
        });

        let mut form = multipart::Form::default();
        form.add_text("_field", "first_value");
        form.add_text("_field", "second_value");
        let response = send_form(&srv, form, "/deny").await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let mut form = multipart::Form::default();
        form.add_text("field", "first_value");
        form.add_text("field", "second_value");
        let response = send_form(&srv, form, "/replace").await;
        assert_eq!(response.status(), StatusCode::OK);

        let mut form = multipart::Form::default();
        form.add_text("field", "first_value");
        form.add_text("field", "second_value");
        let response = send_form(&srv, form, "/ignore").await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    /// Test the Limits

    #[derive(MultipartForm)]
    struct TestMemoryUploadLimits {
        field: Bytes,
    }

    #[derive(MultipartForm)]
    struct TestFileUploadLimits {
        field: Tempfile,
    }

    async fn test_upload_limits_memory(
        form: MultipartForm<TestMemoryUploadLimits>,
    ) -> impl Responder {
        assert!(form.field.data.len() > 0);
        HttpResponse::Ok().finish()
    }

    async fn test_upload_limits_file(
        form: MultipartForm<TestFileUploadLimits>,
    ) -> impl Responder {
        assert!(form.field.size > 0);
        HttpResponse::Ok().finish()
    }

    #[actix_rt::test]
    async fn test_memory_limits() {
        let srv = actix_test::start(|| {
            App::new()
                .route("/text", web::post().to(test_upload_limits_memory))
                .route("/file", web::post().to(test_upload_limits_file))
                .app_data(
                    MultipartFormConfig::default()
                        .memory_limit(20)
                        .total_limit(usize::MAX),
                )
        });

        // Exceeds the 20 byte memory limit
        let mut form = multipart::Form::default();
        form.add_text("field", "this string is 28 bytes long");
        let response = send_form(&srv, form, "/text").await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        // Memory limit should not apply when the data is being streamed to disk
        let mut form = multipart::Form::default();
        form.add_text("field", "this string is 28 bytes long");
        let response = send_form(&srv, form, "/file").await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[actix_rt::test]
    async fn test_total_limit() {
        let srv = actix_test::start(|| {
            App::new()
                .route("/text", web::post().to(test_upload_limits_memory))
                .route("/file", web::post().to(test_upload_limits_file))
                .app_data(
                    MultipartFormConfig::default()
                        .memory_limit(usize::MAX)
                        .total_limit(20),
                )
        });

        // Within the 20 byte limit
        let mut form = multipart::Form::default();
        form.add_text("field", "7 bytes");
        let response = send_form(&srv, form, "/text").await;
        assert_eq!(response.status(), StatusCode::OK);

        // Exceeds the 20 byte overall limit
        let mut form = multipart::Form::default();
        form.add_text("field", "this string is 28 bytes long");
        let response = send_form(&srv, form, "/text").await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        // Exceeds the 20 byte overall limit
        let mut form = multipart::Form::default();
        form.add_text("field", "this string is 28 bytes long");
        let response = send_form(&srv, form, "/file").await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[derive(MultipartForm)]
    struct TestFieldLevelLimits {
        #[multipart(limit = "30B")]
        field: Vec<Bytes>,
    }

    async fn test_field_level_limits_route(
        form: MultipartForm<TestFieldLevelLimits>,
    ) -> impl Responder {
        assert!(form.field.len() > 0);
        HttpResponse::Ok().finish()
    }

    #[actix_rt::test]
    async fn test_field_level_limits() {
        let srv = actix_test::start(|| {
            App::new()
                .route("/", web::post().to(test_field_level_limits_route))
                .app_data(
                    MultipartFormConfig::default()
                        .memory_limit(usize::MAX)
                        .total_limit(usize::MAX),
                )
        });

        // Within the 30 byte limit
        let mut form = multipart::Form::default();
        form.add_text("field", "this string is 28 bytes long");
        let response = send_form(&srv, form, "/").await;
        assert_eq!(response.status(), StatusCode::OK);

        // Exceeds the the 30 byte limit
        let mut form = multipart::Form::default();
        form.add_text("field", "this string is more than 30 bytes long");
        let response = send_form(&srv, form, "/").await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        // Total of values (14 bytes) is within 30 byte limit for "field"
        let mut form = multipart::Form::default();
        form.add_text("field", "7 bytes");
        form.add_text("field", "7 bytes");
        let response = send_form(&srv, form, "/").await;
        assert_eq!(response.status(), StatusCode::OK);

        // Total of values exceeds 30 byte limit for "field"
        let mut form = multipart::Form::default();
        form.add_text("field", "this string is 28 bytes long");
        form.add_text("field", "this string is 28 bytes long");
        let response = send_form(&srv, form, "/").await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
