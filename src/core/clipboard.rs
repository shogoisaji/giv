/// Maximum number of bytes that will be copied via OSC 52.  Many terminals
/// (iTerm2, WezTerm, foot, …) cap OSC 52 sequences somewhere between ~100 KB
/// and ~1 MB; staying well under the common floor avoids silent failures or
/// corrupted terminal state when copying a large diff or commit message.
pub(crate) const OSC52_MAX_BYTES: usize = 100_000;

/// Dependency-free OSC 52 clipboard copy.
///
/// Writes the OSC 52 escape sequence directly to stderr so it reaches the
/// terminal even while the alt-screen is active.  Many modern terminal
/// emulators (kitty, WezTerm, iTerm2, foot, etc.) honour this.
///
/// Text exceeding [`OSC52_MAX_BYTES`] is truncated to that length before
/// encoding, so the sequence stays within terminal caps.
///
/// Format:  ESC ] 52 ; c ; <base64(text)> BEL
#[cfg(not(test))]
pub fn osc52_copy(text: &str) {
    // Use write to stderr; stderr is not redirected by the shell wrapper.
    let mut stderr = std::io::stderr();
    // Best-effort: if the terminal doesn't support OSC 52, silently ignore.
    let _ = osc52_copy_to(&mut stderr, text);
}

/// Test builds should not write terminal escape sequences to the test runner's
/// stderr. Unit tests verify the generated sequence through `osc52_sequence`.
#[cfg(test)]
pub fn osc52_copy(text: &str) {
    let _ = text;
}

fn osc52_copy_to(w: &mut impl std::io::Write, text: &str) -> std::io::Result<()> {
    w.write_all(osc52_sequence(text).as_bytes())
}

fn osc52_sequence(text: &str) -> String {
    let bytes = text.as_bytes();
    let bytes = if bytes.len() > OSC52_MAX_BYTES {
        &bytes[..OSC52_MAX_BYTES]
    } else {
        bytes
    };
    format!("\x1b]52;c;{}\x07", base64_encode(bytes))
}

/// Minimal base64 encoder (RFC 4648, no padding variants needed here).
fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    let mut chunks = input.chunks_exact(3);

    for chunk in chunks.by_ref() {
        let b0 = chunk[0] as usize;
        let b1 = chunk[1] as usize;
        let b2 = chunk[2] as usize;

        out.push(TABLE[b0 >> 2] as char);
        out.push(TABLE[((b0 & 0x3) << 4) | (b1 >> 4)] as char);
        out.push(TABLE[((b1 & 0xf) << 2) | (b2 >> 6)] as char);
        out.push(TABLE[b2 & 0x3f] as char);
    }

    match chunks.remainder() {
        [b0] => {
            let b0 = *b0 as usize;
            out.push(TABLE[b0 >> 2] as char);
            out.push(TABLE[(b0 & 0x3) << 4] as char);
            out.push('=');
            out.push('=');
        }
        [b0, b1] => {
            let b0 = *b0 as usize;
            let b1 = *b1 as usize;
            out.push(TABLE[b0 >> 2] as char);
            out.push(TABLE[((b0 & 0x3) << 4) | (b1 >> 4)] as char);
            out.push(TABLE[(b1 & 0xf) << 2] as char);
            out.push('=');
        }
        _ => {}
    }

    out
}

#[cfg(test)]
mod tests {
    use super::{base64_encode, osc52_copy_to, osc52_sequence, OSC52_MAX_BYTES};

    #[test]
    fn test_base64_empty() {
        assert_eq!(base64_encode(b""), "");
    }

    #[test]
    fn test_base64_one_byte() {
        assert_eq!(base64_encode(b"f"), "Zg==");
    }

    #[test]
    fn test_base64_two_bytes() {
        assert_eq!(base64_encode(b"fo"), "Zm8=");
    }

    #[test]
    fn test_base64_three_bytes() {
        assert_eq!(base64_encode(b"foo"), "Zm9v");
    }

    #[test]
    fn test_base64_hello_world() {
        assert_eq!(base64_encode(b"Hello, World!"), "SGVsbG8sIFdvcmxkIQ==");
    }

    /// Verify the full OSC52 escape sequence framing.
    ///
    /// Format:  ESC ] 52 ; c ; <base64(text)> BEL
    /// i.e. "\x1b]52;c;<b64>\x07"
    #[test]
    fn osc52_escape_framing() {
        // We verify the framing by constructing the expected sequence manually.
        let text = "abc123";
        let expected_b64 = base64_encode(text.as_bytes());
        // Sanity-check the base64 value.
        assert_eq!(expected_b64, "YWJjMTIz");

        // The full OSC 52 sequence must be: ESC ] 52 ; c ; <b64> BEL
        let expected_seq = osc52_sequence(text);
        assert!(
            expected_seq.starts_with("\x1b]52;c;"),
            "OSC 52 prefix missing"
        );
        assert!(expected_seq.ends_with("\x07"), "BEL terminator missing");
        assert!(
            expected_seq.contains("YWJjMTIz"),
            "base64 payload missing from sequence"
        );
    }

    #[test]
    fn osc52_copy_to_writes_sequence_to_supplied_writer() {
        let mut out = Vec::new();
        osc52_copy_to(&mut out, "abc123").expect("write sequence");
        assert_eq!(String::from_utf8(out).unwrap(), "\x1b]52;c;YWJjMTIz\x07");
    }

    /// Verify that the RFC 4648 alphabet is correct end-to-end.
    #[test]
    fn base64_rfc4648_known_vectors() {
        // RFC 4648 §10 test vectors.
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    /// Text within the limit is copied verbatim (no truncation).
    #[test]
    fn osc52_within_limit_is_not_truncated() {
        let text = "x".repeat(OSC52_MAX_BYTES);
        let seq = osc52_sequence(&text);
        // The base64 payload decodes back to exactly OSC52_MAX_BYTES bytes.
        let payload = seq
            .strip_prefix("\x1b]52;c;")
            .and_then(|s| s.strip_suffix("\x07"))
            .unwrap();
        let decoded_len = payload.len() * 3 / 4 - payload.chars().filter(|&c| c == '=').count();
        assert_eq!(decoded_len, OSC52_MAX_BYTES);
    }

    /// Text exceeding the limit is truncated to OSC52_MAX_BYTES so the OSC 52
    /// sequence stays within terminal caps (~100 KB–1 MB).
    #[test]
    fn osc52_over_limit_is_truncated() {
        let text = "y".repeat(OSC52_MAX_BYTES + 100);
        let seq = osc52_sequence(&text);
        let payload = seq
            .strip_prefix("\x1b]52;c;")
            .and_then(|s| s.strip_suffix("\x07"))
            .unwrap();
        let decoded_len = payload.len() * 3 / 4 - payload.chars().filter(|&c| c == '=').count();
        assert_eq!(decoded_len, OSC52_MAX_BYTES);
    }
}
