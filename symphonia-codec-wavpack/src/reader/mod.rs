// Symphonia
// Copyright (c) 2019-2022 The Project Symphonia Developers.
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use symphonia_core::errors::unsupported_error;
use symphonia_core::support_format;

use symphonia_core::audio::Channels;
use symphonia_core::codecs::{CodecParameters, CODEC_TYPE_AAC};
use symphonia_core::errors::{decode_error, seek_error, Result, SeekErrorKind, end_of_stream_error, Error};
use symphonia_core::formats::prelude::*;
use symphonia_core::io::*;
use symphonia_core::meta::{Metadata, MetadataLog};
use symphonia_core::probe::{Descriptor, Instantiate, QueryDescriptor};

use symphonia_core::formats::prelude::*;
use symphonia_core::io::*;


use symphonia_core::codecs::{CodecType, CODEC_TYPE_WAVPACK_PCM_I_16, CODEC_TYPE_WAVPACK_DSD};

use log::{debug, error};

const STREAM_MARKER: [u8; 4] = *b"wvpk";

/// The maximum number of frames that will be in a packet.
/// Since there are no real packets in WavPack, this is arbitrary, used same value as MP3.
const MAX_FRAMES_PER_PACKET: u64 = 1152;

pub fn try_channel_count_to_mask(count: u16) -> Result<Channels> {
    (1..=32)
        .contains(&count)
        .then(|| Channels::from_bits(((1u64 << count) - 1) as u32))
        .flatten()
        .ok_or(Error::DecodeError("riff: invalid channel count"))
}

fn combine_values(u32_value: u32, u8_value: u8) -> u64 {
    let u32_as_u64 = (u32_value as u64) << 8;
    let combined_value = u32_as_u64 | (u8_value as u64);
    combined_value
}

enum Encoding {
    PCM,
    DSD,
}

struct Header
{
    block_size: u32,
    version: u16,
    // Number of samples in this block, 0 == non-audio block
    block_samples : u32,
    // First sample in the block relative to the start
    block_index: u64,
    // Total samples in file, note that a sample actually refers to a frame, 
    //   so (header.bits_per_sample / 8) * n_channels as u32 is amount of audio data
    total_samples : Option<u64>,
    // Blocks are either stereo or mono, > 2 channel make used of chained blocks (either stero or mono) and use an additional flag to signal their order in a sequence. 
    crc: u32,
    stereo: bool,
    bits_per_sample: u32,
    // 1: Hyrbrid mode, 0: Lossy
    hybrid_mode : bool,
    // 1: Joint stereo, 0: True stereo 
    joint_stereo: bool,
    // 1: Cross-channel decorrelation, 0: Indepedant channels
    cross_channel_decorrelation : bool,
    // 1: Hybrid noise shapring, 0: Flat noise spectrum in hybrid
    hybrid_noise_shaping : bool,
    // 1: Floating point data 0: Int data 
    floating_point_data : bool,
    // Use extended size ints (> 24bit mode) or shifted ints
    extended_size : bool, 
    // 1: Hybrid mode parameters control noise level (not used yet?), 0: Params control bitrate
    hybrid_mode_params_control_noise_level : bool,
    // For <= 2 channels this is always true, used for > 2 channel setups
    first_block_in_sequence : bool,
    // For <= 2 channels this is always true, used for > 2 channel setups
    last_block_in_sequence : bool,
    // Amount of data left-shift after decode (0-31 places)
    data_left_shift : u32,
    // Number of bits integers require -1
    max_magnitude : u32,
    // 0b1111 means custom or unknown
    sample_rate : u32,
    // Block contains checksum in last 2 or 4 bytes
    contains_checksum: bool,
    // Use IRR for negative hybrid noise shaping
    irr : bool, 
    // False stereo: Data is mono but output is stereo
    false_stereo: bool,
    encoding : Encoding
}

impl Header {
    const SIZE: usize = 4 * 8;
}

fn decode_header(source: &mut MediaSourceStream) -> Result<Header> {
    let marker = source.read_quad_bytes()?;

    if marker != STREAM_MARKER {
        return unsupported_error("wavpack: missing riff stream marker");
    }

    // Size of entire block - 8
    let ck_size = source.read_u32()?;
    let version = source.read_u16()?;
    
    if version != 0x402 && version != 0x410{
        return unsupported_error("wavpack: unsupported version");
    }

    // upper 8 bits of 40-bit block_index for first sample relative to the file start
    let block_index_u8 = source.read_u8()?;
    // upper 8 bits of 40-bit total_samples for entire file
    let total_samples_u8 = source.read_u8()?;

    // lower 32 bits of 40-bit total_samples, only valid if block_index == 0
    // value of -1 (erm this is unsigned though??) means unknown len
    let total_samples_u32 = source.read_u32()?;
    let block_index_u32 = source.read_u32()?;

    // The first block with audio determines format of entire file.
    let block_samples = source.read_u32()?;
    if block_samples == 0{
        debug!("wavpack: non-audioblock");
    }

    let block_index = combine_values(block_index_u32, block_index_u8);
    // TODO: value of -1 means total len, but dont know if that refers to total_samples or block_index
    let total_samples = match block_index {
        0 => Some(combine_values(total_samples_u32, total_samples_u8)),
        _=> None
    };

    if total_samples.is_some(){
        debug!("total samples {}", total_samples.unwrap());
    }

    let flags = source.read_u32()?;
    let crc = source.read_u32()?;

    let bitdepth = flags & 0b11;

    let bits_per_sample = match bitdepth {
        0 => 8,     // Should actuall be 1-8 bits / sample, but should not have a big impact on audio
        1 => 16,    // 9-16 bits / sample
        2 => 24,    // 15-24 bits / sample
        3 => 32,    // 25-32 bits / sample
        _=> return unsupported_error("wavpack: bitdepth")
    };

    let stereo = ((flags >> 2) & 0b1) == 0;
    if !stereo {
        debug!("mono");
    } else {
        debug!("stereo");
    }
    
    let hybrid_mode = ((flags >> 3) & 0b1) == 1;
    if !hybrid_mode{
        debug!("lossless mode");
    } else {
        debug!("hybrid mode mode");
    }

    let joint_stereo = ((flags >> 4) & 0b1) == 1;
    if !joint_stereo{
        debug!("true stereo");
    } else {
        debug!("joint stereo");
    }

    let cross_channel_decorrelation = ((flags >> 5) & 0b1) == 1;
    if !cross_channel_decorrelation{
        debug!("indepedant channels");
    } else {
        debug!("cross-channel decorrelation");
    }

    let hybrid_noise_shaping = ((flags >> 6) & 0b1) == 1;
    if !hybrid_noise_shaping {
        debug!("flat noise spectrum in hybrid");
    } else {
        debug!("hybrid noise shaping");
    }

    let floating_point_data = ((flags >> 7) & 0b1) == 1;
    if !floating_point_data {
        debug!("integer data");
    } else {
        debug!("floating point data");
    }

    let extended_size = ((flags >> 8) & 0b1) == 1;
    if extended_size {
        debug!("extended size ints (> 24bit mode) or shifted ints");
    }

    let hybrid_mode_params_control_noise_level = ((flags >> 9) & 0b1) == 0;
    if hybrid_mode_params_control_noise_level {
        debug!("hybrid mode parameters control noise level (not used yet)");
    } else {
        debug!("hyrbid mode parameters control bit rate");
    }

    let hybrid_noise_balanced = ((flags >> 10) & 0b1) == 1;
    if hybrid_noise_balanced {
        debug!("hybrid noise balanced between channels");
    }

    let first_block_in_sequence = ((flags >> 11) & 0b1) == 1;
    if first_block_in_sequence {
        debug!("first block in sequence (for multichannel)");
    }

    let last_block_in_sequence = ((flags >> 12) & 0b1) == 1;
    if last_block_in_sequence {
        debug!("last block in sequence (for multichannel)");
    }

    let shifted_number = flags >> 13;
    // amount of data left-shift after decode (0-31 places)
    let data_left_shift = shifted_number & 0b0001_1111;
    debug!("data left shift {}", data_left_shift);

    let shifted_number = flags >> 18;
    // (number of bits integers require -1)
    let max_magnitude = shifted_number & 0b0001_1111;
    debug!("maximum magnitude of decoded data {}", max_magnitude);

    let shifted_number = flags >> 23;
    let sample_rate = shifted_number & 0b0000_1111;
    if sample_rate == 0b1111{
        debug!("unknown/custom samplerate");
    } else {
        debug!("sampling rate {}", sample_rate);
    }

    let contains_checksum = ((flags >> 28) & 0b1) == 1;
    if contains_checksum {
        debug!("block contains checksum in last 2 or 4 bytes");
    }

    let irr = ((flags >> 29) & 0b1) == 1;
    if irr {
        debug!("use IRR for negative hybrid noise shaping");
    }

    let false_stereo = ((flags >> 30) & 0b1) == 1;
    if false_stereo {
        debug!("False stereo, data is mono but output is stereo");
    }
    
    let encoding = match (flags >> 31) & 0b1 {
        0 => Encoding::PCM,
        _=> Encoding::DSD,
    };

    return Ok(Header {
        block_size : ck_size + 8, 
        version,
        block_samples,
        block_index,
        total_samples,
        crc,
        stereo,
        bits_per_sample,
        hybrid_mode,
        joint_stereo,
        cross_channel_decorrelation,
        hybrid_noise_shaping,
        floating_point_data,
        extended_size,
        hybrid_mode_params_control_noise_level,
        first_block_in_sequence,
        last_block_in_sequence,
        data_left_shift,
        max_magnitude,
        sample_rate,
        contains_checksum,
        irr,
        false_stereo,
        encoding
    });

}

pub struct WavPackReader {
    reader: MediaSourceStream,
    tracks: Vec<Track>,
    cues: Vec<Cue>,
    metadata: MetadataLog,
    data_start_pos: u64,
}

impl QueryDescriptor for WavPackReader {
    fn query() -> &'static [Descriptor] {
        &[support_format!(
            "wv",
            "WavPack",
            &["wv"],
            &["audio/x-wavpack"],
            &[b"wvpk"]
        )]
    }

    fn score(_context: &[u8]) -> u8 {
        255
    }
}

impl FormatReader for WavPackReader {
    fn try_new(mut source: MediaSourceStream, _options: &FormatOptions) -> Result<Self> {
        let original_pos = source.pos();
        source.ensure_seekback_buffer(Header::SIZE);

        let header = decode_header(&mut source)?;
        source.seek_buffered_rev(Header::SIZE);
        
        
        let mut codec_params = CodecParameters::new();
        let mut metadata: MetadataLog = Default::default();

        let codec = match header.encoding {
            // TODO: support more PCM, floating point etc
            Encoding::PCM => CODEC_TYPE_WAVPACK_PCM_I_16,
            Encoding::DSD => CODEC_TYPE_WAVPACK_DSD,
        };
        
        let n_channels = match header.stereo {
            true => 2,
            false => 1,
        };

        // TODO: this is probably not right at all
        let channels = try_channel_count_to_mask(n_channels)?;

        //TODO: samplerate
        codec_params
            .for_codec(codec)
            .with_bits_per_coded_sample(header.bits_per_sample)
            .with_bits_per_sample(header.bits_per_sample)
            .with_channels(channels)
            .with_frames_per_block(header.block_samples as u64)
            .with_max_frames_per_packet(MAX_FRAMES_PER_PACKET * header.block_samples as u64)
            ;

        let data_start_pos =  original_pos + header.block_index;

        return Ok(WavPackReader {
            reader: source,
            tracks: vec![Track::new(0, codec_params)],
            cues: Vec::new(),
            metadata,
            data_start_pos,
        });
    }

    fn next_packet(&mut self) -> Result<Packet> {
        // TODO:Ok(Packet::new_from_boxed_slice(0, pts, dur, packet_buf))

        if self.tracks.is_empty() {
            return decode_error("wavpack: no tracks");
        }

        todo!("next_packet");
        
    }

    fn metadata(&mut self) -> Metadata<'_> {
        self.metadata.metadata()
    }

    fn cues(&self) -> &[Cue] {
        &self.cues
    }

    fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    fn seek(&mut self, _mode: SeekMode, to: SeekTo) -> Result<SeekedTo> {
        todo!("seek");
    }   
    
    fn into_inner(self: Box<Self>) -> MediaSourceStream {
        self.reader
    }
}