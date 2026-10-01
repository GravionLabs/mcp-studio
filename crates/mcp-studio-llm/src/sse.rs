//! A small parser for server-sent events (`text/event-stream`), as used by streaming model APIs.

/// One event of the stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    /// The `event:` field, if the event had one.
    pub event: Option<String>,
    /// The `data:` lines joined with newlines.
    pub data: String,
}

/// Feed it the bytes of the response as they arrive; it returns the events that are complete.
/// Chunk boundaries may fall anywhere, including inside a line or a multi-byte character.
#[derive(Debug, Default)]
pub struct SseParser {
    buffer: Vec<u8>,
    event: Option<String>,
    data: Vec<String>,
}

impl SseParser {
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        self.buffer.extend_from_slice(chunk);
        let mut events = Vec::new();
        while let Some(end) = self.buffer.iter().position(|b| *b == b'\n') {
            let raw: Vec<u8> = self.buffer.drain(..=end).collect();
            let line = String::from_utf8_lossy(&raw[..raw.len() - 1]);
            let line = line.strip_suffix('\r').unwrap_or(&line);
            if line.is_empty() {
                // A blank line ends the event.
                if self.event.is_some() || !self.data.is_empty() {
                    events.push(SseEvent {
                        event: self.event.take(),
                        data: std::mem::take(&mut self.data).join("\n"),
                    });
                }
            } else if line.starts_with(':') {
                // A comment, often used to keep the connection alive.
            } else {
                let (field, value) = line.split_once(':').unwrap_or((line, ""));
                let value = value.strip_prefix(' ').unwrap_or(value);
                match field {
                    "event" => self.event = Some(value.to_owned()),
                    "data" => self.data.push(value.to_owned()),
                    _ => {}
                }
            }
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(parser: &mut SseParser, chunks: &[&str]) -> Vec<SseEvent> {
        chunks
            .iter()
            .flat_map(|c| parser.feed(c.as_bytes()))
            .collect()
    }

    #[test]
    fn parses_events_with_names_and_data() {
        let events = all(
            &mut SseParser::default(),
            &["event: ping\ndata: {}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"],
        );
        assert_eq!(
            events,
            vec![
                SseEvent {
                    event: Some("ping".into()),
                    data: "{}".into()
                },
                SseEvent {
                    event: Some("message_stop".into()),
                    data: "{\"type\":\"message_stop\"}".into()
                },
            ]
        );
    }

    #[test]
    fn chunks_may_split_lines_and_events_anywhere() {
        let whole = "event: a\ndata: one\n\ndata: two\n\n";
        for split in 1..whole.len() {
            let (first, second) = whole.split_at(split);
            let events = all(&mut SseParser::default(), &[first, second]);
            assert_eq!(events.len(), 2, "split at {split}");
            assert_eq!(events[0].data, "one");
            assert_eq!(events[1].event, None);
        }
    }

    #[test]
    fn multi_byte_characters_survive_chunk_boundaries() {
        let bytes = "data: 日本語 ✓\n\n".as_bytes();
        for split in 1..bytes.len() {
            let mut parser = SseParser::default();
            let mut events = parser.feed(&bytes[..split]);
            events.extend(parser.feed(&bytes[split..]));
            assert_eq!(events.len(), 1, "split at {split}");
            assert_eq!(events[0].data, "日本語 ✓");
        }
    }

    #[test]
    fn handles_crlf_comments_multiline_data_and_missing_space() {
        let events = all(
            &mut SseParser::default(),
            &[": keep-alive\r\n\r\nevent: x\r\ndata:no space\r\ndata: second\r\n\r\n"],
        );
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "no space\nsecond");
        assert_eq!(events[0].event.as_deref(), Some("x"));
    }

    #[test]
    fn an_incomplete_event_is_not_returned() {
        let mut parser = SseParser::default();
        assert!(parser.feed(b"data: partial\n").is_empty());
        assert_eq!(parser.feed(b"\n").len(), 1);
    }
}
