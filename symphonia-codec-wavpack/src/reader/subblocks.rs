// Symphonia
// Copyright (c) 2019-2024 The Project Symphonia Developers.
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use log::debug;

use symphonia_core::errors::{decode_error, Result};
use symphonia_core::io::*;

const CHANNEL_INFO_ID: u8 = 0x0d;
const CHANNEL_IDENTITIES_ID: u8 = 0x2b;
const SUB_BLOCK_ID_MASK: u8 = 0x3f;
const SUB_BLOCK_LARGE: u8 = 0x80;
const SUB_BLOCK_ODD_SIZE: u8 = 0x40;

pub struct ChannelInfo {
    pub channel_count: u32,
    pub channel_mask: u32,
}

pub struct ChannelIdentities {
    pub identities: Vec<u8>,
}

#[allow(dead_code)]
pub enum SubBlock {
    Unknown(Vec<u8>),
    // Padding
    Dummy(Vec<u8>),
    // Decorrelation terms & deltas (fixed)
    DecorrelationTerms(Vec<u8>),
    // Initial decorrelation weights
    DecorrelationWeights(Vec<u8>),
    // Decorrelation sample history
    DecorrelationSamples(Vec<u8>),
    // Initial entropy variables
    EntropyVariables(Vec<u8>),
    // Entropy variables specific to hybrid mode
    HybridProfile(Vec<u8>),
    // Info needed for hybrid lossless (wvc) mode
    ShapingWeights(Vec<u8>),
    // Specific info for floating point decode
    FloatInfo(Vec<u8>),
    // Specific info for decoding integers > 24  bits, or data requiring shift after decode
    Int32Info(Vec<u8>),
    // Normal compressed audio bitstream (wv file)
    WvBitStream(Vec<u8>),
    // Correction file bitstream (wvc file)
    WvcBitStream(Vec<u8>),
    // special extended bitstream for floating point data or integers > 24 bit
    //(can be in either wv or wvc file, depending.....)
    WvxBitStream(Vec<u8>),
    // Channel count and WAVE speaker mask for a track with more than two channels.
    ChannelInfo(ChannelInfo),
    // contains compressed DSD audio (ver 5.0+)
    DsdBlock(Vec<u8>),
    // RIFF header for .wav files (before audio)
    RiffHeader(Vec<u8>),
    // RIFF trailer for .wav files (after audio)
    RiffTrailer(Vec<u8>),
    // some encoding details for info purposes
    ConfigChecksum(Vec<u8>),
    // 16-byte MD5 sum of raw audio data
    Md5Checksum(Vec<u8>),
    // non-standard sampling rate info
    SampleRate(Vec<u8>),
    // header for non-wav files (ver 5.0+)
    AltHeader(Vec<u8>),
    // trailer for non-wav files (ver 5.0+)
    AltTrailer(Vec<u8>),
    // target filename extension
    AltExtension(Vec<u8>),
    // 16-byte MD5 sum of raw audio data with non-wav standard (e.g., big-endian)
    AltMd5Checksum(Vec<u8>),
    // new file configuration stuff including file type, non-wav formats (e.g., big endian), and CAF channel layouts and reordering
    NewConfigBlock(Vec<u8>),
    // identities of non-MS channels
    ChannelIdentities(ChannelIdentities),
    // 2- or 4-byte checksum of entire block
    BlockChecksum(Vec<u8>),
}

pub fn decode(source: &mut impl ReadBytes) -> Result<SubBlock> {
    let id = source.read_u8()?;
    if id & 0x3f == 0x3f {
        debug!("unique metadata function ID");
    }

    let size_in_words;
    if id & SUB_BLOCK_LARGE == SUB_BLOCK_LARGE {
        // debug!("large block, > 255 words");
        let b = source.read_triple_bytes()?;
        size_in_words = (b[0] as u32)
            | ((b[1] as u32) << 8)
            | ((b[2] as u32) << 16);
    } else {
        size_in_words = source.read_byte()? as u32;
    }
    let datasize = size_in_words * 2;

    if datasize % 2 != 0 {
        debug!("wavpack: Weird data size");
    }

    // debug!("subblock data size {}", datasize);
    let mut data: Vec<u8> = vec![0; datasize as usize];
    source.read_buf_exact(&mut data)?;

    // Check if last byte is padding.
    if id & SUB_BLOCK_ODD_SIZE == SUB_BLOCK_ODD_SIZE {
        data.pop();
    }

    // todo: use consts for sub block types
    let sub_block = match id & SUB_BLOCK_ID_MASK {
        0x0 => {
            SubBlock::Dummy(data)
        },
        0x2 => {
            SubBlock::DecorrelationTerms(data)
        },
        0x3 => {
            SubBlock::DecorrelationWeights(data)
        },
        0x4 => {
            SubBlock::DecorrelationSamples(data)
        },
        0x5 => {
            SubBlock::EntropyVariables(data)
        },
        0x6 => {
            SubBlock::HybridProfile(data)
        },
        0x7 => {
            SubBlock::ShapingWeights(data)
        },
        0x8 => {
            SubBlock::FloatInfo(data)
        },
        0x9 => {
            SubBlock::Int32Info(data)
        },
        0xA => {
            SubBlock::WvBitStream(data)
        },
        0xB => {
            SubBlock::WvcBitStream(data)
        },
        0xC => {
            SubBlock::WvxBitStream(data)
        },
        CHANNEL_INFO_ID => {
            SubBlock::ChannelInfo(parse_channel_info(&data)?)
        },
        0xE => {
            SubBlock::DsdBlock(data)
        },
        0x21 => {
            SubBlock::RiffHeader(data)
        },
        0x22 => {
            SubBlock::RiffTrailer(data)
        },
        0x25 => {
            SubBlock::ConfigChecksum(data)
        },
        0x26 => {
            SubBlock::Md5Checksum(data)
        },
        0x27 => {
            SubBlock::SampleRate(data)
        },
        0x23 => {
            SubBlock::AltHeader(data)
        },
        0x24 => {
            SubBlock::AltTrailer(data)
        },
        0x28 => {
            SubBlock::AltExtension(data)
        },
        0x29 => {
            SubBlock::AltMd5Checksum(data)
        },
        0x2A => {
            SubBlock::NewConfigBlock(data)
        },
        CHANNEL_IDENTITIES_ID => {
            SubBlock::ChannelIdentities(parse_channel_identities(&data)?)
        },
        0x2F => {
            SubBlock::BlockChecksum(data)
        },
        _ => {
            debug!("WavPack: Unknown subblock id: {}", id);
            SubBlock::Unknown(data)
        },
    };
    return Ok(sub_block);
}

fn parse_channel_identities(payload: &[u8]) -> Result<ChannelIdentities> {
    // Identities are numbered from 1, so 0 is rejected. 255 means undefined.
    if payload.is_empty() || payload.contains(&0) {
        return decode_error("wavpack: invalid channel identities");
    }
    Ok(ChannelIdentities { identities: payload.to_vec() })
}

fn parse_channel_info(payload: &[u8]) -> Result<ChannelInfo> {
    // 6 or 7 bytes: 12-bit channel count, 12-bit stream count, then a 3- or 4-byte mask.
    // 1 to 5 bytes: the first byte is the channel count and the remaining bytes are the mask.

    // This byte layout is not described outside the reference decoder, hence we use that as reference
    // TODO: validate
    let (channel_count, channel_mask) = if payload.len() == 6 || payload.len() == 7 {
        let channel_count = (u32::from(payload[0]) | (u32::from(payload[2] & 0x0f) << 8)) + 1;
        // TODO: check stream count

        let mut channel_mask = u32::from(payload[3]) | (u32::from(payload[4]) << 8) | (u32::from(payload[5]) << 16);
        if payload.len() == 7 {
            channel_mask |= u32::from(payload[6]) << 24;
        }
        (channel_count, channel_mask)
    } else if (1..=5).contains(&payload.len()) {
        let channel_count = u32::from(payload[0]);
        let mut channel_mask = 0u32;
        let mut shift = 0;
        for byte in &payload[1..] {
            channel_mask |= u32::from(*byte) << shift;
            shift += 8;
        }
        (channel_count, channel_mask)
    } else {
        return decode_error("wavpack: unexpected len for channel info, expected 1-5 or 6-7 bytes");
    };

    if channel_count == 0 {
        return decode_error("wavpack: invalid channel info, expected channel count >= 1");
    }
    if channel_mask.count_ones() > channel_count {
        return decode_error("wavpack: invalid channel info, channel mask has more set bits than channels",);
    }

    Ok(ChannelInfo { channel_count, channel_mask })
}
