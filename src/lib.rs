//! A Rust wrapper for [openstack/liberasurecode].
//!
//! [openstack/liberasurecode]: https://github.com/openstack/liberasurecode
//!
//!
//! # Prerequisites to Build
//!
//! This crate requires the following packages for building [openstack/liberasurecode] in the build script:
//! - C compiler (e.g., `gcc`)
//! - `git`
//! - `make`
//! - `automake`
//! - `autoconf`
//! - `libtool`
//!
//! For example, on Ubuntu, you can install those by executing the following command:
//! ```console
//! $ sudo apt install gcc git make automake autoconf libtool
//! ```
//!
//!
//! # Examples
//!
//! Basic usage:
//! ```
//! use liberasurecode::{ErasureCoder, Error};
//! use std::num::NonZeroUsize;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let data_fragments = NonZeroUsize::new(4).ok_or("too few fragments")?;
//! let parity_fragments = NonZeroUsize::new(2).ok_or("too few fragments")?;
//! let mut coder = ErasureCoder::new(data_fragments, parity_fragments)?;
//! let input = vec![0, 1, 2, 3];
//!
//! // Encodes `input` to data and parity fragments
//! let fragments = coder.encode(&input)?;
//!
//! // Decodes the original data from the fragments (or a part of those)
//! assert_eq!(Ok(&input), coder.decode(&fragments[0..]).as_ref());
//! assert_eq!(Ok(&input), coder.decode(&fragments[1..]).as_ref());
//! assert_eq!(Ok(&input), coder.decode(&fragments[2..]).as_ref());
//! assert_eq!(Err(Error::InsufficientFragments), coder.decode(&fragments[3..]));
//! # Ok(())
//! # }
//! ```
#![warn(missing_docs)]
extern crate libc;

use std::num::NonZeroUsize;
use std::slice;
use std::time::Duration;


pub use crate::result::{Error, Result};

mod c_api;
mod result;

/// Erasure coding backends that can be used for encoding and decoding data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Backend {
    /// Read-Solomon erasure coding provided by `jerasure` library.
    JerasureRsVand,

    /// Cauchy base Read-Solomon erasure coding provided by `jerasure` library (default).
    JerasureRsCauchy,

   /// ISA-L base providedd by libisal library
   IsalRsVand,

    /// ISA-L RS CAUCHY variant
    IsalRsCauchy,
}
impl Default for Backend {
    /// `Backend::JerasureRsCauchy`を返す.
    fn default() -> Self {
        Backend::IsalRsCauchy
    }
}

/// Available checksum algorithms for validating decoded data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Checksum {
    /// No checksum (default).
    None,

    /// CRC32.
    Crc32,

    /// MD5.
    Md5,

    /// XxHash
    XxHash,
}
impl Default for Checksum {
    /// `Checksum::None`を返す.
    fn default() -> Self {
        Checksum::None
    }
}

/// [`ErasureCoder`] builder.
///
/// [`ErasureCoder`]: ./struct.ErasureCoder.html
#[derive(Debug, Clone)]
pub struct Builder {
    data_fragments: NonZeroUsize,
    parity_fragments: NonZeroUsize,
    backend: Backend,
    checksum: Checksum,
}
impl Builder {
    /// The default backend.
    pub const DEFAULT_BACKEND: Backend = Backend::IsalRsCauchy;

    /// The default checksum algorithm.
    pub const DEFAULT_CHECKSUM: Checksum = Checksum::XxHash;

    /// Makes a new `Builder` with the default settings.
    ///
    /// `data_fragments` and `parity_fragments` are
    /// the number of data fragments and parity fragments respectively.
    pub fn new(data_fragments: NonZeroUsize, parity_fragments: NonZeroUsize) -> Self {
        Builder {
            data_fragments,
            parity_fragments,
            backend: Self::DEFAULT_BACKEND,
            checksum: Self::DEFAULT_CHECKSUM,
        }
    }

    /// Sets the type of the erasure coding backend used by the resulting instance.
    ///
    /// The default value is `DEFAULT_BACKEND`.
    pub fn backend(&mut self, backend: Backend) -> &mut Self {
        self.backend = backend;
        self
    }

    /// Sets the checksum algorithm used by the resulting instance.
    ///
    /// The default value is `DEFAULT_CHECKSUM`.
    pub fn checksum(&mut self, checksum: Checksum) -> &mut Self {
        self.checksum = checksum;
        self
    }

    /// Builds a new [`ErasureCoder`] instance with the given settings.
    ///
    /// [`ErasureCoder`]: ./struct.ErasureCoder.html
    pub fn finish(&self) -> Result<ErasureCoder> {
        let backend_id = match self.backend {
            Backend::JerasureRsCauchy => c_api::EcBackendId::JERASURE_RS_CAUCHY,
            Backend::JerasureRsVand => c_api::EcBackendId::JERASURE_RS_VAND,
            Backend::IsalRsVand => c_api::EcBackendId::ISA_L_RS_VAND,
            Backend::IsalRsCauchy => c_api::EcBackendId::ISA_L_RS_CAUCHY,
        };
        let checksum_type = match self.checksum {
            Checksum::None => c_api::EcChecksumType::NONE,
            Checksum::Crc32 => c_api::EcChecksumType::CRC32,
            Checksum::Md5 => c_api::EcChecksumType::MD5,
            Checksum::XxHash => c_api::EcChecksumType::XXHASH,
        };
        let ec_args = c_api::EcArgs {
            k: self.data_fragments.get() as libc::c_int,
            m: self.parity_fragments.get() as libc::c_int,
            w: 32,
            hd: self.parity_fragments.get() as libc::c_int,
            priv_args: [0; 5],
            ct: checksum_type,
        };

        if self.data_fragments.get() == 1 && self.parity_fragments.get() == 1 {
            // Using this parameters, some backend will abort during executing `reconstruct` function.
            return Err(Error::InvalidParams);
        }

        // The creation of coder instance is not thread-safe, so we protect it by the global lock.
        with_global_lock(|| {
            let coder = c_api::instance_create(backend_id, &ec_args)
                .map(|desc| ErasureCoder {
                    data_fragments: self.data_fragments,
                    parity_fragments: self.parity_fragments,
                    desc,
                })
                .map_err(Error::from_error_code)?;

            // `SIGSEGV` may be raised if encodings are executed (in parallel) immediately after creation.
            // To prevent it, sleeps the current thread for a little while.
            std::thread::sleep(Duration::from_millis(10));
            Ok(coder)
        })
    }
}

/// Erasure coder.
///
/// # Examples
///
/// ```
/// use liberasurecode::{ErasureCoder, Error};
/// use std::num::NonZeroUsize;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let data_fragments = NonZeroUsize::new(4).ok_or("too few fragments")?;
/// let parity_fragments = NonZeroUsize::new(2).ok_or("too few fragments")?;
/// let mut coder = ErasureCoder::new(data_fragments, parity_fragments)?;
/// let data = vec![0, 1, 2, 3];
/// let encoded = coder.encode(&data)?;
///
/// assert_eq!(Ok(&data), coder.decode(&encoded[0..]).as_ref());
/// assert_eq!(Ok(&data), coder.decode(&encoded[1..]).as_ref());
/// assert_eq!(Ok(&data), coder.decode(&encoded[2..]).as_ref());
/// assert_eq!(Err(Error::InsufficientFragments),
///            coder.decode(&encoded[3..]));
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct ErasureCoder {
    data_fragments: NonZeroUsize,
    parity_fragments: NonZeroUsize,
    desc: c_api::Desc,
}

/// `ErasureChunkContext` stores all computation of by chunk erasure coding
pub struct ErasureChunkContext  {
    data:  Vec<*mut u8>,
    codings: Vec<*mut u8>,
    nr_subgroups: usize,
    chunk_size: usize,
    frag_len: usize,
    data_len: usize,
    k: usize,
    n: usize,
    desc: Desc,
}

#[repr(C, packed)]
#[derive(Debug)]
struct FragmentHeaderMetadata {
    idx: u32,
    size: u32,
    frag_backend_metadata_size: u32,
    orig_data_size: u64,
    cksum_type: u8,
    cksum: [u32; 8],
    cksum_mismatch: u8,
    backend_id: u8,
    backend_vesion: u32,
}

static  LIBERASURECODE_FRAG_HEADER_MAGIC: u32 = 0xb0c5ecc;

#[repr(C, packed)]
#[derive(Debug)]
struct FragmentHeader {
    meta : FragmentHeaderMetadata,
    magic: u32,
    libec_version: u32,
    metadata_cksum: u32,
    aligned_padding: [u8; 9],
}

use std::alloc::{alloc, dealloc, Layout};
use std::sync::{Mutex};
use rayon::prelude::{IntoParallelIterator, ParallelIterator};
use crate::c_api::{Desc, encode_chunk};


unsafe impl Send for ErasureChunkContext {}
unsafe impl Sync for ErasureChunkContext {}

impl Drop for ErasureChunkContext {

    fn drop(&mut self) {
        unsafe {
            let l = Layout::from_size_align(self.frag_len, 16).unwrap();
            for p in &self.data {
                dealloc(*p, l);
            }
        }
    }
}

impl ErasureChunkContext {
    pub fn new(desc: Desc, data_len: usize, k: usize, n: usize, chunk_size: usize) -> Self {
        let block_size = chunk_size * k;
        let mut nr_sub_groups = data_len / block_size;
        if nr_sub_groups * block_size != data_len {
            nr_sub_groups += 1;
        }

        let fragment_len = (std::mem::size_of::<FragmentHeader>() +
            chunk_size) * nr_sub_groups;

        ErasureChunkContext {
            desc,
            data: (0..k)
                .map(|_|
                    unsafe {
                        let r = alloc(Layout::from_size_align(fragment_len, 16).unwrap());
                        r
                    })
                .collect(),
            codings: (0..n)
                .map(|_|
                    unsafe {
                        let p = alloc(Layout::from_size_align(fragment_len, 16).unwrap());
                        println!("in function prepare alloc coding={:p} with len={}", p, fragment_len);
                        p
                    })
                .collect(),
            nr_subgroups: nr_sub_groups,
            chunk_size,
            frag_len: fragment_len,
            data_len,
            k,
            n,
        }
    }

    pub fn encode_matrix(&mut self, data: &[u8]) {
        let nr_sub_group = self.nr_subgroups;
        (0..nr_sub_group)
            .into_par_iter()
            .for_each(| i|
                {
                    self.do_chunk(data,  i);
                }
            );
    }

    /// Compute a sub chunk coding thru `ErasureChunkContext`
    fn do_chunk(& self, data: &[u8], nth: usize) {
        let mut vec_data: Vec<*mut u8> = Vec::new();
        let mut vec_coding: Vec<*mut u8> = Vec::new();
        assert!(nth < self.nr_subgroups);

        let mut tot_len = 0;

        let mut off_start = self.k * nth * self.chunk_size;

        let one_cell_size = std::mem::size_of::<FragmentHeader>() + self.chunk_size;

        for i in 0..self.k {
            let ptr = self.data.get(i).unwrap();
            let off_end = {
                let mut  ret = off_start + self.chunk_size;
                if off_start + self.chunk_size > data.len() {
                    ret = data.len()
                }
                ret
            };
            let data_offset = &data[off_start..off_end];


            let (_start_hdr, start_data) =
                unsafe {
                    let wh = ptr.add(nth * one_cell_size);
                    let p = &mut *(wh as *mut FragmentHeader);
                    (p).magic = LIBERASURECODE_FRAG_HEADER_MAGIC;
                    (p, wh.add(std::mem::size_of::<FragmentHeader>()))
                };
            let to_copy = off_end - off_start;
            unsafe {
                std::ptr::copy(data_offset.as_ptr(),start_data, to_copy);
            }
            tot_len += to_copy;
            vec_data.push(start_data);
            off_start += self.chunk_size;
        }
        for i in 0..self.n {
            let ptr = self.codings.get(i).unwrap();
            let (_start_hdr, start_data) =
                unsafe {
                    let wh = ptr.add(nth * one_cell_size);
                    let p = &mut *(wh as *mut FragmentHeader);
                    p.magic = LIBERASURECODE_FRAG_HEADER_MAGIC;
                    (p, wh.add(std::mem::size_of::<FragmentHeader>()))
                };
            println!("register subchunk coding {:p}", start_data);
            vec_coding.push(start_data);
        }
        encode_chunk(self.desc,vec_data.as_slice(),
                     vec_coding.as_slice(), self.chunk_size,
                     tot_len, self.k, self.n);
    }

    pub fn rawmap(&self) {
        for w in 0..self.nr_subgroups {
            for i in 0..self.k {
                let mut ptr = *self.data.get(i).unwrap();
                let f = unsafe {
                    ptr = ptr.add(w * (self.chunk_size + std::mem::size_of::<FragmentHeader>()));
                    &*(ptr as *mut FragmentHeader)
                };
                println!("subchunk={} data{} : {:#?}", w, i, f);
            }

            for i in 0..self.n {
                let mut ptr = *self.codings.get(i).unwrap();
                let f = unsafe {
                    ptr = ptr.add(w * (self.chunk_size + std::mem::size_of::<FragmentHeader>()));
                    &*(ptr as *mut FragmentHeader)
                };
                println!("subchunk={} coding{} : {:#?}", w, i, f);
            }
        }
    }

}


impl ErasureCoder {
    /// Makes a new `ErasureCoder` instance with the default settings.
    ///
    /// This is equivalent to `Builder::new(data_fragments, parity_fragments).finish()`.
    pub fn new(data_fragments: NonZeroUsize, parity_fragments: NonZeroUsize) -> Result<Self> {
        Builder::new(data_fragments, parity_fragments).finish()
    }

    /// Returns the number of data fragments specified to the coder.
    pub fn data_fragments(&self) -> NonZeroUsize {
        self.data_fragments
    }

    /// Returns the number of parity fragments specified to the coder.
    pub fn parity_fragments(&self) -> NonZeroUsize {
        self.parity_fragments
    }

    /// The total number of data fragments and parity fragments specified to the coder.
    pub fn fragments(&self) -> NonZeroUsize {
        unsafe {
            NonZeroUsize::new_unchecked(self.data_fragments.get() + self.parity_fragments.get())
        }
    }

    /// Encodes the given data to data and parity fragments.
    pub fn encode(&mut self, data: &[u8]) -> Result<Vec<Vec<u8>>> {
        let (encoded_data, encoded_parity, fragment_len) =
            c_api::encode(self.desc, data).map_err(Error::from_error_code)?;

        let mut fragments = Vec::with_capacity(self.fragments().get());

        let data_fragments =
            unsafe { slice::from_raw_parts(encoded_data, self.data_fragments.get()) };
        fragments.extend((0..self.data_fragments.get()).map(|i| {
            Vec::from(unsafe { slice::from_raw_parts(data_fragments[i], fragment_len as usize) })
        }));

        let parity_fragments =
            unsafe { slice::from_raw_parts(encoded_parity, self.parity_fragments.get()) };
        fragments.extend((0..self.parity_fragments.get()).map(|i| {
            Vec::from(unsafe { slice::from_raw_parts(parity_fragments[i], fragment_len as usize) })
        }));

        c_api::encode_cleanup(self.desc, encoded_data, encoded_parity)
            .map_err(Error::from_error_code)?;
        Ok(fragments)
    }


    /// Decodes the original data from the given fragments.
    pub fn decode<T: AsRef<[u8]>>(&mut self, fragments: &[T]) -> Result<Vec<u8>> {
        if fragments.is_empty() {
            return Err(Error::InsufficientFragments);
        }
        let data_fragments = &fragments.iter().map(AsRef::as_ref).collect::<Vec<_>>()[..];

        let (data, data_len) =
            c_api::decode(self.desc, data_fragments, false).map_err(Error::from_error_code)?;
        let buf = Vec::from(unsafe { slice::from_raw_parts(data, data_len as usize) });
        c_api::decode_cleanup(self.desc, data).map_err(Error::from_error_code)?;
        Ok(buf)
    }

    /// Reconstructs the fragment specified by the given index from other available fragments.
    ///
    /// # Errors
    ///
    /// This function will return `Error::InvalidParams` if the given index is bigger or equal
    /// than the total number of parity_fragments and data_fragments.
    pub fn reconstruct<T, F>(&mut self, index: usize, available_fragments: T) -> Result<Vec<u8>>
    where
        T: Iterator<Item = F>,
        F: AsRef<[u8]>,
    {
        if index >= self.fragments().get() {
            return Err(Error::InvalidParams);
        }

        let fragments = available_fragments.collect::<Vec<_>>();
        let fragments = fragments.iter().map(AsRef::as_ref).collect::<Vec<_>>();
        c_api::reconstruct_fragment(self.desc, &fragments[..], index)
            .map_err(Error::from_error_code)
    }
}
impl Drop for ErasureCoder {
    fn drop(&mut self) {
        let _ = c_api::instance_destroy(self.desc);
    }
}

fn with_global_lock<F, T>(f: F) -> T
where
    F: FnOnce() -> T,
{
    use std::sync::{Once};

    static mut MUTEX: Option<Mutex<()>> = None;
    static INIT: Once = Once::new();
    INIT.call_once(|| unsafe {
        MUTEX = Some(Mutex::default());
    });

    let _guard = unsafe {
        MUTEX
            .as_ref()
            .expect("Never fails")
            .lock()
            .expect("Poisoned global lock")
    };
    f()
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;

    use super::*;
    use crate::result::Error;

    #[test]
    fn it_works() {
        let mut coder = ErasureCoder::new(non_zero(4), non_zero(2)).unwrap();
        let data = vec![0, 1, 2, 3];
        let encoded = coder.encode(&data).unwrap();

        assert_eq!(Ok(&data), coder.decode(&encoded[0..]).as_ref());
        assert_eq!(Ok(&data), coder.decode(&encoded[1..]).as_ref());
        assert_eq!(Ok(&data), coder.decode(&encoded[2..]).as_ref());
        assert_eq!(
            Err(Error::InsufficientFragments),
            coder.decode(&encoded[3..])
        );
    }

    #[test]
    fn reconstruct_works() {
        let mut coder = ErasureCoder::new(non_zero(4), non_zero(4)).unwrap();
        let data = vec![0, 1, 2, 3];
        let encoded = coder.encode(&data).unwrap();

        for i in 0..coder.fragments().get() {
            assert_eq!(
                coder.reconstruct(
                    i,
                    encoded
                        .iter()
                        .enumerate()
                        .filter(|&(index, _)| index != i)
                        .map(|(_, f)| f)
                        .take(4),
                ),
                Ok(encoded[i].clone())
            );
        }
    }

    #[test]
    fn reconstruct_works_for_all_fragments() -> Result<()> {
        let k = 6;
        let m = 3;
        let len = 0xc0de;
        let mut coder = ErasureCoder::new(non_zero(k), non_zero(m)).unwrap();
        let mut data = vec![0; len];
        let mut seed: u32 = 0xdeadbeef;
        for item in data.iter_mut() {
            *item = (seed >> 16) as u8;
            seed = seed.wrapping_mul(0x15151).wrapping_add(0x31111111);
        }
        let encoded = coder.encode(&data).unwrap();
        assert_eq!(encoded.len(), k + m);

        // Exhaustively checks all patterns.
        for alive in 0usize..1 << (k + m) {
            // If not exactly k fragments are alive, skip.
            if alive.count_ones() as usize != k {
                continue;
            }
            let mut fragments = vec![];
            for (index, row) in encoded.iter().enumerate() {
                if (alive & 1 << index) != 0 {
                    fragments.push(row.clone());
                }
            }
            assert_eq!(fragments.len(), k);
            for (index, row) in encoded.iter().enumerate() {
                if (alive & 1 << index) == 0 {
                    // if index is not alive, reconstruct it and check the validity.
                    let reconstructed = coder.reconstruct(index, fragments.iter())?;
                    assert_eq!(&reconstructed, row);
                }
            }
        }
        Ok(())
    }
    #[test]
    fn reconstruct_fails() {
        let mut coder = ErasureCoder::new(non_zero(4), non_zero(4)).unwrap();
        let data = vec![0, 1, 2, 3];
        let encoded = coder.encode(&data).unwrap();

        assert!(coder.reconstruct(7, encoded.iter()).is_ok());
        assert_eq!(
            coder.reconstruct(8, encoded.iter()),
            Err(Error::InvalidParams)
        );
        assert_eq!(
            coder.reconstruct(9, encoded.iter()),
            Err(Error::InvalidParams)
        );
    }

    #[test]
    fn chunk_matrix() {
        let coder = ErasureCoder::new(non_zero(4), non_zero(1)).unwrap();
        //let data = vec![5; 262144];
        let data = (0..262144).map(|val : u32|
                                                            {
                                                                let p : u8 = 'a' as u8 + ((val % 24) as u8);
                                                                p
                                                            })
                                        .collect::<Vec<u8>>();
        //coder.do_chunk(data.as_slice(), &mut encoded, 0);
        //coder.encode_matrix(data.as_slice(), 32768);
        let mut encoded = ErasureChunkContext::new(coder.desc, data.len(),4, 1,32768);

        encoded.encode_matrix(data.as_slice());
        encoded.rawmap();
    }

    #[test]
    fn various_params() {
        for backend in [Backend::JerasureRsCauchy, Backend::JerasureRsVand].iter() {
            for checksum in [Checksum::None, Checksum::Crc32, Checksum::Md5].iter() {
                for data_fragments in (3..6).map(non_zero) {
                    for parity_fragments in (1..4).map(non_zero) {
                        let mut coder = Builder::new(data_fragments, parity_fragments)
                            .backend(*backend)
                            .checksum(*checksum)
                            .finish()
                            .unwrap_or_else(|_| {
                                panic!(
                                    "Cannot make coder instance: k={}, m={}, b={:?}, \
                                 c={:?}",
                                    data_fragments, parity_fragments, backend, checksum
                                )
                            });

                        let data = vec![0, 1, 2, 3];
                        let encoded = coder.encode(&data).unwrap();

                        for i in 0..parity_fragments.get() {
                            assert_eq!(Ok(&data), coder.decode(&encoded[i..]).as_ref());
                        }
                        assert_eq!(
                            Err(Error::InsufficientFragments),
                            coder.decode(&encoded[parity_fragments.get() + 1..])
                        );
                    }
                }
            }
        }
    }

    fn non_zero(n: usize) -> NonZeroUsize {
        NonZeroUsize::new(n).expect("Must be a non zero number")
    }
}
