use std::error::Error;
use std::io;
use std::thread::sleep;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use log::{debug, error, info};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::unbounded_channel;

use ws::request::Request;
use ws::response::{Response, NOT_FOUND, OK};
use ws::websocket::{Message, Opcode, WebSocket};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init();

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    info!("server started at http://{}", listener.local_addr()?);

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
    let path = req.uri().split("?").next().unwrap();
    if path == "/" {
        let mut response = Response::bytes(OK, include_bytes!("echo_html.html"));
        response.add_header("content-type", "text/html;charset=UTF-8");
        response.write(&mut stream).await?;
        return Ok(());
    }

    if path != "/ws" {
        Response::status(NOT_FOUND).write(&mut stream).await?;
        return Ok(());
    }

    let mut ws = match WebSocket::upgrade(&req, stream).await? {
        Some(ws) => ws,
        None => {
            error!("upgrade failed");
            return Ok(());
        }
    };
    let mut close_send = false;

    let (tx, mut rx) = unbounded_channel();
    std::thread::spawn(move || loop {
        sleep(Duration::from_secs(2));
        match tx.send(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis(),
        ) {
            Ok(()) => {}
            Err(_) => break,
        }
    });

    loop {
        tokio::select! {
            msg = ws.receive() => {
                match msg? {
                    Some(msg) => {
                        debug!("receive {:?}", msg);
                        let opcode = msg.opcode();
                        let payload = msg.payload().to_vec();
                        if echo_msg(&mut ws, opcode, &payload, &mut close_send).await? {
                            break;
                        }
                    }
                    None => break,
                }
            }
            msg = rx.recv() => {
                if let Some(msg) = msg {
                    let msg = format!("{}", msg);
                    let reply = Message::new(Opcode::Text, msg.as_bytes());
                    ws.send(reply).await?;
                }
            }
        }
    }

    Ok(())
}

async fn echo_msg(
    ws: &mut WebSocket<TcpStream>,
    opcode: Opcode,
    payload: &[u8],
    close_send: &mut bool,
) -> io::Result<bool> {
    match opcode {
        Opcode::Ping | Opcode::Text | Opcode::Binary => {
            let reply = Message::new(opcode, payload);
            ws.send(reply).await?;
        }
        Opcode::Pong => {}
        Opcode::Close if *close_send => return Ok(true),
        Opcode::Close => {
            ws.send(Message::new(Opcode::Close, &[])).await?;
            *close_send = true;
        }
    }
    Ok(false)
}
