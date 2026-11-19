//! Server-sent events, as the WHATWG HTML standard defines the `text/event-stream` format,
//! decoded from bytes that arrive cut anywhere: inside a line, between `\r` and `\n`, inside a
//! multi-byte character.
//!
//! Bytes are buffered until a whole line is in, and only whole lines are decoded as UTF-8; a
//! line break never falls inside a multi-byte character, so a character split across pieces
//! arrives whole.
//!
//! The buffers are bounded: a line or an event's data longer than [`MAX_LINE_BYTES`] or
//! [`MAX_DATA_BYTES`] is an [`Overflow`], so an endpoint that streams without line breaks
//! cannot grow the browser's memory without end.

#[cfg(test)]
mod tests;

/// The longest line read. A token's chunk of JSON is a few hundred bytes.
pub(crate) const MAX_LINE_BYTES: usize = 1 << 20;

/// The most data one event may carry, its lines together.
pub(crate) const MAX_DATA_BYTES: usize = 1 << 20;

/// One event: its type, when the server named one, and its data lines joined by `\n`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Event {
    /// The `event:` field; `None` for the default `message` type.
    pub name: Option<String>,
    /// The `data:` fields, joined by line breaks.
    pub data: String,
}

/// A line or an event outgrew its bound; the stream is not one to read further.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Overflow;

/// Decodes a stream of pieces into events.
#[derive(Debug, Default)]
pub(crate) struct Decoder {
    /// The bytes of the line being read.
    line: Vec<u8>,
    /// True right after a `\r`, so that a `\n` next is part of the same line break.
    after_cr: bool,
    /// True once the first line was read; only that one may start with a byte order mark.
    started: bool,
    name: Option<String>,
    data: String,
    /// True once a `data:` field arrived for the event being read, even an empty one.
    has_data: bool,
}

impl Decoder {
    /// Takes the next piece of the stream; returns the events it completed.
    pub(crate) fn push(&mut self, bytes: &[u8]) -> Result<Vec<Event>, Overflow> {
        let mut events = Vec::new();
        for &byte in bytes {
            if std::mem::take(&mut self.after_cr) && byte == b'\n' {
                continue;
            }
            match byte {
                b'\n' => self.end_line(&mut events)?,
                b'\r' => {
                    self.after_cr = true;
                    self.end_line(&mut events)?;
                }
                other => {
                    if self.line.len() == MAX_LINE_BYTES {
                        return Err(self.overflow());
                    }
                    self.line.push(other);
                }
            }
        }
        Ok(events)
    }

    /// The stream ended. The standard drops an event the stream ended in the middle of; this
    /// keeps it, as a server that closes right after its last `data:` line still meant it.
    pub(crate) fn finish(mut self) -> Result<Option<Event>, Overflow> {
        let mut events = Vec::new();
        if !self.line.is_empty() {
            self.end_line(&mut events)?;
        }
        self.end_line(&mut events)?;
        Ok(events.pop())
    }

    fn end_line(&mut self, events: &mut Vec<Event>) -> Result<(), Overflow> {
        let bytes = std::mem::take(&mut self.line);
        let text = String::from_utf8_lossy(&bytes);
        let line = if std::mem::replace(&mut self.started, true) {
            &text
        } else {
            text.strip_prefix('\u{feff}').unwrap_or(&text)
        };
        if line.is_empty() {
            self.dispatch(events);
            return Ok(());
        }
        if line.starts_with(':') {
            return Ok(());
        }
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line, ""),
        };
        match field {
            "data" => {
                if self.data.len() + value.len() + 1 > MAX_DATA_BYTES {
                    return Err(self.overflow());
                }
                self.data.push_str(value);
                self.data.push('\n');
                self.has_data = true;
            }
            "event" => self.name = Some(value.to_owned()),
            _ => {}
        }
        Ok(())
    }

    fn dispatch(&mut self, events: &mut Vec<Event>) {
        let name = self.name.take();
        let mut data = std::mem::take(&mut self.data);
        if std::mem::take(&mut self.has_data) {
            data.pop();
            events.push(Event { name, data });
        }
    }

    /// Lets go of what was buffered, so a stream that overflowed holds no memory.
    fn overflow(&mut self) -> Overflow {
        *self = Self::default();
        Overflow
    }
}
