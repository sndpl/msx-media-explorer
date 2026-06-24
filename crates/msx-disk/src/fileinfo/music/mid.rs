//! Standard MIDI file (`.mid`).
//!
//! `MThd` chunk: format (u16 BE at 8), track count (u16 BE at 10), division
//! (u16 BE at 12). RoboPlay's `mid.c` validates the same bytes. The title is not
//! part of the header; it is conventionally the first track's name meta-event
//! (`FF 03`) or a text meta-event (`FF 01`), which we extract here.

use super::{read_string, MusicInfo};

pub(super) fn parse(bytes: &[u8]) -> Option<MusicInfo> {
    // Header: "MThd" + length 6.
    if bytes.len() < 14 || &bytes[0..4] != b"MThd" || bytes[4..8] != [0, 0, 0, 6] {
        return None;
    }
    let format = u16::from_be_bytes([bytes[8], bytes[9]]);
    if format > 2 {
        return None;
    }
    let ntrks = u16::from_be_bytes([bytes[10], bytes[11]]);

    let title = first_track(bytes).and_then(|t| meta_text(t, 0x03).or_else(|| meta_text(t, 0x01)));

    let mut extra = vec![
        ("Format".to_string(), format!("SMF format {format}")),
        ("Tracks".to_string(), ntrks.to_string()),
    ];
    if let Some(t) = first_track(bytes).and_then(|t| meta_text(t, 0x02)) {
        extra.push(("Copyright".to_string(), t));
    }

    Some(MusicInfo {
        format: "Standard MIDI file",
        title,
        author: None,
        positions: None,
        subsongs: Some(0),
        channels: None,
        extra,
    })
}

/// The bytes of the first `MTrk` chunk's data (excluding the 8-byte chunk head).
fn first_track(bytes: &[u8]) -> Option<&[u8]> {
    let mut i = 14; // past the 14-byte MThd chunk
    while i + 8 <= bytes.len() {
        let len =
            u32::from_be_bytes([bytes[i + 4], bytes[i + 5], bytes[i + 6], bytes[i + 7]]) as usize;
        let data_start = i + 8;
        let data_end = data_start.checked_add(len)?;
        if &bytes[i..i + 4] == b"MTrk" {
            return bytes.get(data_start..data_end.min(bytes.len()));
        }
        i = data_end;
    }
    None
}

/// First text of meta-event `kind` (`FF <kind> <vlq-len> <text>`) in `track`.
fn meta_text(track: &[u8], kind: u8) -> Option<String> {
    let mut i = 0;
    while i + 2 < track.len() {
        if track[i] == 0xFF && track[i + 1] == kind {
            let (len, used) = read_vlq(&track[i + 2..])?;
            let start = i + 2 + used;
            return read_string(track, start, len);
        }
        i += 1;
    }
    None
}

/// Read a MIDI variable-length quantity; returns `(value, bytes_consumed)`.
fn read_vlq(bytes: &[u8]) -> Option<(usize, usize)> {
    let mut value = 0usize;
    for (n, &b) in bytes.iter().take(4).enumerate() {
        value = (value << 7) | usize::from(b & 0x7F);
        if b & 0x80 == 0 {
            return Some((value, n + 1));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mthd(format: u16, ntrks: u16) -> Vec<u8> {
        let mut h = b"MThd\x00\x00\x00\x06".to_vec();
        h.extend_from_slice(&format.to_be_bytes());
        h.extend_from_slice(&ntrks.to_be_bytes());
        h.extend_from_slice(&96u16.to_be_bytes()); // division
        h
    }

    fn mtrk(events: &[u8]) -> Vec<u8> {
        let mut t = b"MTrk".to_vec();
        t.extend_from_slice(&(events.len() as u32).to_be_bytes());
        t.extend_from_slice(events);
        t
    }

    #[test]
    fn parses_header_and_track_name() {
        // delta 0 (0x00), FF 03 len=5 "INTRO"
        let mut events = vec![0x00, 0xFF, 0x03, 0x05];
        events.extend_from_slice(b"INTRO");
        let mut buf = mthd(1, 2);
        buf.extend(mtrk(&events));

        let info = parse(&buf).unwrap();
        assert_eq!(info.title.as_deref(), Some("INTRO"));
        assert_eq!(info.format, "Standard MIDI file");
        assert!(info
            .extra
            .contains(&("Format".to_string(), "SMF format 1".to_string())));
        assert!(info
            .extra
            .contains(&("Tracks".to_string(), "2".to_string())));
    }

    #[test]
    fn rejects_non_midi() {
        assert!(parse(b"NOTMIDIHEADER!!").is_none());
    }
}
