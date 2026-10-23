//! Bounded OSC 7501 decoder. Terminal content is status evidence, never auth.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Idle,
    Working,
    Permission,
    Input,
    External,
    Done,
    Error,
}

#[derive(Default)]
pub struct Decoder {
    state: u8,
    body: Vec<u8>,
    last: Option<Status>,
}

impl Decoder {
    pub fn feed(&mut self, data: &str) -> Vec<Status> {
        let mut results = Vec::new();
        for byte in data.bytes() {
            match (self.state, byte) {
                (0, 0x1b) => self.state = 1,
                (1, b']') => {
                    self.body.clear();
                    self.state = 2;
                }
                (2 | 3, 7) | (3, b'\\') => {
                    if let Some(status) = parse(&self.body) {
                        // Do not let rendering chatter produce repeated alerts.
                        if self.last.as_ref() != Some(&status) {
                            self.last = Some(status.clone());
                            results.push(status);
                        }
                    }
                    self.body.clear();
                    self.state = 0;
                }
                (2, 0x1b) => self.state = 3,
                (2, byte) if self.body.len() < 4096 => self.body.push(byte),
                (2, _) => {
                    self.body.clear();
                    self.state = 4;
                }
                (3, b']') => {
                    self.body.clear();
                    self.state = 2;
                }
                (3, _) => {
                    self.body.clear();
                    self.state = 0;
                }
                (4, 0x1b) => self.state = 5,
                (4, 7) | (5, b'\\') => self.state = 0,
                (5, b']') => {
                    self.body.clear();
                    self.state = 2;
                }
                (5, _) => self.state = 4,
                (1, 0x1b) => {}
                (0, _) | (1, _) => self.state = 0,
                _ => {}
            }
        }
        results
    }
}

fn parse(body: &[u8]) -> Option<Status> {
    let text = std::str::from_utf8(body).ok()?.strip_prefix("7501;")?;
    let mut state = None;
    let mut app = None;
    let mut kind = None;
    for pair in text.split(':') {
        let (key, value) = pair.split_once('=')?;
        if value.contains(';') || value.chars().any(char::is_control) {
            return None;
        }
        let slot = match key {
            "state" => &mut state,
            "app" => &mut app,
            "kind" => &mut kind,
            "id" => {
                // TUI emits the root record. Ignore unrelated nested records.
                if !value.is_empty() {
                    return None;
                }
                continue;
            }
            _ => continue,
        };
        if slot.replace(value).is_some() {
            return None;
        }
    }
    if app != Some("dsh-tui") {
        return None;
    }
    Some(match state? {
        "idle" => Status::Idle,
        "working" => Status::Working,
        "done" => Status::Done,
        "error" => Status::Error,
        "blocked" => match kind? {
            "permission" => Status::Permission,
            "question" => Status::Input,
            "auth" => Status::External,
            _ => return None,
        },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decodes_every_chunk_boundary_and_multiple_records() {
        let sequence = "\x1b]7501;state=working:app=dsh-tui\x1b\\\x1b]7501;state=blocked:app=dsh-tui:kind=permission\x07";
        for split in 0..sequence.len() {
            let mut decoder = Decoder::default();
            let mut got = decoder.feed(&sequence[..split]);
            got.extend(decoder.feed(&sequence[split..]));
            assert_eq!(got, [Status::Working, Status::Permission]);
        }
    }
    #[test]
    fn rejects_spoofed_app_duplicates_nested_records_and_oversize_recovers() {
        let mut d = Decoder::default();
        for text in [
            "7501;state=done:app=other",
            "7501;state=done:state=working:app=dsh-tui",
            "7501;state=done:app=dsh-tui:id=child",
            "7501;state=blocked:app=dsh-tui:kind=bad",
        ] {
            assert!(d.feed(&format!("\x1b]{text}\x07")).is_empty());
        }
        assert!(
            d.feed(&format!("\x1b]7501;{}\x07", "x".repeat(5000)))
                .is_empty()
        );
        assert_eq!(
            d.feed("\x1b]7501;state=idle:app=dsh-tui\x1b\\"),
            [Status::Idle]
        );
        assert!(d.feed("\x1b]7501;state=idle:app=dsh-tui\x07").is_empty());
        assert_eq!(
            d.feed("\x1b]7501;state=error:app=dsh-tui\x07"),
            [Status::Error]
        );
    }
}
