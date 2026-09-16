// Symphonia
// Copyright (c) 2019-2024 The Project Symphonia Developers.
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::io::Seek;

use symphonia_core::codecs::audio::well_known::CODEC_ID_WAVPACK;
use symphonia_core::support_format;

use symphonia_core::errors::{decode_error, unsupported_error, Result};
use symphonia_core::formats::prelude::*;
use symphonia_core::io::*;
use symphonia_core::formats::probe::{ProbeFormatData, ProbeableFormat, Score, Scoreable};
use symphonia_core::formats::well_known::FORMAT_ID_WAVPACK;
use symphonia_core::formats::FormatReader;
use symphonia_core::meta::{Metadata, MetadataLog};
use symphonia_core::audio::{Channels, layouts};
use symphonia_core::codecs::CodecParameters;
use symphonia_core::codecs::audio::AudioCodecParameters;
use symphonia_core::audio::sample::SampleFormat;

use log::{trace, debug};

mod subblocks;
use subblocks::SubBlock;

const MAX_FRAMES_PER_PACKET: u64 = 1152;

const MAX_BLOCK_SIZE:u64 = 1000;

const WAVPACK_FORMAT_INFO: FormatInfo = FormatInfo {
    format: FORMAT_ID_WAVPACK,
    short_name: "wavpack",
    long_name: "Wavpack",
};

const STREAM_MARKER: [u8; 4] = *b"wvpk";

const SAMPLE_RATES: [u32; 15] = [
    6000, 8000, 9600, 11025, 12000, 16000, 22050, 24000, 32000, 44100,
    48000, 64000, 88200, 96000, 192000
];

macro_rules! combine_values {
    ($u32_value:expr, $u8_value:expr) => {
        (($u8_value as u64) << 32) | ($u32_value as u64)
    };
}

pub struct WavPackReader<'a> {
    reader: MediaSourceStream<'a>,
    tracks: Vec<Track>,
    metadata: MetadataLog,
    chapters: Option<ChapterGroup>,
    next_packet_ts: u64,
}

impl<'s> WavPackReader<'s> {
    pub fn try_new(mut mss: MediaSourceStream<'s>, opts: FormatOptions) -> Result<Self> {
        let original_pos = mss.pos();
        _ = find_next_block(&mut mss, 100);
        let header = Header::decode(&mut mss)?;
        if header.get_block_index() != 0 {
            debug!("First block is not first block after all. Total samples unknown");
        }
        // TODO: more extensive check
        let channel_layout = match header.is_stereo() {
            true => layouts::CHANNEL_LAYOUT_STEREO,
            false => layouts::CHANNEL_LAYOUT_MONO,
        };

        let mut codec_params = AudioCodecParameters::new();
        codec_params
            .for_codec(CODEC_ID_WAVPACK)
            .with_bits_per_coded_sample(header.get_bytes_per_sample() * 8)
            .with_bits_per_sample(header.get_bytes_per_sample() * 8)
            .with_channels(channel_layout)
        ;

        let sample_format = match header.get_encoding(){
            Encoding::PCM => match header.get_bytes_per_sample(){
                1 => SampleFormat::S8,
                2 => SampleFormat::S16,
                3 => SampleFormat::S24,
                4 => SampleFormat::S32,
                //todo: float
                _ => return decode_error("WavPack: Invalid sample format")
            },
            Encoding::DSD => return unsupported_error("WavPack: DSD unsupported"),
        };

        codec_params.with_sample_format(sample_format);

        let sample_rate = header.get_sample_rate();
        if let Some(sample_rate) = sample_rate {
            codec_params.with_sample_rate(sample_rate);
        }
        
        let metadata: MetadataLog = Default::default();

        let mut track = Track::new(0);
        track.with_codec_params(CodecParameters::Audio(codec_params));
        
        mss.seek(std::io::SeekFrom::Start(original_pos))?;
        
        return Ok(WavPackReader {
            reader: mss,
            tracks: vec![track],
            metadata,
            chapters: None,
            next_packet_ts: 0,
        });

    }
}

impl ProbeableFormat<'_> for WavPackReader<'_> {
    fn try_probe_new(
        mss: MediaSourceStream<'_>,
        opts: FormatOptions,
    ) -> Result<Box<dyn FormatReader + '_>> {
        Ok(Box::new(WavPackReader::try_new(mss, opts)?))
    }

    fn probe_data() -> &'static [ProbeFormatData] {
        &[
            support_format!(
                WAVPACK_FORMAT_INFO,
                &["wv"],
                &["audio/x-wavpack"],
                &[b"wvpk"]
            ),
        ]
    }
}

impl Scoreable for WavPackReader<'_> {
    fn score(_src: ScopedStream<&mut MediaSourceStream<'_>>) -> Result<Score> {
        Ok(Score::Supported(255))
    }
}

impl FormatReader for WavPackReader<'_> {
    fn format_info(&self) -> &FormatInfo {
        &WAVPACK_FORMAT_INFO
    }

    fn next_packet(&mut self) -> Result<Option<Packet>> {
        if self.tracks.is_empty() {
            return decode_error("wavpack: no tracks");
        }

        
        _ = find_next_block(&mut self.reader, 10000);
        let header = Header::decode(&mut self.reader)?;
        loop {
            let sub_block = subblocks::decode(&mut self.reader)?;
            match sub_block {
                // Final sub-block is usually audio, although this is not a strict rule.
                // Version 5.0 a checksum of 4-6 bytes was added.
                SubBlock::WvBitStream(data) => {
                    // block_samples is the per-channel frame count
                    let n = u64::from(header.block_samples);
                    let ts = self.next_packet_ts;
                    self.next_packet_ts += n;

                    return Ok(Some(Packet::new_from_boxed_slice(
                        0,
                        ts,
                        n,
                        data.into_boxed_slice(),
                    )));
                },
                SubBlock::WvcBitStream(data) => {
                    debug!("wvc stream");
                },
                SubBlock::WvxBitStream(data) => {
                    debug!("wvx stream");
                },
                SubBlock::DsdBlock(data) =>{
                    todo!("DSD audio");
                }
                _ => {
                    debug!("some non-audio block");
                }
            }
        }
    }

    fn metadata(&mut self) -> Metadata<'_> {
        self.metadata.metadata()
    }

    fn chapters(&self) -> Option<&ChapterGroup> {
        self.chapters.as_ref()
    }

    fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    fn seek(&mut self, _mode: SeekMode, to: SeekTo) -> Result<SeekedTo> {
        todo!("seek");
    }
    
    fn into_inner<'s>(self: Box<Self>) -> MediaSourceStream<'s>
    where
        Self: 's,
    {
        self.reader
    }

}

enum Encoding {
    PCM,
    DSD,
}

struct Header {
    ck_size: u32,
    version: u16,
    block_index_u8: u8,
    total_samples_u8: u8,
    total_samples_u32:u32,
    block_index_u32:u32,
    block_samples: u32,
    flags: u32,
    crc: u32,
}

impl Header {
    fn decode(reader: &mut MediaSourceStream<'_>) -> Result<Header>{
        let marker = reader.read_quad_bytes()?;
        if marker != STREAM_MARKER{
            return unsupported_error("wavpack: missing marker");
        }
        // Entire block size = ck_size + 8
        let ck_size = reader.read_u32()?;
        let version = reader.read_u16()?;
        let block_index_u8 = reader.read_u8()?;
        let total_samples_u8 = reader.read_u8()?;
        let total_samples_u32 = reader.read_u32()?;
        let block_index_u32 = reader.read_u32()?;
        let block_samples = reader.read_u32()?;
        let flags = reader.read_u32()?;
        let crc = reader.read_u32()?;
        return Ok(Header {
            ck_size,
            version,
            block_index_u8,
            total_samples_u8,
            total_samples_u32,
            block_index_u32,
            block_samples,
            flags,
            crc,
        });

    }
}

impl Header {
    const SIZE: usize = 4 * 8;

    // Index of the first sample in the block relative to file start
    fn get_block_index(&self) -> u64 {
        combine_values!(self.block_index_u32, self.block_index_u8)
    }

    fn get_total_samples(&self)->Option<u64>{
        if self.total_samples_u32 == 0xFFFFFFFF{
            // Indicates unknown
            return None;
        }
        return Some(combine_values!(self.total_samples_u32, self.total_samples_u8));
    }

    fn get_bytes_per_sample(&self)->u32{
        // TODO: use constants for flag values
        return (self.flags & 3) + 1;
    }

    fn get_encoding(&self) -> Encoding {
        match (self.flags >> 31) & 0b1 {
            0 => Encoding::PCM,
            _=> Encoding::DSD,
        }
    }

    // TODO: better way to get n channels
    fn get_n_channels(&self) -> u32{
        if self.is_stereo() {
            2
        }else {
            1
        }
    }

    fn get_n_samples(&self) -> u32{
        // In wavpack a "sample" is what we usually refer to as frame
        self.block_samples / self.get_n_channels()
    }

    fn is_stereo(&self) -> bool {
        ((self.flags >> 2) & 0b1) == 0
    }

    fn get_sample_rate(&self) -> Option<u32> {
        let shifted_number = self.flags >> 23;
        let sample_rate = shifted_number & 0b0000_1111;
        if sample_rate == 0b1111{
            debug!("wavpack: unknown/custom samplerate");
            return None;
        } 

        if sample_rate < SAMPLE_RATES.len() as u32 {
            Some(SAMPLE_RATES[sample_rate as usize])
        } else {
            debug!("wavpack: invalid samplerate index {}", sample_rate);
            None
        }
    }
}

fn find_next_block(source: &mut MediaSourceStream<'_>, max_bytes: usize) -> Result<u64>{
    let mut bytes_read = 0;
    source.ensure_seekback_buffer(max_bytes);
    loop {
        if bytes_read + 4 >= max_bytes {
            return decode_error("no block found")
        }
        let b = source.read_u8()?;
        bytes_read += 1;
        
        if b == b'w' {
            let b = source.read_triple_bytes()?;
            bytes_read += 3;
    
            if b == *b"vpk" {
                source.seek_buffered_rev(4);
                return Ok(bytes_read as u64);
            }
        }
    };
}