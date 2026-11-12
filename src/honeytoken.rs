//! Finds agent-tripwire honeytokens in text.
//!
//! A planted value is `TRIPWIRE` followed by an 8-character id in base32, six
//! random characters and two check characters. The check characters are a
//! checksum over the random ones, so a candidate is only treated as a
//! honeytoken when it is a real tripwire token rather than a coincidence.
//!
//! The checksum here is the same one agent-tripwire uses, so the two agree on
//! what is a token.

use sha2::{Digest, Sha256};

/// The marker every planted value starts with.
pub const MARKER: &str = "TRIPWIRE";

/// The length of the id that follows the marker.
pub const ID_LEN: usize = 8;

/// How many of the id characters are random; the rest are the checksum.
const RANDOM_LEN: usize = 6;

/// The alphabet the id is written in.
const BASE32: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// A honeytoken found in some text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Honeytoken {
    /// Where the token starts in the text.
    pub start: usize,
    /// Where the token ends in the text.
    pub end: usize,
    /// The identifier part of the token, in upper case.
    pub id: String,
}

/// Find every honeytoken in `text`, in the order they appear.
pub fn find(text: &str) -> Vec<Honeytoken> {
    let bytes = text.as_bytes();
    let marker = MARKER.as_bytes();
    let span = marker.len() + ID_LEN;
    let mut found = Vec::new();
    let mut i = 0;

    while i + span <= bytes.len() {
        if bytes[i..i + marker.len()].eq_ignore_ascii_case(marker) {
            let id: String = bytes[i + marker.len()..i + span]
                .iter()
                .map(|b| b.to_ascii_uppercase() as char)
                .collect();
            if is_valid_id(&id) {
                found.push(Honeytoken {
                    start: i,
                    end: i + span,
                    id,
                });
                i += span;
                continue;
            }
        }
        i += 1;
    }
    found
}

/// Whether an id is a well-formed tripwire id.
pub fn is_valid_id(id: &str) -> bool {
    id.len() == ID_LEN
        && id.bytes().all(|b| BASE32.contains(&b))
        && id[RANDOM_LEN..] == check_chars(&id[..RANDOM_LEN])
}

/// The id for a given random part, with its check characters appended.
///
/// agent-tripwire picks the random part itself; this is the deterministic
/// counterpart, which is what tests and fixtures need.
pub fn id_with_check_chars(random: &str) -> String {
    format!("{random}{}", check_chars(random))
}

/// The two check characters for a random part of an id.
fn check_chars(random: &str) -> String {
    let digest = Sha256::new()
        .chain_update(b"agent-tripwire:")
        .chain_update(random.as_bytes())
        .finalize();
    let bits = (u16::from(digest[0]) << 2) | u16::from(digest[1] >> 6);
    [bits >> 5, bits & 0x1f]
        .iter()
        .map(|&i| BASE32[usize::from(i)] as char)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An id whose check characters are correct, built the way tripwire does.
    fn make_id(random: &str) -> String {
        format!("{random}{}", check_chars(random))
    }

    #[test]
    fn finds_a_honeytoken() {
        let id = make_id("ABCDEF");
        let text = format!("the value is {MARKER}{id} and more");
        let found = find(&text);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, id);
        assert_eq!(&text[found[0].start..found[0].end], format!("{MARKER}{id}"));
    }

    #[test]
    fn ignores_a_marker_with_a_bad_checksum() {
        let id = make_id("ABCDEF");
        let mut bad = id.clone();
        let last = bad.pop().unwrap();
        bad.push(if last == 'A' { 'B' } else { 'A' });
        assert!(find(&format!("{MARKER}{bad}")).is_empty());
    }

    #[test]
    fn ignores_a_marker_with_the_wrong_alphabet() {
        assert!(find(&format!("{MARKER}AAAAAA01")).is_empty());
    }

    #[test]
    fn ignores_ordinary_text() {
        assert!(find("TRIPWIRE is a word in the documentation").is_empty());
        assert!(find("nothing to see here").is_empty());
    }

    #[test]
    fn finds_several_tokens() {
        let one = make_id("ABCDEF");
        let two = make_id("GHIJKL");
        let text = format!("{MARKER}{one} and {MARKER}{two}");
        let found = find(&text);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].id, one);
        assert_eq!(found[1].id, two);
    }

    #[test]
    fn matches_the_marker_in_any_case() {
        let id = make_id("ABCDEF");
        let text = format!("tripwire{id}");
        let found = find(&text);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, id);
    }
}
