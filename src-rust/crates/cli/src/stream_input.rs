//! Bounded, untrusted stdin content. Only text and inline PNG/JPEG can enter a turn.

use std::io::Cursor;

use base64::{engine::general_purpose::STANDARD, Engine};
use claurst_core::types::{ContentBlock, ImageSource, Message};
use serde_json::Value;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt};

pub(super) const MAX_FRAME_BYTES: usize = 24 * 1024 * 1024;
const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;
const MAX_CONTENT_BYTES: usize = 16 * 1024 * 1024;
const MAX_IMAGES: usize = 4;
const MAX_IMAGE_EDGE: u32 = 2048;

pub(super) enum FrameRead {
    Frame(String),
    Rejected(&'static str),
    Eof,
}

/// Limit allocation before JSON parsing, and discard the rest of a rejected
/// physical line so its suffix can never become another input frame.
pub(super) async fn read_frame<R: AsyncBufRead + Unpin>(
    reader: &mut R,
) -> std::io::Result<FrameRead> {
    let mut bytes = Vec::new();
    let n = (&mut *reader)
        .take(MAX_FRAME_BYTES as u64 + 1)
        .read_until(b'\n', &mut bytes)
        .await?;
    if n == 0 {
        return Ok(FrameRead::Eof);
    }
    if n > MAX_FRAME_BYTES {
        if !bytes.ends_with(b"\n") {
            loop {
                let chunk = reader.fill_buf().await?;
                if chunk.is_empty() {
                    break;
                }
                let end = chunk.iter().position(|byte| *byte == b'\n');
                let consumed = end.map_or(chunk.len(), |index| index + 1);
                reader.consume(consumed);
                if end.is_some() {
                    break;
                }
            }
        }
        return Ok(FrameRead::Rejected("stream-json frame exceeds 24 MiB"));
    }
    Ok(match String::from_utf8(bytes) {
        Ok(line) => FrameRead::Frame(line),
        Err(_) => FrameRead::Rejected("stream-json frame is not UTF-8"),
    })
}

pub(super) fn user_message(content: Option<&Value>, allow_images: bool) -> Result<Message, String> {
    let blocks = match content {
        None | Some(Value::Null) => return Ok(Message::user("")),
        Some(Value::String(text)) => {
            if text.len() > MAX_CONTENT_BYTES {
                return Err("message content exceeds 16 MiB".into());
            }
            return Ok(Message::user(text));
        }
        Some(Value::Array(blocks)) => blocks,
        _ => return Err("frame content must be text or an array of blocks".into()),
    };
    let mut accepted = Vec::new();
    let mut images = 0;
    let mut total = 0;
    for block in blocks {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                let text = block
                    .get("text")
                    .and_then(Value::as_str)
                    .ok_or("text block must contain string text")?;
                total += text.len();
                accepted.push(ContentBlock::Text {
                    text: text.to_owned(),
                });
            }
            Some("image") if allow_images => {
                images += 1;
                if images > MAX_IMAGES {
                    return Err("a message can contain at most four images".into());
                }
                let source = block.get("source").ok_or("image block missing source")?;
                if source.get("type").and_then(Value::as_str) != Some("base64")
                    || source.get("url").is_some()
                {
                    return Err("images must use inline base64 sources, without URLs".into());
                }
                let mime = source
                    .get("media_type")
                    .and_then(Value::as_str)
                    .ok_or("image source missing media_type")?;
                let format = match mime {
                    "image/png" => image::ImageFormat::Png,
                    "image/jpeg" => image::ImageFormat::Jpeg,
                    _ => return Err("only image/png and image/jpeg are supported".into()),
                };
                let data = source
                    .get("data")
                    .and_then(Value::as_str)
                    .ok_or("image source missing base64 data")?;
                if data.len() > MAX_IMAGE_BYTES.div_ceil(3) * 4 {
                    return Err("image exceeds 5 MiB".into());
                }
                let decoded = STANDARD
                    .decode(data)
                    .map_err(|_| "image data is not valid base64")?;
                if decoded.is_empty() || decoded.len() > MAX_IMAGE_BYTES {
                    return Err("image must contain between one byte and 5 MiB".into());
                }
                if image::guess_format(&decoded).ok() != Some(format) {
                    return Err("image bytes do not match the declared MIME type".into());
                }
                let mut reader = image::ImageReader::with_format(Cursor::new(&decoded), format);
                let mut limits = image::Limits::default();
                limits.max_image_width = Some(MAX_IMAGE_EDGE);
                limits.max_image_height = Some(MAX_IMAGE_EDGE);
                limits.max_alloc = Some(64 * 1024 * 1024);
                reader.limits(limits);
                reader
                    .decode()
                    .map_err(|_| "image is corrupt or exceeds decoding limits (2048-pixel edge)")?;
                total += decoded.len();
                accepted.push(ContentBlock::Image {
                    source: ImageSource {
                        source_type: "base64".into(),
                        media_type: Some(mime.to_owned()),
                        data: Some(data.to_owned()),
                        url: None,
                    },
                });
            }
            _ => {
                return Err(
                    "unsupported input block; only user text and inline images are accepted".into(),
                )
            }
        }
        if total > MAX_CONTENT_BYTES {
            return Err("message content exceeds 16 MiB".into());
        }
    }
    if images == 0 {
        if total + accepted.len().saturating_sub(1) > MAX_CONTENT_BYTES {
            return Err("message content exceeds 16 MiB".into());
        }
        // Keep the established text-only block separator and wire shape.
        let text = accepted
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        Ok(Message::user(text))
    } else {
        Ok(Message::user_blocks(accepted))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGMQVDIGAACuAGcVHqFfAAAAAElFTkSuQmCC";

    fn image_block(data: &str, mime: &str) -> Value {
        json!({"type":"image","source":{"type":"base64","media_type":mime,"data":data}})
    }

    #[test]
    fn ordered_text_and_image_blocks_reach_api_serialization() {
        let content = json!([
            {"type":"text","text":"before"},image_block(PNG,"image/png"),
            {"type":"text","text":"after"}
        ]);
        let message = user_message(Some(&content), true).unwrap();
        let wire = claurst_api::types::ApiMessage::from(&message);
        assert_eq!(wire.role, "user");
        assert_eq!(wire.content, content);
    }

    #[test]
    fn png_and_jpeg_decode_and_mime_must_match() {
        let mut jpeg = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image::RgbImage::new(2, 2))
            .write_to(&mut jpeg, image::ImageFormat::Jpeg)
            .unwrap();
        let encoded = STANDARD.encode(jpeg.into_inner());
        assert!(user_message(Some(&json!([image_block(&encoded, "image/jpeg")])), true).is_ok());
        assert!(user_message(Some(&json!([image_block(&encoded, "image/png")])), true).is_err());
        assert!(user_message(Some(&json!([image_block(PNG, "image/jpeg")])), true).is_err());
    }

    #[test]
    fn image_bounds_apply_to_count_size_dimensions_and_decoding() {
        let block = image_block(PNG, "image/png");
        assert!(user_message(Some(&Value::Array(vec![block.clone(); MAX_IMAGES])), true).is_ok());
        assert!(user_message(Some(&Value::Array(vec![block; MAX_IMAGES + 1])), true).is_err());
        let mut bytes = STANDARD.decode(PNG).unwrap();
        bytes.resize(MAX_IMAGE_BYTES, 0);
        let at_limit = image_block(&STANDARD.encode(&bytes), "image/png");
        assert!(user_message(Some(&json!([at_limit])), true).is_ok());
        bytes.push(0);
        assert!(user_message(
            Some(&json!([image_block(&STANDARD.encode(&bytes), "image/png")])),
            true
        )
        .is_err());
        let mut oversized = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image::RgbImage::new(MAX_IMAGE_EDGE + 1, 1))
            .write_to(&mut oversized, image::ImageFormat::Png)
            .unwrap();
        assert!(user_message(
            Some(&json!([image_block(
                &STANDARD.encode(oversized.into_inner()),
                "image/png"
            )])),
            true
        )
        .is_err());
        let truncated = STANDARD.encode(&STANDARD.decode(PNG).unwrap()[..40]);
        assert!(user_message(Some(&json!([image_block(&truncated, "image/png")])), true).is_err());
    }

    #[test]
    fn total_decoded_content_is_bounded() {
        let mut bytes = STANDARD.decode(PNG).unwrap();
        bytes.resize(MAX_IMAGE_BYTES, 0);
        let block = image_block(&STANDARD.encode(bytes), "image/png");
        assert!(user_message(Some(&Value::Array(vec![block; 4])), true).is_err());
        assert!(user_message(Some(&Value::String("a".repeat(MAX_CONTENT_BYTES))), true).is_ok());
        assert!(user_message(
            Some(&Value::String("a".repeat(MAX_CONTENT_BYTES + 1))),
            true
        )
        .is_err());
    }

    #[test]
    fn text_array_separators_count_toward_the_content_limit() {
        let at_limit = json!([{"type":"text","text":"a".repeat(MAX_CONTENT_BYTES - 1)},{"type":"text","text":""}]);
        assert_eq!(
            user_message(Some(&at_limit), true)
                .unwrap()
                .get_all_text()
                .len(),
            MAX_CONTENT_BYTES
        );
        let over_limit =
            json!([{"type":"text","text":"a".repeat(MAX_CONTENT_BYTES)},{"type":"text","text":""}]);
        assert!(user_message(Some(&over_limit), true).is_err());
    }

    #[test]
    fn malformed_sources_and_assistant_images_are_refused() {
        for source in [
            json!({"type":"base64","media_type":"image/png","data":""}),
            json!({"type":"base64","media_type":"image/png","data":PNG,"url":"https://example.invalid"}),
            json!({"type":"base64","data":PNG}),
            json!({"type":"base64","media_type":"image/png"}),
            json!({"type":"url","url":"https://example.invalid"}),
        ] {
            assert!(user_message(Some(&json!([{"type":"image","source":source}])), true).is_err());
        }
        assert!(user_message(Some(&json!([image_block(PNG, "image/png")])), false).is_err());
    }

    #[tokio::test]
    async fn oversized_line_is_drained_without_parsing_its_suffix() {
        let mut bytes = vec![b'x'; MAX_FRAME_BYTES + 10];
        bytes.extend_from_slice(
            b"{\"role\":\"user\",\"content\":\"not a frame\"}\nactual next frame\n",
        );
        let mut reader = Cursor::new(bytes);
        assert!(matches!(
            read_frame(&mut reader).await.unwrap(),
            FrameRead::Rejected(_)
        ));
        assert!(
            matches!(read_frame(&mut reader).await.unwrap(), FrameRead::Frame(line) if line == "actual next frame\n")
        );
        assert!(matches!(
            read_frame(&mut reader).await.unwrap(),
            FrameRead::Eof
        ));
    }

    #[tokio::test]
    async fn bounded_reader_handles_exact_limit_utf8_and_final_line() {
        let mut bytes = vec![b'x'; MAX_FRAME_BYTES - 1];
        bytes.extend_from_slice(b"\n\xff\nlast frame");
        let mut reader = Cursor::new(bytes);
        assert!(
            matches!(read_frame(&mut reader).await.unwrap(), FrameRead::Frame(line) if line.len() == MAX_FRAME_BYTES)
        );
        assert!(matches!(
            read_frame(&mut reader).await.unwrap(),
            FrameRead::Rejected(_)
        ));
        assert!(
            matches!(read_frame(&mut reader).await.unwrap(), FrameRead::Frame(line) if line == "last frame")
        );
        assert!(matches!(
            read_frame(&mut reader).await.unwrap(),
            FrameRead::Eof
        ));
    }
}
