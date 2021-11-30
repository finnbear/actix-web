use actix_http::{
    body::BoxBody,
    http::{header::IntoHeaderPair, Error as HttpError, HeaderMap, StatusCode},
};
use bytes::{Bytes, BytesMut};

use crate::{Error, HttpRequest, HttpResponse, HttpResponseBuilder};

/// Trait implemented by types that can be converted to an HTTP response.
///
/// Any types that implement this trait can be used in the return type of a handler.
pub trait Responder {
    /// Convert self to `HttpResponse`.
    fn respond_to(self, req: &HttpRequest) -> HttpResponse<BoxBody>;

    /// Override a status code for a Responder.
    ///
    /// ```
    /// use actix_web::{http::StatusCode, HttpRequest, Responder};
    ///
    /// fn index(req: HttpRequest) -> impl Responder {
    ///     "Welcome!".with_status(StatusCode::OK)
    /// }
    /// ```
    fn with_status(self, status: StatusCode) -> CustomResponder<Self>
    where
        Self: Sized,
    {
        CustomResponder::new(self).with_status(status)
    }

    /// Insert header to the final response.
    ///
    /// Overrides other headers with the same name.
    ///
    /// ```
    /// use actix_web::{web, HttpRequest, Responder};
    /// use serde::Serialize;
    ///
    /// #[derive(Serialize)]
    /// struct MyObj {
    ///     name: String,
    /// }
    ///
    /// fn index(req: HttpRequest) -> impl Responder {
    ///     web::Json(MyObj { name: "Name".to_owned() })
    ///         .with_header(("x-version", "1.2.3"))
    /// }
    /// ```
    fn with_header<H>(self, header: H) -> CustomResponder<Self>
    where
        Self: Sized,
        H: IntoHeaderPair,
    {
        CustomResponder::new(self).with_header(header)
    }
}

impl Responder for HttpResponse {
    #[inline]
    fn respond_to(self, _: &HttpRequest) -> HttpResponse<BoxBody> {
        self
    }
}

impl Responder for actix_http::Response<BoxBody> {
    #[inline]
    fn respond_to(self, _: &HttpRequest) -> HttpResponse<BoxBody> {
        HttpResponse::from(self)
    }
}

impl Responder for HttpResponseBuilder {
    #[inline]
    fn respond_to(mut self, _: &HttpRequest) -> HttpResponse<BoxBody> {
        self.finish()
    }
}

impl Responder for actix_http::ResponseBuilder {
    #[inline]
    fn respond_to(mut self, req: &HttpRequest) -> HttpResponse<BoxBody> {
        self.finish().map_into_boxed_body().respond_to(req)
    }
}

impl<T: Responder> Responder for Option<T> {
    fn respond_to(self, req: &HttpRequest) -> HttpResponse<BoxBody> {
        match self {
            Some(val) => val.respond_to(req),
            None => HttpResponse::new(StatusCode::NOT_FOUND),
        }
    }
}

impl<T, E> Responder for Result<T, E>
where
    T: Responder,
    E: Into<Error>,
{
    fn respond_to(self, req: &HttpRequest) -> HttpResponse<BoxBody> {
        match self {
            Ok(val) => val.respond_to(req),
            Err(e) => HttpResponse::from_error(e.into()),
        }
    }
}

impl<T: Responder> Responder for (T, StatusCode) {
    fn respond_to(self, req: &HttpRequest) -> HttpResponse<BoxBody> {
        let mut res = self.0.respond_to(req);
        *res.status_mut() = self.1;
        res
    }
}

macro_rules! impl_responder {
    ($res: ty, $ct: path) => {
        impl Responder for $res {
            fn respond_to(self, _: &HttpRequest) -> HttpResponse<BoxBody> {
                HttpResponse::Ok().content_type($ct).body(self)
            }
        }
    };
}

impl_responder!(&'static str, mime::TEXT_PLAIN_UTF_8);

impl_responder!(String, mime::TEXT_PLAIN_UTF_8);

// impl_responder!(&'_ String, mime::TEXT_PLAIN_UTF_8);

// impl_responder!(Cow<'_, str>, mime::TEXT_PLAIN_UTF_8);

impl_responder!(&'static [u8], mime::APPLICATION_OCTET_STREAM);

impl_responder!(Bytes, mime::APPLICATION_OCTET_STREAM);

impl_responder!(BytesMut, mime::APPLICATION_OCTET_STREAM);

/// Allows overriding status code and headers for a responder.
pub struct CustomResponder<T> {
    responder: T,
    status: Option<StatusCode>,
    headers: Result<HeaderMap, HttpError>,
}

impl<T: Responder> CustomResponder<T> {
    fn new(responder: T) -> Self {
        CustomResponder {
            responder,
            status: None,
            headers: Ok(HeaderMap::new()),
        }
    }

    /// Override a status code for the Responder's response.
    ///
    /// ```
    /// use actix_web::{HttpRequest, Responder, http::StatusCode};
    ///
    /// fn index(req: HttpRequest) -> impl Responder {
    ///     "Welcome!".with_status(StatusCode::OK)
    /// }
    /// ```
    pub fn with_status(mut self, status: StatusCode) -> Self {
        self.status = Some(status);
        self
    }

    /// Insert header to the final response.
    ///
    /// Overrides other headers with the same name.
    ///
    /// ```
    /// use actix_web::{web, HttpRequest, Responder};
    /// use serde::Serialize;
    ///
    /// #[derive(Serialize)]
    /// struct MyObj {
    ///     name: String,
    /// }
    ///
    /// fn index(req: HttpRequest) -> impl Responder {
    ///     web::Json(MyObj { name: "Name".to_string() })
    ///         .with_header(("x-version", "1.2.3"))
    ///         .with_header(("x-version", "1.2.3"))
    /// }
    /// ```
    pub fn with_header<H>(mut self, header: H) -> Self
    where
        H: IntoHeaderPair,
    {
        if let Ok(ref mut headers) = self.headers {
            match header.try_into_header_pair() {
                Ok((key, value)) => headers.append(key, value),
                Err(e) => self.headers = Err(e.into()),
            };
        }

        self
    }
}

impl<T: Responder> Responder for CustomResponder<T> {
    fn respond_to(self, req: &HttpRequest) -> HttpResponse {
        let headers = match self.headers {
            Ok(headers) => headers,
            Err(err) => return HttpResponse::from_error(Error::from(err)),
        };

        let mut res = self.responder.respond_to(req);

        if let Some(status) = self.status {
            *res.status_mut() = status;
        }

        for (k, v) in headers {
            // TODO: before v4, decide if this should be append instead
            res.headers_mut().insert(k, v);
        }

        res
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use actix_service::Service;
    use bytes::{Bytes, BytesMut};

    use actix_http::body::to_bytes;

    use super::*;
    use crate::{
        error,
        http::{header::CONTENT_TYPE, HeaderValue, StatusCode},
        test::{assert_body_eq, init_service, TestRequest},
        web, App,
    };

    #[actix_rt::test]
    async fn test_option_responder() {
        let srv = init_service(
            App::new()
                .service(web::resource("/none").to(|| async { Option::<&'static str>::None }))
                .service(web::resource("/some").to(|| async { Some("some") })),
        )
        .await;

        let req = TestRequest::with_uri("/none").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let req = TestRequest::with_uri("/some").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_body_eq!(resp, b"some");
    }

    #[actix_rt::test]
    async fn test_responder() {
        let req = TestRequest::default().to_http_request();

        let res = "test".respond_to(&req);
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("text/plain; charset=utf-8")
        );
        assert_eq!(
            to_bytes(res.into_body()).await.unwrap(),
            Bytes::from_static(b"test"),
        );

        let res = b"test".respond_to(&req);
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("application/octet-stream")
        );
        assert_eq!(
            to_bytes(res.into_body()).await.unwrap(),
            Bytes::from_static(b"test"),
        );

        let res = "test".to_string().respond_to(&req);
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("text/plain; charset=utf-8")
        );
        assert_eq!(
            to_bytes(res.into_body()).await.unwrap(),
            Bytes::from_static(b"test"),
        );

        // let res = (&"test".to_string()).respond_to(&req);
        // assert_eq!(res.status(), StatusCode::OK);
        // assert_eq!(
        //     res.headers().get(CONTENT_TYPE).unwrap(),
        //     HeaderValue::from_static("text/plain; charset=utf-8")
        // );
        // assert_eq!(
        //     to_bytes(res.into_body()).await.unwrap(),
        //     Bytes::from_static(b"test"),
        // );

        // let s = String::from("test");
        // let res = Cow::Borrowed(s.as_str()).respond_to(&req);
        // assert_eq!(res.status(), StatusCode::OK);
        // assert_eq!(res.body().bin_ref(), b"test");
        // assert_eq!(
        //     res.headers().get(CONTENT_TYPE).unwrap(),
        //     HeaderValue::from_static("text/plain; charset=utf-8")
        // );

        // let res = Cow::<'_, str>::Owned(s).respond_to(&req);
        // assert_eq!(res.status(), StatusCode::OK);
        // assert_eq!(res.body().bin_ref(), b"test");
        // assert_eq!(
        //     res.headers().get(CONTENT_TYPE).unwrap(),
        //     HeaderValue::from_static("text/plain; charset=utf-8")
        // );

        // let res = Cow::Borrowed("test").respond_to(&req);
        // assert_eq!(res.status(), StatusCode::OK);
        // assert_eq!(res.body().bin_ref(), b"test");
        // assert_eq!(
        //     res.headers().get(CONTENT_TYPE).unwrap(),
        //     HeaderValue::from_static("text/plain; charset=utf-8")
        // );

        let res = Bytes::from_static(b"test").respond_to(&req);
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("application/octet-stream")
        );
        assert_eq!(
            to_bytes(res.into_body()).await.unwrap(),
            Bytes::from_static(b"test"),
        );

        let res = BytesMut::from(b"test".as_ref()).respond_to(&req);
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("application/octet-stream")
        );
        assert_eq!(
            to_bytes(res.into_body()).await.unwrap(),
            Bytes::from_static(b"test"),
        );

        // InternalError
        let res = error::InternalError::new("err", StatusCode::BAD_REQUEST).respond_to(&req);
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    #[actix_rt::test]
    async fn test_result_responder() {
        let req = TestRequest::default().to_http_request();

        // Result<I, E>
        let resp = Ok::<_, Error>("test".to_string()).respond_to(&req);
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("text/plain; charset=utf-8")
        );
        assert_eq!(
            to_bytes(resp.into_body()).await.unwrap(),
            Bytes::from_static(b"test"),
        );

        let res = Err::<String, _>(error::InternalError::new("err", StatusCode::BAD_REQUEST))
            .respond_to(&req);

        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    #[actix_rt::test]
    async fn test_custom_responder() {
        let req = TestRequest::default().to_http_request();
        let res = "test"
            .to_string()
            .with_status(StatusCode::BAD_REQUEST)
            .respond_to(&req);

        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            to_bytes(res.into_body()).await.unwrap(),
            Bytes::from_static(b"test"),
        );

        let res = "test"
            .to_string()
            .with_header(("content-type", "json"))
            .respond_to(&req);

        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("json")
        );
        assert_eq!(
            to_bytes(res.into_body()).await.unwrap(),
            Bytes::from_static(b"test"),
        );
    }

    #[actix_rt::test]
    async fn test_tuple_responder_with_status_code() {
        let req = TestRequest::default().to_http_request();
        let res = ("test".to_string(), StatusCode::BAD_REQUEST).respond_to(&req);
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            to_bytes(res.into_body()).await.unwrap(),
            Bytes::from_static(b"test"),
        );

        let req = TestRequest::default().to_http_request();
        let res = ("test".to_string(), StatusCode::OK)
            .with_header((CONTENT_TYPE, mime::APPLICATION_JSON))
            .respond_to(&req);
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("application/json")
        );
        assert_eq!(
            to_bytes(res.into_body()).await.unwrap(),
            Bytes::from_static(b"test"),
        );
    }
}
