//! ScT's static route declarations over the shared engine.
//!
//! Invalid declarations fail during setup. HTTP execution and filesystem serving
//! remain in the prebuilt engine; application work uses the same engine runtime.
pub use engine::{Json, http, middleware, response, tracing};
use simple_server::engine_web as engine;
fn construct<R>(function: impl FnOnce() -> R) -> R {
    function()
}

pub mod extract {
    pub use simple_server::engine_web::Request;
    pub use simple_server::engine_web::extract::*;
}
pub mod body {
    pub use simple_server::engine_web::{Body, BodyError, Bytes};
    pub async fn to_bytes(body: Body, limit: usize) -> Result<Bytes, BodyError> {
        body.collect(limit).await
    }
}
#[derive(Clone)]
pub struct Router<S = ()>(engine::Router<S>);
impl<S: Clone + Send + Sync + 'static> Default for Router<S> {
    fn default() -> Self {
        Self::new()
    }
}
impl<S: Clone + Send + Sync + 'static> Router<S> {
    pub fn new() -> Self {
        construct(|| Self(engine::Router::new().expect("engine router")))
    }
    pub fn route(self, path: &str, methods: routing::MethodRouter<S>) -> Self {
        construct(|| Self(self.0.route(path, methods.0).expect("valid ScT route")))
    }
    pub fn merge(self, other: Self) -> Self {
        construct(|| Self(self.0.merge(other.0).expect("distinct ScT routes")))
    }
    pub fn fallback_static_dir(self, path: impl AsRef<std::path::Path>) -> Self {
        construct(|| {
            Self(
                self.0
                    .fallback_static_dir(path)
                    .expect("native static directory"),
            )
        })
    }
    pub fn with_state<S2: Clone + Send + Sync + 'static>(self, state: S) -> Router<S2> {
        construct(|| Router(self.0.with_state(state).expect("bound ScT state")))
    }
    pub fn layer<L, B>(self, layer: L) -> Self
    where
        L: engine::middleware::Layer<engine::Route> + Clone + Send + Sync + 'static,
        L::Service: engine::Service<
                engine::Request,
                Response = http::Response<B>,
                Error = std::convert::Infallible,
            > + Clone
            + Send
            + Sync
            + 'static,
        <L::Service as engine::Service<engine::Request>>::Future: Send + 'static,
        B: engine::HttpBody<Data = engine::Bytes> + Send + 'static,
        B::Error: Into<engine::body::BoxError>,
    {
        construct(|| Self(self.0.layer(layer).expect("valid ScT middleware")))
    }
}
pub mod routing {
    use super::{construct, engine};
    use engine::{Handler, Method};
    pub struct MethodRouter<S = ()>(pub(super) engine::MethodRouter<S>);
    macro_rules! methods {
        ($($name:ident => $method:ident),*) => {$(
            pub fn $name<S, H, T>(handler: H) -> MethodRouter<S>
            where S: Clone + Send + Sync + 'static, H: Handler<T, S>, T: 'static {
                construct(|| MethodRouter(engine::MethodRouter::new().expect("engine methods").on_handler(Method::$method, handler).expect("ScT method")))
            }
            impl<S: Clone + Send + Sync + 'static> MethodRouter<S> {
                pub fn $name<H, T>(self, handler: H) -> Self where H: Handler<T, S>, T: 'static {
                    construct(|| Self(self.0.on_handler(Method::$method, handler).expect("distinct ScT method")))
                }
            }
        )*};
    }
    methods!(get => GET, post => POST, put => PUT, patch => PATCH, delete => DELETE);
    pub fn any<S, H, T>(handler: H) -> MethodRouter<S>
    where
        S: Clone + Send + Sync + 'static,
        H: Handler<T, S>,
        T: 'static,
    {
        construct(|| {
            MethodRouter(engine::MethodRouter::any_handler(handler).expect("ScT fallback methods"))
        })
    }
    pub fn get_service<S, T, B>(service: T) -> MethodRouter<S>
    where
        S: Clone + Send + Sync + 'static,
        T: engine::Service<
                engine::Request,
                Response = engine::http::Response<B>,
                Error = std::convert::Infallible,
            > + Clone
            + Send
            + Sync
            + 'static,
        T::Future: Send + 'static,
        B: engine::HttpBody<Data = engine::Bytes> + Send + 'static,
        B::Error: Into<engine::body::BoxError>,
    {
        construct(|| {
            MethodRouter(
                engine::MethodRouter::new()
                    .expect("engine methods")
                    .on_service(Method::GET, service)
                    .expect("ScT liveness service"),
            )
        })
    }
}
pub struct TcpListener(engine::TcpListener);
impl TcpListener {
    pub fn local_addr(&self) -> std::io::Result<std::net::SocketAddr> {
        Ok(self.0.local_addr())
    }
}
pub async fn bind(address: impl ToString) -> std::io::Result<TcpListener> {
    engine::bind(address.to_string()).await.map(TcpListener)
}

pub async fn serve(
    listener: TcpListener,
    router: Router,
    shutdown: simple_server::engine_lifecycle::Shutdown,
) -> std::io::Result<()> {
    engine::serve(listener.0, router.0, shutdown).await
}
