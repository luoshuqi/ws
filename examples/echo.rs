use std::env::set_var;
use std::error::Error;
use std::time::Duration;

use log::{debug, error, info};
use tokio::net::{TcpListener, TcpStream};

use ws::request::Request;
use ws::websocket::{Message, Opcode, WebSocket};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    set_var("RUST_LOG", "debug");
    env_logger::init();

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    info!("server started at ws://{}", listener.local_addr()?);

    loop {
        let (stream, addr) = listener.accept().await?;
        tokio::spawn(async move {
            match handle_client(stream).await {
                Ok(()) => {}
                Err(err) => eprintln!("{}: {:?}", addr, err),
            }
        });
    }
}

async fn handle_client(mut stream: TcpStream) -> Result<(), Box<dyn Error>> {
    let mut buf = vec![0u8; 2048];
    let req = Request::new(&mut stream, &mut buf, Duration::from_secs(60)).await?;

    let mut ws = match WebSocket::upgrade(&req, stream).await? {
        Some(ws) => ws,
        None => {
            error!("upgrade failed");
            return Ok(());
        }
    };
    let mut close_send = false;

    loop {
        match ws.receive().await? {
            Some(msg) => {
                debug!("receive {:?}", msg);
                match msg.opcode() {
                    Opcode::Ping | Opcode::Text | Opcode::Binary => {
                        let payload = msg.payload().to_vec();
                        let reply = Message::new(msg.opcode(), &payload);
                        ws.send(reply).await?;
                    }
                    Opcode::Pong => {}
                    Opcode::Close if close_send => break,
                    Opcode::Close => {
                        ws.send(Message::new(Opcode::Close, &[])).await?;
                        close_send = true;
                    }
                }
            }
            None => break,
        }
    }

    Ok(())
}
