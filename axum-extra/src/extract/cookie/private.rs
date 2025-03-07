use super::{cookies_from_request, set_cookies, Cookie, Key};
use axum::{
    extract::{FromRef, FromRequestParts},
    response::{IntoResponse, IntoResponseParts, Response, ResponseParts},
};
use cookie::PrivateJar;
use http::{request::Parts, HeaderMap};
use std::{convert::Infallible, fmt, marker::PhantomData};

/// Extractor that grabs private cookies from the request and manages the jar.
///
/// All cookies will be private and encrypted with a [`Key`]. This makes it suitable for storing
/// private data.
///
/// Note that methods like [`PrivateCookieJar::add`], [`PrivateCookieJar::remove`], etc updates the
/// [`PrivateCookieJar`] and returns it. This value _must_ be returned from the handler as part of
/// the response for the changes to be propagated.
///
/// # Example
///
/// ```rust
/// use axum::{
///     Router,
///     routing::{post, get},
///     extract::FromRef,
///     response::{IntoResponse, Redirect},
///     http::StatusCode,
/// };
/// use axum_extra::{
///     TypedHeader,
///     headers::authorization::{Authorization, Bearer},
///     extract::cookie::{PrivateCookieJar, Cookie, Key},
/// };
///
/// async fn set_secret(
///     jar: PrivateCookieJar,
/// ) -> (PrivateCookieJar, Redirect) {
///     let updated_jar = jar.add(Cookie::new("secret", "secret-data"));
///     (updated_jar, Redirect::to("/get"))
/// }
///
/// async fn get_secret(jar: PrivateCookieJar) {
///     if let Some(data) = jar.get("secret") {
///         // ...
///     }
/// }
///
/// // our application state
/// #[derive(Clone)]
/// struct AppState {
///     // that holds the key used to encrypt cookies
///     key: Key,
/// }
///
/// // this impl tells `PrivateCookieJar` how to access the key from our state
/// impl FromRef<AppState> for Key {
///     fn from_ref(state: &AppState) -> Self {
///         state.key.clone()
///     }
/// }
///
/// let state = AppState {
///     // Generate a secure key
///     //
///     // You probably don't wanna generate a new one each time the app starts though
///     key: Key::generate(),
/// };
///
/// let app = Router::new()
///     .route("/set", post(set_secret))
///     .route("/get", get(get_secret))
///     .with_state(state);
/// # let _: axum::Router = app;
/// ```
///
/// If you have been using `Arc<AppState>` you cannot implement `FromRef<Arc<AppState>> for Key`.
/// You can use a new type instead:
///
/// ```rust
/// # use axum::extract::FromRef;
/// # use axum_extra::extract::cookie::{PrivateCookieJar, Cookie, Key};
/// use std::sync::Arc;
/// use std::ops::Deref;
///
/// #[derive(Clone)]
/// struct AppState(Arc<InnerState>);
///
/// // deref so you can still access the inner fields easily
/// impl Deref for AppState {
///     type Target = InnerState;
///
///     fn deref(&self) -> &Self::Target {
///         &self.0
///     }
/// }
///
/// struct InnerState {
///     key: Key
/// }
///
/// impl FromRef<AppState> for Key {
///     fn from_ref(state: &AppState) -> Self {
///         state.0.key.clone()
///     }
/// }
/// ```
pub struct PrivateCookieJar<K = Key> {
    jar: cookie::CookieJar,
    key: Key,
    // The key used to extract the key. Allows users to use multiple keys for different
    // jars. Maybe a library wants its own key.
    _marker: PhantomData<K>,
}

impl<K> fmt::Debug for PrivateCookieJar<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PrivateCookieJar")
            .field("jar", &self.jar)
            .field("key", &"REDACTED")
            .finish()
    }
}

impl<S, K> FromRequestParts<S> for PrivateCookieJar<K>
where
    S: Send + Sync,
    K: FromRef<S> + Into<Key>,
{
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let k = K::from_ref(state);
        let key = k.into();
        let PrivateCookieJar {
            jar,
            key,
            _marker: _,
        } = PrivateCookieJar::from_headers(&parts.headers, key);
        Ok(PrivateCookieJar {
            jar,
            key,
            _marker: PhantomData,
        })
    }
}

impl PrivateCookieJar {
    /// Create a new `PrivateCookieJar` from a map of request headers.
    ///
    /// The valid cookies in `headers` will be added to the jar.
    ///
    /// This is intended to be used in middleware and other where places it might be difficult to
    /// run extractors. Normally you should create `PrivateCookieJar`s through [`FromRequestParts`].
    ///
    /// [`FromRequestParts`]: axum::extract::FromRequestParts
    pub fn from_headers(headers: &HeaderMap, key: Key) -> Self {
        let mut jar = cookie::CookieJar::new();
        let mut private_jar = jar.private_mut(&key);
        for cookie in cookies_from_request(headers) {
            if let Some(cookie) = private_jar.decrypt(cookie) {
                private_jar.add_original(cookie);
            }
        }

        Self {
            jar,
            key,
            _marker: PhantomData,
        }
    }

    /// Create a new empty `PrivateCookieJarIter`.
    ///
    /// This is intended to be used in middleware and other places where it might be difficult to
    /// run extractors. Normally you should create `PrivateCookieJar`s through [`FromRequestParts`].
    ///
    /// [`FromRequestParts`]: axum::extract::FromRequestParts
    pub fn new(key: Key) -> Self {
        Self {
            jar: Default::default(),
            key,
            _marker: PhantomData,
        }
    }
}

impl<K> PrivateCookieJar<K> {
    /// Get a cookie from the jar.
    ///
    /// If the cookie exists and can be decrypted then it is returned in plaintext.
    ///
    /// # Example
    ///
    /// ```rust
    /// use axum_extra::extract::cookie::PrivateCookieJar;
    /// use axum::response::IntoResponse;
    ///
    /// async fn handle(jar: PrivateCookieJar) {
    ///     let value: Option<String> = jar
    ///         .get("foo")
    ///         .map(|cookie| cookie.value().to_owned());
    /// }
    /// ```
    pub fn get(&self, name: &str) -> Option<Cookie<'static>> {
        self.private_jar().get(name)
    }

    /// Remove a cookie from the jar.
    ///
    /// # Example
    ///
    /// ```rust
    /// use axum_extra::extract::cookie::{PrivateCookieJar, Cookie};
    /// use axum::response::IntoResponse;
    ///
    /// async fn handle(jar: PrivateCookieJar) -> PrivateCookieJar {
    ///     jar.remove(Cookie::from("foo"))
    /// }
    /// ```
    #[must_use]
    pub fn remove<C: Into<Cookie<'static>>>(mut self, cookie: C) -> Self {
        self.private_jar_mut().remove(cookie);
        self
    }

    /// Add a cookie to the jar.
    ///
    /// The value will automatically be percent-encoded.
    ///
    /// # Example
    ///
    /// ```rust
    /// use axum_extra::extract::cookie::{PrivateCookieJar, Cookie};
    /// use axum::response::IntoResponse;
    ///
    /// async fn handle(jar: PrivateCookieJar) -> PrivateCookieJar {
    ///     jar.add(Cookie::new("foo", "bar"))
    /// }
    /// ```
    #[must_use]
    #[allow(clippy::should_implement_trait)]
    pub fn add<C: Into<Cookie<'static>>>(mut self, cookie: C) -> Self {
        self.private_jar_mut().add(cookie);
        self
    }

    /// Authenticates and decrypts `cookie`, returning the plaintext version if decryption succeeds
    /// or `None` otherwise.
    pub fn decrypt(&self, cookie: Cookie<'static>) -> Option<Cookie<'static>> {
        self.private_jar().decrypt(cookie.clone())
    }

    /// Get an iterator over all cookies in the jar.
    ///
    /// Only cookies with valid authenticity and integrity are yielded by the iterator.
    pub fn iter(&self) -> impl Iterator<Item = Cookie<'static>> + '_ {
        PrivateCookieJarIter {
            jar: self,
            iter: self.jar.iter(),
        }
    }

    fn private_jar(&self) -> PrivateJar<&'_ cookie::CookieJar> {
        self.jar.private(&self.key)
    }

    fn private_jar_mut(&mut self) -> PrivateJar<&'_ mut cookie::CookieJar> {
        self.jar.private_mut(&self.key)
    }
    /// Add a signed cookie with the specified prefix to the jar.
    ///
    /// The cookie's value will be signed using the jar's key, and the prefix will determine the
    /// cookie's name and attributes (e.g., `Secure`, `Path=/` for `__Host-`).
    ///
    /// # Example
    /// ```rust
    /// use axum_extra::extract::cookie::{PrivateCookieJar, Cookie};
    /// use cookie::prefix::Host;
    ///
    /// async fn handler(jar: PrivateCookieJar) -> PrivateCookieJar {
    ///     jar.add_prefixed(Host, Cookie::new("session_id", "value"))
    /// }
    /// ```
    #[must_use]
    pub fn add_prefixed<P: cookie::prefix::Prefix>(
        self,
        _prefix: P,
        cookie: Cookie<'static>,
    ) -> Self {
        let mut jar = self.jar;
        jar.remove(Cookie::new(cookie.name().to_owned(), ""));

        let prefixed_name = format!("{}{}", P::PREFIX, cookie.name());
        let mut new_cookie = cookie;
        new_cookie.set_name(prefixed_name);
        jar.private_mut(&self.key).add(new_cookie);

        Self {
            jar,
            key: self.key,
            _marker: self._marker,
        }
    }
    /// Get a signed cookie with the specified prefix from the jar.
    ///
    /// If the cookie exists and its signature is valid, it is returned with its original name
    /// (without the prefix) and plaintext value.
    ///
    /// # Example
    /// ```rust
    /// use axum_extra::extract::cookie::PrivateCookieJar;
    ///
    /// async fn handler(jar: PrivateCookieJar) {
    ///     if let Some(cookie) = jar.get_prefixed(cookie::prefix::Host, "session_id") {
    ///         let value = cookie.value();
    ///     }
    /// }
    /// ```
    pub fn get_prefixed<P: cookie::prefix::Prefix>(
        &self,
        _prefix: P,
        name: &str,
    ) -> Option<Cookie<'static>> {
        let prefixed_name = format!("{}{name}", P::PREFIX);
        self.jar
            .get(&prefixed_name)
            .and_then(|c| self.decrypt(c.clone()))
    }
    /// Remove a signed cookie with the specified prefix from the jar.
    ///
    /// # Example
    /// ```rust
    /// use axum_extra::extract::cookie::PrivateCookieJar;
    /// use cookie::prefix::Host;
    ///
    /// async fn handler(jar: PrivateCookieJar) -> PrivateCookieJar {
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

impl<K> IntoResponseParts for PrivateCookieJar<K> {
    type Error = Infallible;

    fn into_response_parts(self, mut res: ResponseParts) -> Result<ResponseParts, Self::Error> {
        set_cookies(self.jar, res.headers_mut());
        Ok(res)
    }
}

impl<K> IntoResponse for PrivateCookieJar<K> {
    fn into_response(self) -> Response {
        (self, ()).into_response()
    }
}

struct PrivateCookieJarIter<'a, K> {
    jar: &'a PrivateCookieJar<K>,
    iter: cookie::Iter<'a>,
}

impl<K> Iterator for PrivateCookieJarIter<'_, K> {
    type Item = Cookie<'static>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let cookie = self.iter.next()?;

            if let Some(cookie) = self.jar.get(cookie.name()) {
                return Some(cookie);
            }
        }
    }
}

impl<K> Clone for PrivateCookieJar<K> {
    fn clone(&self) -> Self {
        Self {
            jar: self.jar.clone(),
            key: self.key.clone(),
            _marker: self._marker,
        }
    }
}
