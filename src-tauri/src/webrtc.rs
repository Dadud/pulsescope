//! WebRTC Opus media contract helpers.
//!
//! ICE/DTLS transport is not implemented in this build, so media sessions stay
//! HTTP 501 with PCM WebSocket as the working fallback. These helpers lock the
//! SDP/RTP numbers the Opus path must use once libopus and ICE exist: payload
//! type 111, 20 ms frames, 48 kHz timestamps advancing by 960.

use serde::{Deserialize, Serialize};

pub const OPUS_PAYLOAD_TYPE: u8 = 111;
pub const OPUS_CLOCK_RATE_HZ: u32 = 48_000;
pub const OPUS_FRAME_MS: u32 = 20;
pub const OPUS_TIMESTAMP_STEP: u32 = 960; // 48000 * 0.020
pub const OPUS_CHANNELS: u8 = 2;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct OpusRtpHeader {
    pub payload_type: u8,
    pub sequence: u16,
    pub timestamp: u32,
    pub ssrc: u32,
    pub marker: bool,
}

pub fn sdp_offer_fragment() -> String {
    format!(
        "m=audio 9 UDP/TLS/RTP/SAVPF {OPUS_PAYLOAD_TYPE}\r\n\
         a=rtpmap:{OPUS_PAYLOAD_TYPE} opus/{OPUS_CLOCK_RATE_HZ}/{OPUS_CHANNELS}\r\n\
         a=fmtp:{OPUS_PAYLOAD_TYPE} minptime={OPUS_FRAME_MS};useinbandfec=1;usedtx=0\r\n\
         a=ptime:{OPUS_FRAME_MS}\r\n\
         a=maxptime:{OPUS_FRAME_MS}\r\n\
         a=rtcp-mux\r\n\
         a=rtcp-rsize\r\n"
    )
}

pub fn next_header(previous: Option<&OpusRtpHeader>, ssrc: u32, marker: bool) -> OpusRtpHeader {
    match previous {
        Some(prev) => OpusRtpHeader {
            payload_type: OPUS_PAYLOAD_TYPE,
            sequence: prev.sequence.wrapping_add(1),
            timestamp: prev.timestamp.wrapping_add(OPUS_TIMESTAMP_STEP),
            ssrc,
            marker,
        },
        None => OpusRtpHeader {
            payload_type: OPUS_PAYLOAD_TYPE,
            sequence: 0,
            timestamp: 0,
            ssrc,
            marker,
        },
    }
}

pub fn encode_rtp_header(header: &OpusRtpHeader) -> [u8; 12] {
    let mut out = [0u8; 12];
    out[0] = 0x80;
    out[1] = header.payload_type & 0x7F;
    if header.marker {
        out[1] |= 0x80;
    }
    out[2..4].copy_from_slice(&header.sequence.to_be_bytes());
    out[4..8].copy_from_slice(&header.timestamp.to_be_bytes());
    out[8..12].copy_from_slice(&header.ssrc.to_be_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sdp_advertises_opus_pt_111_20ms() {
        let sdp = sdp_offer_fragment();
        assert!(sdp.contains("opus/48000/2"));
        assert!(sdp.contains("a=rtpmap:111"));
        assert!(sdp.contains("a=ptime:20"));
        assert!(sdp.contains("usedtx=0"));
        assert!(sdp.contains("useinbandfec=1"));
    }

    #[test]
    fn rtp_timestamps_advance_960() {
        let first = next_header(None, 0xAABB_CCDD, true);
        let second = next_header(Some(&first), 0xAABB_CCDD, false);
        assert_eq!(first.payload_type, OPUS_PAYLOAD_TYPE);
        assert_eq!(second.sequence, 1);
        assert_eq!(second.timestamp, OPUS_TIMESTAMP_STEP);
        let wire = encode_rtp_header(&second);
        assert_eq!(wire[1] & 0x7F, OPUS_PAYLOAD_TYPE);
        assert_eq!(&wire[4..8], &OPUS_TIMESTAMP_STEP.to_be_bytes());
    }
}
