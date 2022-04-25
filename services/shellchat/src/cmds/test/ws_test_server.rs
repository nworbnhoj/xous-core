// The MIT License (MIT)
// Copyright (c) 2019 David Haig

// Modified to act as a simple websocket test server @nworbnhoj

// Test websocket server that listens on localhost port 1337.
// The server will echo all Text and Ping messages back to
// the client as well as responding to any opening and closing handshakes.

use core::result::Result;
use embedded_websocket as ws;
use std::net::{TcpListener, TcpStream};
use std::str::Utf8Error;
use std::thread;
use std::{
    io::{Read, Write},
    usize,
};
use ws::{
    framer::{Framer, FramerError},
    WebSocketContext, WebSocketSendMessageType, WebSocketServer,
};

#[derive(Debug)]
pub enum WebServerError {
    Io(std::io::Error),
    Framer(FramerError<std::io::Error>),
    WebSocket(ws::Error),
    Utf8Error,
}

impl From<std::io::Error> for WebServerError {
    fn from(err: std::io::Error) -> WebServerError {
        WebServerError::Io(err)
    }
}

impl From<FramerError<std::io::Error>> for WebServerError {
    fn from(err: FramerError<std::io::Error>) -> WebServerError {
        WebServerError::Framer(err)
    }
}

impl From<ws::Error> for WebServerError {
    fn from(err: ws::Error) -> WebServerError {
        WebServerError::WebSocket(err)
    }
}

impl From<Utf8Error> for WebServerError {
    fn from(_: Utf8Error) -> WebServerError {
        WebServerError::Utf8Error
    }
}

pub fn main() -> std::io::Result<()> {
    let addr = "127.0.0.1:1337";
    let listener = TcpListener::bind(addr)?;
    log::info!("Listening on: {}", addr);

    // accept connections and process them serially
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                thread::spawn(|| match handle_client(stream) {
                    Ok(()) => log::info!("Connection closed"),
                    Err(e) => log::info!("Error: {:?}", e),
                });
            }
            Err(e) => log::info!("Failed to establish a connection: {}", e),
        }
    }

    Ok(())
}

fn handle_client(mut stream: TcpStream) -> Result<(), WebServerError> {
    log::info!("Client connected {}", stream.peer_addr()?);
    let mut read_buf = [0; 4000];
    let mut read_cursor = 0;

    if let Some(websocket_context) = read_header(&mut stream, &mut read_buf, &mut read_cursor)?
    {
        // this is a websocket upgrade HTTP request
        let mut write_buf = [0; 4000];
        let mut frame_buf = [0; 4000];
        let mut websocket = WebSocketServer::new_server();
        let mut framer = Framer::new(
            &mut read_buf,
            &mut read_cursor,
            &mut write_buf,
            &mut websocket,
        );
        
        // complete the opening handshake with the client
        match framer.accept(&mut stream, &websocket_context) {
            Ok(()) => log::info!("Accepted Websocket connection"),
            Err(e) => log::info!("Failed to accept websocket connection: {:?}", e),
        };

        // read websocket frames
        while let text = framer.read_text(&mut stream, &mut frame_buf)? {
            log::info!("Received: {}", text.unwrap());

            // send the text back to the client
            match framer.write(
                &mut stream,
                WebSocketSendMessageType::Text,
                true,
                text.unwrap().as_bytes(),
            ) {
                Ok(_) => log::info!("Wrote to websocket: {:?} ", text.unwrap()),
                Err(e) => log::info!("Failed to write to websocket: {:?}", e),
            }
        }

        log::info!("Closing websocket connection");
        Ok(())
    } else {
        Ok(())
    }
}

fn read_header(
    stream: &mut TcpStream,
    read_buf: &mut [u8],
    read_cursor: &mut usize,
) -> Result<Option<WebSocketContext>, WebServerError> {
    loop {
        let mut headers = [httparse::EMPTY_HEADER; 16];
        let mut request = httparse::Request::new(&mut headers);

        let received_size = stream.read(&mut read_buf[*read_cursor..])?;

        match request
            .parse(&read_buf[..*read_cursor + received_size])
            .unwrap()
        {
            httparse::Status::Complete(len) => {
                // if we read exactly the right amount of bytes for the HTTP header then read_cursor would be 0
                *read_cursor += received_size - len;
                let headers = request.headers.iter().map(|f| (f.name, f.value));
                match ws::read_http_header(headers)? {
                    Some(websocket_context) => match request.path {
                        Some("/test") => {
                            return Ok(Some(websocket_context));
                        }
                        _ => return_404_not_found(stream, request.path)?,
                    },
                    None => {
                        handle_non_websocket_http_request(stream, request.path)?;
                    }
                }
                return Ok(None);
            }
            // keep reading while the HTTP header is incomplete
            httparse::Status::Partial => *read_cursor += received_size,
        }
    }
}

fn handle_non_websocket_http_request(
    stream: &mut TcpStream,
    path: Option<&str>,
) -> Result<(), WebServerError> {
    log::info!("Received file request: {:?}", path);
    return_404_not_found(stream, None)?;
    Ok(())
}

fn return_404_not_found(
    stream: &mut TcpStream,
    unknown_path: Option<&str>,
) -> Result<(), WebServerError> {
    log::info!("Unknown path: {:?}", unknown_path);
    let html = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
    stream.write_all(&html.as_bytes())?;
    Ok(())
}
