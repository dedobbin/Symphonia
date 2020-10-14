// Symphonia
// Copyright (c) 2020 The Project Symphonia Developers.
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use symphonia_core::errors::{Result, decode_error};
use symphonia_core::io::ByteStream;

use crate::atoms::{Atom, AtomHeader, AtomIterator, AtomType, TfhdAtom, TrunAtom};

/// Track fragment atom.
#[derive(Debug)]
pub struct TrafAtom {
    /// Atom header.
    header: AtomHeader,
    /// Track fragment header.
    pub tfhd: TfhdAtom,
    /// Track fragment runs.
    pub truns: Vec<TrunAtom>,
}

impl Atom for TrafAtom {
    fn header(&self) -> AtomHeader {
        self.header
    }

    fn read<B: ByteStream>(reader: &mut B, header: AtomHeader) -> Result<Self> {
        let mut tfhd = None;
        let mut truns = Vec::new();

        let mut iter = AtomIterator::new(reader, header);

        while let Some(header) = iter.next()? {
            match header.atype {
                AtomType::TrackFragmentHeader => {
                    tfhd = Some(iter.read_atom::<TfhdAtom>()?);
                }
                AtomType::TrackFragmentRun => {
                    let trun = iter.read_atom::<TrunAtom>()?;
                    truns.push(trun);
                }
                _ => ()
            }
        }

        // Tfhd is mandatory.
        if tfhd.is_none() {
            return decode_error("missing tfhd atom");
        }

        Ok(TrafAtom {
            header,
            tfhd: tfhd.unwrap(),
            truns,
        })
    }
}