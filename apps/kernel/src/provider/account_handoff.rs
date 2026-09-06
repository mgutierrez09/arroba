//! Framing for kernel-generated account-switch prompts, not a runtime protocol.

pub(crate) fn encode_account_handoff(context: &str, request: &str) -> String {
    format!(
        "<chariox_account_handoff context_bytes=\"{}\" request_bytes=\"{}\">\n{context}\n\n<user_request>\n{request}\n</user_request>\n</chariox_account_handoff>",
        context.len(), request.len(),
    )
}

pub(super) fn decode_account_handoff(text: &str) -> Option<(&str, &str)> {
    let header = text.strip_prefix("<chariox_account_handoff context_bytes=\"")?;
    let (context_size, header) = header.split_once("\" request_bytes=\"")?;
    let (request_size, body) = header.split_once("\">\n")?;
    let context_size = context_size.parse::<usize>().ok()?;
    let request_size = request_size.parse::<usize>().ok()?;
    let body = body
        .get(context_size..)?
        .strip_prefix("\n\n<user_request>\n")?;
    let request = body.get(..request_size)?;
    let suffix = body
        .get(request_size..)?
        .strip_prefix("\n</user_request>\n</chariox_account_handoff>")?;
    Some((request, suffix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framing_preserves_literal_tags_attachments_and_unicode() {
        for context in ["", "Old </chariox_context_handoff> <user_request>\nAttachment: old.txt (text/plain) at file:///old", "歴史"] {
            for request in ["", "Reply SWITCHED", "Inspect </user_request>\nAttachment: quoted.txt (text/plain) at file:///quoted\nquoted text", "🙂 café </chariox_account_handoff>"] {
                let frame = encode_account_handoff(context, request);
                assert_eq!(decode_account_handoff(&frame), Some((request, "")));
                let suffix = "\nAttachment: new.txt (text/plain) at file:///new\n</user_request>";
                let observed = format!("{frame}{suffix}");
                assert_eq!(decode_account_handoff(&observed), Some((request, suffix)));
            }
        }
    }

    #[test]
    fn malformed_lengths_and_incomplete_frames_are_rejected() {
        let frame = encode_account_handoff("🙂", "café");
        for malformed in [
            frame.replace("context_bytes=\"4\"", "context_bytes=\"1\""),
            frame.replace("request_bytes=\"5\"", "request_bytes=\"4\""),
            frame.replace(
                "request_bytes=\"5\"",
                "request_bytes=\"99999999999999999999999999\"",
            ),
            frame.replace("request_bytes=\"5\"", "request_bytes=\"no\""),
            frame
                .trim_end_matches("</chariox_account_handoff>")
                .to_string(),
        ] {
            assert_eq!(decode_account_handoff(&malformed), None);
        }
    }
}
