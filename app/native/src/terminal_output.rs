use std::collections::VecDeque;

use crate::{
    terminal_backend::TerminalBackendResult,
    terminal_runtime_contract::{TerminalRuntimeOutputEvent, TerminalRuntimeOutputReadResult},
};

pub const OUTPUT_CAPACITY_BYTES: usize = 1024 * 1024;
pub const MIN_OUTPUT_READ_BYTES: usize = 4;

struct OutputChunk {
    start_cursor: u64,
    data: String,
}

pub struct OutputBuffer {
    capacity: usize,
    size: usize,
    next_cursor: u64,
    chunks: VecDeque<OutputChunk>,
}

impl OutputBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            size: 0,
            next_cursor: 0,
            chunks: VecDeque::new(),
        }
    }

    pub fn next_cursor(&self) -> u64 {
        self.next_cursor
    }

    pub fn push(&mut self, runtime_id: &str, data: String) -> TerminalRuntimeOutputEvent {
        let event = TerminalRuntimeOutputEvent::new(runtime_id, self.next_cursor, data);
        self.next_cursor = event.end_cursor;
        let mut chunk = OutputChunk {
            start_cursor: event.start_cursor,
            data: event.data.clone(),
        };
        if chunk.data.len() > self.capacity {
            let mut offset = chunk.data.len() - self.capacity;
            while !chunk.data.is_char_boundary(offset) {
                offset += 1;
            }
            chunk.start_cursor += offset as u64;
            chunk.data.drain(..offset);
        }
        self.size += chunk.data.len();
        self.chunks.push_back(chunk);
        while self.size > self.capacity {
            if let Some(removed) = self.chunks.pop_front() {
                self.size -= removed.data.len();
            }
        }
        event
    }

    pub fn read(
        &self,
        runtime_id: &str,
        requested_cursor: u64,
        max_bytes: usize,
    ) -> TerminalBackendResult<TerminalRuntimeOutputReadResult> {
        let earliest_cursor = self
            .chunks
            .front()
            .map(|chunk| chunk.start_cursor)
            .unwrap_or(self.next_cursor);
        let cursor = requested_cursor.max(earliest_cursor).min(self.next_cursor);
        let max_bytes = max_bytes.max(MIN_OUTPUT_READ_BYTES);
        let mut data = String::new();
        let mut next_cursor = cursor;
        for chunk in &self.chunks {
            let chunk_end = chunk.start_cursor + chunk.data.len() as u64;
            if chunk_end <= next_cursor || chunk.start_cursor > next_cursor {
                continue;
            }
            let mut offset = (next_cursor - chunk.start_cursor) as usize;
            while offset < chunk.data.len() && !chunk.data.is_char_boundary(offset) {
                offset += 1;
            }
            next_cursor = chunk.start_cursor + offset as u64;
            let remaining = max_bytes.saturating_sub(data.len());
            if remaining == 0 {
                break;
            }
            let mut end = (offset + remaining).min(chunk.data.len());
            while end > offset && !chunk.data.is_char_boundary(end) {
                end -= 1;
            }
            if end == offset {
                break;
            }
            data.push_str(&chunk.data[offset..end]);
            next_cursor = chunk.start_cursor + end as u64;
            if data.len() == max_bytes {
                break;
            }
        }
        Ok(TerminalRuntimeOutputReadResult {
            runtime_id: runtime_id.into(),
            requested_cursor,
            earliest_cursor,
            next_cursor,
            truncated: requested_cursor < earliest_cursor,
            data,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::OutputBuffer;

    #[test]
    fn reads_utf8_incrementally_and_reports_truncation() {
        let mut output = OutputBuffer::new(6);
        let first = output.push("runtime-1", "abc".into());
        let second = output.push("runtime-1", "中def".into());
        assert_eq!((first.start_cursor, first.end_cursor), (0, 3));
        assert_eq!((second.start_cursor, second.end_cursor), (3, 9));
        let truncated = output.read("runtime-1", 0, 32).unwrap();
        assert!(truncated.truncated);
        assert_eq!(truncated.earliest_cursor, 3);
        assert_eq!(truncated.data, "中def");
        assert_eq!(output.read("runtime-1", 6, 3).unwrap().data, "def");
        let small = output.read("runtime-1", 3, 1).unwrap();
        assert_eq!(small.data, "中");
        assert!(small.next_cursor > 3);
    }
}
