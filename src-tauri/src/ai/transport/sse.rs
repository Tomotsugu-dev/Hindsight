//! Splits an SSE response body into events.

use super::error_chain;

/// Reads the response body and returns the data of each event, in order.
///
/// Reading stops after the event for which `is_last` returns `true`.
///
/// If reading the body fails, the first value has the events read before the failure.
/// The second value is a message for the user with the event count and the cause.
///
/// An event that has not ended when the body ends is still returned.
/// The protocol decides if the reply is complete.
/// Dropping that event could make a complete reply look cut.
pub(super) async fn read_events(
    resp: reqwest::Response,
    is_last: impl Fn(&str) -> bool,
) -> (Vec<String>, Option<String>) {
    use futures_util::StreamExt;

    let mut lines = Lines::default();
    let mut events = Events::default();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(bytes) => {
                for line in lines.push(&bytes) {
                    if events.push_line(&line) && events.ended.last().is_some_and(|d| is_last(d)) {
                        return (events.finish(), None);
                    }
                }
            }
            Err(e) => {
                let cut = format!(
                    "cannot read the rest of the stream (events received: {}): {}",
                    events.ended.len(),
                    error_chain(&e)
                );
                return (events.finish(), Some(cut));
            }
        }
    }
    if let Some(line) = lines.finish() {
        events.push_line(&line);
    }
    (events.finish(), None)
}

#[derive(Default)]
struct Lines {
    buf: String,
}

impl Lines {
    fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        self.buf.push_str(&String::from_utf8_lossy(bytes));
        let mut out = Vec::new();
        while let Some(end) = self.buf.find('\n') {
            let line: String = self.buf.drain(..=end).collect();
            out.push(line[..end].to_string());
        }
        out
    }

    fn finish(self) -> Option<String> {
        (!self.buf.is_empty()).then_some(self.buf)
    }
}

#[derive(Default)]
struct Events {
    ended: Vec<String>,
    data: Option<String>,
}

impl Events {
    /// Returns `true` when the line ends an event.
    fn push_line(&mut self, line: &str) -> bool {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            if let Some(data) = self.data.take() {
                self.ended.push(data);
                return true;
            }
        } else if let Some(value) = line.strip_prefix("data:") {
            let value = value.strip_prefix(' ').unwrap_or(value);
            match &mut self.data {
                Some(data) => {
                    data.push('\n');
                    data.push_str(value);
                }
                None => self.data = Some(value.to_string()),
            }
        }
        false
    }

    fn finish(mut self) -> Vec<String> {
        if let Some(data) = self.data.take() {
            self.ended.push(data);
        }
        self.ended
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events(chunks: &[&[u8]]) -> Vec<String> {
        let mut lines = Lines::default();
        let mut events = Events::default();
        for chunk in chunks {
            for line in lines.push(chunk) {
                events.push_line(&line);
            }
        }
        if let Some(line) = lines.finish() {
            events.push_line(&line);
        }
        events.finish()
    }

    /// 只取 data 字段：注释、event 字段跳过，\r\n 行尾也认。
    #[test]
    fn keeps_only_data_fields() {
        let stream = b"\n: keep-alive\nevent: message\ndata: a\r\n\r\ndata:b\n\n";
        assert_eq!(events(&[stream]), ["a", "b"]);
    }

    /// 空行之前的几行 data 是同一个事件，用 \n 连起来。
    #[test]
    fn joins_the_data_lines_of_one_event() {
        let stream = b"data: a\ndata: b\n\ndata: c\n\n";
        assert_eq!(events(&[stream]), ["a\nb", "c"]);
    }

    /// 只去掉冒号后的一个空格，其余空白原样保留。
    #[test]
    fn removes_only_one_leading_space() {
        assert_eq!(events(&[b"data:  a \n\n"]), [" a "]);
    }

    /// 最后一个事件没有空行、最后一行没有换行，也不丢。
    #[test]
    fn keeps_the_last_line_without_newline() {
        assert_eq!(events(&[b"data: a\n\ndata: b"]), ["a", "b"]);
    }
}
