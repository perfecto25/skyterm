use std::io::{Read, Write};
use std::thread;

use anyhow::{Context, Result};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};

pub struct PtyHandle {
    pub writer: Box<dyn Write + Send>,
    pub master: Box<dyn portable_pty::MasterPty + Send>,
    pub rx: async_channel::Receiver<Vec<u8>>,
    #[allow(dead_code)]
    pub child: Box<dyn portable_pty::Child + Send + Sync>,
}

pub fn spawn(cols: u16, rows: u16) -> Result<PtyHandle> {
    let sys = native_pty_system();
    let pair = sys
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .context("openpty")?;

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    let mut cmd = CommandBuilder::new(&shell);
    cmd.env("TERM", "xterm-256color");
    if let Ok(cwd) = std::env::current_dir() {
        cmd.cwd(cwd);
    }
    let child = pair.slave.spawn_command(cmd).context("spawn shell")?;
    drop(pair.slave);

    let writer = pair.master.take_writer().context("take_writer")?;
    let mut reader = pair.master.try_clone_reader().context("clone_reader")?;

    let (tx, rx) = async_channel::unbounded::<Vec<u8>>();
    thread::Builder::new()
        .name("skyterm-pty-reader".into())
        .spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if tx.send_blocking(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        log::warn!("pty read error: {e}");
                        break;
                    }
                }
            }
        })
        .context("spawn reader thread")?;

    Ok(PtyHandle {
        writer,
        master: pair.master,
        rx,
        child,
    })
}
