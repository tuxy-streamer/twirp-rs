use std::net::SocketAddr;
use std::time::UNIX_EPOCH;

use twirp::async_trait::async_trait;
use twirp::axum::routing::get;
use twirp::{invalid_argument, Router};

pub mod service {
    pub mod haberdash {
        pub mod v1 {
            include!(concat!(env!("OUT_DIR"), "/service.haberdash.v1.rs"));
        }
    }
}
use service::haberdash::v1::{
    self as haberdash, GetStatusRequest, GetStatusResponse, MakeHatRequest, MakeHatResponse,
};

async fn ping() -> &'static str {
    "Pong\n"
}

/// You can run this end-to-end example by running both a server and a client and observing the requests/responses.
///
/// 1. Run the server:
/// ```sh
/// cargo run --bin simple-server
/// ```
///
/// 2. In another shell, run the client:
/// ```sh
/// cargo run --bin client
/// ```
#[tokio::main]
pub async fn main() {
    let api_impl = HaberdasherApiServer {};
    let app = Router::new()
        .nest("/twirp", haberdash::router(api_impl))
        .route("/_ping", get(ping))
        .fallback(twirp::server::not_found_handler);

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    let tcp_listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind");
    println!("Listening on {addr}");
    if let Err(e) = twirp::axum::serve(tcp_listener, app).await {
        eprintln!("server error: {}", e);
    }
}

// Note: If your server type can't be Clone, consider wrapping it in `std::sync::Arc`.
#[derive(Clone)]
struct HaberdasherApiServer;

#[async_trait]
impl haberdash::HaberdasherApi for HaberdasherApiServer {
    async fn make_hat(
        &self,
        req: twirp::Request<MakeHatRequest>,
    ) -> twirp::Result<twirp::Response<MakeHatResponse>> {
        let data = req.into_body();
        if data.inches == 0 {
            return Err(invalid_argument("inches"));
        }

        println!("got {data:?}");
        let ts = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let mut resp = twirp::Response::new(MakeHatResponse {
            color: "black".to_string(),
            name: "top hat".to_string(),
            size: data.inches,
            timestamp: Some(prost_wkt_types::Timestamp {
                seconds: ts.as_secs() as i64,
                nanos: 0,
            }),
        });
        // Demonstrate adding custom extensions to the response (this could be handled by middleware).
        resp.extensions_mut().insert(ResponseInfo(42));
        Ok(resp)
    }

    async fn get_status(
        &self,
        _req: twirp::Request<GetStatusRequest>,
    ) -> twirp::Result<twirp::Response<GetStatusResponse>> {
        Ok(twirp::Response::new(GetStatusResponse {
            status: "making hats".to_string(),
        }))
    }
}

// Demonstrate sending back custom extensions from the handlers.
#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Debug, Default)]
struct ResponseInfo(u16);

#[cfg(test)]
mod test {
    use twirp::client::Client;
    use twirp::url::Url;
    use twirp::TwirpErrorCode;

    use crate::service::haberdash::v1::HaberdasherApi;

    use super::*;

    #[tokio::test]
    async fn success() {
        let api = HaberdasherApiServer {};
        let res = api
            .make_hat(twirp::Request::new(MakeHatRequest { inches: 1 }))
            .await;
        assert!(res.is_ok());
        let res = res.unwrap().into_body();
        assert_eq!(res.size, 1);
    }

    #[tokio::test]
    async fn invalid_request() {
        let api = HaberdasherApiServer {};
        let res = api
            .make_hat(twirp::Request::new(MakeHatRequest { inches: 0 }))
            .await;
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert_eq!(err.code, TwirpErrorCode::InvalidArgument);
    }

    /// A running network server task, bound to an arbitrary port on localhost, chosen by the OS
    struct NetServer {
        port: u16,
        server_task: tokio::task::JoinHandle<()>,
        shutdown_sender: tokio::sync::oneshot::Sender<()>,
    }

    impl NetServer {
        async fn start(api_impl: HaberdasherApiServer) -> Self {
            let app = Router::new()
                .nest("/twirp", haberdash::router(api_impl))
                .route("/_ping", get(ping))
                .fallback(twirp::server::not_found_handler);

            let tcp_listener = tokio::net::TcpListener::bind("localhost:0")
                .await
                .expect("failed to bind");
            let addr = tcp_listener.local_addr().unwrap();
            println!("Listening on {addr}");
            let port = addr.port();

            let (shutdown_sender, shutdown_receiver) = tokio::sync::oneshot::channel::<()>();
            let server_task = tokio::spawn(async move {
                let shutdown_receiver = async move {
                    shutdown_receiver.await.unwrap();
                };
                if let Err(e) = twirp::axum::serve(tcp_listener, app)
                    .with_graceful_shutdown(shutdown_receiver)
                    .await
                {
                    eprintln!("server error: {}", e);
                }
            });

            NetServer {
                port,
                server_task,
                shutdown_sender,
            }
        }

        async fn shutdown(self) {
            self.shutdown_sender.send(()).unwrap();
            self.server_task.await.unwrap();
        }
    }

    #[tokio::test]
    async fn test_net() {
        let api_impl = HaberdasherApiServer {};
        let server = NetServer::start(api_impl).await;

        let url = Url::parse(&format!("http://localhost:{}/twirp/", server.port)).unwrap();
        let client = Client::from_base_url(url);
        let resp = client
            .make_hat(twirp::Request::new(MakeHatRequest { inches: 1 }))
            .await;
        println!("{:?}", resp);
        let data = resp.unwrap().into_body();
        assert_eq!(data.size, 1);

        server.shutdown().await;
    }
}
