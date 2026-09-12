//! CLI-only output ownership. No blocking stdout worker or detached task.
use std::{
    fs::File,
    io::{self, Write},
    os::{
        fd::{AsRawFd, RawFd},
        unix::fs::{FileTypeExt, MetadataExt},
    },
    time::Duration,
};

use nix::fcntl::{FcntlArg, OFlag, fcntl};
use tokio::{io::unix::AsyncFd, sync::mpsc::Receiver};
use turnforge::{CancellationToken, Event, model::ModelDelta};

pub async fn forward(
    mut events: Receiver<Event>,
    json: bool,
    cancel: &CancellationToken,
) -> io::Result<()> {
    let mut output = Output::stdout()?;
    while let Some(event) = events.recv().await {
        let bytes = if json {
            let mut bytes = serde_json::to_vec(&event).map_err(io::Error::other)?;
            bytes.push(b'\n');
            bytes
        } else {
            match event {
                Event::ModelDelta {
                    delta: ModelDelta::Text { text },
                } => text.into_bytes(),
                Event::RunFinished { .. } => vec![b'\n'],
                _ => continue,
            }
        };
        let write = output.write_all(&bytes);
        tokio::pin!(write);
        // A slow sink gets a bounded opportunity to drain. After cancellation
        // we still try to deliver terminal events, but never wait indefinitely.
        let result = tokio::select! {
            biased;
            _ = cancel.cancelled() => tokio::time::timeout(Duration::from_millis(100), &mut write).await,
            result = tokio::time::timeout(Duration::from_secs(2), &mut write) => result,
        };
        match result {
            Ok(result) => result?,
            Err(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "stdout stalled; output may be incomplete",
                ));
            }
        }
    }
    Ok(())
}

enum Output {
    Pollable(AsyncFd<Descriptor>),
    File(File),
}

impl Output {
    fn stdout() -> io::Result<Self> {
        let file = File::from(nix::unistd::dup(io::stdout()).map_err(io::Error::from)?);
        // epoll/kqueue do not portably support regular files. Local file output
        // uses ordinary writes; network/special filesystem stalls are out of scope.
        let metadata = file.metadata()?;
        let dev_null = metadata.file_type().is_char_device()
            && metadata.rdev() == std::fs::metadata("/dev/null")?.rdev();
        if metadata.is_file() || dev_null {
            return Ok(Self::File(file));
        }
        let flags =
            OFlag::from_bits_truncate(fcntl(&file, FcntlArg::F_GETFL).map_err(io::Error::from)?);
        fcntl(&file, FcntlArg::F_SETFL(flags | OFlag::O_NONBLOCK)).map_err(io::Error::from)?;
        Ok(Self::Pollable(AsyncFd::new(Descriptor { file, flags })?))
    }

    async fn write_all(&mut self, mut bytes: &[u8]) -> io::Result<()> {
        match self {
            Self::File(file) => file.write_all(bytes),
            Self::Pollable(fd) => {
                while !bytes.is_empty() {
                    let mut ready = fd.writable().await?;
                    match ready.try_io(|fd| {
                        nix::unistd::write(&fd.get_ref().file, bytes).map_err(io::Error::from)
                    }) {
                        Ok(Ok(0)) => return Err(io::ErrorKind::WriteZero.into()),
                        Ok(Ok(count)) => bytes = &bytes[count..],
                        Ok(Err(error)) => return Err(error),
                        Err(_) => continue,
                    }
                }
                Ok(())
            }
        }
    }
}

struct Descriptor {
    file: File,
    flags: OFlag,
}
impl AsRawFd for Descriptor {
    fn as_raw_fd(&self) -> RawFd {
        self.file.as_raw_fd()
    }
}
impl Drop for Descriptor {
    fn drop(&mut self) {
        // dup shares the open-file description; restore the host's original flags.
        let _ = fcntl(&self.file, FcntlArg::F_SETFL(self.flags));
    }
}
