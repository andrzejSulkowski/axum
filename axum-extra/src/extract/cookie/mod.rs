//! Cookie parsing and cookie jar management.
//!
//! See [`CookieJar`], [`SignedCookieJar`], and [`PrivateCookieJar`] for more details.

use axum::{
    extract::FromRequestParts,
    response::{IntoResponse, IntoResponseParts, Response, ResponseParts},
};
use http::{
    header::{COOKIE, SET_COOKIE},
    request::Parts,
    HeaderMap,
};
use std::convert::Infallible;

#[cfg(feature = "cookie-private")]
mod private;
#[cfg(feature = "cookie-signed")]
mod signed;

#[cfg(feature = "cookie-private")]
pub use self::private::PrivateCookieJar;
#[cfg(feature = "cookie-signed")]
pub use self::signed::SignedCookieJar;

pub use cookie::{Cookie, Expiration, SameSite};

#[cfg(any(feature = "cookie-signed", feature = "cookie-private"))]
pub use cookie::Key;

/// Extractor that grabs cookies from the request and manages the jar.
///
/// Note that methods like [`CookieJar::add`], [`CookieJar::remove`], etc updates the [`CookieJar`]
/// and returns it. This value _must_ be returned from the handler as part of the response for the
/// changes to be propagated.
///
/// # Example
///
/// ```rust
/// use axum::{
///     Router,
///     routing::{post, get},
///     response::{IntoResponse, Redirect},
///     http::StatusCode,
/// };
/// use axum_extra::{
///     TypedHeader,
///     headers::authorization::{Authorization, Bearer},
///     extract::cookie::{CookieJar, Cookie},
/// };
///
/// async fn create_session(
///     TypedHeader(auth): TypedHeader<Authorization<Bearer>>,
///     jar: CookieJar,
/// ) -> Result<(CookieJar, Redirect), StatusCode> {
///     if let Some(session_id) = authorize_and_create_session(auth.token()).await {
///         Ok((
///             // the updated jar must be returned for the changes
///             // to be included in the response
///             jar.add(Cookie::new("session_id", session_id)),
///             Redirect::to("/me"),
///         ))
///     } else {
///         Err(StatusCode::UNAUTHORIZED)
///     }
/// }
///
/// async fn me(jar: CookieJar) -> Result<(), StatusCode> {
///     if let Some(session_id) = jar.get("session_id") {
///         // fetch and render user...
///         # Ok(())
///     } else {
///         Err(StatusCode::UNAUTHORIZED)
///     }
/// }
///
/// async fn authorize_and_create_session(token: &str) -> Option<String> {
///     // authorize the user and create a session...
///     # todo!()
/// }
///
/// let app = Router::new()
///     .route("/sessions", post(create_session))
///     .route("/me", get(me));
/// # let app: Router = app;
/// ```
#[derive(Debug, Default, Clone)]
pub struct CookieJar {
    jar: cookie::CookieJar,
}

impl<S> FromRequestParts<S> for CookieJar
where
    S: Send + Sync,
{
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(Self::from_headers(&parts.headers))
    }
}

fn cookies_from_request(headers: &HeaderMap) -> impl Iterator<Item = Cookie<'static>> + '_ {
    headers
        .get_all(COOKIE)
        .into_iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|cookie| Cookie::parse_encoded(cookie.to_owned()).ok())
}

impl CookieJar {
    /// Create a new `CookieJar` from a map of request headers.
    ///
    /// The cookies in `headers` will be added to the jar.
    ///
    /// This is intended to be used in middleware and other places where it might be difficult to
    /// run extractors. Normally you should create `CookieJar`s through [`FromRequestParts`].
    ///
    /// [`FromRequestParts`]: axum::extract::FromRequestParts
    pub fn from_headers(headers: &HeaderMap) -> Self {
        let mut jar = cookie::CookieJar::new();
        for cookie in cookies_from_request(headers) {
            jar.add_original(cookie);
        }
        Self { jar }
    }

    /// Create a new empty `CookieJar`.
    ///
    /// This is intended to be used in middleware and other places where it might be difficult to
    /// run extractors. Normally you should create `CookieJar`s through [`FromRequestParts`].
    ///
    /// If you need a jar that contains the headers from a request use `impl From<&HeaderMap> for
    /// CookieJar`.
    ///
    /// [`FromRequestParts`]: axum::extract::FromRequestParts
    pub fn new() -> Self {
        Self::default()
    }

    /// Get a cookie from the jar.
    ///
    /// # Example
    ///
    /// ```rust
    /// use axum_extra::extract::cookie::CookieJar;
    /// use axum::response::IntoResponse;
    ///
    /// async fn handle(jar: CookieJar) {
    ///     let value: Option<String> = jar
    ///         .get("foo")
    ///         .map(|cookie| cookie.value().to_owned());
    /// }
    /// ```
    pub fn get(&self, name: &str) -> Option<&Cookie<'static>> {
        self.jar.get(name)
    }

    /// Remove a cookie from the jar.
    ///
    /// # Example
    ///
    /// ```rust
    /// use axum_extra::extract::cookie::{CookieJar, Cookie};
    /// use axum::response::IntoResponse;
    ///
    /// async fn handle(jar: CookieJar) -> CookieJar {
    ///     jar.remove(Cookie::from("foo"))
    /// }
    /// ```
    #[must_use]
    pub fn remove<C: Into<Cookie<'static>>>(mut self, cookie: C) -> Self {
        self.jar.remove(cookie);
        self
    }

    /// Add a cookie to the jar.
    ///
    /// The value will automatically be percent-encoded.
    ///
    /// # Example
    ///
    /// ```rust
    /// use axum_extra::extract::cookie::{CookieJar, Cookie};
    /// use axum::response::IntoResponse;
    ///
    /// async fn handle(jar: CookieJar) -> CookieJar {
    ///     jar.add(Cookie::new("foo", "bar"))
    /// }
    /// ```
    #[must_use]
    #[allow(clippy::should_implement_trait)]
    pub fn add<C: Into<Cookie<'static>>>(mut self, cookie: C) -> Self {
        self.jar.add(cookie);
        self
    }

    /// Get an iterator over all cookies in the jar.
    pub fn iter(&self) -> impl Iterator<Item = &'_ Cookie<'static>> {
        self.jar.iter()
    }

    /// Add a cookie with the specified prefix to the jar.
    ///
    /// # Example
    /// ```rust
    /// use axum_extra::extract::cookie::{CookieJar, Cookie};
    /// use cookie::prefix::{Host, Secure};
    ///
    /// async fn handler(jar: CookieJar) -> CookieJar {
    ///     // Add a cookie with the "__Host-" prefix
    ///     let with_host = jar.clone().add_prefixed(Host, Cookie::new("session_id", "value"));
    ///
    ///     // Add a cookie with the "__Secure-" prefix
    ///     let _with_secure = jar.add_prefixed(Secure, Cookie::new("auth", "token"));
    ///
    ///     with_host
    /// }
    /// ```
    #[must_use]
    pub fn add_prefixed<P: cookie::prefix::Prefix>(
        mut self,
        prefix: P,
        cookie: Cookie<'static>,
    ) -> Self {
        let mut prefixed_jar = self.jar.prefixed_mut(prefix);
        prefixed_jar.add(cookie);
        self
    }

    /// Get a signed cookie with the specified prefix from the jar.
    ///
    /// If the cookie exists and its signature is valid, it is returned with its original name
    /// (without the prefix) and plaintext value.
    ///
    /// # Example
    /// ```rust
    /// use axum_extra::extract::cookie::{CookieJar, Cookie};
    /// use cookie::prefix::{Host, Secure};
    ///
    /// async fn handler(jar: CookieJar) {
    ///     if let Some(cookie) = jar.get_prefixed(cookie::prefix::Host, "session_id") {
    ///         let value = cookie.value();
    ///     }
    /// }
    /// ```
    pub fn get_prefixed<P: cookie::prefix::Prefix>(
        &self,
        prefix: P,
        name: &str,
    ) -> Option<Cookie<'static>> {
        let prefixed_jar = self.jar.prefixed(prefix);
        prefixed_jar.get(name)
    }

    /// Remove a cookie with the specified prefix from the jar.
    ///
    /// # Example
    /// ```rust
    /// use axum_extra::extract::cookie::CookieJar;
    /// use cookie::prefix::{Host, Secure};
    ///
    /// async fn handler(jar: CookieJar) -> CookieJar {
    ///     // Remove a cookie with the "__Host-" prefix
    ///     jar.remove_prefixed(Host, "session_id")
    /// }
    /// ```
    #[must_use]
    pub fn remove_prefixed<P, S>(mut self, prefix: P, name: S) -> Self
    where
        P: cookie::prefix::Prefix,
        S: Into<String>,
    {
        let mut prefixed_jar = self.jar.prefixed_mut(prefix);
        prefixed_jar.remove(name.into());
        self
    }
}

impl IntoResponseParts for CookieJar {
    type Error = Infallible;

    fn into_response_parts(self, mut res: ResponseParts) -> Result<ResponseParts, Self::Error> {
        set_cookies(self.jar, res.headers_mut());
        Ok(res)
    }
}

impl IntoResponse for CookieJar {
    fn into_response(self) -> Response {
        (self, ()).into_response()
    }
}

fn set_cookies(jar: cookie::CookieJar, headers: &mut HeaderMap) {
    for cookie in jar.delta() {
        if let Ok(header_value) = cookie.encoded().to_string().parse() {
            headers.append(SET_COOKIE, header_value);
        }
    }

    // we don't need to call `jar.reset_delta()` because `into_response_parts` consumes the cookie
    // jar so it cannot be called multiple times.
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, extract::FromRef, http::Request, routing::get, Router};
    use cookie::prefix::Host;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    macro_rules! cookie_test {
        ($name:ident, $jar:ty) => {
            #[tokio::test]
            async fn $name() {
                async fn set_cookie(jar: $jar) -> impl IntoResponse {
                    jar.add(Cookie::new("key", "value"))
                }

                async fn get_cookie(jar: $jar) -> impl IntoResponse {
                    jar.get("key").unwrap().value().to_owned()
                }

                async fn remove_cookie(jar: $jar) -> impl IntoResponse {
                    jar.remove(Cookie::from("key"))
                }

                let state = AppState {
                    key: Key::generate(),
                    custom_key: CustomKey(Key::generate()),
                };

                let app = Router::new()
                    .route("/set", get(set_cookie))
                    .route("/get", get(get_cookie))
                    .route("/remove", get(remove_cookie))
                    .with_state(state);

                let res = app
                    .clone()
                    .oneshot(Request::builder().uri("/set").body(Body::empty()).unwrap())
                    .await
                    .unwrap();
                let cookie_value = res.headers()["set-cookie"].to_str().unwrap();

                assert!(cookie_value.starts_with("key="));

                // For signed/private cookies, verify that the plaintext value is not directly visible
                // (only for signed and private jars, not for the regular CookieJar)
                if std::any::type_name::<$jar>().contains("Private")
                    || std::any::type_name::<$jar>().contains("Signed")
                {
                    assert!(!cookie_value.contains("key=value"));
                } else {
                    assert!(cookie_value.contains("key=value"));
                }

                let res = app
                    .clone()
                    .oneshot(
                        Request::builder()
                            .uri("/get")
                            .header("cookie", cookie_value)
                            .body(Body::empty())
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                let body = body_text(res).await;
                assert_eq!(body, "value");

                let res = app
                    .clone()
                    .oneshot(
                        Request::builder()
                            .uri("/remove")
                            .header("cookie", cookie_value)
                            .body(Body::empty())
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                assert!(res.headers()["set-cookie"]
                    .to_str()
                    .unwrap()
                    .contains("key=;"));
            }
        };
    }

    macro_rules! cookie_prefixed_test {
        ($name:ident, $jar:ty) => {
            #[tokio::test]
            async fn $name() {
                async fn set_cookie_prefixed(jar: $jar) -> impl IntoResponse {
                    jar.add_prefixed(Host, Cookie::new("key", "value"))
                }

                async fn get_cookie_prefixed(jar: $jar) -> impl IntoResponse {
                    jar.get_prefixed(Host, "key").unwrap().value().to_owned()
                }

                async fn remove_cookie_prefixed(jar: $jar) -> impl IntoResponse {
                    jar.remove_prefixed(Host, "key")
                }

                let state = AppState {
                    key: Key::generate(),
                    custom_key: CustomKey(Key::generate()),
                };

                let app = Router::new()
                    .route("/set", get(set_cookie_prefixed))
                    .route("/get", get(get_cookie_prefixed))
                    .route("/remove", get(remove_cookie_prefixed))
                    .with_state(state);

                let res = app
                    .clone()
                    .oneshot(Request::builder().uri("/set").body(Body::empty()).unwrap())
                    .await
                    .unwrap();
                let cookie_value = res.headers()["set-cookie"].to_str().unwrap();
                assert!(cookie_value.contains("__Host-key"));

                // For signed/private cookies, verify that the plaintext value is not directly visible
                // (only for signed and private jars, not for the regular CookieJar)
                if std::any::type_name::<$jar>().contains("Private")
                    || std::any::type_name::<$jar>().contains("Signed")
                {
                    assert!(!cookie_value.contains("key=value"));
                } else {
                    assert!(cookie_value.contains("key=value"));
                }

                // Extract just the cookie part (before the first semicolon)
                // Set-Cookie: __Host-key=value; Secure; Path=/ -> __Host-key=value
                let cookie_header_value = cookie_value.split(';').next().unwrap().trim();

                let res = app
                    .clone()
                    .oneshot(
                        Request::builder()
                            .uri("/get")
                            .header("cookie", cookie_header_value)
                            .body(Body::empty())
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                let body = body_text(res).await;
                assert_eq!(body, "value");

                let res = app
                    .clone()
                    .oneshot(
                        Request::builder()
                            .uri("/remove")
                            .header("cookie", cookie_value)
                            .body(Body::empty())
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                assert!(res.headers()["set-cookie"]
                    .to_str()
                    .unwrap()
                    .contains("__Host-key=;"));
            }
        };
    }

    cookie_test!(plaintext_cookies, CookieJar);

    #[cfg(feature = "cookie-signed")]
    cookie_test!(signed_cookies, SignedCookieJar);
    #[cfg(feature = "cookie-signed")]
    cookie_prefixed_test!(signed_cookies_prefixed, SignedCookieJar);
    #[cfg(feature = "cookie-signed")]
    cookie_test!(signed_cookies_with_custom_key, SignedCookieJar<CustomKey>);
    #[cfg(feature = "cookie-signed")]
    cookie_prefixed_test!(
        signed_cookies_prefixed_with_custom_key,
        SignedCookieJar<CustomKey>
    );

    #[cfg(feature = "cookie-private")]
    cookie_test!(private_cookies, PrivateCookieJar);
    #[cfg(feature = "cookie-private")]
    cookie_prefixed_test!(private_cookies_prefixed, PrivateCookieJar);
    #[cfg(feature = "cookie-private")]
    cookie_test!(private_cookies_with_custom_key, PrivateCookieJar<CustomKey>);
    #[cfg(feature = "cookie-private")]
    cookie_prefixed_test!(
        private_cookies_prefixed_with_custom_key,
        PrivateCookieJar<CustomKey>
    );

    #[derive(Clone)]
    struct AppState {
        key: Key,
        custom_key: CustomKey,
    }

    impl FromRef<AppState> for Key {
        fn from_ref(state: &AppState) -> Key {
            state.key.clone()
        }
    }

    impl FromRef<AppState> for CustomKey {
        fn from_ref(state: &AppState) -> CustomKey {
            state.custom_key.clone()
        }
    }

    #[derive(Clone)]
    struct CustomKey(Key);

    impl From<CustomKey> for Key {
        fn from(custom: CustomKey) -> Self {
            custom.0
        }
    }

    #[cfg(feature = "cookie-signed")]
    #[tokio::test]
    async fn signed_cannot_access_invalid_cookies() {
        async fn get_cookie(jar: SignedCookieJar) -> impl IntoResponse {
            format!("{:?}", jar.get("key"))
        }

        let state = AppState {
            key: Key::generate(),
            custom_key: CustomKey(Key::generate()),
        };

        let app = Router::new()
            .route("/get", get(get_cookie))
            .with_state(state);

        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/get")
                    .header("cookie", "key=value")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = body_text(res).await;
        assert_eq!(body, "None");
    }

    async fn body_text<B>(body: B) -> String
    where
        B: axum::body::HttpBody,
        B::Error: std::fmt::Debug,
    {
        let bytes = body.collect().await.unwrap().to_bytes();
        String::from_utf8(bytes.to_vec()).unwrap()
    }
}
