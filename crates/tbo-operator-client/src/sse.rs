//! Minimal Server-Sent Events (SSE) frame parser.
//!
//! The operator event stream (`GET /v1/events/stream`) emits `text/event-stream`
//! frames: a `ready` handshake, `booth-event` frames whose `data` is a JSON
//! [`BoothEventRecord`](tbo_core::domain::BoothEventRecord), and periodic `ping`
//! heartbeats. This parser is transport-agnostic — it accepts raw byte chunks
//! (which may split lines or UTF-8 sequences arbitrarily) and yields whole
//! events — so it can be unit-tested without a network.

/// A single dispatched SSE event.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SseEvent {
    /// The `event:` field, or `None` when the frame used the default type.
    pub event: Option<String>,
    /// The accumulated `data:` payload (lines joined by `\n`).
    pub data: String,
    /// The `id:` field, when present.
    pub id: Option<String>,
}

impl SseEvent {
    /// Whether this frame carries no field at all (a lone blank line), which
    /// the spec says must not be dispatched.
    fn is_empty(&self) -> bool {
        self.event.is_none() && self.id.is_none() && self.data.is_empty()
    }
}

/// Incremental SSE parser that buffers partial input across byte chunks.
#[derive(Debug, Default)]
pub struct SseParser {
    /// Bytes received but not yet terminated by a newline.
    buf: Vec<u8>,
    /// The event currently being assembled.
    current: SseEvent,
    /// Whether any field has been seen for the current event.
    saw_field: bool,
}

impl SseParser {
    /// Create an empty parser.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed a chunk of bytes and return any events completed by it.
    ///
    /// Lines are only processed once terminated by `\n`; a trailing `\r` is
    /// stripped (CRLF tolerance). A blank line dispatches the assembled event.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        self.buf.extend_from_slice(chunk);
        let mut events = Vec::new();
        while let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
            let mut line: Vec<u8> = self.buf.drain(..=pos).collect();
            line.pop(); // drop the '\n'
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            let text = String::from_utf8_lossy(&line).into_owned();
            if let Some(event) = self.process_line(&text) {
                events.push(event);
            }
        }
        events
    }

    /// Apply one decoded line, returning a dispatched event on a blank line.
    fn process_line(&mut self, line: &str) -> Option<SseEvent> {
        if line.is_empty() {
            return self.dispatch();
        }
        if line.starts_with(':') {
            // Comment line; ignore.
            return None;
        }
        let (field, value) = split_field(line);
        match field {
            "event" => {
                self.current.event = Some(value.to_owned());
                self.saw_field = true;
            }
            "data" => {
                if !self.current.data.is_empty() {
                    self.current.data.push('\n');
                }
                self.current.data.push_str(value);
                self.saw_field = true;
            }
            "id" => {
                self.current.id = Some(value.to_owned());
                self.saw_field = true;
            }
            _ => {
                // Unknown field (e.g. `retry`): ignored.
            }
        }
        None
    }

    /// Finalize the current event on a blank line, if it carries any field.
    fn dispatch(&mut self) -> Option<SseEvent> {
        if !self.saw_field {
            return None;
        }
        let event = std::mem::take(&mut self.current);
        self.saw_field = false;
        if event.is_empty() { None } else { Some(event) }
    }
}

/// Split a field line into `(name, value)`, dropping one optional leading space
/// from the value. A line with no colon is treated as a field with empty value.
fn split_field(line: &str) -> (&str, &str) {
    line.find(':').map_or((line, ""), |idx| {
        let (name, rest) = line.split_at(idx);
        let value = rest.get(1..).unwrap_or("");
        (name, value.strip_prefix(' ').unwrap_or(value))
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::{SseEvent, SseParser};

    #[test]
    fn parses_a_single_named_event() {
        let mut parser = SseParser::new();
        let events = parser.push(b"id: 42\nevent: booth-event\ndata: {\"a\":1}\n\n");
        assert_eq!(
            events,
            vec![SseEvent {
                event: Some("booth-event".to_owned()),
                data: "{\"a\":1}".to_owned(),
                id: Some("42".to_owned()),
            }]
        );
    }

    #[test]
    fn joins_multiple_data_lines_with_newlines() {
        let mut parser = SseParser::new();
        let events = parser.push(b"data: line1\ndata: line2\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "line1\nline2");
    }

    #[test]
    fn reassembles_events_split_across_chunks() {
        let mut parser = SseParser::new();
        assert!(parser.push(b"event: booth-ev").is_empty());
        assert!(parser.push(b"ent\ndata: {\"id\"").is_empty());
        let events = parser.push(b":\"x\"}\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event.as_deref(), Some("booth-event"));
        assert_eq!(events[0].data, "{\"id\":\"x\"}");
    }

    #[test]
    fn handles_crlf_and_ignores_comments() {
        let mut parser = SseParser::new();
        let events = parser.push(b": keep-alive\r\nevent: ping\r\ndata: ts\r\n\r\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event.as_deref(), Some("ping"));
        assert_eq!(events[0].data, "ts");
    }

    #[test]
    fn blank_lines_without_fields_dispatch_nothing() {
        let mut parser = SseParser::new();
        assert!(parser.push(b"\n\n\n").is_empty());
    }

    #[test]
    fn value_without_leading_space_is_preserved() {
        let mut parser = SseParser::new();
        let events = parser.push(b"data:no-space\n\n");
        assert_eq!(events[0].data, "no-space");
    }

    #[test]
    fn multibyte_utf8_split_across_chunks_is_not_corrupted() {
        let mut parser = SseParser::new();
        // "é" is 0xC3 0xA9; split the two bytes across chunks.
        assert!(parser.push(b"data: \xc3").is_empty());
        let events = parser.push(b"\xa9\n\n");
        assert_eq!(events[0].data, "é");
    }

    #[test]
    fn emits_two_events_from_one_chunk() {
        let mut parser = SseParser::new();
        let events = parser.push(b"event: ready\ndata: ok\n\nevent: ping\ndata: t\n\n");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event.as_deref(), Some("ready"));
        assert_eq!(events[1].event.as_deref(), Some("ping"));
    }
}
