// Symphonia
// Copyright (c) 2019-2022 The Project Symphonia Developers.
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use symphonia_core::support_codec;
use symphonia_core::audio::{AsAudioBufferRef, AudioBuffer, AudioBufferRef, Signal, SignalSpec};
use symphonia_core::codecs::{Decoder, DecoderOptions, FinalizeResult, CodecDescriptor, CodecParameters, CodecType};
use symphonia_core::codecs::{CODEC_TYPE_WAVPACK_PCM_FLOAT, CODEC_TYPE_WAVPACK_PCM_I_8, CODEC_TYPE_WAVPACK_PCM_I_16, CODEC_TYPE_WAVPACK_PCM_I_24, CODEC_TYPE_WAVPACK_PCM_I_32, CODEC_TYPE_WAVPACK_DSD};
use symphonia_core::errors::{unsupported_error, Result};
use symphonia_core::formats::Packet;


pub struct WavPackDecoder {
    params: CodecParameters,
    // inner_decoder: InnerDecoder,
    buf: AudioBuffer<i32>,
}

impl Decoder for WavPackDecoder {
    fn try_new(params: &CodecParameters, _options: &DecoderOptions) -> Result<Self> {
        
        let frames = match params.max_frames_per_packet {
            Some(frames) => frames,
            _ => return unsupported_error("wavpack: maximum frames per packet is required"),
        };

        let rate = match params.sample_rate {
            Some(rate) => rate,
            _ => return unsupported_error("wavpack: sample rate is required"),
        };
        let spec = if let Some(channels) = params.channels {
            SignalSpec::new(rate, channels)
        }
        else if let Some(layout) = params.channel_layout {
            SignalSpec::new_with_layout(rate, layout)
        }
        else {
            return unsupported_error("wavpack: channels or channel_layout is required");
        };

        Ok(WavPackDecoder {
            params: params.clone(),
            buf: AudioBuffer::new(frames, spec),
        })
    }

    fn supported_codecs() -> &'static [CodecDescriptor] {
        &[
            support_codec!(CODEC_TYPE_WAVPACK_PCM_FLOAT, "wavpack_pcm_float", "WavPack PCM floats"),
            support_codec!(CODEC_TYPE_WAVPACK_PCM_I_8, "wavpack_pcm_i_8", "WavPack PCM integers 1-8 bits / sample"),
            support_codec!(CODEC_TYPE_WAVPACK_PCM_I_16, "wavpack_pcm_i_16", "WavPack PCM integers 9-16 bits / sample"),
            support_codec!(CODEC_TYPE_WAVPACK_PCM_I_24, "wavpack_pcm_i_24", "WavPack PCM integers 25-32 bits / sample / sample"),
            support_codec!(CODEC_TYPE_WAVPACK_PCM_I_32, "wavpack_pcm_i_32", "WavPack PCM integers 15-24 bits / sample"),
            support_codec!(CODEC_TYPE_WAVPACK_DSD, "adpcm_ima_wav", "ADPCM IMA WAV"),
        ]
    }

    fn reset(&mut self) {
        // No state is stored between packets, therefore do nothing.
    }

    fn codec_params(&self) -> &CodecParameters {
        &self.params
    }

    fn decode(&mut self, packet: &Packet) -> Result<AudioBufferRef<'_>> {
        todo!("decode");
        // if let Err(e) = self.decode_inner(packet) {
        //     self.buf.clear();
        //     Err(e)
        // }
        // else {
        //     Ok(self.buf.as_audio_buffer_ref())
        // }
    }

    fn finalize(&mut self) -> FinalizeResult {
        Default::default()
    }

    fn last_decoded(&self) -> AudioBufferRef<'_> {
        self.buf.as_audio_buffer_ref()
    }
}