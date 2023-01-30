
use symphonia_core::probe::{Descriptor, Instantiate, QueryDescriptor};
use symphonia_core::support_format;

use symphonia_core::audio::Channels;
use symphonia_core::codecs::CodecParameters;
use symphonia_core::codecs::{
    CODEC_TYPE_PCM_S16BE
};
use symphonia_core::errors::{decode_error, end_of_stream_error, unsupported_error};
use symphonia_core::errors::{Result};
use symphonia_core::formats::prelude::*;
use symphonia_core::io::*;
use symphonia_core::meta::{Metadata, MetadataLog};

use extended::Extended;

const AIFF_STREAM_MARKER: [u8; 4] = *b"FORM";
const AIFF_FORM_TYPE: [u8; 4] = *b"AIFF";
const COMPRESSED_FORM_TYPE: [u8; 4] = *b"AIFC";
const COM_CHUNK_ID: [u8; 4] = *b"COMM";
const SSND_CHUNK_ID: [u8; 4] = *b"SSND";

/// The maximum number of frames that will be in a packet.
/// TODO: i took this from symphonia-format-wav/src/lib.rs but i don't know if it's correct
const AIFF_MAX_FRAMES_PER_PACKET: u64 = 1152;

// Wrapper adapter for packetization, we use 1 block == 1 frame ( == 1 sample for each channel)
pub(crate) struct PacketInfo {
    block_size: u64,
    frames_per_block: u64,
    max_blocks_per_packet: u64,
}

impl PacketInfo {
    fn get_frames(&self, data_len: u64) -> u64 {
        data_len / self.block_size * self.frames_per_block
    }
}

pub struct AiffReader{
    reader: MediaSourceStream,
    tracks: Vec<Track>,
    cues: Vec<Cue>,
    packet_info: PacketInfo,
    metadata: MetadataLog,
    data_start_pos: u64,
    data_end_pos: u64,
}

impl QueryDescriptor for AiffReader {
    fn query() -> &'static [Descriptor] {
        &[
            // WAVE RIFF form
            support_format!(
                "aiff",
                "aiff",
                &["aif", "aiff"],
                &[],
                &[b"FORM"]
            ),
        ]
    }

    fn score(_context: &[u8]) -> u8 {
        255
    }
}

#[derive(Debug)]
struct CommonChunk{
    num_channels: i16,
    num_sample_frames: u32,
    sample_size: i16,
    sample_rate: u32,
}

#[derive(Debug)]
struct SoundChunk{
    offset: u32,
    block_size: u32,
}

#[derive(Debug)]
struct UnknownChunk{
    id: [u8; 4],
    data_size: u32,
}

impl FormatReader for AiffReader {
    fn try_new(mut source: MediaSourceStream, _options: &FormatOptions) -> Result<Self> {
        // TODO: support for loop points
        let marker = source.read_quad_bytes()?;
        
        if marker != AIFF_STREAM_MARKER {
            return unsupported_error("aiff: missing form stream marker");
        }

        //filesize - 4 for FORM - 4 for size (this value)
        let size = source.read_quad_bytes()?;
        let form_data_size = u32::from_be_bytes(size);
    
        // - 4 for FORM - u32 for size of data in formChunk
        let file_size = form_data_size as u64 + 8;
    
        let form_type = source.read_quad_bytes()?;
        match form_type {
            AIFF_FORM_TYPE => {},
            COMPRESSED_FORM_TYPE => {
                return unsupported_error("aiff: compressed audio not supported");
            },
            _ => {
                return unsupported_error("aiff: unsupported form type");
            }
        };
    
        // Next data are the local chunks, only common and sound chunks are required
        let mut common_chunk = CommonChunk {
            num_channels: 0,
            num_sample_frames: 0,
            sample_size: 0,
            sample_rate: 0,
        };

        let mut sound_chunk = SoundChunk {
            offset: 0,
            block_size: 0,
        };

        // Keep track of other local chunks
        let mut unknown_chunks = Vec::new();
        
        loop {
            if source.pos() >= file_size {
                panic!("aiff: No SSND chunk was found");
            }
    
            let id = source.read_quad_bytes()?;
            match id {
                COM_CHUNK_ID => {
                    let data_size = source.read_quad_bytes()?;
                    let _data_size = u32::from_be_bytes(data_size);
                    // TODO: warn if data_size != 18
    
                    let num_channels = source.read_double_bytes()?;
                    let num_channels = i16::from_be_bytes(num_channels);
                    
                    let num_sample_frames = source.read_quad_bytes()?;
                    let num_sample_frames = u32::from_be_bytes(num_sample_frames);
    
                    let sample_size = source.read_double_bytes()?;
                    let sample_size = i16::from_be_bytes(sample_size);
    
                    let mut sample_rate: [u8; 10] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
                    let _res = source.read_buf(sample_rate.as_mut());
                    let sample_rate =  Extended::from_be_bytes(sample_rate);
                    
                    common_chunk = CommonChunk{
                        num_channels,
                        num_sample_frames,
                        sample_size,
                        sample_rate: sample_rate.to_f64() as u32
                    };
                },
                SSND_CHUNK_ID =>{
                    let _data_size = source.read_quad_bytes()?;
                    //let _data_size = u32::from_be_bytes(data_size);
    
                    let offset = source.read_quad_bytes()?;
                    let offset = u32::from_be_bytes(offset);
                    
                    let block_size = source.read_quad_bytes()?;
                    let block_size = u32::from_be_bytes(block_size);
    
                    if offset != 0 || block_size != 0{
                        // Usage of this feature seems rather rare, don't support for now
                        return unsupported_error("aiff: does not support block aligning");
                    }
                    sound_chunk = SoundChunk{
                        offset,
                        block_size,
                    };

                    // Sound chunk should be last so, end
                    break;
                },
                _ => {
                    //TODO: test
                    let data_size = source.read_quad_bytes()?;
                    let data_size = u32::from_be_bytes(data_size);
                    //TODO: store raw data
                    let _res = source.ignore_bytes(data_size as u64);
                    
                    let chunk = UnknownChunk {
                        id,
                        data_size,
                    };
                    unknown_chunks.push(chunk);
                }
            }
        }

        let channels = match common_chunk.num_channels {
            1 => Channels::FRONT_LEFT,
            2 => Channels::FRONT_LEFT | Channels::FRONT_RIGHT,
            _ => {
                return unsupported_error("aiff: unsupported number of channels");
            }
        };

        let codec = match common_chunk.sample_size {
            16 => CODEC_TYPE_PCM_S16BE,
            _ => {
                // TODO: support other samples sizes divible by 8
                // TODO: if not divisible by 8, support for padding bytes
                return decode_error(
                    "aiff: bits per sample for fmt_pcm must be 8, 16, 24 or 32 bits",
                )
            }
        };
        
        let packet_info = PacketInfo{
            block_size: (common_chunk.num_channels * common_chunk.sample_size) as u64 / 8, //TODO: check if this is correct   
            frames_per_block: 1,
            max_blocks_per_packet: AIFF_MAX_FRAMES_PER_PACKET,
        };

        let max_frames_per_packet = packet_info.max_blocks_per_packet * packet_info.frames_per_block;
        let mut codec_params = CodecParameters::new();
        codec_params
            .for_codec(codec)
            .with_packet_data_integrity(true)
            .with_sample_rate(common_chunk.sample_rate)
            .with_bits_per_sample(common_chunk.sample_size as u32)
            .with_channels(channels)
            .with_sample_rate(common_chunk.sample_rate)
            .with_time_base(TimeBase::new(1, common_chunk.sample_rate))
            .with_n_frames(u64::from(common_chunk.num_sample_frames))
            .with_max_frames_per_packet(max_frames_per_packet)
            .with_frames_per_block(packet_info.frames_per_block);

        // TODO: fill metadata
        let metadata: MetadataLog = Default::default();
        
        let data_start_pos = source.pos();

        return Ok(AiffReader {
            reader: source,
            tracks: vec![Track::new(0, codec_params)],
            cues: Vec::new(),
            metadata,
            packet_info,
            data_start_pos,
            data_end_pos: file_size - 1,
        });
    }

    fn next_packet(&mut self) -> Result<Packet> {
        // TODO: Same as symphonia-format-wav, should probably share code
        let pos = self.reader.pos();
        if self.tracks.is_empty() {
            return decode_error("aiff: no tracks");
        }
        if self.packet_info.block_size == 0 {
            return decode_error("aiff: block size is 0");
        }

        // Determine the number of complete blocks remaining in the data chunk.
        let num_blocks_left = if pos < self.data_end_pos {
            (self.data_end_pos - pos) / self.packet_info.block_size
        }
        else {
            0
        };

        if num_blocks_left == 0 {
            return end_of_stream_error();
        }

        let blocks_per_packet = num_blocks_left.min(self.packet_info.max_blocks_per_packet);

        let dur = blocks_per_packet * self.packet_info.frames_per_block;
        let packet_len = blocks_per_packet * self.packet_info.block_size;

        // Copy the frames.
        let packet_buf = self.reader.read_boxed_slice(packet_len as usize)?;

        // The packet timestamp is the position of the first byte of the first frame in the
        // packet relative to the start of the data chunk divided by the length per frame.
        let pts = self.packet_info.get_frames(pos - self.data_start_pos);

        Ok(Packet::new_from_boxed_slice(0, pts, dur, packet_buf))
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

    fn seek(&mut self, _mode: SeekMode, _to: SeekTo) -> Result<SeekedTo> {
        todo!("aiff seek");
    }

    fn into_inner(self: Box<Self>) -> MediaSourceStream {
        self.reader
    }
}