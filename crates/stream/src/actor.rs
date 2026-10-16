//! The output actor: one thread that owns the running [`Sink`], the newest picture and the frame clock.
//!
//! ```text
//!  app (render results)                 output thread (owns sink + canvas)
//!  ────────────────────                 ───────────────────────────────────
//!  OutputHandle::send_picture ────────► newest picture (older ones are dropped)
//!  OutputHandle::start(spec) ─────────► sink = open(spec); replies with a description or the setup text
//!  OutputHandle::stop / status ───────► closes the sink / reports frames sent and the last error
//!                                       every 1/fps: fit the picture to the canvas and send it
//! ```
//!
//! The last picture is sent again at every tick, so a puppet standing still still produces video.

use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, TrySendError, bounded, select, unbounded};
use serde::{Deserialize, Serialize};

use crate::picture::Picture;
use crate::{Error, Kind, Sink, Spec};

/// How long a caller waits for the thread to answer.
const ANSWER: Duration = Duration::from_secs(8);
/// The wake-up period while no output runs.
const IDLE: Duration = Duration::from_millis(100);

/// What the thread reports about itself.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    /// Whether an output is running.
    pub active: bool,
    /// Which one.
    pub kind: Option<Kind>,
    /// Where it goes (a URL, a device, a source name).
    pub detail: String,
    /// Pictures sent since it started.
    pub frames_sent: u64,
    /// Canvas size.
    pub width: u32,
    /// Canvas size.
    pub height: u32,
    /// Frames per second.
    pub fps: u32,
    /// Why the last output stopped on its own, if it did.
    pub last_error: Option<String>,
}

enum Request {
    Start(Box<Spec>, Sender<Result<Status, String>>),
    Stop(Sender<Status>),
    Status(Sender<Status>),
    Shutdown,
}

/// A cheap-to-clone way to talk to the output thread: document commands hold one.
#[derive(Clone, Debug)]
pub struct OutputLink {
    requests: Sender<Request>,
    pictures: Sender<Picture>,
}

impl OutputLink {
    /// A link to nothing: every call fails with [`Error::Gone`]. For documents built without an output thread.
    #[must_use]
    pub fn disconnected() -> Self {
        let (requests, _) = unbounded();
        let (pictures, _) = bounded(1);
        Self { requests, pictures }
    }

    /// Offers the newest picture. Never blocks: when the thread has not taken the previous one yet, this one is
    /// dropped (the next is newer anyway).
    pub fn send_picture(&self, picture: Picture) {
        let _: Result<(), TrySendError<Picture>> = self.pictures.try_send(picture);
    }

    fn ask<T>(&self, make: impl FnOnce(Sender<T>) -> Request) -> Result<T, Error> {
        let (reply, answer) = bounded(1);
        self.requests.send(make(reply)).map_err(|_| Error::Gone)?;
        answer.recv_timeout(ANSWER).map_err(|_| Error::Gone)
    }

    /// Starts the output `spec` describes, replacing a running one.
    ///
    /// # Errors
    /// [`Error::Output`] carrying the reason (the setup text when the machine lacks something) when it fails to
    /// start, [`Error::Gone`] when the thread has ended.
    pub fn start(&self, spec: &Spec) -> Result<Status, Error> {
        self.ask(|reply| Request::Start(Box::new(spec.clone()), reply))?.map_err(Error::Output)
    }

    /// Stops the running output and reports the final status.
    ///
    /// # Errors
    /// [`Error::Gone`] when the thread has ended.
    pub fn stop(&self) -> Result<Status, Error> {
        self.ask(Request::Stop)
    }

    /// The current status.
    ///
    /// # Errors
    /// [`Error::Gone`] when the thread has ended.
    pub fn status(&self) -> Result<Status, Error> {
        self.ask(Request::Status)
    }
}

/// The app's end of the output thread. Dropping it stops the output and joins the thread.
#[derive(Debug)]
pub struct OutputHandle {
    link: OutputLink,
    thread: Option<JoinHandle<()>>,
}

impl OutputHandle {
    /// Starts the thread (no output runs until [`OutputLink::start`]).
    #[must_use]
    pub fn spawn() -> Self {
        let (requests, request_rx) = unbounded();
        let (pictures, picture_rx) = bounded(1);
        let thread = std::thread::Builder::new().name("charcraft-output".into()).spawn(move || Worker::default().run(&request_rx, &picture_rx)).ok();
        Self { link: OutputLink { requests, pictures }, thread }
    }

    /// A link for commands and pictures.
    #[must_use]
    pub fn link(&self) -> OutputLink {
        self.link.clone()
    }
}

impl std::ops::Deref for OutputHandle {
    type Target = OutputLink;

    fn deref(&self) -> &OutputLink {
        &self.link
    }
}

impl Drop for OutputHandle {
    fn drop(&mut self) {
        let _ = self.link.requests.send(Request::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[derive(Default)]
struct Worker {
    sink: Option<Box<dyn Sink>>,
    spec: Option<Spec>,
    latest: Option<Picture>,
    canvas: Option<Picture>,
    status: Status,
}

impl Worker {
    fn run(mut self, requests: &Receiver<Request>, pictures: &Receiver<Picture>) {
        let mut due = Instant::now();
        loop {
            let wait = due.saturating_duration_since(Instant::now());
            select! {
                recv(requests) -> request => match request {
                    Ok(request) => {
                        if !self.handle(request) {
                            break;
                        }
                    }
                    Err(_) => break,
                },
                recv(pictures) -> picture => {
                    if let Ok(picture) = picture {
                        self.canvas = None;
                        self.latest = Some(picture);
                    }
                },
                default(wait) => {}
            }
            if Instant::now() >= due {
                due = Instant::now().checked_add(self.period()).unwrap_or_else(Instant::now);
                self.tick();
            }
        }
        self.close();
    }

    fn period(&self) -> Duration {
        match (&self.sink, &self.spec) {
            (Some(_), Some(spec)) => Duration::from_secs(1).checked_div(spec.fps.max(1)).unwrap_or(IDLE),
            _ => IDLE,
        }
    }

    fn handle(&mut self, request: Request) -> bool {
        match request {
            Request::Start(spec, reply) => {
                self.close();
                let result = crate::open(&spec);
                let answer = match result {
                    Ok(sink) => {
                        self.status = Status {
                            active: true,
                            kind: Some(spec.kind),
                            detail: sink.describe(),
                            frames_sent: 0,
                            width: spec.width,
                            height: spec.height,
                            fps: spec.fps,
                            last_error: None,
                        };
                        self.sink = Some(sink);
                        self.spec = Some(*spec);
                        self.canvas = None;
                        Ok(self.status.clone())
                    }
                    Err(e) => {
                        self.status.last_error = Some(e.to_string());
                        Err(e.to_string())
                    }
                };
                let _ = reply.send(answer);
            }
            Request::Stop(reply) => {
                self.close();
                let _ = reply.send(self.status.clone());
            }
            Request::Status(reply) => {
                let _ = reply.send(self.status.clone());
            }
            Request::Shutdown => return false,
        }
        true
    }

    /// Closes the running output, keeping the counters for the final status.
    fn close(&mut self) {
        self.sink = None;
        self.spec = None;
        self.status.active = false;
    }

    /// Sends the newest picture, fitted to the canvas, to the sink.
    fn tick(&mut self) {
        let (Some(sink), Some(spec)) = (self.sink.as_mut(), self.spec.as_ref()) else { return };
        let Some(latest) = self.latest.as_ref() else { return };
        if self.canvas.is_none() {
            match latest.fitted(spec.width, spec.height) {
                Ok(canvas) => self.canvas = Some(canvas),
                Err(e) => {
                    self.status.last_error = Some(e.to_string());
                    self.close();
                    return;
                }
            }
        }
        let Some(canvas) = self.canvas.as_ref() else { return };
        match sink.send(canvas) {
            Ok(()) => self.status.frames_sent = self.status.frames_sent.saturating_add(1),
            Err(e) => {
                self.status.last_error = Some(e.to_string());
                self.close();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpStream;

    use super::*;
    use crate::Kind;

    fn web_spec() -> Spec {
        Spec { kind: Kind::Web, width: 64, height: 32, fps: 60, ..Spec::default() }
    }

    fn wait_for(handle: &OutputHandle, mut done: impl FnMut(&Status) -> bool) -> Status {
        for _ in 0..500 {
            let s = handle.status().unwrap();
            if done(&s) {
                return s;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("the output never reached the state");
    }

    #[test]
    fn a_started_web_output_repeats_the_newest_picture_at_the_frame_rate() {
        let handle = OutputHandle::spawn();
        assert!(!handle.status().unwrap().active);
        let started = handle.start(&web_spec()).unwrap();
        assert!(started.active && started.detail.starts_with("http://127.0.0.1:"), "{started:?}");
        handle.send_picture(Picture::solid(100, 100, [10, 200, 30, 255]).unwrap());
        let s = wait_for(&handle, |s| s.frames_sent >= 5);
        assert_eq!((s.width, s.height, s.fps), (64, 32, 60));
        // The picture reaches a client fitted to the 64x32 canvas (square picture, bars left and right).
        let address = started.detail.trim_start_matches("http://").split('/').next().unwrap().to_owned();
        let mut client = TcpStream::connect(&address).unwrap();
        client.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        write!(client, "GET /frame.png HTTP/1.1\r\n\r\n").unwrap();
        let mut response = Vec::new();
        let _ = client.read_to_end(&mut response);
        let at = response.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
        let png = image::load_from_memory_with_format(&response[at..], image::ImageFormat::Png).unwrap().to_rgba8();
        assert_eq!((png.width(), png.height()), (64, 32));
        assert_eq!(png.get_pixel(32, 16).0, [10, 200, 30, 255]);
        assert_eq!(png.get_pixel(2, 16).0[3], 0, "the bar beside the square picture is transparent");
        let stopped = handle.stop().unwrap();
        assert!(!stopped.active && stopped.frames_sent >= 5);
    }

    #[test]
    fn starting_an_unavailable_output_reports_why_and_leaves_the_actor_usable() {
        let handle = OutputHandle::spawn();
        let err = handle.start(&Spec { kind: Kind::Spout, ..Spec::default() }).unwrap_err();
        assert!(err.to_string().contains("Spout"), "{err}");
        assert!(handle.status().unwrap().last_error.is_some());
        assert!(handle.start(&web_spec()).is_ok(), "a later start works");
    }

    #[test]
    fn nothing_is_sent_before_a_picture_exists_and_pictures_without_an_output_are_ignored() {
        let handle = OutputHandle::spawn();
        handle.send_picture(Picture::solid(8, 8, [1, 2, 3, 255]).unwrap());
        handle.start(&web_spec()).unwrap();
        std::thread::sleep(Duration::from_millis(80));
        let s = handle.status().unwrap();
        assert!(s.active);
        handle.stop().unwrap();
        let after = handle.status().unwrap();
        assert!(!after.active);
    }

    #[test]
    fn dropping_the_handle_ends_the_thread_and_closes_the_output() {
        let handle = OutputHandle::spawn();
        handle.start(&web_spec()).unwrap();
        drop(handle);
    }
}
