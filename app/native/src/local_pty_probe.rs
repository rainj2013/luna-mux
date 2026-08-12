//! Cross-platform PTY spike used to validate the M2.1 library choice.
//! Production local runtime ownership and process cleanup are implemented in
//! the next milestone behind `TerminalBackend`.

use std::{
    io::{Read, Write},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

pub struct LocalPtyProbe {
    child: Box<dyn portable_pty::Child + Send + Sync>,
    writer: Box<dyn Write + Send>,
    output: mpsc::Receiver<Vec<u8>>,
    master: Box<dyn portable_pty::MasterPty + Send>,
}

impl LocalPtyProbe {
    pub fn spawn() -> Result<Self, String> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: 24,
                cols: 100,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| error.to_string())?;
        let command = default_command();
        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| error.to_string())?;
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|error| error.to_string())?;
        let (sender, output) = mpsc::channel();
        thread::spawn(move || {
            let mut buffer = [0_u8; 4096];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(size) => {
                        if sender.send(buffer[..size].to_vec()).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        let writer = pair
            .master
            .take_writer()
            .map_err(|error| error.to_string())?;
        Ok(Self {
            child,
            writer,
            output,
            master: pair.master,
        })
    }

    pub fn write(&mut self, input: &[u8]) -> Result<(), String> {
        self.writer
            .write_all(input)
            .map_err(|error| error.to_string())?;
        self.writer.flush().map_err(|error| error.to_string())
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), String> {
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| error.to_string())
    }

    pub fn read_until(&mut self, needle: &[u8], timeout: Duration) -> Result<Vec<u8>, String> {
        let deadline = Instant::now() + timeout;
        let mut output = Vec::new();
        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if let Ok(chunk) = self
                .output
                .recv_timeout(remaining.min(Duration::from_millis(100)))
            {
                if chunk.windows(4).any(|window| window == b"\x1b[6n") {
                    self.writer
                        .write_all(b"\x1b[1;1R")
                        .and_then(|_| self.writer.flush())
                        .map_err(|error| error.to_string())?;
                }
                output.extend(chunk);
                if output.windows(needle.len()).any(|window| window == needle) {
                    return Ok(output);
                }
            }
        }
        Err(format!(
            "PTY 未在 {:?} 内输出目标内容；已收到：{}",
            timeout,
            String::from_utf8_lossy(&output)
        ))
    }

    pub fn kill(&mut self) -> Result<(), String> {
        self.child.kill().map_err(|error| error.to_string())
    }
}

fn default_command() -> CommandBuilder {
    #[cfg(windows)]
    {
        let mut command = CommandBuilder::new("cmd.exe");
        command.args([
            "/D",
            "/C",
            "chcp 65001 > nul & echo LunaMux PTY ✓ & ping 127.0.0.1 -n 30 > nul",
        ]);
        return command;
    }
    #[cfg(not(windows))]
    {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
        let mut command = CommandBuilder::new(shell);
        command.args(["-lc", "printf 'LunaMux PTY ✓\\n'; sleep 30"]);
        command
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_pty_library_can_spawn_unicode_and_resize() {
        let mut probe = LocalPtyProbe::spawn().expect("spawn local PTY");
        let output = probe
            .read_until("LunaMux PTY ✓".as_bytes(), Duration::from_secs(5))
            .expect("read PTY output");
        assert!(String::from_utf8_lossy(&output).contains("LunaMux PTY"));
        probe.resize(120, 40).expect("resize PTY");
        probe.write(&[3]).expect("interrupt PTY");
        probe.kill().expect("cleanup PTY");
    }
}
