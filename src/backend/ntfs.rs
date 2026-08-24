//! Pure-Rust NTFS structure pass.
//!
//! Recovers files by parsing the MFT directly (no libntfs): boot sector
//! geometry, $MFT run list, per-record attributes, data-run resolution (which
//! inherently handles fragmentation), $FILE_NAME parent chains for exact
//! paths, and LZNT1 decompression for compressed files. Deleted files whose
//! records are still present in the MFT are recovered too.
//!
//! Used clusters (every non-resident attribute run in the MFT) are reported
//! back so the carve pass can skip them, mirroring the FAT structure pass.

use std::io;
use std::path::Path;

pub const ATTR_STANDARD_INFORMATION: u32 = 0x10;
pub const ATTR_ATTRIBUTE_LIST: u32 = 0x20;
pub const ATTR_FILE_NAME: u32 = 0x30;
pub const ATTR_OBJECT_ID: u32 = 0x40;
pub const ATTR_SECURITY_DESCRIPTOR: u32 = 0x50;
pub const ATTR_VOLUME_NAME: u32 = 0x60;
pub const ATTR_VOLUME_INFORMATION: u32 = 0x70;
pub const ATTR_DATA: u32 = 0x80;
pub const ATTR_INDEX_ROOT: u32 = 0x90;
pub const ATTR_INDEX_ALLOCATION: u32 = 0xA0;
pub const ATTR_BITMAP: u32 = 0xB0;
pub const ATTR_END: u32 = 0xFFFFFFFF;

pub const MFT_RECORD_ROOT: u64 = 5;

/// Cluster number marker for sparse runs (no physical clusters allocated).
const SPARSE: u64 = u64::MAX;

const MAX_RECORDS: u64 = 10_000_000;
const MAX_PATH_DEPTH: usize = 128;
const MFT_RECORD_MAGIC: &[u8; 4] = b"FILE";
/// Upper bound on how much of a deleted file's intact head we use as the
/// "seed" when scanning for a better copy to rebuild it from.
const SEED_PREFIX_MAX: usize = 4096;

/// Well-known binary signatures checked before writing a partly-overwritten
/// (stale) deleted file, and used to tell "garbage" from "plausible file".
const KNOWN_MAGICS: &[&[u8]] = &[
    b"\xFF\xD8\xFF",      // JPEG
    b"\x89PNG\r\n\x1A\n", // PNG
    b"%PDF-",             // PDF
    b"PK\x03\x04",        // ZIP / DOCX / XLSX
    b"GIF87a",
    b"GIF89a",
    b"BM",      // BMP
    b"II*\x00", // TIFF LE
    b"MM\x00*", // TIFF BE
];

/// A contiguous run of clusters. For sparse runs `cluster == SPARSE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Run {
    pub vcn: u64,
    pub cluster: u64,
    pub length: u64,
}

/// Absolute byte range on the scanned device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteRange {
    pub offset: u64,
    pub size: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct Geometry {
    pub sector_size: u16,
    pub cluster_size: u32,
    pub mft_lcn: u64,
    pub mft_record_size: u32,
    pub index_record_size: u32,
    pub total_sectors: u64,
}

/// Abstraction over the underlying disk so the parser can be unit-tested
/// against a plain file.
pub trait Reader {
    /// Read exactly `buf.len()` bytes at absolute byte `offset`.
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()>;
}

/// `Reader` backed by a plain file (used by tests and the example binary).
pub struct FileReader {
    file: std::fs::File,
}

impl FileReader {
    pub fn new(path: &Path) -> io::Result<Self> {
        Ok(Self {
            file: std::fs::File::open(path)?,
        })
    }
}

impl Reader for FileReader {
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        use std::io::{Read, Seek, SeekFrom};
        let mut file = &self.file;
        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(buf)
    }
}

// ---------------------------------------------------------------------------
// Little-endian readers
// ---------------------------------------------------------------------------

fn u16le(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([b[off], b[off + 1]])
}

fn u32le(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

fn u64le(b: &[u8], off: usize) -> u64 {
    let mut v = [0u8; 8];
    v.copy_from_slice(&b[off..off + 8]);
    u64::from_le_bytes(v)
}

/// Signed little-endian value of `n` (1..=8) bytes with sign extension.
fn read_signed(b: &[u8], n: usize) -> i64 {
    debug_assert!(n >= 1 && n <= 8);
    let mut v: i64 = 0;
    for i in 0..n {
        v |= (b[i] as i64) << (8 * i);
    }
    if n < 8 && (b[n - 1] & 0x80) != 0 {
        v |= !0i64 << (8 * n);
    }
    v
}

// ---------------------------------------------------------------------------
// Mapping pairs (data runs)
// ---------------------------------------------------------------------------

/// Decode an NTFS mapping-pairs run list. `start` points at the first header
/// byte (attribute field `mapping_pairs_offset`).
pub fn parse_mapping_pairs(data: &[u8], start: usize) -> Option<Vec<Run>> {
    let mut runs = Vec::new();
    let mut vcn: i64 = 0;
    let mut prev: i64 = 0; // last physical cluster of a stored run
    let mut i = start;
    loop {
        if i >= data.len() {
            break;
        }
        let header = data[i];
        i += 1;
        if header == 0 {
            break;
        }
        let len_size = (header & 0x0F) as usize;
        let off_size = (header >> 4) as usize;
        if len_size == 0 || len_size > 8 || off_size > 8 {
            return None;
        }
        if i + len_size + off_size > data.len() {
            return None;
        }
        let length = read_signed(&data[i..i + len_size], len_size);
        i += len_size;
        if off_size > 0 {
            prev = prev.wrapping_add(read_signed(&data[i..i + off_size], off_size));
            i += off_size;
        }
        if length > 0 {
            if prev < 0 {
                return None;
            }
            runs.push(Run {
                vcn: vcn as u64,
                cluster: prev as u64,
                length: length as u64,
            });
            vcn += length;
        } else if length < 0 {
            // Sparse run: advances VCN only, consumes no physical clusters.
            runs.push(Run {
                vcn: vcn as u64,
                cluster: SPARSE,
                length: (-length) as u64,
            });
            vcn -= length;
        } else {
            break;
        }
    }
    Some(runs)
}

// ---------------------------------------------------------------------------
// MFT records and attributes
// ---------------------------------------------------------------------------

/// Apply the update sequence array (fixup) to a record. On disk the last two
/// bytes of every 512-byte sector have been overwritten with the raw
/// signature; the USA array holds the true values, which we restore. Accepts
/// both written-out (sector ends match the signature) and already-restored
/// (sector ends match the saved value) records. Returns false for a corrupt
/// record.
fn apply_fixups(rec: &mut [u8]) -> bool {
    if rec.len() < 0x30 {
        return false;
    }
    let usa_offset = u16le(rec, 0x04) as usize;
    let usa_count = u16le(rec, 0x06) as usize;
    if usa_count < 2 || usa_offset + usa_count * 2 > rec.len() {
        return false;
    }
    let usa = u16le(rec, usa_offset);
    for i in 0..usa_count - 1 {
        let sector_end = (i + 1) * 512 - 2; // last two bytes of sector i
        if sector_end + 2 > rec.len() {
            break;
        }
        let saved = u16le(rec, usa_offset + 2 + i * 2);
        let current = u16le(rec, sector_end);
        if current != usa && current != saved {
            return false;
        }
        rec[sector_end] = (saved & 0xFF) as u8;
        rec[sector_end + 1] = (saved >> 8) as u8;
    }
    true
}

/// The un-named $DATA payload of a file record.
#[derive(Debug, Clone)]
pub enum DataAttr {
    Resident(Vec<u8>),
    /// Non-resident. `compressed` selects LZNT1 decoding of the run data.
    Runs {
        runs: Vec<Run>,
        real_size: u64,
        compressed: bool,
    },
}

impl DataAttr {
    pub fn real_size(&self) -> u64 {
        match self {
            DataAttr::Resident(v) => v.len() as u64,
            DataAttr::Runs { real_size, .. } => *real_size,
        }
    }
}

/// Parsed MFT record entry used by the recovery pass.
pub struct MftEntry {
    pub record: u64,
    pub in_use: bool,
    pub is_dir: bool,
    pub parent: u64,
    pub name: String,
    pub data: Option<DataAttr>,
    /// All non-resident runs of every attribute (for used-space marking).
    pub used_runs: Vec<Run>,
}

fn decode_utf16le(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16_lossy(&units)
}

/// Parse a single MFT record buffer into an `MftEntry` (None if the buffer
/// does not contain a valid `$FILE` record).
fn parse_mft_record(rec: &[u8], record_number: u64) -> Option<MftEntry> {
    if rec.len() < 0x38 || &rec[0..4] != MFT_RECORD_MAGIC {
        return None;
    }
    let flags = u16le(rec, 0x16);
    // Extension records (from attribute lists) carry no un-named $DATA or
    // $FILE_NAME of their own, so they are naturally ignored below.
    let in_use = flags & 0x01 != 0;
    let is_dir = flags & 0x02 != 0;

    let mut parent: u64 = 0;
    let mut name: Option<String> = None;
    let mut data: Option<DataAttr> = None;
    let mut used_runs: Vec<Run> = Vec::new();

    let mut p = 0x38usize;
    while p + 8 <= rec.len() {
        let atype = u32le(rec, p);
        if atype == ATTR_END {
            break;
        }
        let alen = u32le(rec, p + 4) as usize;
        if alen == 0 || p + alen > rec.len() {
            break;
        }
        let attr = &rec[p..p + alen];
        if attr.len() >= 0x0C {
            let non_resident = attr[0x08] != 0;
            let name_len = attr[0x09] as usize;
            let name_off = u16le(attr, 0x0A) as usize;
            let attr_flags = u16le(attr, 0x0C);
            let attr_name = if name_len > 0 && name_off + name_len * 2 <= attr.len() {
                Some(decode_utf16le(&attr[name_off..name_off + name_len * 2]))
            } else {
                None
            };

            if non_resident {
                if attr.len() >= 0x40 {
                    let map_off = u16le(attr, 0x20) as usize;
                    let real_size = u64le(attr, 0x30);
                    if map_off < attr.len() {
                        if let Some(runs) = parse_mapping_pairs(attr, map_off) {
                            if !runs.is_empty() && atype != ATTR_ATTRIBUTE_LIST {
                                used_runs.extend(runs.iter().copied());
                            }
                            if atype == ATTR_DATA && attr_name.is_none() && !runs.is_empty() {
                                let compressed = attr_flags & 0x0001 != 0;
                                data = Some(DataAttr::Runs {
                                    runs,
                                    real_size,
                                    compressed,
                                });
                            }
                        }
                    }
                }
            } else {
                if attr.len() < 0x18 {
                    p += alen;
                    continue;
                }
                let value_len = u32le(attr, 0x10) as usize;
                let value_off = u16le(attr, 0x14) as usize;
                if value_off + value_len <= attr.len() {
                    let v = &attr[value_off..value_off + value_len];
                    if atype == ATTR_DATA && attr_name.is_none() {
                        data = Some(DataAttr::Resident(v.to_vec()));
                    } else if atype == ATTR_FILE_NAME && v.len() >= 0x42 {
                        let parent_ref = u64le(v, 0x00) & 0x0000_FFFF_FFFF_FFFF;
                        let name_len = v[0x40] as usize;
                        let namespace = v[0x41];
                        if name_len > 0 && 0x42 + name_len * 2 <= v.len() {
                            let n = decode_utf16le(&v[0x42..0x42 + name_len * 2]);
                            // Prefer the most descriptive namespace.
                            let keep = match (&name, namespace) {
                                (None, _) => true,
                                (Some(_), 1 | 3) => true, // Win32 or Win32&DOS
                                (Some(_), 0) => true,     // POSIX over DOS
                                _ => false,               // keep existing over DOS
                            };
                            if keep {
                                parent = parent_ref;
                                name = Some(n);
                            }
                        }
                    }
                }
            }
        }
        p += alen;
    }

    // Metadata files ($MFT, $Bitmap, ...) are filtered out at recovery time
    // (their names start with '$'), not here: record 0 is the $MFT itself and
    // must still parse so its $DATA gives us the MFT run list.

    Some(MftEntry {
        record: record_number,
        in_use,
        is_dir,
        parent,
        name: name.unwrap_or_default(),
        data,
        used_runs,
    })
}

// ---------------------------------------------------------------------------
// Volume
// ---------------------------------------------------------------------------

/// Allocated-cluster set parsed from the `$Bitmap` file (MFT record 6).
/// Used to tell whether a deleted file's runs still point at untouched
/// (unallocated) clusters or were overwritten by newer data.
#[derive(Debug, Clone)]
pub struct ClusterBitmap {
    /// One bit per cluster, LSB-first words.
    words: Vec<u64>,
    pub cluster_count: u64,
}

impl ClusterBitmap {
    /// Build from raw `$Bitmap` payload bytes. Bits beyond the cluster count
    /// are ignored.
    pub fn from_bytes(bytes: &[u8], cluster_count: u64) -> Self {
        let word_count = cluster_count.div_ceil(64);
        let mut words = vec![0u64; word_count as usize];
        let max_byte = cluster_count.div_ceil(8) as usize;
        for (i, b) in bytes.iter().take(max_byte).enumerate() {
            for bit in 0..8 {
                if b & (1 << bit) != 0 {
                    let idx = i * 8 + bit;
                    if idx < cluster_count as usize {
                        words[idx / 64] |= 1 << (idx % 64);
                    }
                }
            }
        }
        ClusterBitmap {
            words,
            cluster_count,
        }
    }

    pub fn is_allocated(&self, cluster: u64) -> bool {
        if cluster >= self.cluster_count {
            return true; // outside the volume
        }
        let (w, b) = ((cluster / 64) as usize, (cluster % 64) as u64);
        self.words.get(w).map_or(true, |word| word & (1 << b) != 0)
    }
}

pub struct NtfsVolume<R: Reader> {
    reader: R,
    part_offset: u64,
    geom: Geometry,
    mft_runs: Vec<Run>,
    mft_real_size: u64,
    record_count: u64,
    bitmap: ClusterBitmap,
}

impl<R: Reader> NtfsVolume<R> {
    /// Open and validate an NTFS volume whose boot sector is at `part_offset`.
    pub fn open(reader: R, part_offset: u64, sector_size: u16) -> io::Result<Self> {
        let mut boot = [0u8; 512];
        reader.read_at(part_offset, &mut boot)?;
        let geom = parse_boot_sector(&boot, sector_size)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "not an NTFS boot sector"))?;
        let mut vol = NtfsVolume {
            reader,
            part_offset,
            geom,
            mft_runs: Vec::new(),
            mft_real_size: 0,
            record_count: 0,
            bitmap: ClusterBitmap {
                words: Vec::new(),
                cluster_count: 0,
            },
        };
        vol.load_mft()?;
        vol.load_bitmap();
        Ok(vol)
    }

    pub fn geometry(&self) -> &Geometry {
        &self.geom
    }

    pub fn bitmap(&self) -> &ClusterBitmap {
        &self.bitmap
    }

    fn cluster_to_offset(&self, cluster: u64) -> u64 {
        self.part_offset + cluster * self.geom.cluster_size as u64
    }

    /// Best-effort parse of `$Bitmap` (record 6). On failure the empty bitmap
    /// makes every cluster "allocated", which disables stale-file recovery.
    fn load_bitmap(&mut self) {
        let cluster_count =
            self.geom.total_sectors * self.geom.sector_size as u64 / self.geom.cluster_size as u64;
        let slot6 = self.read_records_bulk(6, 1);
        let data = match slot6.get(0).map(|e| e.as_ref()) {
            Some(Some(e)) => match &e.data {
                Some(d) => d,
                None => return,
            },
            _ => return,
        };
        let raw: Vec<u8> = match data {
            DataAttr::Resident(v) => v.clone(),
            DataAttr::Runs {
                runs,
                real_size,
                compressed,
                ..
            } if !compressed => {
                let cs = self.geom.cluster_size as u64;
                let mut raw: Vec<u8> = Vec::new();
                let mut written: u64 = 0;
                for r in runs {
                    if r.cluster == SPARSE || written >= *real_size {
                        continue;
                    }
                    let emit = (r.length * cs).min(*real_size - written);
                    let mut buf = vec![0u8; emit as usize];
                    if self
                        .reader
                        .read_at(self.cluster_to_offset(r.cluster), &mut buf)
                        .is_err()
                    {
                        return;
                    }
                    raw.extend_from_slice(&buf);
                    written += emit;
                }
                raw
            }
            _ => return,
        };
        self.bitmap = ClusterBitmap::from_bytes(&raw, cluster_count);
    }

    fn load_mft(&mut self) -> io::Result<()> {
        let rec_size = self.geom.mft_record_size as usize;
        // Record 0 is the $MFT itself; its $DATA attribute gives the full run
        // list of the MFT file.
        let mut rec = vec![0u8; rec_size];
        self.reader
            .read_at(self.cluster_to_offset(self.geom.mft_lcn), &mut rec)?;
        if !apply_fixups(&mut rec) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "bad $MFT record",
            ));
        }
        let entry = parse_mft_record(&rec, 0).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "cannot parse $MFT record")
        })?;
        match entry.data {
            Some(DataAttr::Runs {
                runs,
                real_size,
                compressed,
                ..
            }) if !compressed => {
                self.mft_runs = runs;
                self.mft_real_size = real_size;
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "$MFT has no non-resident $DATA",
                ));
            }
        }
        self.record_count = (self.mft_real_size / rec_size as u64).min(MAX_RECORDS);
        Ok(())
    }

    /// Physical offset of MFT record `n` in the volume.
    fn record_phys(&self, n: u64) -> Option<u64> {
        let pos = n * self.geom.mft_record_size as u64;
        if pos + self.geom.mft_record_size as u64 > self.mft_real_size {
            return None;
        }
        let cluster_size = self.geom.cluster_size as u64;
        let vcn = pos / cluster_size;
        let run = self
            .mft_runs
            .iter()
            .find(|r| r.cluster != SPARSE && vcn >= r.vcn && vcn < r.vcn + r.length)?;
        let within = pos - run.vcn * cluster_size;
        Some(self.cluster_to_offset(run.cluster) + within)
    }

    /// Read MFT records `start..start+count`, returning one slot per record
    /// (None for records that did not parse). Tries a single bulk read when
    /// the span falls inside one run.
    fn read_records_bulk(&self, start: u64, count: u64) -> Vec<Option<MftEntry>> {
        let rec_size = self.geom.mft_record_size as usize;
        let cluster_size = self.geom.cluster_size as u64;
        let mut out: Vec<Option<MftEntry>> = Vec::with_capacity(count as usize);

        let pos = start * rec_size as u64;
        let bytes = count * rec_size as u64;
        let vcn_start = pos / cluster_size;
        let vcn_end = (pos + bytes).div_ceil(cluster_size);
        let contiguous = self
            .mft_runs
            .iter()
            .find(|r| r.cluster != SPARSE && vcn_start >= r.vcn && vcn_end <= r.vcn + r.length);

        if let Some(run) = contiguous {
            let mut buf = vec![0u8; bytes as usize];
            let phys = self.cluster_to_offset(run.cluster) + (pos - run.vcn * cluster_size);
            if self.reader.read_at(phys, &mut buf).is_ok() {
                for i in 0..count {
                    let mut rec =
                        buf[(i as usize * rec_size)..((i as usize + 1) * rec_size)].to_vec();
                    if apply_fixups(&mut rec) {
                        out.push(parse_mft_record(&rec, start + i));
                    } else {
                        out.push(None);
                    }
                }
                return out;
            }
        }
        // Fall back to per-record reads.
        for i in 0..count {
            if let Some(phys) = self.record_phys(start + i) {
                let mut rec = vec![0u8; rec_size];
                if self.reader.read_at(phys, &mut rec).is_ok() && apply_fixups(&mut rec) {
                    out.push(parse_mft_record(&rec, start + i));
                } else {
                    out.push(None);
                }
            } else {
                out.push(None);
            }
        }
        out
    }
}

/// Parse the NTFS boot sector and derive geometry.
pub fn parse_boot_sector(boot: &[u8], sector_size: u16) -> Option<Geometry> {
    if boot.len() < 0x50 || &boot[0x03..0x0B] != b"NTFS    " {
        return None;
    }
    let bps = u16le(boot, 0x0B);
    let spc = boot[0x0D];
    if bps != sector_size || bps == 0 || spc == 0 || spc > 128 || !spc.is_power_of_two() {
        return None;
    }
    let cluster_size = (bps as u32) * (spc as u32);
    let total_sectors = u64le(boot, 0x28);
    let mft_lcn = u64le(boot, 0x30);
    if mft_lcn == 0 || total_sectors == 0 {
        return None;
    }
    let cpf = boot[0x40] as i8;
    let cpi = boot[0x44] as i8;
    let mft_record_size = record_size_from_byte(cpf, cluster_size)?;
    let index_record_size = record_size_from_byte(cpi, cluster_size)?;
    Some(Geometry {
        sector_size: bps,
        cluster_size,
        mft_lcn,
        mft_record_size,
        index_record_size,
        total_sectors,
    })
}

fn record_size_from_byte(b: i8, cluster_size: u32) -> Option<u32> {
    if b > 0 {
        Some((b as u32).checked_mul(cluster_size)?)
    } else if b < 0 {
        let shift = (-(b as i32)) as u32;
        if shift >= 32 {
            None
        } else {
            Some(1u32 << shift)
        }
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// LZNT1 decompression
// ---------------------------------------------------------------------------

fn lznt1_decompress(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 2 <= data.len() {
        let header = u16::from_le_bytes([data[i], data[i + 1]]);
        i += 2;
        let chunk_len = (header & 0x0FFF) as usize + 1;
        if i + chunk_len > data.len() {
            break;
        }
        let chunk = &data[i..i + chunk_len];
        i += chunk_len;
        if header & 0x8000 != 0 {
            lznt1_decompress_chunk(chunk, &mut out);
        } else {
            out.extend_from_slice(chunk);
        }
    }
    out
}

fn lznt1_decompress_chunk(mut chunk: &[u8], out: &mut Vec<u8>) {
    while !chunk.is_empty() && out.len() < 4096 {
        let flags = chunk[0];
        chunk = &chunk[1..];
        for i in 0..8 {
            if chunk.is_empty() || out.len() >= 4096 {
                break;
            }
            if flags & (1 << i) == 0 {
                out.push(chunk[0]);
                chunk = &chunk[1..];
            } else {
                if chunk.len() < 2 {
                    return;
                }
                let word = u16::from_le_bytes([chunk[0], chunk[1]]);
                chunk = &chunk[2..];
                let mut pos = out.len().wrapping_sub(1);
                let mut l_mask: u16 = 0x0FFF;
                let mut o_shift: u32 = 12;
                while pos >= 0x10 {
                    l_mask >>= 1;
                    o_shift = o_shift.saturating_sub(1);
                    pos >>= 1;
                }
                let length = (word & l_mask) as usize + 3;
                let mut offset = (word >> o_shift) as usize + 1;
                if offset > out.len() {
                    offset = out.len();
                }
                if offset == 0 {
                    continue;
                }
                let start = out.len() - offset;
                for _ in 0..length {
                    let src = start + (out.len() - start) % offset;
                    out.push(out[src]);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Recovery
// ---------------------------------------------------------------------------

pub struct RecoverParams<'a> {
    /// Optional normalized Windows-style filter (e.g. `Documents`) limiting
    /// recovery to that subtree. `None` recovers the whole volume.
    pub filter: Option<&'a str>,
    /// Destination directory (recup_dir.<dir_num>). Created if missing.
    pub out_root: &'a Path,
}

pub struct RecoverResult {
    pub files: u64,
    pub used_ranges: Vec<ByteRange>,
}

fn path_within(parent: &str, child: &str) -> bool {
    if parent.is_empty() {
        return true;
    }
    if child.len() >= parent.len() && parent.eq_ignore_ascii_case(&child[..parent.len()]) {
        return child.len() == parent.len() || child.as_bytes()[parent.len()] == b'/';
    }
    false
}

fn sanitize_component(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c == '/' || c == '\\' || c == ':' || (c as u32) < 0x20 {
                '_'
            } else {
                c
            }
        })
        .collect()
}

/// Resolve the full path of `record` using $FILE_NAME parent chains: the
/// record's own $FILE_NAME (when `record` is not the root) followed by each
/// parent's, joined with '/'. Returns None when the chain is broken or
/// cyclic. The root yields `Some("")`. Recovery uses this with
/// `entry.parent`, so the results are directory prefixes.
fn resolve_dir_path(entries: &[Option<MftEntry>], record: u64) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut cur = record;
    for _ in 0..MAX_PATH_DEPTH {
        if cur == MFT_RECORD_ROOT {
            parts.reverse();
            return Some(parts.join("/"));
        }
        let entry = entries.get(cur as usize)?.as_ref()?;
        if entry.name.is_empty() {
            return None;
        }
        if parts.iter().any(|p| p.eq_ignore_ascii_case(&entry.name)) {
            return None; // cycle
        }
        parts.push(entry.name.clone());
        cur = entry.parent;
    }
    None
}

/// Read the payload of a `DataAttr` into memory.
fn read_data<R: Reader>(
    volume: &NtfsVolume<R>,
    data: &DataAttr,
    out: &mut Vec<u8>,
) -> io::Result<()> {
    match data {
        DataAttr::Resident(v) => out.extend_from_slice(v),
        DataAttr::Runs {
            runs,
            real_size,
            compressed,
        } => {
            let cluster_size = volume.geom.cluster_size as u64;
            let target = *real_size;
            if *compressed {
                // All runs hold compressed bytes; decompress then truncate.
                let mut raw: Vec<u8> = Vec::new();
                for r in runs {
                    if r.cluster == SPARSE {
                        continue;
                    }
                    let mut buf = vec![0u8; (r.length * cluster_size) as usize];
                    volume
                        .reader
                        .read_at(volume.cluster_to_offset(r.cluster), &mut buf)?;
                    raw.extend_from_slice(&buf);
                }
                let mut dec = lznt1_decompress(&raw);
                dec.truncate(target as usize);
                out.extend_from_slice(&dec);
            } else {
                let mut written: u64 = 0;
                for r in runs {
                    if written >= target {
                        break;
                    }
                    let chunk = r.length * cluster_size;
                    let emit = chunk.min(target - written);
                    if r.cluster == SPARSE {
                        out.resize(out.len() + emit as usize, 0);
                    } else {
                        let mut buf = vec![0u8; emit as usize];
                        volume
                            .reader
                            .read_at(volume.cluster_to_offset(r.cluster), &mut buf)?;
                        out.extend_from_slice(&buf);
                    }
                    written += emit;
                }
            }
        }
    }
    Ok(())
}

fn write_unique(path: &Path, data: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    if path.exists() {
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".to_string());
        let ext = path
            .extension()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        for i in 1..10000u32 {
            let candidate = if ext.is_empty() {
                path.with_file_name(format!("{}~{}", stem, i))
            } else {
                path.with_file_name(format!("{}~{}.{}", stem, i, ext))
            };
            if !candidate.exists() {
                return std::fs::write(candidate, data);
            }
        }
    }
    std::fs::write(path, data)
}

fn merge_ranges(mut v: Vec<ByteRange>) -> Vec<ByteRange> {
    if v.len() < 2 {
        return v;
    }
    v.sort_unstable_by_key(|r| r.offset);
    let mut merged: Vec<ByteRange> = Vec::with_capacity(v.len());
    for r in v {
        if let Some(last) = merged.last_mut() {
            if r.offset <= last.offset + last.size {
                let end = (r.offset + r.size).max(last.offset + last.size);
                last.size = end - last.offset;
                continue;
            }
        }
        merged.push(r);
    }
    merged
}

// ---------------------------------------------------------------------------
// Seeded rebuild of overwritten (stale) deleted files
// ---------------------------------------------------------------------------

/// Abstraction over the volume needed by the seeded-rebuild helpers, so they
/// can be exercised against a synthetic mock in tests instead of a full NTFS
/// image. `NtfsVolume` implements it for the real path.
pub trait Tile {
    fn cluster_size(&self) -> u64;
    fn cluster_count(&self) -> u64;
    /// Absolute byte offset of the start of `cluster`.
    fn cluster_offset(&self, cluster: u64) -> u64;
    fn is_allocated(&self, cluster: u64) -> bool;
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()>;
}

impl<R: Reader> Tile for NtfsVolume<R> {
    fn cluster_size(&self) -> u64 {
        self.geom.cluster_size as u64
    }
    fn cluster_count(&self) -> u64 {
        self.bitmap.cluster_count
    }
    fn cluster_offset(&self, cluster: u64) -> u64 {
        self.part_offset + cluster * self.geom.cluster_size as u64
    }
    fn is_allocated(&self, cluster: u64) -> bool {
        self.bitmap.is_allocated(cluster)
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        self.reader.read_at(offset, buf)
    }
}

/// Whether any cluster of a deleted file's runs is currently allocated (i.e.
/// has been overwritten by newer data).
fn runs_are_stale(tile: &impl Tile, runs: &[Run]) -> bool {
    runs.iter()
        .any(|r| r.cluster != SPARSE && (0..r.length).any(|k| tile.is_allocated(r.cluster + k)))
}

/// The intact head of a (deleted) file, taken from leading runs whose
/// clusters are still untouched in `$Bitmap`. Returns `None` when the very
/// first bytes of the file were overwritten (nothing left to seed a scan
/// with).
fn intact_head(tile: &impl Tile, data: &DataAttr) -> Option<Vec<u8>> {
    let DataAttr::Runs {
        runs, real_size, ..
    } = data
    else {
        return None;
    };
    if *real_size == 0 {
        return None;
    }
    let cs = tile.cluster_size();
    let cap = (SEED_PREFIX_MAX as u64).min(cs);
    let mut out: Vec<u8> = Vec::new();
    for r in runs {
        if r.cluster == SPARSE || !(0..r.length).all(|k| !tile.is_allocated(r.cluster + k)) {
            break;
        }
        let emit = (r.length * cs)
            .min(*real_size - out.len() as u64)
            .min(cap - out.len() as u64);
        if emit == 0 {
            break;
        }
        let mut buf = vec![0u8; emit as usize];
        tile.read_at(tile.cluster_offset(r.cluster), &mut buf)
            .ok()?;
        out.extend_from_slice(&buf);
        if out.len() >= cap as usize {
            break;
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn has_known_magic(bytes: &[u8]) -> bool {
    KNOWN_MAGICS.iter().any(|m| bytes.starts_with(m))
}

/// Scan the whole volume for a surviving copy of the deleted file, seeded by
/// its (intact) head bytes. The file's own original clusters are skipped to
/// avoid matching the stale location itself. Unallocated clusters are tried
/// first (untouched copies), then allocated ones (e.g. a re-saved version).
fn find_seeded_copy(tile: &impl Tile, seed: &[u8], skip_runs: &[Run]) -> Option<u64> {
    let cs = tile.cluster_size();
    let n = tile.cluster_count();
    let need = (seed.len() as u64).min(cs) as usize;
    let in_skip = |c: u64| {
        skip_runs
            .iter()
            .any(|r| r.cluster != SPARSE && c >= r.cluster && c < r.cluster + r.length)
    };
    for want_free in [true, false] {
        for c in 0..n {
            if in_skip(c) || tile.is_allocated(c) != !want_free {
                continue;
            }
            let mut buf = vec![0u8; need];
            if tile.read_at(tile.cluster_offset(c), &mut buf).is_err() {
                continue;
            }
            if buf[..need] == seed[..need] {
                return Some(c);
            }
        }
    }
    None
}

/// Read `size` bytes of consecutive clusters starting at `start`.
fn read_from(tile: &impl Tile, start: u64, size: u64) -> io::Result<Vec<u8>> {
    let cs = tile.cluster_size();
    let mut out = Vec::new();
    let mut written: u64 = 0;
    let mut c = start;
    while written < size {
        let emit = cs.min(size - written);
        let mut buf = vec![0u8; emit as usize];
        tile.read_at(tile.cluster_offset(c), &mut buf)?;
        out.extend_from_slice(&buf);
        written += emit;
        c += 1;
    }
    Ok(out)
}

enum Rebuild {
    /// Found and read a surviving copy of the full file.
    Full,
    /// No surviving copy, but the intact head carries a known signature; the
    /// (truncated) head is the best we can do.
    Partial,
    /// Nothing salvageable; the caller should skip the file instead of
    /// writing garbage.
    Skip,
}

/// Rebuild an overwritten deleted file: use its intact head as a seed to
/// scan the volume for a surviving full copy; otherwise fall back to writing
/// the intact head when it matches a known file signature.
fn stale_rebuild(tile: &impl Tile, data: &DataAttr, out: &mut Vec<u8>) -> Rebuild {
    let DataAttr::Runs {
        runs, real_size, ..
    } = data
    else {
        return Rebuild::Skip;
    };
    let Some(seed) = intact_head(tile, data) else {
        return Rebuild::Skip;
    };
    if let Some(copy) = find_seeded_copy(tile, &seed, runs) {
        if let Ok(bytes) = read_from(tile, copy, *real_size) {
            out.extend_from_slice(&bytes);
            return Rebuild::Full;
        }
    }
    if has_known_magic(&seed) {
        out.extend_from_slice(&seed);
        return Rebuild::Partial;
    }
    Rebuild::Skip
}

/// Recover files from the volume into `params.out_root`, honoring the optional
/// filter. Returns the number recovered plus the used byte ranges to hand to
/// the carve pass. `on_progress` is called periodically; `is_stopped` can
/// abort the pass early (already-collected used ranges are still returned).
pub fn recover<R: Reader>(
    volume: &NtfsVolume<R>,
    params: &RecoverParams,
    on_progress: &mut dyn FnMut(u64, &str),
    on_file: &mut dyn FnMut(&str, u64),
    is_stopped: &mut dyn FnMut() -> bool,
) -> io::Result<RecoverResult> {
    let count = volume.record_count;
    let batch = 256u64;
    // Normalize the Windows-style filter (`\Documents\Data`) to a plain
    // forward-slash path used by `path_within`.
    let filter = params.filter.map(|f| {
        f.trim()
            .replace('\\', "/")
            .trim_start_matches('/')
            .to_string()
    });

    // Pass 1: parse every record, building the indexed entry table (paths are
    // resolved via parent chains afterwards) and collecting used runs.
    let mut entries: Vec<Option<MftEntry>> = Vec::with_capacity(count as usize);
    let mut all_used: Vec<ByteRange> = Vec::new();
    let mut start = 0u64;
    while start < count {
        if is_stopped() {
            break;
        }
        let n = batch.min(count - start);
        let parsed = volume.read_records_bulk(start, n);
        for slot in parsed {
            if let Some(mut entry) = slot {
                for r in std::mem::take(&mut entry.used_runs) {
                    if r.cluster != SPARSE {
                        all_used.push(ByteRange {
                            offset: volume.cluster_to_offset(r.cluster),
                            size: r.length * volume.geom.cluster_size as u64,
                        });
                    }
                }
                entries.push(Some(entry));
            } else {
                entries.push(None);
            }
        }
        start += n;
        if start % 8192 == 0 {
            on_progress(start, "");
        }
    }

    // Mark the MFT file itself as used.
    for r in &volume.mft_runs {
        if r.cluster != SPARSE {
            all_used.push(ByteRange {
                offset: volume.cluster_to_offset(r.cluster),
                size: r.length * volume.geom.cluster_size as u64,
            });
        }
    }

    let used_ranges = merge_ranges(all_used);

    // Pass 2: recover every record carrying an un-named $DATA.
    let mut files: u64 = 0;
    for entry in entries.iter().flatten() {
        if is_stopped() {
            break;
        }
        if entry.is_dir || entry.name.is_empty() || entry.name.starts_with('$') {
            continue;
        }
        let Some(data) = &entry.data else {
            continue;
        };
        if data.real_size() == 0 {
            continue;
        }
        let full = match resolve_dir_path(&entries, entry.parent) {
            Some(dir) => {
                if dir.is_empty() {
                    entry.name.clone()
                } else {
                    format!("{}/{}", dir, entry.name)
                }
            }
            None => {
                if filter.is_some() {
                    continue;
                }
                format!("$DELETED/{}", entry.name)
            }
        };
        if let Some(filter) = filter.as_deref() {
            if !path_within(filter, &full) {
                continue;
            }
        }
        let mut buf: Vec<u8> = Vec::new();
        let stale = volume.bitmap.cluster_count > 0
            && !entry.in_use
            && matches!(&data, DataAttr::Runs { runs, .. } if runs_are_stale(volume, runs));
        if stale {
            match stale_rebuild(volume, data, &mut buf) {
                Rebuild::Full | Rebuild::Partial => {}
                Rebuild::Skip => continue, // overwritten, nothing salvageable
            }
        } else if read_data(volume, data, &mut buf).is_err() {
            continue;
        }
        let mut out_path = params.out_root.to_path_buf();
        for comp in full.split('/') {
            let c = sanitize_component(comp);
            if c.is_empty() {
                continue;
            }
            out_path.push(c);
        }
        if write_unique(&out_path, &buf).is_ok() {
            files += 1;
            on_file(&full, buf.len() as u64);
        }
        if files % 64 == 0 {
            on_progress(files, &full);
        }
    }
    on_progress(files, "");

    Ok(RecoverResult { files, used_ranges })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    fn boot_sector() -> Vec<u8> {
        let mut b = vec![0u8; 512];
        b[3..11].copy_from_slice(b"NTFS    ");
        b[0x0B..0x0D].copy_from_slice(&512u16.to_le_bytes());
        b[0x0D] = 8; // 8 sectors/cluster -> 4096 bytes
        b[0x28..0x30].copy_from_slice(&131071u64.to_le_bytes()); // total sectors
        b[0x30..0x38].copy_from_slice(&4u64.to_le_bytes()); // mft_lcn
        b[0x40] = 0xF6; // -10 -> 1024-byte records
        b[0x44] = 0xF7; // -9 -> 512-byte index records
        b
    }

    #[test]
    fn parse_boot_sector_ok() {
        let g = parse_boot_sector(&boot_sector(), 512).expect("valid boot sector");
        assert_eq!(g.cluster_size, 4096);
        assert_eq!(g.sector_size, 512);
        assert_eq!(g.mft_lcn, 4);
        assert_eq!(g.mft_record_size, 1024);
        assert_eq!(g.index_record_size, 512);
        assert_eq!(g.total_sectors, 131071);
    }

    #[test]
    fn parse_boot_sector_rejects_non_ntfs() {
        let b = boot_sector();
        assert!(parse_boot_sector(&b[..16], 512).is_none()); // too short
        let mut bad = b.clone();
        bad[0x03] = b'F';
        assert!(parse_boot_sector(&bad, 512).is_none());
        let mut bad = b.clone();
        bad[0x0D] = 0; // zero sectors/cluster
        assert!(parse_boot_sector(&bad, 512).is_none());
    }

    #[test]
    fn parse_mapping_pairs_single_run() {
        // VCN 0, length 19, LCN 4 (no offset byte needed: writes explicit).
        let pairs = [0x11, 0x13, 0x04];
        let runs = parse_mapping_pairs(&pairs, 0).expect("pairs");
        assert_eq!(
            runs,
            vec![Run {
                vcn: 0,
                cluster: 4,
                length: 19
            }]
        );
    }

    #[test]
    fn parse_mapping_pairs_delta_and_zero_terminator() {
        // Run 1: len 5 @ LCN 10 (header 0x11: len=1, off=1). Run 2 via signed
        // delta: len 3, LCN -= 2 (header 0x21: len=1, off=2, delta 0xFFFE).
        let pairs = [0x11, 0x05, 10, 0x21, 0x03, 0xFE, 0xFF, 0x00];
        let runs = parse_mapping_pairs(&pairs, 0).expect("pairs");
        assert_eq!(
            runs,
            vec![
                Run {
                    vcn: 0,
                    cluster: 10,
                    length: 5
                },
                Run {
                    vcn: 5,
                    cluster: 8,
                    length: 3
                },
            ]
        );
    }

    #[test]
    fn apply_fixups_restores_sector_ends() {
        let mut rec = vec![0u8; 1024];
        rec[0..4].copy_from_slice(b"FILE");
        rec[4..6].copy_from_slice(&0x18u16.to_le_bytes()); // usa_offset
        rec[6..8].copy_from_slice(&3u16.to_le_bytes()); // usa_count (2 sectors)
        rec[0x18..0x1A].copy_from_slice(&0xEEFFu16.to_le_bytes()); // signature
        rec[0x1A..0x1C].copy_from_slice(&0x1111u16.to_le_bytes()); // saved s1
        rec[0x1C..0x1E].copy_from_slice(&0x2222u16.to_le_bytes()); // saved s2
        rec[0x1FE..0x200].copy_from_slice(&0xEEFFu16.to_le_bytes()); // fixed state
        rec[0x3FE..0x400].copy_from_slice(&0xEEFFu16.to_le_bytes());
        assert!(apply_fixups(&mut rec));
        assert_eq!(u16le(&rec, 0x1FE), 0x1111, "saved value restored");
        assert_eq!(u16le(&rec, 0x3FE), 0x2222, "saved value restored");
    }

    #[test]
    fn apply_fixups_accepts_unrestored_record() {
        let mut rec = vec![0u8; 1024];
        rec[0..4].copy_from_slice(b"FILE");
        rec[4..6].copy_from_slice(&0x18u16.to_le_bytes());
        rec[6..8].copy_from_slice(&3u16.to_le_bytes());
        rec[0x18..0x1A].copy_from_slice(&0xEEFFu16.to_le_bytes());
        rec[0x1A..0x1C].copy_from_slice(&0x1111u16.to_le_bytes());
        rec[0x1C..0x1E].copy_from_slice(&0x2222u16.to_le_bytes());
        // Sector ends carry originals already.
        rec[0x1FE..0x200].copy_from_slice(&0x1111u16.to_le_bytes());
        rec[0x3FE..0x400].copy_from_slice(&0x2222u16.to_le_bytes());
        assert!(apply_fixups(&mut rec));
    }

    #[test]
    fn apply_fixups_rejects_corrupt() {
        let mut rec = vec![0u8; 1024];
        rec[0..4].copy_from_slice(b"FILE");
        rec[4..6].copy_from_slice(&0x18u16.to_le_bytes());
        rec[6..8].copy_from_slice(&3u16.to_le_bytes());
        rec[0x18..0x1A].copy_from_slice(&0xEEFFu16.to_le_bytes());
        rec[0x1A..0x1C].copy_from_slice(&0x1111u16.to_le_bytes());
        rec[0x1C..0x1E].copy_from_slice(&0x2222u16.to_le_bytes());
        rec[0x1FE] = 0x00; // matches neither signature nor saved value
        assert!(!apply_fixups(&mut rec));
    }

    #[test]
    fn lznt1_uncompressed_frame_passthrough() {
        // Header 0x000A: chunk length 11, compressed bit clear -> verbatim.
        let mut data = vec![0x0A, 0x00];
        data.extend_from_slice(b"hello world");
        assert_eq!(lznt1_decompress(&data), b"hello world");
    }

    #[test]
    fn lznt1_backref_expansion() {
        // One chunk (compressed), block with 3 literals + 1 backref to "ABC".
        let chunk = [0x08, b'A', b'B', b'C', 0x00, 0x20];
        let mut data = vec![0x05, 0x80]; // chunk_len=6, compressed bit set
        data.extend_from_slice(&chunk);
        assert_eq!(lznt1_decompress(&data), b"ABCABC");
    }

    #[test]
    fn path_within_cases() {
        assert!(path_within("", "readme.txt"));
        assert!(path_within("Documents", "Documents/report.txt"));
        assert!(path_within("documents", "DOCUMENTS/REPORT.TXT")); // insensitive
        assert!(path_within("Documents/Data", "Documents/Data/secret.bin"));
        assert!(path_within("Documents", "Documents"));
        assert!(!path_within("Doc", "Other/file"));
        assert!(!path_within("DocumentsX", "Documents/report.txt")); // no prefix bleed
    }

    #[test]
    fn sanitize_component_maps_illegal_chars() {
        assert_eq!(sanitize_component("a/b\\c:d\x01e"), "a_b_c_d_e");
        assert_eq!(sanitize_component("plain"), "plain");
        assert_eq!(sanitize_component(""), "");
    }

    #[test]
    fn merge_ranges_merges_overlapping_and_adjacent() {
        let v = vec![
            ByteRange {
                offset: 100,
                size: 50,
            }, // 100-150
            ByteRange {
                offset: 140,
                size: 20,
            }, // overlaps
            ByteRange {
                offset: 150,
                size: 10,
            }, // adjacent
            ByteRange {
                offset: 500,
                size: 8,
            }, // disjoint
        ];
        let m = merge_ranges(v);
        assert_eq!(
            m,
            vec![
                ByteRange {
                    offset: 100,
                    size: 60
                },
                ByteRange {
                    offset: 500,
                    size: 8
                },
            ]
        );
    }

    #[test]
    fn resolve_dir_path_uses_parent_chain() {
        fn dir(record: u64, name: &str, parent: u64) -> MftEntry {
            MftEntry {
                record,
                in_use: true,
                is_dir: true,
                parent,
                name: name.to_string(),
                data: None,
                used_runs: Vec::new(),
            }
        }
        let entries = vec![
            None,
            None,
            None,
            None,
            None,
            Some(dir(5, ".", 5)),         // root
            Some(dir(6, "Data", 7)),      // Data under Documents
            Some(dir(7, "Documents", 5)), // Documents under root
        ];
        assert_eq!(resolve_dir_path(&entries, 5), Some(String::new()));
        assert_eq!(resolve_dir_path(&entries, 7), Some("Documents".to_string()));
        assert_eq!(
            resolve_dir_path(&entries, 6),
            Some("Documents/Data".to_string())
        );
        // Broken chain -> None.
        assert_eq!(resolve_dir_path(&entries, 3), None);
    }

    #[test]
    fn write_unique_appends_counter_on_collision() {
        let dir = std::env::temp_dir().join("ntfs_write_unique_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.txt");
        write_unique(&p, b"one").unwrap();
        write_unique(&p, b"two").unwrap();
        write_unique(&p, b"three").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"one");
        assert_eq!(std::fs::read(dir.join("a~1.txt")).unwrap(), b"two");
        assert_eq!(std::fs::read(dir.join("a~2.txt")).unwrap(), b"three");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// End-to-end against the NTFS test image built in /tmp. Skipped (with a
    /// message) when the image is not present.
    #[test]
    fn recovery_on_real_image() {
        let img = "/tmp/opencode/ntfs.img";
        let dest = std::env::temp_dir().join("ntfs_recovery_test");
        if !std::path::Path::new(img).exists() {
            eprintln!("skipping: {img} missing");
            return;
        }
        let _ = std::fs::remove_dir_all(&dest);
        let reader = FileReader::new(Path::new(img)).expect("open image");
        let vol = NtfsVolume::open(reader, 0, 512).expect("open volume");
        let stopped = AtomicBool::new(false);
        let mut stop = || stopped.load(Ordering::Relaxed);
        let mut progress = |_: u64, _: &str| {};
        let result = recover(
            &vol,
            &RecoverParams {
                filter: None,
                out_root: &dest,
            },
            &mut progress,
            &mut |_, _| {},
            &mut stop,
        )
        .expect("recover");
        assert_eq!(result.files, 5, "4 live files + deleted gone.txt");
        for (rel, want) in [
            ("readme.txt", b"ROOT-README-CONTENT-1234567890\n".as_slice()),
            (
                "Documents/report.txt",
                b"DOCUMENTS-REPORT-LOREM-IPSUM-DOLOR\n",
            ),
            (
                "Documents/Data/secret.bin",
                b"SECRET-DATA-BINARY-BYTES-9876543210\n",
            ),
            ("MISC/other.txt", b"MISC-OTHER-UNRELATED-CONTENT\n"),
        ] {
            let got = std::fs::read(dest.join(rel)).unwrap_or_else(|_| panic!("missing {}", rel));
            assert_eq!(got, want, "content of {}", rel);
        }
        assert!(!result.used_ranges.is_empty(), "carve must skip used space");
        let _ = std::fs::remove_dir_all(&dest);
    }

    #[test]
    fn recovery_with_filter_on_real_image() {
        let img = "/tmp/opencode/ntfs.img";
        let dest = std::env::temp_dir().join("ntfs_filter_test");
        if !std::path::Path::new(img).exists() {
            eprintln!("skipping: {img} missing");
            return;
        }
        let _ = std::fs::remove_dir_all(&dest);
        let reader = FileReader::new(Path::new(img)).unwrap();
        let vol = NtfsVolume::open(reader, 0, 512).unwrap();
        let stopped = AtomicBool::new(false);
        let result = recover(
            &vol,
            &RecoverParams {
                filter: Some("\\Documents\\Data"),
                out_root: &dest,
            },
            &mut |_, _| {},
            &mut |_, _| {},
            &mut || stopped.load(Ordering::Relaxed),
        )
        .unwrap();
        assert_eq!(result.files, 1);
        let got = std::fs::read(dest.join("Documents/Data/secret.bin")).unwrap();
        assert_eq!(got, b"SECRET-DATA-BINARY-BYTES-9876543210\n");
        let _ = std::fs::remove_dir_all(&dest);
    }

    // -- Seeded rebuild (synthetic cluster tile) ------------------------------

    /// Tiniest possible volume mock: fixed-size clusters backed by a byte
    /// buffer, with a per-cluster allocation map.
    struct FakeTile {
        bytes: Vec<u8>,
        cs: u64,
        alloc: Vec<bool>,
    }

    impl FakeTile {
        fn new(cs: u64, alloc: &[bool]) -> Self {
            let n = alloc.len() as u64;
            Self {
                bytes: vec![0u8; (n * cs) as usize],
                cs,
                alloc: alloc.to_vec(),
            }
        }
        fn set(&mut self, cluster: u64, data: &[u8]) {
            let start = (cluster * self.cs) as usize;
            self.bytes[start..start + data.len()].copy_from_slice(data);
        }
    }

    impl Tile for FakeTile {
        fn cluster_size(&self) -> u64 {
            self.cs
        }
        fn cluster_count(&self) -> u64 {
            self.alloc.len() as u64
        }
        fn cluster_offset(&self, cluster: u64) -> u64 {
            cluster * self.cs
        }
        fn is_allocated(&self, cluster: u64) -> bool {
            self.alloc.get(cluster as usize).copied().unwrap_or(true)
        }
        fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
            let o = offset as usize;
            let end = o + buf.len();
            if end > self.bytes.len() {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "EOF"));
            }
            buf.copy_from_slice(&self.bytes[o..end]);
            Ok(())
        }
    }

    const FAKE_CS: u64 = 8; // tiny clusters for compact fixtures

    #[test]
    fn runs_are_stale_only_when_bitmap_allocates() {
        let tile = FakeTile::new(FAKE_CS, &[false, true, false]);
        let runs = vec![Run {
            vcn: 0,
            cluster: 0,
            length: 1,
        }];
        assert!(!runs_are_stale(&tile, &runs));
        let runs = vec![Run {
            vcn: 0,
            cluster: 1,
            length: 1,
        }];
        assert!(runs_are_stale(&tile, &runs));
        // Partially allocated run is stale too.
        let runs = vec![Run {
            vcn: 0,
            cluster: 1,
            length: 2,
        }];
        assert!(runs_are_stale(&tile, &runs));
    }

    #[test]
    fn intact_head_stops_at_overwritten_run() {
        let mut tile = FakeTile::new(FAKE_CS, &[false, true, false, false]);
        tile.set(0, b"HEAD#$AB");
        tile.set(1, b"STALEZZZ"); // stale, ignored
        tile.set(3, b"BODYZZZZ");
        let data = DataAttr::Runs {
            runs: vec![
                Run {
                    vcn: 0,
                    cluster: 0,
                    length: 1,
                },
                Run {
                    vcn: 1,
                    cluster: 1,
                    length: 1,
                },
                Run {
                    vcn: 2,
                    cluster: 3,
                    length: 1,
                },
            ],
            real_size: 24,
            compressed: false,
        };
        // Head stops at the stale run 1 -> only cluster 0.
        assert_eq!(intact_head(&tile, &data), Some(b"HEAD#$AB".to_vec()));
        // Overwriting the very first cluster leaves nothing to seed.
        let data2 = DataAttr::Runs {
            runs: vec![Run {
                vcn: 0,
                cluster: 1,
                length: 1,
            }],
            real_size: 8,
            compressed: false,
        };
        assert!(intact_head(&tile, &data2).is_none());
    }

    #[test]
    fn find_seeded_copy_prefers_free_and_skips_originals() {
        let mut tile = FakeTile::new(FAKE_CS, &[true, false, false, true, false]);
        tile.set(1, b"SEED0000"); // free copy (but it's the stale original)
        tile.set(2, b"SEED0000"); // free copy
        tile.set(4, b"SEED0000"); // allocated copy
        let skip = vec![Run {
            vcn: 0,
            cluster: 1,
            length: 1,
        }];
        let seed = b"SEED0000".to_vec();
        // Cluster 1 is skipped, cluster 2 is the first free match.
        assert_eq!(find_seeded_copy(&tile, &seed, &skip), Some(2));
        // With the free match gone it falls back to an allocated copy.
        tile.alloc[2] = true;
        assert_eq!(find_seeded_copy(&tile, &seed, &skip), Some(4));
    }

    #[test]
    fn stale_rebuild_finds_full_copy() {
        let mut tile = FakeTile::new(FAKE_CS, &[false, true, false, false, false]);
        // Original file: cluster 0 intact, cluster 1 overwritten.
        tile.set(0, b"\x89PNG\r\n\x1a\n");
        tile.set(1, b"GARBAGE!");
        // A surviving full copy at clusters 3..4 (free).
        tile.set(3, b"\x89PNG\r\n\x1a\n");
        tile.set(4, b"PART2XY!");
        let data = DataAttr::Runs {
            runs: vec![
                Run {
                    vcn: 0,
                    cluster: 0,
                    length: 1,
                },
                Run {
                    vcn: 1,
                    cluster: 1,
                    length: 1,
                },
            ],
            real_size: 16,
            compressed: false,
        };
        assert!(runs_are_stale(&tile, &runs_of(&data)));
        let mut out = Vec::new();
        assert!(matches!(
            stale_rebuild(&tile, &data, &mut out),
            Rebuild::Full
        ));
        assert_eq!(out, b"\x89PNG\r\n\x1a\nPART2XY!");
    }

    #[test]
    fn stale_rebuild_partial_when_only_head_survives() {
        let mut tile = FakeTile::new(FAKE_CS, &[false, true]);
        tile.set(0, b"\x89PNG\r\n\x1a\n");
        tile.set(1, b"GARBAGE!");
        let data = DataAttr::Runs {
            runs: vec![
                Run {
                    vcn: 0,
                    cluster: 0,
                    length: 1,
                },
                Run {
                    vcn: 1,
                    cluster: 1,
                    length: 1,
                },
            ],
            real_size: 16,
            compressed: false,
        };
        let mut out = Vec::new();
        assert!(matches!(
            stale_rebuild(&tile, &data, &mut out),
            Rebuild::Partial
        ));
        assert_eq!(out, b"\x89PNG\r\n\x1a\n".to_vec());
    }

    #[test]
    fn stale_rebuild_skips_when_head_overwritten_or_unknown() {
        // Head overwritten -> nothing to seed.
        let mut tile = FakeTile::new(FAKE_CS, &[true, false]);
        tile.set(0, b"GARBAGE!");
        tile.set(1, b"PARABLES");
        let data = DataAttr::Runs {
            runs: vec![Run {
                vcn: 0,
                cluster: 0,
                length: 1,
            }],
            real_size: 8,
            compressed: false,
        };
        let mut out = Vec::new();
        assert!(matches!(
            stale_rebuild(&tile, &data, &mut out),
            Rebuild::Skip
        ));
        // Intact head but no known signature and no copy -> skip (no garbage).
        let mut tile = FakeTile::new(FAKE_CS, &[false]);
        tile.set(0, b"PLAINTX1");
        let data = DataAttr::Runs {
            runs: vec![Run {
                vcn: 0,
                cluster: 0,
                length: 1,
            }],
            real_size: 8,
            compressed: false,
        };
        out.clear();
        assert!(matches!(
            stale_rebuild(&tile, &data, &mut out),
            Rebuild::Skip
        ));
    }

    fn runs_of(data: &DataAttr) -> Vec<Run> {
        match data {
            DataAttr::Runs { runs, .. } => runs.clone(),
            DataAttr::Resident(_) => Vec::new(),
        }
    }
}
