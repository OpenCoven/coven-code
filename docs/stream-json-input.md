# Stream-json input

The source implementation accepts ordered text and inline image blocks in user frames passed to `coven-code --print --input-format stream-json --output-format stream-json`. Images remain typed image content in the query transcript and existing provider serialization. Image-only input triggers a turn. Plain strings, legacy role/content frames and text-only assistant prefill keep their established behavior; text-only arrays retain their newline separator.

```json
{"type":"user","message":{"role":"user","content":[{"type":"text","text":"Describe this photo"},{"type":"image","source":{"type":"base64","media_type":"image/jpeg","data":"<standard base64 JPEG bytes>"}}]}}
```

Input limits are 24 MiB per physical JSON line including its newline, 16 MiB of image file bytes after base64 decoding plus UTF-8 text (including text-only array separators) per message, four images per message, 5 MiB per decoded image and a 2048-pixel maximum image edge. Each image must have a base64 source with the declared `image/png` or `image/jpeg` MIME type, matching image bytes and successful bounded decoding. URL sources, malformed base64, corrupt images, unsupported media and tool/thinking/document blocks are refused. An oversized line is drained to its newline before the next frame is read, so its suffix cannot become another prompt.

A refused frame emits a terminal `result` with `is_error: true`, subtype `error_during_execution` and zero turns. No part of that frame is added to the query transcript or dispatched to a provider. The process remains available for a later valid frame. Unknown typed control frames remain ignored.

This is an engine input capability, not evidence that a phone can upload photos or that every provider supports vision. The controlled `stream_images` fixture runs the actual binary, query loop and keyless Claude adapter against a recording fake CLI. It proves image delivery to that adapter and whole-frame refusal, without inference or provider credentials. The API serialization test checks ordered image/text bytes. The Claude CLI adapter preserves the current turn’s text/image order on both resume and restart; a restart prepends the existing text-only history summary. Historical images are not reconstructed if the CLI session cannot be resumed. Codex currently declares `image_input: false` and is not qualified by these tests. A real installed-engine image-marker response, protected release and consumer capability negotiation remain required before enabling mobile photo controls.
