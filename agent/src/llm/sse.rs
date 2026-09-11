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
    // The retained partial line was already searched through this byte offset.
    // Do not rescan it when the transport supplies another small fragment.
    scanned: usize,
    event: Option<String>,
    data: Vec<String>,
    event_bytes: usize,
}

impl SseDecoder {
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<SseFrame>> {
        self.buffer.extend_from_slice(bytes);
        if self.buffer.len() > MAX_EVENT_BYTES {
            bail!("SSE event exceeded {} bytes", MAX_EVENT_BYTES);
        }

        let mut frames = Vec::new();
        let mut consumed = 0;
        while let Some(relative) = self.buffer[self.scanned..]
            .iter()
            .position(|byte| *byte == b'\n')
        {
            let newline = self.scanned + relative;
            let line = &self.buffer[consumed..newline];
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            let line = std::str::from_utf8(line).map_err(|error| {
                anyhow::anyhow!("provider returned invalid UTF-8 in SSE frame: {error}")
            })?;
            consumed = newline + 1;
            self.scanned = consumed;
            if line.is_empty() {
                if let Some(frame) = self.finish_frame() {
                    frames.push(frame);
                }
                continue;
            }
            // Bound the entire unfinished event, not just the current line.
            // Counting framing also bounds allocations from many empty data lines.
            self.event_bytes = self.event_bytes.saturating_add(line.len() + 1);
            if self.event_bytes > MAX_EVENT_BYTES {
                bail!("SSE event exceeded {} bytes", MAX_EVENT_BYTES);
            }
            if line.starts_with(':') {
                continue;
            }
            let (field, value) = line
                .split_once(':')
                .map(|(field, value)| (field, value.strip_prefix(' ').unwrap_or(value)))
                .unwrap_or((line, ""));
            match field {
                "event" => self.event = Some(value.to_string()),
                "data" => self.data.push(value.to_string()),
                _ => {}
            }
        }
        // Move the unfinished tail once per push, not once per line. A chunk
        // containing many small SSE lines otherwise incurs quadratic copying.
        self.buffer.drain(..consumed);
        self.scanned = self.buffer.len();
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
        self.event_bytes = 0;
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
    fn many_short_frames_preserve_order_and_incomplete_utf8_tail() {
        let mut wire: String = (0..20_000).map(|i| format!("data: {i}\n\n")).collect();
        wire.push_str("data: ");
        let mut bytes = wire.into_bytes();
        bytes.extend_from_slice(&[0xe4, 0xb8]); // an unfinished Chinese character
        let mut decoder = SseDecoder::default();
        let frames = decoder.push(&bytes).unwrap();
        assert_eq!(frames.len(), 20_000);
        for (i, frame) in frames.iter().enumerate() {
            assert_eq!(frame.data, i.to_string());
        }
        assert_eq!(decoder.buffer, b"data: \xe4\xb8");
        assert_eq!(decoder.scanned, decoder.buffer.len());
        assert_eq!(decoder.push(&[0xad, b'\n', b'\n']).unwrap()[0].data, "中");
        assert!(decoder.buffer.is_empty());
        assert_eq!(decoder.scanned, 0);
    }

    #[test]
    fn every_transport_split_preserves_utf8_crlf_and_final_unterminated_line() {
        let wire = ": ping\r\nevent: custom\r\ndata: 中文\r\ndata: café\r\n\r\ndata: tail🦀";
        let expected = vec![
            SseFrame {
                event: Some("custom".into()),
                data: "中文\ncafé".into(),
            },
            SseFrame {
                event: None,
                data: "tail🦀".into(),
            },
        ];
        for split in 0..=wire.len() {
            let mut decoder = SseDecoder::default();
            let mut frames = decoder.push(&wire.as_bytes()[..split]).unwrap();
            frames.extend(decoder.push(&wire.as_bytes()[split..]).unwrap());
            frames.extend(decoder.finish().unwrap());
            assert_eq!(frames, expected, "transport split {split}");
        }
    }

    #[test]
    fn fragmented_partial_line_remembers_the_scanned_prefix() {
        let mut decoder = SseDecoder::default();
        decoder.push(b"data: ").unwrap();
        for _ in 0..4096 {
            assert!(decoder.push(b"x").unwrap().is_empty());
            assert_eq!(decoder.scanned, decoder.buffer.len());
        }
        assert_eq!(decoder.finish().unwrap()[0].data, "x".repeat(4096));
    }

    #[test]
    fn rejects_an_unbounded_event_buffer() {
        let mut decoder = SseDecoder::default();
        let error = decoder.push(&vec![b'x'; MAX_EVENT_BYTES + 1]).unwrap_err();
        assert!(error.to_string().contains("SSE event exceeded"));
    }

    #[test]
    fn bounds_multiline_events_and_resets_at_frame_boundary() {
        let mut decoder = SseDecoder::default();
        let line = format!("data: {}\n", "x".repeat(1024));
        for _ in 0..2 {
            for _ in 0..4096 {
                assert!(decoder.push(line.as_bytes()).unwrap().is_empty());
            }
            assert_eq!(decoder.push(b"\n").unwrap().len(), 1);
        }
        let mut rejected = false;
        for _ in 0..8192 {
            if decoder.push(line.as_bytes()).is_err() {
                rejected = true;
                break;
            }
        }
        assert!(rejected);
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
