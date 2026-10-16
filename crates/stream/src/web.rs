//! The web source: a tiny HTTP server on the loopback interface that serves the stream as MJPEG video
//! (`/stream.mjpg`) and the newest picture as a PNG with transparency (`/frame.png`). OBS reads either as a Media
//! source or a Browser source, and any browser can look at it. It works the same on every platform, which makes it
//! the way to reach OBS Virtual Camera (and so every video call program) where the system has no loopback driver.
//!
//! One thread owns the listener, the connected clients and the newest picture; the output feeds it over a channel.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::thread::JoinHandle;
use std::time::Duration;

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, TrySendError, bounded};

use crate::picture::Picture;
use crate::{Error, Sink, Spec};

/// Most video clients served at once.
const MAX_CLIENTS: usize = 8;
/// How long a write to one client may take before it is dropped, so a stalled client cannot stall the stream.
const WRITE_TIMEOUT: Duration = Duration::from_millis(400);
/// How long to wait for the request line of a new connection.
const READ_TIMEOUT: Duration = Duration::from_millis(300);
/// The longest request head read (the first line is all that is used).
const MAX_REQUEST: usize = 4096;
/// JPEG quality of the video stream.
const JPEG_QUALITY: u8 = 85;

const PAGE: &str = "<!doctype html><meta charset=utf-8><title>CharCraft stream</title>\
<style>body{margin:0;background:#15161b;color:#ddd;font:14px sans-serif}\
.checker{background:conic-gradient(#333 25%,#444 0 50%,#333 0 75%,#444 0) 0 0/24px 24px;display:inline-block}\
img{display:block;max-width:100%}</style>\
<p>Video: <code>/stream.mjpg</code> &middot; picture with transparency: <code>/frame.png</code></p>\
<div class=checker><img id=p src=/frame.png alt=\"CharCraft\"></div>\
<script>setInterval(()=>{document.getElementById('p').src='/frame.png?'+Date.now()},200)</script>";

/// The sink: forwards pictures to the server thread.
pub struct WebSink {
    frames: Option<Sender<Picture>>,
    thread: Option<JoinHandle<()>>,
    address: SocketAddr,
}

impl WebSink {
    /// Starts the server on `127.0.0.1:port` (0 picks a free port).
    pub fn start(spec: &Spec) -> Result<Self, Error> {
        let listener =
            TcpListener::bind((Ipv4Addr::LOCALHOST, spec.port)).map_err(|e| Error::Output(format!("cannot listen on 127.0.0.1:{}: {e}", spec.port)))?;
        listener.set_nonblocking(true).map_err(|e| Error::Output(e.to_string()))?;
        let address = listener.local_addr().map_err(|e| Error::Output(e.to_string()))?;
        let (frames, incoming) = bounded(2);
        let backdrop = spec.background.flatten_colour();
        let thread = std::thread::Builder::new()
            .name("charcraft-web-source".into())
            .spawn(move || Server { listener, clients: Vec::new(), latest: None, backdrop }.run(&incoming))
            .map_err(|e| Error::Output(e.to_string()))?;
        Ok(Self { frames: Some(frames), thread: Some(thread), address })
    }

    /// The address clients connect to.
    pub const fn address(&self) -> SocketAddr {
        self.address
    }
}

impl Sink for WebSink {
    fn describe(&self) -> String {
        format!("http://{}/ (video /stream.mjpg, picture /frame.png)", self.address())
    }

    fn send(&mut self, picture: &Picture) -> Result<(), Error> {
        let Some(frames) = &self.frames else { return Err(Error::Gone) };
        match frames.try_send(picture.clone()) {
            // A full queue means the server is busy with the previous picture: this one is skipped.
            Ok(()) | Err(TrySendError::Full(_)) => Ok(()),
            Err(TrySendError::Disconnected(_)) => Err(Error::Output("the web source stopped".to_owned())),
        }
    }
}

impl Drop for WebSink {
    fn drop(&mut self) {
        self.frames = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Server {
    listener: TcpListener,
    clients: Vec<TcpStream>,
    latest: Option<Picture>,
    backdrop: [u8; 3],
}

impl Server {
    fn run(mut self, incoming: &Receiver<Picture>) {
        loop {
            match incoming.recv_timeout(Duration::from_millis(15)) {
                Ok(picture) => self.publish(picture),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            while let Ok((stream, _)) = self.listener.accept() {
                self.accept(stream);
            }
        }
    }

    fn publish(&mut self, picture: Picture) {
        if !self.clients.is_empty()
            && let Ok(jpeg) = picture.to_jpeg(self.backdrop, JPEG_QUALITY)
        {
            self.clients.retain_mut(|client| write_part(client, &jpeg).is_ok());
        }
        self.latest = Some(picture);
    }

    fn accept(&mut self, mut stream: TcpStream) {
        let _ = stream.set_nonblocking(false);
        let _ = stream.set_read_timeout(Some(READ_TIMEOUT));
        let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
        let Some(path) = read_path(&mut stream) else { return };
        let route = path.split('?').next().unwrap_or("/");
        match route {
            "/" | "/index.html" => respond(&mut stream, "200 OK", "text/html; charset=utf-8", PAGE.as_bytes()),
            "/frame.png" => match self.latest.as_ref().map(Picture::to_png) {
                Some(Ok(png)) => respond(&mut stream, "200 OK", "image/png", &png),
                _ => respond(&mut stream, "503 Service Unavailable", "text/plain", b"no picture yet"),
            },
            "/stream.mjpg" => {
                if self.clients.len() >= MAX_CLIENTS {
                    respond(&mut stream, "503 Service Unavailable", "text/plain", b"too many viewers");
                    return;
                }
                let head = "HTTP/1.1 200 OK\r\nContent-Type: multipart/x-mixed-replace; boundary=frame\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n";
                if stream.write_all(head.as_bytes()).is_err() {
                    return;
                }
                if let Some(Ok(jpeg)) = self.latest.as_ref().map(|p| p.to_jpeg(self.backdrop, JPEG_QUALITY))
                    && write_part(&mut stream, &jpeg).is_err()
                {
                    return;
                }
                self.clients.push(stream);
            }
            _ => respond(&mut stream, "404 Not Found", "text/plain", b"not found"),
        }
    }
}

/// The request path of the first line of an HTTP GET, if the connection sent one.
fn read_path(stream: &mut TcpStream) -> Option<String> {
    let mut head = Vec::new();
    let mut chunk = [0_u8; 512];
    while head.len() < MAX_REQUEST && !head.windows(2).any(|w| w == b"\r\n") {
        let n = stream.read(&mut chunk).ok().filter(|n| *n > 0)?;
        head.extend_from_slice(chunk.get(..n)?);
    }
    let line = String::from_utf8_lossy(&head).lines().next()?.to_owned();
    let mut parts = line.split_whitespace();
    (parts.next()? == "GET").then(|| parts.next().map(str::to_owned))?
}

fn respond(stream: &mut TcpStream, status: &str, content_type: &str, body: &[u8]) {
    let head =
        format!("HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n", body.len());
    let _ = stream.write_all(head.as_bytes()).and_then(|()| stream.write_all(body));
}

fn write_part(stream: &mut TcpStream, jpeg: &[u8]) -> std::io::Result<()> {
    let head = format!("--frame\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n", jpeg.len());
    stream.write_all(head.as_bytes())?;
    stream.write_all(jpeg)?;
    stream.write_all(b"\r\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get(address: SocketAddr, path: &str) -> Vec<u8> {
        let mut stream = TcpStream::connect(address).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        write!(stream, "GET {path} HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
        let mut out = Vec::new();
        let _ = stream.read_to_end(&mut out);
        out
    }

    fn body_of(response: &[u8]) -> &[u8] {
        let at = response.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        &response[at + 4..]
    }

    #[test]
    fn the_server_serves_a_page_a_png_with_alpha_and_404s() {
        let mut sink = WebSink::start(&Spec::default()).unwrap();
        let address = sink.address();
        assert!(address.ip().is_loopback());
        let page = get(address, "/");
        assert!(String::from_utf8_lossy(&page).contains("200 OK") && String::from_utf8_lossy(&page).contains("stream.mjpg"));
        assert!(String::from_utf8_lossy(&get(address, "/frame.png")).contains("503"), "no picture yet");
        let picture = Picture::new(2, 2, vec![255, 0, 0, 255, 0, 255, 0, 0, 0, 0, 255, 128, 9, 9, 9, 255]).unwrap();
        sink.send(&picture).unwrap();
        let mut png = Vec::new();
        for _ in 0..100 {
            let response = get(address, "/frame.png?1");
            if String::from_utf8_lossy(&response[..response.len().min(15)]).contains("200") {
                png = body_of(&response).to_vec();
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let decoded = image::load_from_memory_with_format(&png, image::ImageFormat::Png).unwrap().to_rgba8();
        assert_eq!(decoded.get_pixel(1, 0).0, [0, 255, 0, 0], "the transparent pixel keeps its alpha");
        assert_eq!(decoded.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert!(String::from_utf8_lossy(&get(address, "/nope")).contains("404"));
    }

    #[test]
    fn the_video_stream_delivers_jpeg_parts_as_pictures_arrive() {
        let mut sink = WebSink::start(&Spec::default()).unwrap();
        let mut client = TcpStream::connect(sink.address()).unwrap();
        client.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        write!(client, "GET /stream.mjpg HTTP/1.1\r\n\r\n").unwrap();
        std::thread::sleep(Duration::from_millis(100));
        let picture = Picture::solid(32, 16, [200, 50, 50, 255]).unwrap();
        let mut seen = Vec::new();
        let mut buffer = [0_u8; 4096];
        for _ in 0..200 {
            sink.send(&picture).unwrap();
            if let Ok(n) = client.read(&mut buffer) {
                seen.extend_from_slice(&buffer[..n]);
            }
            if seen.windows(2).any(|w| w == [0xff, 0xd8]) {
                break;
            }
        }
        let text = String::from_utf8_lossy(&seen);
        assert!(text.contains("multipart/x-mixed-replace") && text.contains("image/jpeg"), "{text}");
        assert!(seen.windows(2).any(|w| w == [0xff, 0xd8]), "a JPEG part arrived");
    }

    #[test]
    fn a_port_in_use_is_reported() {
        let first = WebSink::start(&Spec::default()).unwrap();
        let taken = Spec { port: first.address().port(), ..Spec::default() };
        assert!(matches!(WebSink::start(&taken), Err(Error::Output(m)) if m.contains("cannot listen")));
    }
}
