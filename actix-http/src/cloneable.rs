use std::cell::RefCell;
use std::rc::Rc;
use std::task::{Context, Poll};

use actix_service::Service;

/// Service that allows to turn non-clone service to a service with `Clone` impl
///
/// # Panics
/// CloneableService might panic with some creative use of thread local storage.
/// See https://github.com/actix/actix-web/issues/1295 for example
#[doc(hidden)]
pub(crate) struct CloneableService<T>(Rc<RefCell<T>>);

impl<T> CloneableService<T> {
    pub(crate) fn new(service: T) -> Self {
        Self(Rc::new(RefCell::new(service)))
    }
}

impl<T> Clone for CloneableService<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T: Service<Req>, Req> Service<Req> for CloneableService<T> {
    type Response = T::Response;
    type Error = T::Error;
    type Future = T::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.0.borrow_mut().poll_ready(cx)
    }

    fn call(&mut self, req: Req) -> Self::Future {
        self.0.borrow_mut().call(req)
    }
}
