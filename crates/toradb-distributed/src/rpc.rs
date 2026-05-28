use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};

use crate::protocol::{Request, Response};

pub fn send_response(stream: &mut TcpStream, resp: &Response) -> Result<(), String> {
    let body = serde_json::to_vec(resp).map_err(|e| e.to_string())?;
    let len = (body.len() as u32).to_le_bytes();
    stream.write_all(&len).map_err(|e| e.to_string())?;
    stream.write_all(&body).map_err(|e| e.to_string())?;
    stream.flush().map_err(|e| e.to_string())
}

pub fn recv_request(stream: &mut TcpStream) -> Result<Request, String> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).map_err(|e| e.to_string())?;
    let len = u32::from_le_bytes(len_buf) as usize;
    if len > 64 * 1024 * 1024 {
        return Err("RPC request too large".into());
    }
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).map_err(|e| e.to_string())?;
    serde_json::from_slice(&body).map_err(|e| e.to_string())
}

pub fn call(addr: &str, req: &Request) -> Result<Response, String> {
    let mut stream = TcpStream::connect(addr).map_err(|e| format!("connect {addr}: {e}"))?;
    let body = serde_json::to_vec(req).map_err(|e| e.to_string())?;
    let len = (body.len() as u32).to_le_bytes();
    stream.write_all(&len).map_err(|e| e.to_string())?;
    stream.write_all(&body).map_err(|e| e.to_string())?;
    stream.flush().map_err(|e| e.to_string())?;

    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).map_err(|e| e.to_string())?;
    let rlen = u32::from_le_bytes(len_buf) as usize;
    if rlen > 64 * 1024 * 1024 {
        return Err("RPC response too large".into());
    }
    let mut resp_body = vec![0u8; rlen];
    stream.read_exact(&mut resp_body).map_err(|e| e.to_string())?;
    serde_json::from_slice(&resp_body).map_err(|e| e.to_string())
}

pub fn serve_one<F>(addr: &str, handler: F) -> Result<(), String>
where
    F: FnOnce(Request) -> Response,
{
    let listener = TcpListener::bind(addr).map_err(|e| format!("bind {addr}: {e}"))?;
    let (mut stream, _) = listener
        .accept()
        .map_err(|e| format!("accept: {e}"))?;
    let req = recv_request(&mut stream)?;
    let resp = handler(req);
    send_response(&mut stream, &resp)
}

pub fn serve<F>(addr: &str, mut handler: F) -> Result<(), String>
where
    F: FnMut(Request) -> Response + Send + 'static,
{
    let listener = TcpListener::bind(addr).map_err(|e| format!("bind {addr}: {e}"))?;
    for conn in listener.incoming() {
        let Ok(mut stream) = conn else { continue };
        let req = match recv_request(&mut stream) {
            Ok(r) => r,
            Err(e) => {
                let _ = send_response(&mut stream, &Response::err(e));
                continue;
            }
        };
        let resp = handler(req);
        let _ = send_response(&mut stream, &resp);
    }
    Ok(())
}
