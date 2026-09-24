// Copyright 2020 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use crate::{HyperConnectorFuture, SocketOptions, TcpOptions, TcpStream, parse_ip_addr};
use futures::io;
use http::uri::{Scheme, Uri};
use hyper_util::rt::TokioIo;
use log::{debug, warn};
use netext::TokioAsyncReadExt;
use rustls::RootCertStore;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::task::{Context, Poll};
use tokio::net;
use tower_service::Service;

fn load_certs_from_env(
    cert_file: Option<&Path>,
    cert_dir: Option<&Path>,
) -> Option<Vec<rustls::pki_types::CertificateDer<'static>>> {
    let file_path = cert_file.and_then(|path| {
        if path.exists() {
            Some(path)
        } else {
            warn!("SSL_CERT_FILE is set to {path:?}, but the path does not exist");
            None
        }
    });

    let dir_path = cert_dir.and_then(|path| {
        if path.exists() {
            Some(path)
        } else {
            warn!("SSL_CERT_DIR is set to {path:?}, but the path does not exist");
            None
        }
    });

    if file_path.is_none() && dir_path.is_none() {
        return None;
    }

    debug!(
        "Loading TLS CA certificates from SSL_CERT_FILE={file_path:?}, SSL_CERT_DIR={dir_path:?}"
    );
    let res = rustls_native_certs::load_certs_from_paths(file_path, dir_path);
    for err in &res.errors {
        warn!(
            "Error loading TLS CA certificates from env (file={file_path:?}, dir={dir_path:?}): {err}"
        );
    }
    if res.certs.is_empty() {
        warn!(
            "No valid TLS CA certificates loaded from configured SSL paths (file={file_path:?}, dir={dir_path:?})"
        );
    } else {
        debug!(
            "Loaded {} TLS CA certificates from env (file={file_path:?}, dir={dir_path:?})",
            res.certs.len()
        );
    }

    Some(res.certs)
}

fn load_native_certs() -> Vec<rustls::pki_types::CertificateDer<'static>> {
    let env_file = std::env::var_os("SSL_CERT_FILE").map(PathBuf::from);
    let env_dir = std::env::var_os("SSL_CERT_DIR").map(PathBuf::from);

    if let Some(certs) = load_certs_from_env(env_file.as_deref(), env_dir.as_deref()) {
        return certs;
    }

    debug!("Loading TLS CA certificates from platform root store");
    rustls_native_certs::load_native_certs()
        .expect("Could not load TLS CA certificates from platform root store")
}

pub fn new_root_cert_store() -> Arc<RootCertStore> {
    // It can be expensive to parse the certs, so cache them
    static ROOT_STORE: LazyLock<Arc<RootCertStore>> = LazyLock::new(|| {
        let mut root_store = rustls::RootCertStore::empty();

        let certs = load_native_certs();

        if !certs.is_empty() {
            let (added, ignored) = root_store.add_parsable_certificates(certs);

            if ignored != 0 {
                warn!("Failed to load {ignored} certificates into the root store");
            }

            if added == 0 {
                panic!("Unable to load any TLS CA certificates from platform root store")
            }
        }

        Arc::new(root_store)
    });

    Arc::clone(&ROOT_STORE)
}

/// A Async-std-compatible implementation of hyper's `Connect` trait which allows
/// creating a TcpStream to a particular destination.
#[derive(Clone, Debug)]
pub struct HyperConnector {
    tcp_options: TcpOptions,
    socket_options: SocketOptions,
}

impl From<(TcpOptions, SocketOptions)> for HyperConnector {
    fn from((tcp_options, socket_options): (TcpOptions, SocketOptions)) -> Self {
        Self { tcp_options, socket_options }
    }
}

impl HyperConnector {
    pub fn new() -> Self {
        Self::from_tcp_options(TcpOptions::default())
    }

    pub fn from_tcp_options(tcp_options: TcpOptions) -> Self {
        Self { tcp_options, socket_options: SocketOptions::default() }
    }
}

impl Service<Uri> for HyperConnector {
    type Response = TokioIo<TcpStream>;
    type Error = std::io::Error;
    type Future = HyperConnectorFuture;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, dst: Uri) -> Self::Future {
        let self_ = self.clone();
        HyperConnectorFuture { fut: Box::pin(async move { self_.call_async(dst).await }) }
    }
}

impl HyperConnector {
    async fn call_async(&self, dst: Uri) -> Result<TokioIo<TcpStream>, io::Error> {
        let port = match dst.port() {
            Some(p) => p.as_u16(),
            None => {
                if dst.scheme() == Some(&Scheme::HTTPS) {
                    443
                } else {
                    80
                }
            }
        };

        let host = match dst.host() {
            Some(host) => host,
            _ => return Err(io::Error::other("missing host in Uri")),
        };

        let addr = parse_ip_addr(host, port, |_| async {
            Err(io::Error::other("does not yet support non-integer zone ids"))
        })
        .await?;

        if self.socket_options.bind_device.is_some() {
            unimplemented!(
                "TODO(https://fxbug.dev/42083862) fuchsia-hyper does not support bind_device on non-fuchsia devices"
            );
        }

        let stream = if let Some(addr) = addr {
            net::TcpStream::connect(addr).await?
        } else {
            resolve_host_port(host, port).await?
        };
        let () = self.tcp_options.apply(&stream)?;

        Ok(TokioIo::new(TcpStream { stream: stream.into_multithreaded_futures_stream() }))
    }
}

const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Resolve a hostname into an address.
async fn resolve_host_port(host: &str, port: u16) -> Result<net::TcpStream, io::Error> {
    // TODO(https://fxbug.dev/42075095): Implement happy eyeballs algorithm to make this
    // more efficient.
    let mut last_err = None;
    let addrs = net::lookup_host((host, port)).await?;
    for addr in addrs {
        match tokio::time::timeout(CONNECT_TIMEOUT, net::TcpStream::connect(addr)).await {
            Ok(Ok(stream)) => {
                return Ok(stream);
            }
            Ok(Err(err)) => {
                debug!("Connection attempt to {addr} for {host}:{port} failed: {err}");
                last_err = Some(err);
            }
            Err(_) => {
                debug!(
                    "Connection attempt to {addr} for {host}:{port} timed out after {CONNECT_TIMEOUT:?}"
                );
                last_err =
                    Some(io::Error::new(io::ErrorKind::TimedOut, "connection attempt timed out"));
            }
        }
    }

    if let Some(err) = last_err {
        Err(err)
    } else {
        Err(io::Error::other("destination resolved to no address"))
    }
}

////////////////////////////////////////////////////////////////////////////////
///// tests

#[cfg(test)]
mod test {
    use super::{load_certs_from_env, resolve_host_port};
    use crate::*;
    use anyhow::{Error, Result};
    use futures::future::BoxFuture;
    use futures::stream::FuturesUnordered;
    use futures::{StreamExt, TryStreamExt};
    use http_body_util::BodyExt as _;
    use hyper::{Response, StatusCode};
    use std::convert::Infallible;
    use std::io::Write;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::net::TcpListener;

    const TEST_CERT_1: &str = "-----BEGIN CERTIFICATE-----\n\
MIICGzCCAaGgAwIBAgIQQdKd0XLq7qeAwSxs6S+HUjAKBggqhkjOPQQDAzBPMQsw\n\
CQYDVQQGEwJVUzEpMCcGA1UEChMgSW50ZXJuZXQgU2VjdXJpdHkgUmVzZWFyY2gg\n\
R3JvdXAxFTATBgNVBAMTDElTUkcgUm9vdCBYMjAeFw0yMDA5MDQwMDAwMDBaFw00\n\
MDA5MTcxNjAwMDBaME8xCzAJBgNVBAYTAlVTMSkwJwYDVQQKEyBJbnRlcm5ldCBT\n\
ZWN1cml0eSBSZXNlYXJjaCBHcm91cDEVMBMGA1UEAxMMSVNSRyBSb290IFgyMHYw\n\
EAYHKoZIzj0CAQYFK4EEACIDYgAEzZvVn4CDCuwJSvMWSj5cz3es3mcFDR0HttwW\n\
+1qLFNvicWDEukWVEYmO6gbf9yoWHKS5xcUy4APgHoIYOIvXRdgKam7mAHf7AlF9\n\
ItgKbppbd9/w+kHsOdx1ymgHDB/qo0IwQDAOBgNVHQ8BAf8EBAMCAQYwDwYDVR0T\n\
AQH/BAUwAwEB/zAdBgNVHQ4EFgQUfEKWrt5LSDv6kviejM9ti6lyN5UwCgYIKoZI\n\
zj0EAwMDaAAwZQIwe3lORlCEwkSHRhtFcP9Ymd70/aTSVaYgLXTWNLxBo1BfASdW\n\
tL4ndQavEi51mI38AjEAi/V3bNTIZargCyzuFJ0nN6T5U6VR5CmD1/iQMVtCnwr1\n\
/q4AaOeMSQ+2b1tbFfLn\n\
-----END CERTIFICATE-----\n";

    const TEST_CERT_2: &str = "-----BEGIN CERTIFICATE-----\n\
MIIBtjCCAVugAwIBAgITBmyf1XSXNmY/Owua2eiedgPySjAKBggqhkjOPQQDAjA5\n\
MQswCQYDVQQGEwJVUzEPMA0GA1UEChMGQW1hem9uMRkwFwYDVQQDExBBbWF6b24g\n\
Um9vdCBDQSAzMB4XDTE1MDUyNjAwMDAwMFoXDTQwMDUyNjAwMDAwMFowOTELMAkG\n\
A1UEBhMCVVMxDzANBgNVBAoTBkFtYXpvbjEZMBcGA1UEAxMQQW1hem9uIFJvb3Qg\n\
Q0EgMzBZMBMGByqGSM49AgEGCCqGSM49AwEHA0IABCmXp8ZBf8ANm+gBG1bG8lKl\n\
ui2yEujSLtf6ycXYqm0fc4E7O5hrOXwzpcVOho6AF2hiRVd9RFgdszflZwjrZt6j\n\
QjBAMA8GA1UdEwEB/wQFMAMBAf8wDgYDVR0PAQH/BAQDAgGGMB0GA1UdDgQWBBSr\n\
ttvXBp43rDCGB5Fwx5zEGbF4wDAKBggqhkjOPQQDAgNJADBGAiEA4IWSoxe3jfkr\n\
BqWTrBqYaGFy+uGh0PsceGCmQ5nFuMQCIQCcAu/xlJyzlvnrxir4tiz+OpAUFteM\n\
YyRIHN8wfdVoOw==\n\
-----END CERTIFICATE-----\n";

    struct TestTempDir {
        path: PathBuf,
    }

    impl TestTempDir {
        fn new() -> Self {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let id = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "fuchsia_hyper_test_{}_{}",
                std::process::id(),
                id
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self { path }
        }
    }

    impl Drop for TestTempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    trait AsyncReadWrite: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send {}
    impl<T> AsyncReadWrite for T where T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send {}

    async fn fetch_url<W: Write>(url: hyper::Uri, mut buffer: W) -> Result<StatusCode> {
        let client = new_https_client();

        let mut res = client.get(url).await?;
        let status = res.status();

        if status == StatusCode::OK {
            while let Some(frame) = res.frame().await {
                if let Ok(data) = frame?.into_data() {
                    buffer.write_all(&data)?;
                }
            }
            buffer.flush()?;
        }

        Ok(status)
    }

    #[fuchsia_async::run_singlethreaded(test)]
    async fn test_download_succeeds() -> Result<()> {
        let (listener, addr) = {
            let addr = SocketAddr::new(Ipv6Addr::LOCALHOST.into(), 0);
            let listener = TcpListener::bind(&addr).await.unwrap();
            let local_addr = listener.local_addr().unwrap();
            (listener, local_addr)
        };

        #[cfg(target_os = "fuchsia")]
        let listener =
            listener.incoming().map_err(Error::from).map_ok(|conn| TcpStream { stream: conn });
        #[cfg(not(target_os = "fuchsia"))]
        let listener = netext::TcpListenerStream(listener).map_err(Error::from).map_ok(|conn| {
            TcpStream { stream: netext::TokioAsyncReadExt::into_multithreaded_futures_stream(conn) }
        });

        let mut connections = listener
            .map_ok(|conn| Pin::new(Box::new(conn)) as Pin<Box<dyn AsyncReadWrite>>)
            .boxed();

        let (stop, mut rx_stop) = futures::channel::oneshot::channel();

        let server = async move {
            while let Some(Ok(conn)) = connections.next().await {
                let io = hyper_util::rt::TokioIo::new(conn);
                let service = hyper::service::service_fn(
                    move |_req: hyper::Request<hyper::body::Incoming>| async move {
                        Ok::<_, Infallible>(Response::new(http_body_util::Full::new(
                            hyper::body::Bytes::from("Hello"),
                        )))
                    },
                );
                let builder = hyper_util::server::conn::auto::Builder::new(Executor);
                let mut conn_fut = Box::pin(builder.serve_connection(io, service));
                match futures::future::select(&mut conn_fut, &mut rx_stop).await {
                    futures::future::Either::Left(_) => {}
                    futures::future::Either::Right(_) => {
                        conn_fut.as_mut().graceful_shutdown();
                        let _ = conn_fut.await;
                        break;
                    }
                }
            }
            Ok(())
        };

        let client = async {
            let output: Vec<u8> = Vec::new();
            let status = fetch_url(format!("http://{addr}").parse::<hyper::Uri>().unwrap(), output)
                .await
                .unwrap();
            match status {
                StatusCode::OK | StatusCode::FOUND => {}
                _ => assert!(false, "Unexpected status code: {}", status),
            }
            stop.send(()).expect("server to still be running");
            Ok(())
        };

        let mut tasks: FuturesUnordered<BoxFuture<'_, Result<(), Error>>> = FuturesUnordered::new();
        tasks.push(Box::pin(server));
        tasks.push(Box::pin(client));
        while let Some(Ok(())) = tasks.next().await {}
        Ok(())
    }

    #[fuchsia_async::run_singlethreaded(test)]
    async fn test_download_handles_bad_domain() -> Result<()> {
        let output: Vec<u8> = Vec::new();
        let res = fetch_url("https://domain.invalid".parse::<hyper::Uri>()?, output).await;
        assert!(res.is_err());
        Ok(())
    }

    #[test]
    fn test_load_certs_from_env() {
        let temp_dir = TestTempDir::new();

        // Neither file nor dir provided -> returns None.
        assert!(load_certs_from_env(None, None).is_none());

        // Non-existent file and dir -> returns None.
        let missing_file = temp_dir.path.join("nonexistent.pem");
        let missing_dir = temp_dir.path.join("nonexistent_dir");
        assert!(load_certs_from_env(Some(&missing_file), Some(&missing_dir)).is_none());

        // Valid SSL_CERT_FILE -> loads certificate.
        let cert_file = temp_dir.path.join("bundle.pem");
        std::fs::write(&cert_file, TEST_CERT_1).unwrap();
        let certs = load_certs_from_env(Some(&cert_file), None).expect("should load from file");
        assert_eq!(certs.len(), 1);
        let mut store = rustls::RootCertStore::empty();
        let (added, ignored) = store.add_parsable_certificates(certs);
        assert_eq!(added, 1);
        assert_eq!(ignored, 0);

        // Valid SSL_CERT_DIR -> loads certificate from directory.
        let cert_dir = temp_dir.path.join("certs");
        std::fs::create_dir_all(&cert_dir).unwrap();
        std::fs::write(cert_dir.join("cert2.pem"), TEST_CERT_2).unwrap();
        let dir_certs = load_certs_from_env(None, Some(&cert_dir)).expect("should load from dir");
        assert_eq!(dir_certs.len(), 1);

        // Both SSL_CERT_FILE and SSL_CERT_DIR -> loads from both.
        let both_certs = load_certs_from_env(Some(&cert_file), Some(&cert_dir))
            .expect("should load from file and dir");
        assert_eq!(both_certs.len(), 2);

        // Existing SSL_CERT_FILE with no valid certificates -> returns Some(empty) to override
        // rather than falling back to the system root store.
        let empty_file = temp_dir.path.join("empty.pem");
        std::fs::write(&empty_file, "not a valid pem cert").unwrap();
        let empty_certs = load_certs_from_env(Some(&empty_file), None)
            .expect("explicit existing file should override even when empty");
        assert!(empty_certs.is_empty());
    }

    #[fuchsia_async::run_singlethreaded(test)]
    async fn test_resolve_host_port_connects_to_localhost() -> Result<()> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let port = listener.local_addr()?.port();

        let accept_fut = async move {
            let (_stream, _peer) = listener.accept().await?;
            Ok::<(), io::Error>(())
        };

        let connect_fut = resolve_host_port("localhost", port);
        let (accept_res, connect_res) = futures::future::join(accept_fut, connect_fut).await;
        accept_res?;
        let _stream = connect_res?;
        Ok(())
    }
}
