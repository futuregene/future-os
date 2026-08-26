use anyhow::{bail, Result};

const MAX_EVENT_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseFrame {
    pub event: Option<String>,
    pub data: String,
}

#[derive(Debug, Default)]
pub struct SseDecoder {
    buffer: Vec<u8>,
    event: Option<String>,
    data: Vec<String>,
}

impl SseDecoder {
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<SseFrame>> {
        self.buffer.extend_from_slice(bytes);
        if self.buffer.len() > MAX_EVENT_BYTES {
            bail!("SSE event exceeded {} bytes", MAX_EVENT_BYTES);
        }

        let mut frames = Vec::new();
        while let Some(newline) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let mut line: Vec<u8> = self.buffer.drain(..=newline).collect();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            let line = String::from_utf8(line).map_err(|error| {
                anyhow::anyhow!("provider returned invalid UTF-8 in SSE frame: {error}")
            })?;
            if line.is_empty() {
                if let Some(frame) = self.finish_frame() {
                    frames.push(frame);
                }
                continue;
            }
            if line.starts_with(':') {
                continue;
            }
            let (field, value) = line
                .split_once(':')
                .map(|(field, value)| (field, value.strip_prefix(' ').unwrap_or(value)))
                .unwrap_or((line.as_str(), ""));
            match field {
                "event" => self.event = Some(value.to_string()),
                "data" => self.data.push(value.to_string()),
                _ => {}
            }
        }
        Ok(frames)
    }

    pub fn finish(&mut self) -> Result<Vec<SseFrame>> {
        if !self.buffer.is_empty() {
            self.buffer.push(b'\n');
            let mut frames = self.push(&[])?;
            if let Some(frame) = self.finish_frame() {
                frames.push(frame);
            }
            Ok(frames)
        } else {
            Ok(self.finish_frame().into_iter().collect())
        }
    }

    fn finish_frame(&mut self) -> Option<SseFrame> {
        if self.event.is_none() && self.data.is_empty() {
            return None;
        }
        Some(SseFrame {
            event: self.event.take(),
            data: std::mem::take(&mut self.data).join("\n"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_crlf_multiline_and_comments_across_byte_boundaries() {
        let wire = b": ping\r\nevent: custom\r\ndata: one\r\ndata: two\r\n\r\ndata: three\n\n";
        let mut decoder = SseDecoder::default();
        let mut frames = Vec::new();
        for byte in wire {
            frames.extend(decoder.push(&[*byte]).unwrap());
        }
        assert_eq!(
            frames,
            vec![
                SseFrame {
                    event: Some("custom".into()),
                    data: "one\ntwo".into(),
                },
                SseFrame {
                    event: None,
                    data: "three".into(),
                }
            ]
        );
    }

    #[test]
    fn rejects_an_unbounded_event_buffer() {
        let mut decoder = SseDecoder::default();
        let error = decoder.push(&vec![b'x'; MAX_EVENT_BYTES + 1]).unwrap_err();
        assert!(error.to_string().contains("SSE event exceeded"));
    }

    #[test]
    fn rejects_invalid_utf8_in_frame() {
        let mut decoder = SseDecoder::default();
        let error = decoder.push(&[0xff, 0xfe, b'\n']).unwrap_err();
        assert!(error.to_string().contains("invalid UTF-8"));
    }

    #[test]
    fn ignores_unknown_field_names() {
        let mut decoder = SseDecoder::default();
        let frames = decoder.push(b"id: 42\ndata: hi\n\n").unwrap();
        assert_eq!(
            frames,
            vec![SseFrame {
                event: None,
                data: "hi".into(),
            }]
        );
    }

    #[test]
    fn finish_flushes_buffered_data_without_trailing_newline() {
        let mut decoder = SseDecoder::default();
        assert!(decoder.finish().unwrap().is_empty());
        decoder.push(b"data: tail").unwrap();
        let frames = decoder.finish().unwrap();
        assert_eq!(
            frames,
            vec![SseFrame {
                event: None,
                data: "tail".into(),
            }]
        );
    }
}
