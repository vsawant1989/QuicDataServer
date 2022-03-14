use anyhow::anyhow;
use std::path::Path;
use std::time::{Duration, Instant};
use std::{
    fs,
    io::{Write,self},
    net::{IpAddr, Ipv6Addr, SocketAddr, UdpSocket},
    sync::Arc,
    thread,
};

use bencher::{benchmark_group, benchmark_main, Bencher};
use futures_util::task::SpawnExt;
use futures_util::{AsyncRead, AsyncWriteExt, StreamExt};
use glassbench::*;
use quinn::{Endpoint, IdleTimeout};
use tokio::runtime::{Builder, Runtime};
use tracing::{error, error_span, info};
use tracing_futures::Instrument as _;
use url::Host::Ipv4;

static TRANSACTIONS_4KB: &str = "GET /resources/transactions_4KB.json \r\n";
static TRANSACTIONS_5MB: &str = "GET /resources/transactions_5MB.json \r\n";
static TRANSACTIONS_20MB: &str = "GET /resources/transactions_20MB.json \r\n";
static TRANSACTIONS_38MB: &str = "GET /resources/transactions_38MB.json \r\n";

static TRANSACTIONS_4MB: &str = "GET /resources/transactions_3.9MB.json \r\n";

//glassbench!("Stream", small_data_stream,large_data_stream,large_data_stream_20MB,large_data_stream_38MB,);
glassbench!("Stream", send_10K_transactions,);


fn send_10K_transactions(bench: &mut glassbench::Bench) {
    bench.task("10K transactions with 1 concurrent stream", |task| {
        task.iter(|| {
            send_data( TRANSACTIONS_4MB,1);
        });
    });

}
fn large_data_stream(bench: &mut glassbench::Bench) {
    bench.task("Send Large 5MB Data with 5 concurrent streams", |task| {
        task.iter(|| {
            send_data( TRANSACTIONS_5MB,5);
        });
    });

}
fn large_data_stream_20MB(bench: &mut glassbench::Bench){
    bench.task("Send Large 20MB Data with 1 concurrent streams", |task| {
        task.iter(|| {
            send_data( TRANSACTIONS_20MB,1);
        });
    });
}

fn large_data_stream_38MB(bench: &mut glassbench::Bench){

    bench.task("Send Large 38MB Data with 1 concurrent streams", |task| {
        task.iter(|| {
            send_data( TRANSACTIONS_38MB,1);
        });
    });
}

fn small_data_stream(bench: &mut glassbench::Bench) {
    bench.task("Send Small Data", |task| {
        task.iter(|| {
            send_data( TRANSACTIONS_4KB,10);
        });
    });
}

fn send_data(path : & 'static str, concurrent_streams: usize) {
    let _ = tracing_subscriber::fmt::try_init();
    let ctx = Context::new();
    //let (addr, thread) = ctx.spawn_server();
    let (endpoint, client, runtime) = ctx.make_client( "3.94.109.6:8080".parse::<SocketAddr>().unwrap());
    let client = Arc::new(client);
    let endpoint_new=Arc::new(endpoint.clone());
    let mut handles = Vec::new();
    for _ in 0..concurrent_streams {
        let client = client.clone();
        handles.push(runtime.spawn(async move {
            let (mut stream, recv) = client.open_bi().await.unwrap();
            stream.write_all(path.as_bytes()).await.unwrap();
            stream.finish().await.unwrap();
            let val = recv
                .read_to_end(usize::MAX)
                .await.unwrap();

            println!("Stream Len : {:?}",val.len());

        }));
     }
    runtime.block_on(async {
        for handle in handles {
            handle.await.unwrap();
        }
    });

    drop(client);
    runtime.block_on(endpoint.wait_idle());

}

struct Context {
    server_config: quinn::ServerConfig,
    client_config: quinn::ClientConfig,
}

impl Context {
    #[allow(clippy::field_reassign_with_default)] // https://github.com/rust-lang/rust-clippy/issues/6527
    fn new() -> Self {
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let key = rustls::PrivateKey(cert.serialize_private_key_der());
        let cert = rustls::Certificate(cert.serialize_der().unwrap());
        let mut server_config =
            quinn::ServerConfig::with_single_cert(vec![cert.clone()], key).unwrap();
        Arc::get_mut(&mut server_config.transport)
            .unwrap()
            .max_concurrent_uni_streams(10_u16.into());

        let mut roots = rustls::RootCertStore::empty();
        roots.add(&cert).unwrap();
        Self {
            server_config,
            client_config: quinn::ClientConfig::with_root_certificates(roots),
        }
    }

    pub fn make_client(
        &self,
        server_addr: SocketAddr,
    ) -> (quinn::Endpoint, quinn::Connection, Runtime) {
        let runtime = rt();
        let mut endpoint = {
            let _guard = runtime.enter();
            Endpoint::client("[::]:0".parse().unwrap()).unwrap()
        };

        let quinn::NewConnection { connection, .. } = runtime
            .block_on(async {
                let mut endpoint = {
                    let _guard = runtime.enter();
                    // Endpoint::client(SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 0)).unwrap()
                    Endpoint::client("[::]:0".parse().unwrap()).unwrap()
                };
                let mut roots = rustls::RootCertStore::empty();
                match fs::read(Path::new("resources/certs/cert.der")) {
                    Ok(cert) => {
                        roots.add(&rustls::Certificate(cert));
                    }
                    Err(ref e) if e.kind() == io::ErrorKind::NotFound => {
                        info!("local server certificate not found");
                    }
                    Err(e) => {
                        error!("failed to open local server certificate: {}", e);
                    }
                }
                let mut client_crypto = rustls::ClientConfig::builder()
                    .with_safe_defaults()
                    .with_root_certificates(roots)
                    .with_no_client_auth();
                client_crypto.alpn_protocols = ALPN_QUIC_HTTP.iter().map(|&x| x.into()).collect();
                let mut client_config =quinn::ClientConfig::new(Arc::new(client_crypto));
                Arc::get_mut(&mut client_config.transport)
                    .unwrap()
                    .max_idle_timeout(Some(IdleTimeout::try_from(Duration::from_secs(120)).unwrap()))
                    .max_concurrent_uni_streams(100000_u32.into())
                    .keep_alive_interval(Some(Duration::from_secs(60)));
               // endpoint.set_default_client_config(quinn::ClientConfig::new(Arc::new(client_crypto)));
                endpoint.set_default_client_config(client_config);

                endpoint.connect(server_addr, "localhost")

                    .unwrap()
                    .instrument(error_span!("client"))
                    .await
            })
            .unwrap();
        (endpoint, connection, runtime)
    }
}

fn rt() -> Runtime {
    Builder::new_current_thread().enable_all().build().unwrap()
}

const ALPN_QUIC_HTTP: &[&[u8]] = &[b"hq-30"];