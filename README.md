# ws

#### 介绍

Rust + [tokio](https://tokio.rs) 实现的异步 WebSocket 服务端 

#### 使用示例

```rust
// echo 示例
async fn handle_client(mut stream: TcpStream) -> Result<(), Box<dyn Error>> {
    let mut buf = vec![0u8; 2048];
    let req = Request::new(&mut stream, &mut buf).await?;

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
```

完整示例见 

- [examples/echo](https://gitee.com/luoshuqi/ws/blob/master/examples/echo.rs)
- [examples/echo_html](https://gitee.com/luoshuqi/ws/blob/master/examples/echo_html.rs)