use std::env;
use std::fs::File;
use std::process;

use symphonia_codec_wavpack::WavPackReader;
use symphonia_core::codecs::CodecParameters;
use symphonia_core::formats::{FormatOptions, FormatReader};
use symphonia_core::io::MediaSourceStream;

fn main() {
    let Some(path) = env::args().nth(1) else {
        eprintln!("usage: dump-wavpack-packets <file.wv>");
        process::exit(1);
    };

    let file = match File::open(&path) {
        Ok(file) => file,
        Err(err) => {
            eprintln!("open {path}: {err}");
            process::exit(1);
        }
    };

    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut reader = match WavPackReader::try_new(mss, FormatOptions::default()) {
        Ok(reader) => reader,
        Err(err) => {
            eprintln!("try_new: {err}");
            process::exit(1);
        }
    };

    let info = reader.format_info();
    println!("format: {} ({})", info.long_name, info.short_name);

    for track in reader.tracks() {
        print!("track {}: ", track.id);
        match track.codec_params.as_ref() {
            Some(CodecParameters::Audio(params)) => {
                println!(
                    "codec={:?} rate={:?} format={:?} bits={:?} channels={:?}",
                    params.codec,
                    params.sample_rate,
                    params.sample_format,
                    params.bits_per_sample,
                    params.channels.as_ref().map(|ch| ch.count()),
                );
            }
            other => println!("{other:?}"),
        }
    }

    let mut n = 0;
    loop {
        match reader.next_packet() {
            Ok(Some(packet)) => {
                n += 1;
                println!(
                    "packet {n}: track={} pts={} dur={} bytes={}",
                    packet.track_id(),
                    packet.pts().get(),
                    packet.dur().get(),
                    packet.buf().len(),
                );
            }
            Ok(None) => break,
            Err(err) => {
                eprintln!("next_packet: {err}");
                process::exit(1);
            }
        }
    }

    println!("packets: {n}");
}
