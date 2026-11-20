//! Response body: empty, in-memory bytes, or a stream of byte segments and
//! file ranges read on the blocking pool in 256 KiB chunks, one chunk at a
//! time when hyper asks for it.
//! Pieces are pulled from an iterator only when they are reached, so a long
//! archive never sits in memory. Dropping the body (client gone) drops the
//! open file, and whatever it holds (transfer slots), immediately.

use std::fs::File;
use std::future::Future;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::pin::Pin;
use std::task::{Context, Poll, ready};

use std::sync::{Mutex, PoisonError};

use bytes::Bytes;
use http_body::{Frame, SizeHint};
use tokio::task::JoinHandle;

pub const READ_CHUNK: u64 = 256 * 1024;
const CHUNK_BYTES: usize = READ_CHUNK as usize;

/// Recycled read buffers. A fresh 256 KiB allocation per chunk costs page
/// faults (or, with caching allocators, retained memory); a small pool of
/// buffers handed to hyper and returned on drop avoids both.
static POOL: Mutex<Vec<Vec<u8>>> = Mutex::new(Vec::new());
/// At most this many idle buffers (8 MiB) are kept between chunks.
const POOL_MAX: usize = 32;

fn pool() -> std::sync::MutexGuard<'static, Vec<Vec<u8>>> {
    POOL.lock().unwrap_or_else(PoisonError::into_inner)
}

fn take_buf() -> Vec<u8> {
    pool()
        .pop()
        .unwrap_or_else(|| Vec::with_capacity(CHUNK_BYTES))
}

/// Frees every idle pooled buffer (called by the sweeper once a minute, so
/// an idle server returns to its baseline memory).
pub fn trim_pool() {
    let drained = std::mem::take(&mut *pool());
    drop(drained);
}

/// A chunk owned by a `Bytes`; its buffer goes back to the pool on drop.
struct Pooled(Vec<u8>);

impl AsRef<[u8]> for Pooled {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl Drop for Pooled {
    fn drop(&mut self) {
        let mut buf = std::mem::take(&mut self.0);
        if buf.capacity() == CHUNK_BYTES {
            buf.clear();
            let mut p = pool();
            if p.len() < POOL_MAX {
                p.push(buf);
            }
        }
    }
}

pub enum Piece {
    Mem(Bytes),
    /// `len` bytes of `path` starting at `offset`. `file`, when given, is
    /// already open and positioned at `offset`.
    File {
        file: Option<File>,
        path: PathBuf,
        offset: u64,
        len: u64,
    },
}

impl Piece {
    pub fn len(&self) -> u64 {
        match self {
            Piece::Mem(b) => b.len() as u64,
            Piece::File { len, .. } => *len,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Where a stream's pieces come from, in order.
pub type Source = Box<dyn Iterator<Item = Piece> + Send>;
/// Anything the body keeps alive until it is dropped (slot guards).
pub type Hold = Box<dyn Send>;

type ReadResult = io::Result<(File, Vec<u8>)>;

pub struct StreamBody {
    source: Source,
    /// The piece being sent.
    front: Option<Piece>,
    /// Open handle of the front file piece between reads.
    current: Option<File>,
    /// In-flight read and the number of bytes it was asked for.
    pending: Option<(u64, JoinHandle<ReadResult>)>,
    /// Bytes not yet emitted.
    remaining: u64,
    _hold: Option<Hold>,
}

impl StreamBody {
    /// `len` must be the total length of the pieces `source` yields.
    pub fn new(len: u64, source: Source, hold: Option<Hold>) -> Self {
        StreamBody {
            source,
            front: None,
            current: None,
            pending: None,
            remaining: len,
            _hold: hold,
        }
    }

    /// Moves to the next non-empty piece; false at the end.
    fn advance(&mut self) -> bool {
        self.current = None;
        self.front = self.source.find(|p| !p.is_empty());
        self.front.is_some()
    }

    /// Starts reading the next chunk of the front piece if it is a file
    /// piece with bytes left to request.
    fn start_read(&mut self) {
        let Some(Piece::File {
            file,
            path,
            offset,
            len,
        }) = self.front.as_mut()
        else {
            return;
        };
        if *len == 0 {
            return;
        }
        let want = READ_CHUNK.min(*len);
        *len -= want;
        let pos = *offset;
        *offset = offset.saturating_add(want);
        let handle = self.current.take().or_else(|| file.take());
        let open_path = if handle.is_none() {
            Some(path.clone())
        } else {
            None
        };
        let task = tokio::task::spawn_blocking(move || -> ReadResult {
            let mut f = match (handle, open_path) {
                (Some(f), _) => f,
                (None, Some(p)) => {
                    let mut f = File::open(p)?;
                    f.seek(SeekFrom::Start(pos))?;
                    f
                }
                (None, None) => return Err(io::Error::other("no file to read")),
            };
            let mut buf = take_buf();
            (&mut f).take(want).read_to_end(&mut buf)?;
            Ok((f, buf))
        });
        self.pending = Some((want, task));
    }

    fn poll_next(&mut self, cx: &mut Context<'_>) -> Poll<Option<io::Result<Frame<Bytes>>>> {
        loop {
            if let Some((want, task)) = self.pending.as_mut() {
                let want = *want;
                let res = ready!(Pin::new(task).poll(cx));
                self.pending = None;
                let (file, data) = match res {
                    Ok(Ok(v)) => v,
                    Ok(Err(e)) => return Poll::Ready(Some(Err(e))),
                    Err(e) => return Poll::Ready(Some(Err(io::Error::other(e)))),
                };
                if (data.len() as u64) < want {
                    return Poll::Ready(Some(Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "file shorter than expected",
                    ))));
                }
                let finished = matches!(self.front, Some(Piece::File { len: 0, .. }));
                if finished {
                    drop(file);
                    self.front = None;
                } else {
                    self.current = Some(file);
                }
                self.remaining = self.remaining.saturating_sub(want);
                // The next chunk is read only when hyper asks for it: a peer
                // that stopped reading then pins one chunk, not two. The
                // socket buffer keeps the link busy meanwhile.
                return Poll::Ready(Some(Ok(Frame::data(Bytes::from_owner(Pooled(data))))));
            }
            if self.front.is_none() && !self.advance() {
                return Poll::Ready(None);
            }
            match self.front.as_mut() {
                None => return Poll::Ready(None),
                Some(Piece::Mem(b)) => {
                    let b = std::mem::take(b);
                    self.front = None;
                    self.remaining = self.remaining.saturating_sub(b.len() as u64);
                    return Poll::Ready(Some(Ok(Frame::data(b))));
                }
                Some(Piece::File { .. }) => {
                    self.start_read();
                    if self.pending.is_none() {
                        // nothing left to read in this piece
                        self.front = None;
                    }
                }
            }
        }
    }
}

pub enum Body {
    Empty,
    Full(Option<Bytes>),
    Stream(Box<StreamBody>),
}

impl Body {
    pub fn full(b: impl Into<Bytes>) -> Self {
        Body::Full(Some(b.into()))
    }

    pub fn stream(pieces: Vec<Piece>) -> Self {
        let len = pieces
            .iter()
            .map(Piece::len)
            .fold(0u64, u64::saturating_add);
        Body::Stream(Box::new(StreamBody::new(
            len,
            Box::new(pieces.into_iter()),
            None,
        )))
    }

    /// A stream of `len` bytes from `source`, keeping `hold` alive until the
    /// body is dropped.
    pub fn stream_from(len: u64, source: Source, hold: Option<Hold>) -> Self {
        Body::Stream(Box::new(StreamBody::new(len, source, hold)))
    }
}

impl http_body::Body for Body {
    type Data = Bytes;
    type Error = io::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, io::Error>>> {
        match self.get_mut() {
            Body::Empty => Poll::Ready(None),
            Body::Full(b) => Poll::Ready(b.take().map(|b| Ok(Frame::data(b)))),
            Body::Stream(s) => s.poll_next(cx),
        }
    }

    fn is_end_stream(&self) -> bool {
        match self {
            Body::Empty => true,
            Body::Full(b) => b.is_none(),
            Body::Stream(s) => s.remaining == 0 && s.pending.is_none(),
        }
    }

    fn size_hint(&self) -> SizeHint {
        match self {
            Body::Empty => SizeHint::with_exact(0),
            Body::Full(b) => SizeHint::with_exact(b.as_ref().map_or(0, |b| b.len() as u64)),
            Body::Stream(s) => SizeHint::with_exact(s.remaining),
        }
    }
}

/// Collects a body into memory (tests and small internal uses only).
pub async fn collect(mut body: Body) -> io::Result<Vec<u8>> {
    use http_body_util::BodyExt;
    let mut out = Vec::new();
    while let Some(frame) = body.frame().await {
        if let Ok(data) = frame?.into_data() {
            out.extend_from_slice(&data);
        }
    }
    Ok(out)
}
