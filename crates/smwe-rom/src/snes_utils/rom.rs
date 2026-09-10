use std::sync::Arc;

use thiserror::Error;

use crate::{
    compression::DecompressionError,
    snes_utils::{
        addr::{AddrPc, AddrSnes},
        rom_slice::*,
    },
};

// -------------------------------------------------------------------------------------------------

type ParseErr<'a> = nom::Err<nom::error::Error<&'a [u8]>>;

#[derive(Debug, Error)]
pub enum RomError {
    #[error("Empty ROM file")]
    Empty,
    #[error("Invalid ROM size (not a multiple of 512 bytes): {0} ({0:#x})")]
    Size(usize),
    #[error("Could not PC slice ROM: {0}")]
    SlicePc(PcSlice),
    #[error("Could not SNES slice ROM: {0}")]
    SliceSnes(SnesSlice),
    #[error("Could not decompress ROM slice:\n- {0}")]
    Decompress(DecompressionError),
    #[error("Could not parse ROM slice")]
    Parse,
}

// -------------------------------------------------------------------------------------------------

pub const SMC_HEADER_SIZE: usize = 0x200;

// -------------------------------------------------------------------------------------------------

/// ROM contents, with any SMC header already stripped.
#[derive(Clone)]
pub struct Rom(pub Arc<[u8]>);

impl std::fmt::Debug for Rom {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Rom({} bytes)", self.0.len())
    }
}

impl Rom {
    pub fn new(mut data: Vec<u8>) -> Result<Self, RomError> {
        if data.is_empty() {
            return Err(RomError::Empty);
        }
        match data.len() % 0x400 {
            SMC_HEADER_SIZE => {
                data.drain(..SMC_HEADER_SIZE);
                Ok(Self(Arc::from(data)))
            }
            0 => Ok(Self(Arc::from(data))),
            _ => Err(RomError::Size(data.len())),
        }
    }

    pub fn bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn slice_pc(&self, slice: PcSlice) -> Result<&[u8], RomError> {
        let begin = slice.begin.as_index();
        if slice.is_infinite() { self.0.get(begin..) } else { self.0.get(begin..begin + slice.size) }
            .ok_or(RomError::SlicePc(slice))
    }

    pub fn slice_lorom(&self, slice: SnesSlice) -> Result<&[u8], RomError> {
        let begin = AddrPc::try_from_lorom(slice.begin).map_err(|_| RomError::SliceSnes(slice))?;
        self.slice_pc(PcSlice::new(begin, slice.size))
    }

    /// Everything from `start` to the end of the ROM: for data whose length only the parser
    /// reading it can determine.
    pub fn slice_from(&self, start: AddrSnes) -> Result<&[u8], RomError> {
        self.slice_lorom(SnesSlice::new(start, usize::MAX))
    }

    pub fn parse_pc<'r, Ret, P>(&'r self, slice: PcSlice, f: P) -> Result<Ret, RomError>
    where
        P: nom::Parser<&'r [u8], Ret, nom::error::Error<&'r [u8]>>,
    {
        parse_bytes(self.slice_pc(slice)?, f)
    }

    pub fn parse_lorom<'r, Ret, P>(&'r self, slice: SnesSlice, f: P) -> Result<Ret, RomError>
    where
        P: nom::Parser<&'r [u8], Ret, nom::error::Error<&'r [u8]>>,
    {
        parse_bytes(self.slice_lorom(slice)?, f)
    }

    pub fn decompress_lorom<D>(&self, slice: SnesSlice, decompressor: D) -> Result<Vec<u8>, RomError>
    where
        D: Fn(&[u8]) -> Result<Vec<u8>, DecompressionError>,
    {
        decompressor(self.slice_lorom(slice)?).map_err(RomError::Decompress)
    }
}

pub fn parse_bytes<'a, Ret, P>(bytes: &'a [u8], mut f: P) -> Result<Ret, RomError>
where
    P: nom::Parser<&'a [u8], Ret, nom::error::Error<&'a [u8]>>,
{
    let (_, ret) = f.parse(bytes).map_err(|_: ParseErr| RomError::Parse)?;
    Ok(ret)
}
