//! E-Tracker compiled (`.etc`, `.cop`, `.saa`).
//!
//! RoboPlay's `etc.c`: `ETC_HEADER` begins with five `uint16_t` offsets, then a
//! 20-byte signature "ETracker (C) BY ESI." at header offset 10. A bare module
//! has the header at file offset 0 (signature at 10); a module that bundles the
//! player code has the header at 0x4b3 (signature at 0x4bd). There is no title.

use super::MusicInfo;

const SIGNATURE: &[u8] = b"ETracker (C) BY ESI.";
const SIG_IN_HEADER: usize = 10;
const PLAYER_BASE: usize = 0x4b3;

pub(super) fn parse(bytes: &[u8]) -> Option<MusicInfo> {
    let has_sig = |base: usize| {
        bytes.get(base + SIG_IN_HEADER..base + SIG_IN_HEADER + SIGNATURE.len()) == Some(SIGNATURE)
    };

    let with_player = if has_sig(0) {
        false
    } else if has_sig(PLAYER_BASE) {
        true
    } else {
        return None;
    };

    Some(MusicInfo {
        format: "E-Tracker compiled",
        title: None,
        author: None,
        positions: None,
        subsongs: Some(0),
        channels: Some(6),
        extra: vec![(
            "Player code".to_string(),
            if with_player {
                "included"
            } else {
                "not present"
            }
            .to_string(),
        )],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(base: usize) -> Vec<u8> {
        let mut buf = vec![0u8; base + SIG_IN_HEADER + SIGNATURE.len() + 4];
        let at = base + SIG_IN_HEADER;
        buf[at..at + SIGNATURE.len()].copy_from_slice(SIGNATURE);
        buf
    }

    #[test]
    fn parses_bare_module() {
        let info = parse(&build(0)).unwrap();
        assert_eq!(info.format, "E-Tracker compiled");
        assert_eq!(info.channels, Some(6));
        assert!(info
            .extra
            .contains(&("Player code".to_string(), "not present".to_string())));
    }

    #[test]
    fn detects_bundled_player() {
        let info = parse(&build(PLAYER_BASE)).unwrap();
        assert!(info
            .extra
            .contains(&("Player code".to_string(), "included".to_string())));
    }

    #[test]
    fn rejects_without_signature() {
        assert!(parse(&[0u8; 64]).is_none());
    }
}
