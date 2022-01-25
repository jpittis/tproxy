use std::collections::HashMap;
use std::error::Error;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use clap::Parser;
use futures::FutureExt;
use tokio::io::copy;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use warp::Filter;

/// A simple TCP proxy
#[derive(Parser, Clone, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    /// Address to listen on
    #[clap(short, long)]
    listen_addr: String,

    /// Address to forward to
    #[clap(short, long)]
    upstream_addr: String,

    /// Address to forward to
    #[clap(short, long, default_value = "127.0.0.1:2222")]
    debug_addr: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let html = include_str!("static/index.html");
    let args = Args::parse();
    let state = Arc::new(Mutex::new(State::new()));
    tokio::spawn(listen(args.clone(), state).map(|r| {
        if let Err(err) = r {
            println!("failed to listen; error={}", err);
        }
    }));
    let route = warp::any().map(|| warp::reply::html(html.to_string()));
    warp::serve(route)
        .run(args.debug_addr.parse::<SocketAddr>().unwrap())
        .await;
    Ok(())
}

#[derive(PartialEq, Debug)]
struct State {
    active_connections: usize,
    completed_connections: usize,
    by_addr: HashMap<SocketAddr, ()>,
}

impl State {
    fn new() -> Self {
        Self {
            active_connections: 0,
            completed_connections: 0,
            by_addr: HashMap::new(),
        }
    }
}

async fn listen(args: Args, state: Arc<Mutex<State>>) -> Result<(), Box<dyn Error>> {
    let listener = TcpListener::bind(&args.listen_addr).await?;

    while let Ok((downstream, downstream_addr)) = listener.accept().await {
        tokio::spawn(
            forward(
                downstream,
                args.upstream_addr.clone(),
                state.clone(),
                downstream_addr,
            )
            .map(|r| {
                if let Err(err) = r {
                    println!("failed to forward; error={}", err);
                }
            }),
        );
    }

    Ok(())
}

async fn forward(
    mut downstream: TcpStream,
    upstream_addr: String,
    state: Arc<Mutex<State>>,
    downstream_addr: SocketAddr,
) -> Result<(), Box<dyn Error>> {
    let mut upstream = TcpStream::connect(&upstream_addr).await?;
    state.lock().unwrap().active_connections += 1;
    state.lock().unwrap().by_addr.insert(downstream_addr, ());
    let (mut ri, mut wi) = downstream.split();
    let (mut ro, mut wo) = upstream.split();

    let client_to_server = async {
        copy(&mut ri, &mut wo).await?;
        wo.shutdown().await
    };

    let server_to_client = async {
        copy(&mut ro, &mut wi).await?;
        wi.shutdown().await
    };

    tokio::try_join!(client_to_server, server_to_client)?;

    state.lock().unwrap().active_connections -= 1;
    state.lock().unwrap().completed_connections += 1;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::time::Duration;

    use tokio::io::AsyncReadExt;

    #[tokio::test]
    async fn test_forward() {
        let args = Args {
            listen_addr: "127.0.0.1:3333".to_string(),
            upstream_addr: "127.0.0.1:4444".to_string(),
            debug_addr: "127.0.0.1:2222".to_string(),
        };

        let state = Arc::new(Mutex::new(State::new()));

        let t1 = tokio::spawn(echo(args.upstream_addr.clone()).map(|r| {
            if let Err(err) = r {
                println!("failed to echo; error={}", err);
            }
        }));

        let t2 = tokio::spawn(listen(args.clone(), state.clone()).map(|r| {
            if let Err(err) = r {
                println!("failed to main; error={}", err);
            }
        }));

        tokio::time::sleep(Duration::from_secs(1)).await;

        let mut client1 = TcpStream::connect(&args.listen_addr).await.unwrap();
        client1.write_all(b"Hello!").await.unwrap();
        let mut buf1 = [0; 6];
        client1.read_exact(&mut buf1).await.unwrap();
        assert_eq!(&buf1, b"Hello!");

        assert_eq!(
            *state.lock().unwrap(),
            State {
                active_connections: 1,
                completed_connections: 0,
                by_addr: HashMap::from_iter([(client1.local_addr().unwrap(), ())]),
            }
        );

        client1.shutdown().await.unwrap();
        tokio::time::sleep(Duration::from_secs(1)).await;

        let mut client2 = TcpStream::connect(&args.listen_addr).await.unwrap();
        client2.write_all(b"Hi!").await.unwrap();
        let mut buf2 = [0; 3];
        client2.read_exact(&mut buf2).await.unwrap();
        assert_eq!(&buf2, b"Hi!");

        client2.shutdown().await.unwrap();
        tokio::time::sleep(Duration::from_secs(1)).await;

        assert_eq!(state.lock().unwrap().active_connections, 0);
        assert_eq!(state.lock().unwrap().completed_connections, 2);
        assert_eq!(state.lock().unwrap().by_addr.len(), 2);

        t1.abort();
        t2.abort();
    }

    async fn echo(addr: String) -> Result<(), Box<dyn std::error::Error>> {
        let listener = TcpListener::bind(&addr).await?;

        loop {
            let (mut socket, _) = listener.accept().await?;

            tokio::spawn(async move {
                let mut buf = [0; 1024];

                loop {
                    let n = match socket.read(&mut buf).await {
                        Ok(n) if n == 0 => return,
                        Ok(n) => n,
                        Err(e) => {
                            eprintln!("failed to read from socket; err = {:?}", e);
                            return;
                        }
                    };

                    if let Err(e) = socket.write_all(&buf[0..n]).await {
                        eprintln!("failed to write to socket; err = {:?}", e);
                        return;
                    }
                }
            });
        }
    }
}
