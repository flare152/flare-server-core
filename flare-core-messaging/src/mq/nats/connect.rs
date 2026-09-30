//! 按配置的 URL 连接 NATS，并让 URL 里的凭据真正生效。

use async_nats::{Client, ConnectOptions, ServerAddr};

/// 连接 `url`（单个 NATS 地址）。
///
/// async-nats 会解析 `nats://user:pass@host` / `nats://token@host` 里的凭据，但
/// 建连时并不使用它们；要求鉴权的 broker 会直接以 authorization violation 拒绝。
/// 这里把凭据取出来作为连接选项传入，错误信息不包含凭据。
pub async fn connect(url: &str) -> Result<Client, String> {
    let addr: ServerAddr = url
        .trim()
        .parse()
        .map_err(|_| "invalid NATS server address".to_string())?;
    options_for(&addr)
        .connect(addr)
        .await
        .map_err(|error| error.to_string())
}

fn options_for(addr: &ServerAddr) -> ConnectOptions {
    match (addr.username(), addr.password()) {
        (Some(user), Some(password)) => {
            ConnectOptions::with_user_and_password(percent_decode(user), percent_decode(password))
        }
        (Some(token), None) => ConnectOptions::with_token(percent_decode(token)),
        _ => ConnectOptions::new(),
    }
}

/// URL userinfo 是百分号编码的；非法转义原样保留。
fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && let (Some(high), Some(low)) = (
                bytes.get(i + 1).and_then(|b| hex(*b)),
                bytes.get(i + 2).and_then(|b| hex(*b)),
            )
        {
            out.push((high * 16 + low) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn userinfo_is_percent_decoded() {
        assert_eq!(percent_decode("p%40ss%3Aword"), "p@ss:word");
        assert_eq!(percent_decode("plain"), "plain");
        assert_eq!(percent_decode("bad%zz%4"), "bad%zz%4");
    }

    #[tokio::test]
    async fn embedded_credentials_reach_an_authenticating_server() {
        // A minimal NATS server: send INFO, capture the CONNECT line, then reply with an error.
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let (read, mut write) = socket.into_split();
            write
                .write_all(
                    format!(
                        "INFO {{\"server_id\":\"t\",\"version\":\"2.10.0\",\"host\":\"127.0.0.1\",\"port\":{port},\"max_payload\":1048576,\"proto\":1,\"auth_required\":true}}\r\n"
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            let mut line = String::new();
            BufReader::new(read).read_line(&mut line).await.unwrap();
            write
                .write_all(b"-ERR 'Authorization Violation'\r\n")
                .await
                .unwrap();
            line
        });
        let _ = connect(&format!("nats://core:s%40cret@127.0.0.1:{port}")).await;
        let connect_line = server.await.unwrap();
        assert!(connect_line.starts_with("CONNECT "), "{connect_line}");
        assert!(connect_line.contains("\"user\":\"core\""), "{connect_line}");
        assert!(
            connect_line.contains("\"pass\":\"s@cret\""),
            "{connect_line}"
        );
    }
}
