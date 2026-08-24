use crossbeam_channel::Sender;
use miniz_oxide::inflate::decompress_to_vec_zlib;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FileType {
    Unknown,
    JPEG,
    PNG,
    GIF,
    PDF,
    ZIP,
    BMP,
    RIFF,
    ELF,
    MP3,
    FLAC,
    TIFF,
    PSD,
    EXE,
    JavaClass,
}

impl FileType {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Unknown => "Unknown",
            Self::JPEG => "JPEG Image",
            Self::PNG => "PNG Image",
            Self::GIF => "GIF Image",
            Self::PDF => "PDF Document",
            Self::ZIP => "ZIP Archive",
            Self::BMP => "BMP Image",
            Self::RIFF => "RIFF (AVI/WAV)",
            Self::ELF => "ELF Binary",
            Self::MP3 => "MP3 Audio",
            Self::FLAC => "FLAC Audio",
            Self::TIFF => "TIFF Image",
            Self::PSD => "Photoshop Document",
            Self::EXE => "Windows Executable",
            Self::JavaClass => "Java Class File",
        }
    }

    pub fn header_hex(&self) -> &'static str {
        match self {
            Self::JPEG => "FF D8 FF E0/E1",
            Self::PNG => "89 50 4E 47 0D 0A 1A 0A",
            Self::GIF => "47 49 46 38 39/37 61",
            Self::PDF => "25 50 44 46",
            Self::ZIP => "50 4B 03 04",
            Self::BMP => "42 4D",
            Self::RIFF => "52 49 46 46",
            Self::ELF => "7F 45 4C 46",
            Self::MP3 => "49 44 33",
            Self::FLAC => "66 4C 61 43",
            Self::TIFF => "49 49 2A 00 / 4D 4D 00 2A",
            Self::PSD => "38 42 50 53",
            Self::EXE => "4D 5A",
            Self::JavaClass => "CA FE BA BE",
            Self::Unknown => "\u{2014}",
        }
    }

    pub fn footer_hex(&self) -> &'static str {
        match self {
            Self::JPEG => "FF D9",
            Self::PNG => "49 45 4E 44 AE 42 60 82",
            Self::GIF => "00 3B",
            Self::PDF => "25 25 45 4F 46",
            Self::Unknown => "\u{2014}",
            _ => "\u{2014}",
        }
    }

    pub fn can_repair(&self) -> bool {
        matches!(
            self,
            Self::JPEG | Self::PNG | Self::GIF | Self::PDF | Self::Unknown
        )
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct FileAnalysis {
    pub file_type: FileType,
    pub has_header: bool,
    pub has_footer: bool,
    pub embedded_offset: Option<usize>,
    pub details: String,
    pub total_size: u64,
    pub payload: PayloadStatus,
}

/// State of the image data payload (scan/entropy data for JPEG, IDAT stream
/// for PNG). When the payload is missing, corrupted or shifted the image
/// content itself cannot be recovered, so a structural header/footer repair
/// would only produce a file that opens as blank/garbage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PayloadStatus {
    /// Not applicable (non-image formats or undetectable).
    #[default]
    Unknown,
    /// Payload present and structurally/decodably sound.
    Ok,
    /// No payload data at all.
    Missing,
    /// Payload present but undecodable.
    Corrupt,
    /// Payload present but offset by a few bytes (alignment shift).
    Shifted,
}

impl PayloadStatus {
    pub fn is_recoverable(&self) -> bool {
        matches!(self, Self::Unknown | Self::Ok)
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Unknown => "\u{2014}",
            Self::Ok => "ok",
            Self::Missing => "missing",
            Self::Corrupt => "corrupt",
            Self::Shifted => "shifted",
        }
    }

    pub fn warn_text(&self) -> Option<&'static str> {
        match self {
            Self::Ok | Self::Unknown => None,
            Self::Missing => Some("Image data payload is missing \u{2014} cannot be repaired"),
            Self::Corrupt => Some("Image data payload is corrupted \u{2014} cannot be repaired"),
            Self::Shifted => Some("Image data payload is shifted \u{2014} cannot be repaired"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RepairProgress {
    pub percent: f64,
    pub message: String,
}

#[derive(Debug, Clone)]
pub enum RepairEvent {
    Started,
    Progress(RepairProgress),
    Log(String),
    Complete { output_path: String, size: u64 },
    Error(String),
}

const PNG_SIG: &[u8] = b"\x89PNG\r\n\x1a\n";

pub fn detect_type(data: &[u8]) -> FileType {
    if data.len() < 4 {
        return FileType::Unknown;
    }
    if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return FileType::JPEG;
    }
    if data.starts_with(PNG_SIG) {
        return FileType::PNG;
    }
    if data.starts_with(b"GIF8") {
        return FileType::GIF;
    }
    if data.starts_with(b"%PDF") {
        return FileType::PDF;
    }
    if data.starts_with(&[0x50, 0x4B, 0x03, 0x04])
        || data.starts_with(&[0x50, 0x4B, 0x05, 0x06])
        || data.starts_with(&[0x50, 0x4B, 0x07, 0x08])
    {
        return FileType::ZIP;
    }
    if data.starts_with(b"BM") {
        return FileType::BMP;
    }
    if data.starts_with(b"RIFF") {
        return FileType::RIFF;
    }
    if data.starts_with(&[0x7F, b'E', b'L', b'F']) {
        return FileType::ELF;
    }
    if data.starts_with(b"ID3") {
        return FileType::MP3;
    }
    if data.starts_with(b"fLaC") {
        return FileType::FLAC;
    }
    if data.starts_with(b"II\x2a\x00") || data.starts_with(b"MM\x00\x2a") {
        return FileType::TIFF;
    }
    if data.starts_with(b"8BPS") {
        return FileType::PSD;
    }
    if data.starts_with(b"MZ") {
        return FileType::EXE;
    }
    if data.len() >= 4 && data[..4] == [0xCA, 0xFE, 0xBA, 0xBE] {
        return FileType::JavaClass;
    }
    FileType::Unknown
}

pub fn analyze_file(path: &str) -> Result<FileAnalysis, String> {
    let data = fs::read(path).map_err(|e| format!("Cannot read file: {}", e))?;
    Ok(analyze_bytes(&data))
}

fn analyze_bytes(data: &[u8]) -> FileAnalysis {
    let total_size = data.len() as u64;
    let ft = detect_type(data);

    if matches!(ft, FileType::Unknown) {
        let embedded = find_embedded_png(data);
        if embedded.is_some() {
            return FileAnalysis {
                file_type: ft,
                has_header: false,
                has_footer: false,
                embedded_offset: embedded,
                details: png_chunks_desc(data),
                total_size,
                payload: png_payload_status(data, embedded),
            };
        }
        if let Some(jstart) = find_jpeg_start(data) {
            let info = scan_jpeg(data, jstart);
            return FileAnalysis {
                file_type: FileType::JPEG,
                has_header: jstart == 0,
                has_footer: info.terminated,
                embedded_offset: Some(jstart),
                details: jpeg_details(&info),
                total_size,
                payload: jpeg_payload_from_scan(data, jstart),
            };
        }
        if let Some(bstart) = find_bmp_start(data) {
            let info = scan_bmp(&data[bstart..]);
            return FileAnalysis {
                file_type: FileType::BMP,
                has_header: bstart == 0,
                has_footer: false,
                embedded_offset: Some(bstart),
                details: bmp_details(&info),
                total_size,
                payload: PayloadStatus::Unknown,
            };
        }
        if let Some(rstart) = find_riff_start(data) {
            let info = scan_riff(&data[rstart..]);
            return FileAnalysis {
                file_type: FileType::RIFF,
                has_header: rstart == 0,
                has_footer: false,
                embedded_offset: Some(rstart),
                details: riff_details(&info),
                total_size,
                payload: PayloadStatus::Unknown,
            };
        }
        return FileAnalysis {
            file_type: ft,
            has_header: false,
            has_footer: false,
            embedded_offset: None,
            details: String::new(),
            total_size,
            payload: PayloadStatus::Unknown,
        };
    }

    let has_header = true;
    let (has_footer, embedded, details, payload) = match ft {
        FileType::JPEG => {
            let info = scan_jpeg(data, 0);
            (
                info.terminated,
                Some(0),
                jpeg_details(&info),
                jpeg_payload_from_scan(data, 0),
            )
        }
        FileType::PNG => {
            let off = find_embedded_png(data);
            (
                check_footer(data, ft),
                off,
                png_chunks_desc(data),
                png_payload_status(data, off),
            )
        }
        FileType::BMP => {
            let info = scan_bmp(data);
            let complete = info.declared_size == data.len()
                && info.pixel_end.map_or(false, |e| e <= data.len());
            (
                complete,
                Some(0),
                bmp_details(&info),
                PayloadStatus::Unknown,
            )
        }
        FileType::RIFF => {
            let info = scan_riff(data);
            let complete = info.last_chunk_end == Some(data.len());
            (
                complete,
                Some(0),
                riff_details(&info),
                PayloadStatus::Unknown,
            )
        }
        _ => (
            check_footer(data, ft),
            None,
            String::new(),
            PayloadStatus::Unknown,
        ),
    };

    FileAnalysis {
        file_type: ft,
        has_header,
        has_footer,
        embedded_offset: embedded,
        details,
        total_size,
        payload,
    }
}

fn png_chunks_desc(data: &[u8]) -> String {
    let chunks = find_any_png_chunks(data);
    let desc: Vec<String> = chunks
        .iter()
        .map(|(pos, name)| {
            if *name == "IHDR" {
                if let Some((w, h, _, _)) = try_extract_ihdr(data, *pos) {
                    format!("{} ({}x{})", name, w, h)
                } else {
                    name.to_string()
                }
            } else {
                name.to_string()
            }
        })
        .collect();
    desc.join(", ")
}

fn check_footer(data: &[u8], ft: FileType) -> bool {
    match ft {
        FileType::JPEG => {
            let f = &[0xFF, 0xD9];
            data.len() >= f.len() && data[data.len() - f.len()..] == *f
        }
        FileType::PNG => {
            let f = b"IEND\xae\x42\x60\x82";
            data.len() >= f.len() && data[data.len() - f.len()..] == *f
        }
        FileType::GIF => {
            let f = &[0x00, 0x3B];
            data.len() >= f.len() && data[data.len() - f.len()..] == *f
        }
        FileType::PDF => {
            let f = b"%%EOF";
            data.len() >= f.len() && data[data.len() - f.len()..] == *f
        }
        _ => true,
    }
}

const PNG_CHUNK_TYPES: &[(&[u8; 4], &str)] = &[
    (b"IHDR", "IHDR"),
    (b"IDAT", "IDAT"),
    (b"IEND", "IEND"),
    (b"PLTE", "PLTE"),
    (b"tEXt", "tEXt"),
    (b"zTXt", "zTXt"),
    (b"iTXt", "iTXt"),
    (b"tRNS", "tRNS"),
    (b"gAMA", "gAMA"),
    (b"cHRM", "cHRM"),
    (b"sRGB", "sRGB"),
    (b"iCCP", "iCCP"),
    (b"pHYs", "pHYs"),
    (b"bKGD", "bKGD"),
];

const PNG_EMBEDDED_TYPES: &[&[u8; 4]] = &[b"IHDR", b"IDAT", b"IEND", b"PLTE", b"tEXt"];

fn chunk_type_lookup<'a>() -> std::collections::HashMap<u32, &'a str> {
    PNG_CHUNK_TYPES
        .iter()
        .map(|(sig, name)| (u32::from_le_bytes(**sig), *name))
        .collect()
}

fn find_embedded_png(data: &[u8]) -> Option<usize> {
    if data.len() < 16 {
        return None;
    }
    for i in 1..data.len().saturating_sub(7) {
        if data[i..].starts_with(PNG_SIG) {
            return Some(i);
        }
    }
    let lookup: std::collections::HashMap<u32, ()> = PNG_EMBEDDED_TYPES
        .iter()
        .map(|sig| (u32::from_le_bytes(**sig), ()))
        .collect();
    let limit = data.len().saturating_sub(3);
    let mut i = 0;
    while i < limit {
        if i >= 4 {
            let word = u32::from_le_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]);
            if lookup.contains_key(&word) {
                let claimed =
                    u32::from_be_bytes([data[i - 4], data[i - 3], data[i - 2], data[i - 1]]);
                if claimed > 0 && claimed < 1_000_000 {
                    return Some(i - 4);
                }
            }
        }
        i += 1;
    }
    None
}

fn find_any_png_chunks(data: &[u8]) -> Vec<(usize, &'static str)> {
    let lookup = chunk_type_lookup();
    let mut chunks = Vec::new();
    if data.len() < 4 {
        return chunks;
    }
    let limit = data.len() - 3;
    let mut i = 0;
    while i < limit {
        let word = u32::from_le_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]);
        if let Some(name) = lookup.get(&word) {
            chunks.push((i, *name));
        }
        i += 1;
    }
    chunks
}

type IhdrFields = (u32, u32, u8, u8);

fn try_extract_ihdr(data: &[u8], ihdr_pos: usize) -> Option<IhdrFields> {
    let data_start = ihdr_pos + 4;
    if data_start + 13 <= data.len() {
        let w = u32::from_be_bytes([
            data[data_start],
            data[data_start + 1],
            data[data_start + 2],
            data[data_start + 3],
        ]);
        let h = u32::from_be_bytes([
            data[data_start + 4],
            data[data_start + 5],
            data[data_start + 6],
            data[data_start + 7],
        ]);
        if w > 0 && w <= 65536 && h > 0 && h <= 65536 {
            return Some((w, h, data[data_start + 8], data[data_start + 9]));
        }
    }
    if ihdr_pos >= 4 {
        let len_start = ihdr_pos - 4;
        if len_start + 8 <= data.len() {
            let claimed_len = u32::from_be_bytes([
                data[len_start],
                data[len_start + 1],
                data[len_start + 2],
                data[len_start + 3],
            ]);
            if claimed_len == 13 && len_start + 4 + 4 + 13 + 4 <= data.len() {
                let w = u32::from_be_bytes([
                    data[len_start + 8],
                    data[len_start + 9],
                    data[len_start + 10],
                    data[len_start + 11],
                ]);
                let h = u32::from_be_bytes([
                    data[len_start + 12],
                    data[len_start + 13],
                    data[len_start + 14],
                    data[len_start + 15],
                ]);
                if w > 0 && w <= 65536 && h > 0 && h <= 65536 {
                    return Some((w, h, data[len_start + 16], data[len_start + 17]));
                }
            }
        }
    }
    None
}

fn calc_crc(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFFFFFF;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB88320;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}

fn build_ihdr(width: u32, height: u32, bit_depth: u8, color_type: u8) -> Vec<u8> {
    let mut data = Vec::with_capacity(25);
    data.extend_from_slice(&(13u32).to_be_bytes());
    data.extend_from_slice(b"IHDR");
    data.extend_from_slice(&width.to_be_bytes());
    data.extend_from_slice(&height.to_be_bytes());
    data.push(bit_depth);
    data.push(color_type);
    data.push(0);
    data.push(0);
    data.push(0);
    let crc = calc_crc(&data[4..]);
    data.extend_from_slice(&crc.to_be_bytes());
    data
}

fn build_iend() -> Vec<u8> {
    let mut data = Vec::with_capacity(12);
    data.extend_from_slice(&(0u32).to_be_bytes());
    data.extend_from_slice(b"IEND");
    let crc = calc_crc(&data[4..]);
    data.extend_from_slice(&crc.to_be_bytes());
    data
}

fn has_valid_iend(data: &[u8]) -> bool {
    if data.len() < 12 {
        return false;
    }
    if data.ends_with(b"IEND\xae\x42\x60\x82") {
        return true;
    }
    let iend_sig = b"IEND";
    for i in (8..data.len().saturating_sub(3)).rev() {
        if data[i..].starts_with(iend_sig) {
            return true;
        }
    }
    false
}

fn extract_idat_data(data: &[u8], start: usize) -> Option<Vec<u8>> {
    let mut compressed = Vec::new();
    let mut pos = start;
    loop {
        if pos + 12 > data.len() {
            break;
        }
        let chunk_len =
            u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;
        if chunk_len == 0 || chunk_len > 10_000_000 {
            break;
        }
        if pos + 12 + chunk_len > data.len() {
            break;
        }
        if &data[pos + 4..pos + 8] != b"IDAT" {
            break;
        }
        compressed.extend_from_slice(&data[pos + 8..pos + 8 + chunk_len]);
        pos += 12 + chunk_len;
    }
    if compressed.is_empty() {
        return None;
    }
    decompress_to_vec_zlib(&compressed).ok()
}

/// Classify the state of the PNG image payload (the concatenated zlib stream
/// carried by the IDAT chunks). Checks for a missing stream, an undecodable
/// (corrupt) stream, and a stream whose real start is a few bytes past the
/// natural one (alignment shift).
fn png_payload_status(data: &[u8], offset: Option<usize>) -> PayloadStatus {
    let start = offset.unwrap_or(0);
    if start >= data.len() {
        return PayloadStatus::Missing;
    }
    let offset_start = if data[start..].starts_with(PNG_SIG) {
        start + 8
    } else {
        start
    };
    let mut compressed = Vec::new();
    let mut found = false;
    let mut i = offset_start;
    while i + 8 <= data.len() {
        if &data[i + 4..i + 8] == b"IDAT" {
            let clen =
                u32::from_be_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]) as usize;
            if clen == 0 || clen > 10_000_000 || i + 12 + clen > data.len() {
                break;
            }
            found = true;
            compressed.extend_from_slice(&data[i + 8..i + 8 + clen]);
            i += 12 + clen;
        } else {
            i += 1;
        }
    }
    if !found || compressed.is_empty() {
        return PayloadStatus::Missing;
    }
    if decompress_to_vec_zlib(&compressed).is_ok() {
        return PayloadStatus::Ok;
    }
    for shift in 1..=8 {
        if compressed.len() <= shift {
            break;
        }
        if decompress_to_vec_zlib(&compressed[shift..]).is_ok() {
            return PayloadStatus::Shifted;
        }
    }
    PayloadStatus::Corrupt
}

fn estimate_ihdr(decompressed: &[u8]) -> (u32, u32, u8, u8) {
    let s = decompressed.len() as u64;
    let candidates: &[(u8, u64)] = &[(2, 3), (6, 4)];
    let aspects: &[(u64, u64)] = &[(16, 9), (4, 3), (3, 2), (16, 10), (1, 1), (5, 4)];
    let mut best: Option<(u32, u32, u8, u8, u64)> = None;

    for &(ct, ch) in candidates {
        for w in (16..=16384).step_by(8) {
            let stride = 1 + w as u64 * ch;
            if s % stride != 0 {
                continue;
            }
            let h = s / stride;
            if h < 1 || h > 65536 {
                continue;
            }
            let mut best_dist = u64::MAX;
            for &(aw, ah) in aspects {
                let ratio_num = (w as u64) * ah;
                let ratio_den = h * aw;
                let dist = if ratio_num > ratio_den {
                    ratio_num - ratio_den
                } else {
                    ratio_den - ratio_num
                };
                if dist < best_dist {
                    best_dist = dist;
                }
            }
            let score = best_dist;
            let replace = match best {
                Some((_, _, _, _, old_score)) => {
                    score < old_score || (score == old_score && w > 500 && w > best.unwrap().0)
                }
                None => true,
            };
            if replace {
                best = Some((w as u32, h as u32, 8, ct, score));
            }
        }
    }

    match best {
        Some((w, h, bd, ct, _)) => (w, h, bd, ct),
        None => (1, 1, 8, 6),
    }
}

fn repair_png(data: &[u8], known_offset: Option<usize>) -> Option<Vec<u8>> {
    let offset = known_offset.or_else(|| find_embedded_png(data));
    if let Some(off) = offset {
        let mut out = Vec::new();
        out.extend_from_slice(PNG_SIG);
        let has_sig = data[off..].starts_with(PNG_SIG);
        let payload = if has_sig {
            &data[off + 8..]
        } else {
            &data[off..]
        };
        let genuine_ihdr = find_chunk(payload, b"IHDR").filter(|_| off < 1000);
        if let Some(ihdr_pos) = genuine_ihdr {
            if let Some((w, h, bd, ct)) = try_extract_ihdr(payload, ihdr_pos) {
                let rebuilt = build_ihdr(w, h, bd, ct);
                out.extend_from_slice(&rebuilt);
            } else {
                let len_start = ihdr_pos.saturating_sub(4);
                out.extend_from_slice(&payload[len_start..ihdr_pos + 4 + 13]);
                let crc = calc_crc(&payload[ihdr_pos..ihdr_pos + 4 + 13]);
                out.extend_from_slice(&crc.to_be_bytes());
            }
            let after_ihdr = ihdr_pos + 4 + 13 + 4;
            if after_ihdr < payload.len() {
                out.extend_from_slice(&payload[after_ihdr..]);
            }
        } else if let Some(_idat_pos) = find_chunk(payload, b"IDAT") {
            let (w, h, bd, ct) = extract_idat_data(data, off)
                .as_deref()
                .map(estimate_ihdr)
                .unwrap_or((1, 1, 8, 6));
            let ihdr = build_ihdr(w, h, bd, ct);
            out.extend_from_slice(&ihdr);
            copy_metadata_before(data, off, &mut out);
            if off < data.len() {
                out.extend_from_slice(&data[off..]);
            }
        } else {
            let chunk_start = if has_sig { off + 8 } else { off };
            if chunk_start < data.len() {
                out.extend_from_slice(&data[chunk_start..]);
            }
        }
        if !has_valid_iend(&out) {
            out.extend_from_slice(&build_iend());
        }
        return Some(out);
    }

    let chunks = find_any_png_chunks(data);
    if chunks.is_empty() {
        return None;
    }

    let mut out = Vec::new();
    out.extend_from_slice(PNG_SIG);

    let ihdr_chunk = chunks
        .iter()
        .find(|(pos, name)| *name == "IHDR" && *pos < 1000);
    if let Some(&(pos, _)) = ihdr_chunk {
        if let Some((w, h, bd, ct)) = try_extract_ihdr(data, pos) {
            let rebuilt = build_ihdr(w, h, bd, ct);
            out.extend_from_slice(&rebuilt);
        } else {
            let start = pos.saturating_sub(4);
            let chunk_end = (pos + 4 + 13 + 4).min(data.len());
            out.extend_from_slice(&data[start..chunk_end]);
        }
        let after_ihdr = (pos + 4 + 13 + 4).max(pos.saturating_sub(4) + 4 + 4 + 13 + 4);
        if after_ihdr < data.len() {
            out.extend_from_slice(&data[after_ihdr..]);
        }
    } else if let Some(&(idat_pos, _)) = chunks.iter().find(|(_, name)| *name == "IDAT") {
        let chunk_start = idat_pos.saturating_sub(4);
        let (w, h, bd, ct) = extract_idat_data(data, chunk_start)
            .as_deref()
            .map(estimate_ihdr)
            .unwrap_or((1, 1, 8, 6));
        let ihdr = build_ihdr(w, h, bd, ct);
        out.extend_from_slice(&ihdr);
        copy_metadata_before(&data, chunk_start, &mut out);
        out.extend_from_slice(&data[chunk_start..]);
    } else {
        return None;
    }

    if !has_valid_iend(&out) {
        out.extend_from_slice(&build_iend());
    }
    Some(out)
}

fn copy_metadata_before(data: &[u8], offset: usize, out: &mut Vec<u8>) {
    if offset > 8 && offset <= data.len() {
        let before = &data[8..offset];
        let meta = find_any_png_chunks(before);
        let mut valid: Vec<(usize, usize)> = meta
            .iter()
            .filter_map(|&(pos, _)| {
                if pos >= 4 {
                    let len = u32::from_be_bytes([
                        before[pos - 4],
                        before[pos - 3],
                        before[pos - 2],
                        before[pos - 1],
                    ]);
                    let chunk_end = pos - 4 + 4 + 4 + len as usize + 4;
                    if len > 0 && len < 1_000_000 && chunk_end <= before.len() {
                        return Some((pos - 4, chunk_end));
                    }
                }
                None
            })
            .collect();
        valid.sort();
        valid.dedup();
        let mut last_end = 0usize;
        for (start, end) in &valid {
            if *start >= last_end && *end <= before.len() {
                out.extend_from_slice(&before[*start..*end]);
                last_end = *end;
            }
        }
    }
}

fn find_chunk(data: &[u8], chunk_type: &[u8]) -> Option<usize> {
    if data.len() < 4 {
        return None;
    }
    for i in 0..data.len().saturating_sub(3) {
        if data[i..].starts_with(chunk_type) {
            return Some(i);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// JPEG repair
// ---------------------------------------------------------------------------

const JPEG_SOI: &[u8] = &[0xFF, 0xD8];
const JPEG_EOI: &[u8] = &[0xFF, 0xD9];

#[derive(Debug, Clone, Default)]
struct JpegInfo {
    image_end: Option<usize>,
    width: Option<u32>,
    height: Option<u32>,
    has_sos: bool,
    has_dqt: bool,
    has_dht: bool,
    terminated: bool,
    marker_count: usize,
    /// Number of bytes of entropy-coded scan data walked after SOS.
    entropy_bytes: usize,
    /// Set when a segment length or marker overran the buffer mid-scan,
    /// i.e. the payload marker stream is structurally broken.
    seg_overrun: bool,
}

fn is_jpeg_segment_marker(code: u8) -> bool {
    matches!(code,
        0xC0..=0xCF | 0xDA | 0xDB | 0xDC | 0xDD | 0xDE | 0xDF | 0xE0..=0xEF | 0xFE)
}

fn is_jpeg_sof(code: u8) -> bool {
    matches!(code, 0xC0..=0xCF) && !matches!(code, 0xC4 | 0xC8 | 0xCC)
}

/// Scan entropy-coded data from `start` until a real marker or EOF.
/// Skips stuffed bytes (FF 00), padding (FF FF), and RST markers (FF D0-D7).
fn find_entropy_end(data: &[u8], start: usize) -> usize {
    let len = data.len();
    let mut i = start;
    while i < len {
        if data[i] != 0xFF {
            i += 1;
            continue;
        }
        let nxt = if i + 1 < len { data[i + 1] } else { return len };
        if nxt == 0x00 || nxt == 0xFF {
            i += 2;
            continue;
        }
        if (0xD0..=0xD7).contains(&nxt) {
            i += 2;
            continue;
        }
        return i;
    }
    len
}

/// Like `find_entropy_end`, but tolerant of stray FF bytes inserted into the
/// scan. In a valid JPEG scan the entropy never contains a standalone FF xx
/// (all such bytes are FF 00 stuffed), so any FF xx here is either the real
/// end-of-scan marker or a corrupted/inserted byte. We only stop when the FF xx
/// is followed by a structurally valid segment (matching length, fitting the
/// buffer) or an EOI; otherwise we treat it as noise and skip the FF byte.
fn find_entropy_end_tolerant(data: &[u8], start: usize) -> usize {
    let len = data.len();
    let mut i = start;
    while i < len {
        if data[i] != 0xFF {
            i += 1;
            continue;
        }
        let nxt = if i + 1 < len { data[i + 1] } else { return len };
        if nxt == 0x00 || nxt == 0xFF || (0xD0..=0xD7).contains(&nxt) {
            i += 2;
            continue;
        }
        // Candidate real boundary.
        if nxt == 0xD9 {
            return i; // EOI
        }
        if i + 3 < len {
            let seg_len = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
            if seg_len >= 2 && i + 2 + seg_len <= len {
                return i; // valid segment marker
            }
        }
        i += 1; // noise: skip the stray FF and keep scanning
    }
    len
}

/// When a segment's declared length is corrupt, scan forward to find the next
/// valid JPEG marker. Returns the byte offset of its FF byte, or None.
fn skip_to_next_marker(data: &[u8], mut pos: usize) -> Option<usize> {
    let len = data.len();
    while pos + 1 < len {
        if data[pos] == 0xFF {
            let code = data[pos + 1];
            if code != 0x00 && (is_jpeg_segment_marker(code) || code == 0xD9) {
                return Some(pos);
            }
        }
        pos += 1;
    }
    None
}

// Standard JPEG quantization tables (ISO 10918-1 Annex K)
const STD_LUMA_QT: [u8; 64] = [
    16, 11, 10, 16, 24, 40, 51, 61, 12, 12, 14, 19, 26, 58, 60, 55, 14, 13, 16, 24, 40, 57, 69, 56,
    14, 17, 22, 29, 51, 87, 80, 62, 18, 22, 37, 56, 68, 109, 103, 77, 24, 35, 55, 64, 81, 104, 113,
    92, 49, 64, 78, 87, 103, 121, 120, 101, 72, 92, 95, 98, 112, 100, 103, 99,
];
const STD_CHROMA_QT: [u8; 64] = [
    17, 18, 24, 47, 99, 99, 99, 99, 18, 21, 26, 66, 99, 99, 99, 99, 24, 26, 56, 99, 99, 99, 99, 99,
    47, 66, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
];

// Standard Huffman tables (ISO 10918-1 Annex K.3)
const STD_DC_LUMA_BITS: [u8; 16] = [
    0x00, 0x01, 0x05, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];
const STD_DC_LUMA_VALS: [u8; 12] = [
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B,
];
const STD_AC_LUMA_BITS: [u8; 16] = [
    0x00, 0x02, 0x01, 0x03, 0x03, 0x02, 0x04, 0x03, 0x05, 0x05, 0x04, 0x04, 0x00, 0x00, 0x01, 0x7D,
];
const STD_AC_LUMA_VALS: [u8; 162] = [
    0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07,
    0x22, 0x71, 0x14, 0x32, 0x81, 0x91, 0xA1, 0x08, 0x23, 0x42, 0xB1, 0xC1, 0x15, 0x52, 0xD1, 0xF0,
    0x24, 0x33, 0x62, 0x72, 0x82, 0x09, 0x0A, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x25, 0x26, 0x27, 0x28,
    0x29, 0x2A, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3A, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49,
    0x4A, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5A, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69,
    0x6A, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7A, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89,
    0x8A, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9A, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7,
    0xA8, 0xA9, 0xAA, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA, 0xC2, 0xC3, 0xC4, 0xC5,
    0xC6, 0xC7, 0xC8, 0xC9, 0xCA, 0xD2, 0xD3, 0xD4, 0xD5, 0xD6, 0xD7, 0xD8, 0xD9, 0xDA, 0xE1, 0xE2,
    0xE3, 0xE4, 0xE5, 0xE6, 0xE7, 0xE8, 0xE9, 0xEA, 0xF1, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7, 0xF8,
    0xF9, 0xFA,
];
const STD_DC_CHROMA_BITS: [u8; 16] = [
    0x00, 0x03, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00,
];
const STD_DC_CHROMA_VALS: [u8; 12] = [
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B,
];
const STD_AC_CHROMA_BITS: [u8; 16] = [
    0x00, 0x02, 0x01, 0x02, 0x04, 0x04, 0x03, 0x04, 0x07, 0x05, 0x04, 0x04, 0x00, 0x01, 0x02, 0x77,
];
const STD_AC_CHROMA_VALS: [u8; 162] = [
    0x00, 0x01, 0x02, 0x03, 0x11, 0x04, 0x05, 0x21, 0x31, 0x06, 0x12, 0x41, 0x51, 0x07, 0x61, 0x71,
    0x13, 0x22, 0x32, 0x81, 0x08, 0x14, 0x42, 0x91, 0xA1, 0xB1, 0xC1, 0x09, 0x23, 0x33, 0x52, 0xF0,
    0x15, 0x62, 0x72, 0xD1, 0x0A, 0x16, 0x24, 0x34, 0xE1, 0x25, 0xF1, 0x17, 0x18, 0x19, 0x1A, 0x26,
    0x27, 0x28, 0x29, 0x2A, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3A, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48,
    0x49, 0x4A, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5A, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68,
    0x69, 0x6A, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7A, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87,
    0x88, 0x89, 0x8A, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9A, 0xA2, 0xA3, 0xA4, 0xA5,
    0xA6, 0xA7, 0xA8, 0xA9, 0xAA, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA, 0xC2, 0xC3,
    0xC4, 0xC5, 0xC6, 0xC7, 0xC8, 0xC9, 0xCA, 0xD2, 0xD3, 0xD4, 0xD5, 0xD6, 0xD7, 0xD8, 0xD9, 0xDA,
    0xE2, 0xE3, 0xE4, 0xE5, 0xE6, 0xE7, 0xE8, 0xE9, 0xEA, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7, 0xF8,
    0xF9, 0xFA,
];

/// Build a DQT segment (FF DB) with standard luma + chroma quantization tables.
fn build_standard_dqt() -> Vec<u8> {
    let dqt_payload = 2 + (1 + 64) + (1 + 64);
    let mut dqt = Vec::with_capacity(2 + dqt_payload);
    dqt.push(0xFF);
    dqt.push(0xDB);
    dqt.extend_from_slice(&(dqt_payload as u16).to_be_bytes());
    dqt.push(0x00); // Pq=0, Tq=0 (luminance)
    dqt.extend_from_slice(&STD_LUMA_QT);
    dqt.push(0x01); // Pq=0, Tq=1 (chrominance)
    dqt.extend_from_slice(&STD_CHROMA_QT);
    dqt
}

fn push_dht_table(out: &mut Vec<u8>, tc: u8, th: u8, bits: &[u8; 16], vals: &[u8]) {
    out.push((tc << 4) | (th & 0xF));
    out.extend_from_slice(bits);
    out.extend_from_slice(vals);
}

/// Build a DHT segment (FF C4) with all four standard Huffman tables.
fn build_standard_dht() -> Vec<u8> {
    let tables = (4 * (1 + 16)) + (12 + 162 + 12 + 162);
    let dht_payload = 2 + tables;
    let mut dht = Vec::with_capacity(2 + dht_payload);
    dht.push(0xFF);
    dht.push(0xC4);
    dht.extend_from_slice(&(dht_payload as u16).to_be_bytes());
    push_dht_table(&mut dht, 0, 0, &STD_DC_LUMA_BITS, &STD_DC_LUMA_VALS);
    push_dht_table(&mut dht, 1, 0, &STD_AC_LUMA_BITS, &STD_AC_LUMA_VALS);
    push_dht_table(&mut dht, 0, 1, &STD_DC_CHROMA_BITS, &STD_DC_CHROMA_VALS);
    push_dht_table(&mut dht, 1, 1, &STD_AC_CHROMA_BITS, &STD_AC_CHROMA_VALS);
    dht
}

/// Build a baseline SOF0 segment for the given geometry and component count.
/// Sampling follows standard convention: Y=2x2, CbCr=1x1 for multi-component.
fn build_sof0(width: u32, height: u32, ncomp: u8) -> Vec<u8> {
    let payload = 8 + 3 * ncomp as usize; // Lf (includes length field itself)
    let mut sof = Vec::with_capacity(2 + payload);
    sof.push(0xFF);
    sof.push(0xC0);
    sof.extend_from_slice(&(payload as u16).to_be_bytes());
    sof.push(8);
    sof.extend_from_slice(&height.to_be_bytes());
    sof.extend_from_slice(&width.to_be_bytes());
    sof.push(ncomp);
    for i in 0..ncomp {
        sof.push(i + 1);
        if ncomp == 1 {
            sof.push(0x11);
        } else if i == 0 {
            sof.push(0x22);
        } else {
            sof.push(0x11);
        }
        sof.push(if i == 0 { 0x00 } else { 0x01 });
    }
    sof
}

/// Build a baseline SOS header (FF DA) for the given component count.
fn build_sos_header(ncomp: u8) -> Vec<u8> {
    let payload = 6 + 2 * ncomp as usize; // Ls (includes length field itself)
    let mut sos = Vec::with_capacity(2 + payload);
    sos.push(0xFF);
    sos.push(0xDA);
    sos.extend_from_slice(&(payload as u16).to_be_bytes());
    sos.push(ncomp);
    for i in 0..ncomp {
        sos.push(i + 1);
        if ncomp == 1 {
            sos.push(0x00);
        } else if i == 0 {
            sos.push(0x00);
        } else {
            sos.push(0x11);
        }
    }
    sos.push(0x00);
    sos.push(0x3F);
    sos.push(0x00);
    sos
}

/// Locate where the real JPEG data begins. First tries an explicit SOI marker
/// (`FF D8`), then falls back to the first plausible segment marker for images
/// whose SOI has been zeroed/destroyed.
fn find_jpeg_start(data: &[u8]) -> Option<usize> {
    if data.len() < 4 {
        return None;
    }
    for i in 0..data.len().saturating_sub(2) {
        if data[i] == 0xFF && data[i + 1] == 0xD8 && i + 2 < data.len() && data[i + 2] == 0xFF {
            return Some(i);
        }
    }
    for i in 0..data.len().saturating_sub(1) {
        if data[i] == 0xFF && is_jpeg_segment_marker(data[i + 1]) {
            return Some(i);
        }
    }
    None
}

/// Walk the JPEG marker stream from `start`, recording structure without
/// rebuilding the file. Used by `analyze_file`.
fn scan_jpeg(data: &[u8], start: usize) -> JpegInfo {
    let len = data.len();
    let mut info = JpegInfo::default();
    let mut pos = start;
    let mut guard = 0u64;
    while pos < len && guard < 1_000_000 {
        guard += 1;
        if data[pos] != 0xFF {
            break;
        }
        let mut j = pos + 1;
        while j < len && data[j] == 0xFF {
            j += 1;
        }
        if j >= len {
            break;
        }
        let code = data[j];
        info.marker_count += 1;

        if code == 0x00 {
            break;
        }
        if code == 0xD8 {
            if pos > start {
                break;
            }
            pos = j + 1;
            continue;
        }
        if code == 0xD9 {
            info.terminated = true;
            info.image_end = Some(j + 1);
            break;
        }
        if code == 0x01 || (0xD0..=0xD7).contains(&code) {
            pos = j + 1;
            continue;
        }
        if code == 0xDA {
            info.has_sos = true;
            pos = j + 1;
            if pos + 2 > len {
                break;
            }
            let seg_len = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
            if seg_len < 2 || pos + seg_len > len {
                info.seg_overrun = true;
                pos += 2;
                pos = find_entropy_end(data, pos);
                if pos < len {
                    continue;
                }
                break;
            }
            let ent_start = pos + seg_len;
            pos += seg_len;
            while pos < len {
                if data[pos] != 0xFF {
                    pos += 1;
                    continue;
                }
                let nxt = if pos + 1 < len { data[pos + 1] } else { 0xFF };
                if nxt == 0x00 || nxt == 0xFF {
                    pos += 2;
                    continue;
                }
                if (0xD0..=0xD7).contains(&nxt) {
                    pos += 2;
                    continue;
                }
                break;
            }
            info.entropy_bytes += pos.saturating_sub(ent_start);
            continue;
        }
        if is_jpeg_sof(code) {
            let len_pos = j + 1;
            if len_pos + 2 <= len {
                let seg_len = u16::from_be_bytes([data[len_pos], data[len_pos + 1]]) as usize;
                if seg_len >= 8 && len_pos + seg_len <= len {
                    let h = u16::from_be_bytes([data[len_pos + 3], data[len_pos + 4]]) as u32;
                    let w = u16::from_be_bytes([data[len_pos + 5], data[len_pos + 6]]) as u32;
                    if w > 0 && h > 0 {
                        info.width = Some(w);
                        info.height = Some(h);
                    }
                }
            }
        }
        if code == 0xDB {
            info.has_dqt = true;
        }
        if code == 0xC4 {
            info.has_dht = true;
        }
        pos = j + 1;
        if pos + 2 > len {
            if info.has_sos {
                info.seg_overrun = true;
            }
            break;
        }
        let seg_len = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
        if seg_len < 2 || pos + seg_len > len {
            if info.has_sos {
                info.seg_overrun = true;
            }
            break;
        }
        pos += seg_len;
    }
    info
}

/// Classify the state of the JPEG image payload (the entropy-coded scan
/// data after SOS). Without scan data the image itself is gone, and a broken
/// marker stream in the payload region means the data cannot be trusted.
fn jpeg_payload_status(info: &JpegInfo) -> PayloadStatus {
    if info.seg_overrun {
        return PayloadStatus::Corrupt;
    }
    if !info.has_sos || info.entropy_bytes < 2 {
        return PayloadStatus::Missing;
    }
    PayloadStatus::Ok
}

/// Locate a Start Of Scan marker (`FF DA`) anywhere in the buffer, tolerant of
/// a destroyed SOI/APP/DQT header that would stop the normal marker walk.
fn find_sos_tolerant(data: &[u8]) -> Option<usize> {
    for i in 0..data.len().saturating_sub(3) {
        if data[i] == 0xFF && data[i + 1] == 0xDA {
            let seg_len = u16::from_be_bytes([data[i + 2], data[i + 3]]);
            if seg_len >= 2 {
                return Some(i);
            }
        }
    }
    None
}

/// Payload check that first walks from the located JPEG start; if the walk is
/// cut short by header damage before the SOS, retry from a tolerantly located
/// SOS marker so intact scan data still counts as a recoverable payload.
fn jpeg_payload_from_scan(data: &[u8], start: usize) -> PayloadStatus {
    let info = scan_jpeg(data, start);
    let mut status = jpeg_payload_status(&info);
    if !info.has_sos && !info.seg_overrun && !info.terminated {
        if let Some(sos) = find_sos_tolerant(data) {
            let info2 = scan_jpeg(data, sos);
            status = jpeg_payload_status(&info2);
        }
    }
    status
}

/// Rebuild a valid JPEG from possibly-corrupt/truncated/carved data:
/// restores a missing SOI, drops trailing garbage after the real EOI, and
/// terminates truncated entropy-coded scan data with a fresh EOI.
///
/// If the DQT, SOF or DHT headers are corrupt or missing but SOS + entropy
/// survive, standard ISO 10918-1 Annex K tables are inserted as a fallback so
/// the image remains decodable (colors may be off but the file opens).
fn repair_jpeg(data: &[u8], start: Option<usize>) -> Option<Vec<u8>> {
    let start = start.or_else(|| find_jpeg_start(data)).unwrap_or(0);
    if start >= data.len() {
        return None;
    }
    let len = data.len();
    let mut out = Vec::with_capacity(len - start + 512);
    let mut saw_sos = false;
    let mut saw_dqt = false;
    let mut saw_sof = false;
    let mut saw_dht = false;
    let mut sof_dims: Option<(u32, u32, u8)> = None;
    let mut sos_output_pos: Option<usize> = None;

    if !data[start..].starts_with(JPEG_SOI) {
        out.extend_from_slice(JPEG_SOI);
    }

    let mut pos = start;
    let mut guard = 0u64;
    while pos < len && guard < 1_000_000 {
        guard += 1;
        if data[pos] != 0xFF {
            break;
        }
        let mut j = pos + 1;
        while j < len && data[j] == 0xFF {
            j += 1;
        }
        if j >= len {
            break;
        }
        let code = data[j];

        if code == 0x00 {
            // A `FF 00` here is a destroyed segment marker (e.g. FF DB -> FF 00).
            // Skipping it risks landing on a stray marker inside the destroyed
            // segment's leftover payload, so give up rather than emit a corrupt
            // file. The reference-based repair (`repair_jpeg_with_ref`) is the
            // reliable path for files whose headers are destroyed.
            break;
        }
        if code == 0xD8 {
            if pos > start {
                break;
            }
            out.extend_from_slice(&data[pos..=j]);
            pos = j + 1;
            continue;
        }
        if code == 0xD9 {
            out.extend_from_slice(&data[pos..=j]);
            // Do not return yet: missing DQT/DHT/SOF still need the
            // standard-table fallback, which runs after the loop.
            break;
        }
        if code == 0x01 || (0xD0..=0xD7).contains(&code) {
            out.extend_from_slice(&data[pos..=j]);
            pos = j + 1;
            continue;
        }
        if code == 0xDA {
            if sos_output_pos.is_none() {
                sos_output_pos = Some(out.len());
            }
            out.extend_from_slice(&data[pos..=j]); // FF DA marker
            pos = j + 1;

            let mut sos_ok = false;
            if pos + 2 <= len {
                let seg_len = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
                if seg_len >= 2 && pos + seg_len <= len {
                    out.extend_from_slice(&data[pos..pos + seg_len]);
                    pos += seg_len;
                    let eend = find_entropy_end(data, pos);
                    out.extend_from_slice(&data[pos..eend]);
                    saw_sos = true;
                    sos_ok = true;
                    if eend < len {
                        pos = eend; // continue walking (progressive/multi-scan)
                        continue;
                    }
                }
            }
            if !sos_ok {
                if let Some((_w, _h, nc)) = sof_dims {
                    let recovered = build_sos_header(nc);
                    out.extend_from_slice(&recovered[2..]);
                    let estart = (pos + 2).min(len);
                    let eend = find_entropy_end(data, estart);
                    out.extend_from_slice(&data[estart..eend]);
                    saw_sos = true;
                }
            }
            break;
        }
        out.extend_from_slice(&data[pos..=j]);
        pos = j + 1;
        if pos + 2 > len {
            if let Some(np) = skip_to_next_marker(data, pos) {
                pos = np;
                continue;
            }
            break;
        }
        let seg_len = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
        if seg_len < 2 || pos + seg_len > len {
            if let Some(np) = skip_to_next_marker(data, pos) {
                pos = np;
                continue;
            }
            break;
        }
        if code == 0xDB {
            saw_dqt = true;
        }
        if code == 0xC4 {
            saw_dht = true;
        }
        if is_jpeg_sof(code) && seg_len >= 9 {
            let nc = data[pos + 7] as usize;
            if nc >= 1 && nc <= 4 && seg_len >= 8 + 3 * nc {
                let h = u16::from_be_bytes([data[pos + 3], data[pos + 4]]) as u32;
                let w = u16::from_be_bytes([data[pos + 5], data[pos + 6]]) as u32;
                if w > 0 && w <= 65536 && h > 0 && h <= 65536 {
                    saw_sof = true;
                    sof_dims = Some((w, h, nc as u8));
                }
            }
        }
        out.extend_from_slice(&data[pos..pos + seg_len]);
        pos += seg_len;
    }

    if !saw_sos {
        return None;
    }

    if !saw_dqt || !saw_dht || !saw_sof {
        let dims = sof_dims.or_else(|| exif_dimensions(data).map(|(w, h)| (w, h, 3u8)));
        if let Some((w, h, nc)) = dims {
            let insert_pos = sos_output_pos.unwrap_or(out.len());
            let mut fixed = Vec::with_capacity(out.len() + 512);
            fixed.extend_from_slice(&out[..insert_pos]);
            if !saw_dqt {
                fixed.extend_from_slice(&build_standard_dqt());
            }
            if !saw_sof {
                fixed.extend_from_slice(&build_sof0(w, h, nc));
            }
            if !saw_dht {
                fixed.extend_from_slice(&build_standard_dht());
            }
            fixed.extend_from_slice(&out[insert_pos..]);
            out = fixed;
        }
    }

    if !out.ends_with(JPEG_EOI) {
        out.extend_from_slice(JPEG_EOI);
    }
    Some(out)
}

fn jpeg_details(info: &JpegInfo) -> String {
    let mut parts = Vec::new();
    if let (Some(w), Some(h)) = (info.width, info.height) {
        parts.push(format!("{}x{}", w, h));
    }
    if info.has_sos {
        parts.push("SOS".to_string());
    }
    parts.push(format!("{} markers", info.marker_count));
    if info.terminated {
        parts.push("EOI found".to_string());
    } else {
        parts.push("no EOI".to_string());
    }
    parts.join(", ")
}

// ---------------------------------------------------------------------------
// Reference-based JPEG repair
//
// A known-good image from the same camera family (e.g. IMG_original.JPG, a
// complete Canon EOS 4000D photo) supplies a structural template: the DQT
// quantization tables, the DHT huffman tables, the SOF geometry and the SOS
// header. When a damaged JPEG is missing or has corrupt headers/tables but its
// entropy-coded scan data still survives, the template rebuilds a valid file
// around that data. If no scan data survives, valid entropy is synthesized
// with the template's tables so the output always opens in a viewer.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
struct JpegTemplate {
    dqt: Vec<u8>,
    dht: Vec<u8>,
    sof: Vec<u8>,
    sos_payload: Vec<u8>,
    width: u32,
    height: u32,
    ncomp: u8,
    prec: u8,
    sampling: Vec<(u8, u8)>,
    sos_comps: Vec<(u8, u8, u8)>,
    hmax: u8,
    vmax: u8,
    baseline: bool,
    dc_code0: Vec<Option<(u32, u8)>>,
    eob_code: Vec<Option<(u32, u8)>>,
}

#[derive(Debug, Clone, Default)]
struct JpegParts {
    dqt: Vec<Vec<u8>>,
    dht: Vec<Vec<u8>>,
    sof: Option<Vec<u8>>,
    sof_dims: Option<(u32, u32, u8, u8)>,
    sos_payload: Option<Vec<u8>>,
    entropy: Option<(usize, usize)>,
}

fn jpeg_sof_dims(seg: &[u8]) -> Option<(u32, u32, u8, u8)> {
    if seg.len() < 11 || seg[0] != 0xFF || !is_jpeg_sof(seg[1]) {
        return None;
    }
    let ncomp = seg[9];
    if ncomp == 0 || seg.len() < 10 + ncomp as usize * 3 {
        return None;
    }
    let h = u16::from_be_bytes([seg[5], seg[6]]);
    let w = u16::from_be_bytes([seg[7], seg[8]]);
    if w == 0 || h == 0 {
        return None;
    }
    Some((w as u32, h as u32, ncomp, seg[4]))
}

/// Walk the JPEG marker stream and collect the structural pieces a repair
/// needs: quantization tables, huffman tables, an optional SOF with geometry
/// and sampling, the SOS header, and the raw entropy-coded scan data.
fn jpeg_parts(data: &[u8], start: usize) -> JpegParts {
    let len = data.len();
    let mut parts = JpegParts::default();
    let mut pos = start;
    let mut guard = 0u64;
    while pos < len && guard < 1_000_000 {
        guard += 1;
        if data[pos] != 0xFF {
            break;
        }
        let mut j = pos + 1;
        while j < len && data[j] == 0xFF {
            j += 1;
        }
        if j >= len {
            break;
        }
        let code = data[j];
        if code == 0x00 {
            // Destroyed segment marker (FF xx -> FF 00). Skip forward to the
            // next real marker so intact DQT/DHT/SOF/SOS and entropy below a
            // damaged header are still captured.
            if parts.entropy.is_none() {
                if let Some(np) = skip_to_next_marker(data, pos) {
                    pos = np;
                    continue;
                }
            }
            break;
        }
        if code == 0xD8 {
            if pos > start {
                break;
            }
            pos = j + 1;
            continue;
        }
        if code == 0xD9 || code == 0x01 || (0xD0..=0xD7).contains(&code) {
            break;
        }
        let seg_start = j + 1;
        if seg_start + 2 > len {
            break;
        }
        let seg_len = u16::from_be_bytes([data[seg_start], data[seg_start + 1]]) as usize;
        if seg_len < 2 || seg_start + seg_len > len {
            // Corrupt segment length: hop to the next real marker instead of
            // abandoning the walk (intact scans may follow).
            if parts.entropy.is_none() {
                if let Some(np) = skip_to_next_marker(data, pos) {
                    pos = np;
                    continue;
                }
            }
            break;
        }
        let seg = &data[seg_start - 2..seg_start + seg_len];
        match code {
            0xDB => parts.dqt.push(seg.to_vec()),
            0xC4 => parts.dht.push(seg.to_vec()),
            0xDA => {
                parts.sos_payload = Some(data[seg_start..seg_start + seg_len].to_vec());
                let estart = seg_start + seg_len;
                let mut i = estart;
                let mut end = len;
                while i < len {
                    if data[i] != 0xFF {
                        i += 1;
                        continue;
                    }
                    let nxt = if i + 1 < len { data[i + 1] } else { 0xFF };
                    if nxt == 0x00 || nxt == 0xFF || (0xD0..=0xD7).contains(&nxt) {
                        i += 2;
                        continue;
                    }
                    end = i;
                    break;
                }
                if end > estart {
                    parts.entropy = Some((estart, end));
                }
                break;
            }
            _ => {
                if is_jpeg_sof(code) {
                    if let Some((w, h, n, p)) = jpeg_sof_dims(seg) {
                        parts.sof = Some(seg.to_vec());
                        parts.sof_dims = Some((w, h, n, p));
                    }
                }
            }
        }
        pos = seg_start + seg_len;
    }
    if parts.entropy.is_none() {
        // The structured walk failed (headers destroyed, e.g. heavy random
        // corruption). Fall back to scanning the whole buffer for the most
        // plausible SOS marker and its entropy region, so the reference
        // repair can rebuild the frame around surviving scan data instead of
        // emitting a synthesized gray placeholder.
        if let Some((sos_payload, es, ee)) = find_best_scan(data) {
            parts.sos_payload = Some(sos_payload);
            parts.entropy = Some((es, ee));
        }
    }
    parts
}

/// Scan the buffer for SOS markers (FF DA) whose header is structurally valid
/// (Ns 1..=4 and Ls == 6 + 2*Ns, fitting the buffer), and return the candidate
/// with the largest entropy region. Used when the normal marker walk cannot
/// reach the scan start because the header area is destroyed.
fn find_best_scan(data: &[u8]) -> Option<(Vec<u8>, usize, usize)> {
    let len = data.len();
    let mut best: Option<(Vec<u8>, usize, usize)> = None;
    let mut i = 0usize;
    while i + 6 <= len {
        if data[i] == 0xFF && data[i + 1] == 0xDA {
            let p = i + 2;
            let seg_len = u16::from_be_bytes([data[p], data[p + 1]]) as usize;
            let ns = data[p + 2] as usize;
            if (1..=4).contains(&ns) && seg_len >= 6 && seg_len == 6 + 2 * ns && p + seg_len <= len
            {
                let estart = p + seg_len;
                let eend = find_entropy_end_tolerant(data, estart);
                if eend > estart {
                    let size = eend - estart;
                    let is_bigger = best
                        .as_ref()
                        .map(|(_, s, e)| (e - s) < size)
                        .unwrap_or(true);
                    if is_bigger {
                        best = Some((data[p..p + seg_len].to_vec(), estart, eend));
                    }
                }
            }
        }
        i += 1;
    }
    best
}

fn parse_dht_tables(dht: &[u8]) -> Vec<(u8, u8, Vec<u8>)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 2 <= dht.len() && dht[i] == 0xFF && dht[i + 1] == 0xC4 {
        if i + 4 > dht.len() {
            break;
        }
        let seg_len = u16::from_be_bytes([dht[i + 2], dht[i + 3]]) as usize;
        if seg_len < 2 || i + 2 + seg_len > dht.len() {
            break;
        }
        let mut p = i + 4;
        let end = i + 2 + seg_len;
        while p + 17 <= end {
            let tc = dht[p] >> 4;
            let th = dht[p] & 0xF;
            p += 1;
            let counts = &dht[p..p + 16];
            p += 16;
            let total = counts.iter().map(|c| *c as usize).sum::<usize>();
            if p + total > end {
                break;
            }
            out.push((tc, th, dht[p..p + total].to_vec()));
            p += total;
        }
        i = i + 2 + seg_len;
    }
    out
}

fn dqt_table_count(dqt: &[u8]) -> usize {
    let mut n = 0usize;
    let mut i = 0;
    while i + 4 <= dqt.len() && dqt[i] == 0xFF && dqt[i + 1] == 0xDB {
        let seg_len = u16::from_be_bytes([dqt[i + 2], dqt[i + 3]]) as usize;
        if seg_len < 2 || i + 2 + seg_len > dqt.len() {
            break;
        }
        let mut p = i + 4;
        let end = i + 2 + seg_len;
        while p < end {
            let size = if dqt[p] >> 4 == 0 { 64 } else { 128 };
            if p + 1 + size > end {
                break;
            }
            n += 1;
            p += 1 + size;
        }
        i += 2 + seg_len;
    }
    n
}

fn dqt_has_valid_tables(dqt: &[u8], ncomp: u8) -> bool {
    if dqt.is_empty() {
        return false;
    }
    let mut i = 0;
    let mut n = 0usize;
    while i + 4 <= dqt.len() && dqt[i] == 0xFF && dqt[i + 1] == 0xDB {
        let seg_len = u16::from_be_bytes([dqt[i + 2], dqt[i + 3]]) as usize;
        if seg_len < 2 || i + 2 + seg_len > dqt.len() {
            return false;
        }
        let mut p = i + 4;
        let end = i + 2 + seg_len;
        while p < end {
            let size = if dqt[p] >> 4 == 0 { 64 } else { 128 };
            if p + 1 + size > end {
                return false;
            }
            if dqt[p + 1] == 0 {
                return false;
            }
            n += 1;
            p += 1 + size;
        }
        if p != end {
            return false;
        }
        i += 2 + seg_len;
    }
    n >= 1 && n >= ncomp as usize
}

/// A DHT segment is usable for decoding only if every component referenced by
/// the SOS header has a real (non-empty symbol list) DC and AC table.
fn dht_usable(dht: &[u8], sos_payload: &[u8]) -> bool {
    let tables = parse_dht_tables(dht);
    if tables.is_empty() {
        return false;
    }
    let comps = jpeg_sos_components(sos_payload);
    for (_, dct, act) in comps {
        let dc_ok = tables
            .iter()
            .any(|(tc, th, s)| *tc == 0 && *th == dct && !s.is_empty());
        let ac_ok = tables
            .iter()
            .any(|(tc, th, s)| *tc == 1 && *th == act && !s.is_empty());
        if !dc_ok || !ac_ok {
            return false;
        }
    }
    true
}

fn build_huffman_codes(counts: &[u8], symbols: &[u8]) -> Option<(Vec<(u32, u8)>, u8)> {
    let mut code: u32 = 0;
    let mut k = 0usize;
    let mut max_len = 0u8;
    for bits in 1..=16u8 {
        let n = counts[bits as usize - 1] as usize;
        for _ in 0..n {
            if code as u32 > 0xFFFF {
                return None;
            }
            k += 1;
            code += 1;
        }
        if n > 0 {
            max_len = bits;
        }
        code <<= 1;
    }
    if k != symbols.len() {
        return None;
    }
    let mut out = Vec::with_capacity(symbols.len());
    let mut c: u32 = 0;
    for bits in 1..=16u8 {
        let n = counts[bits as usize - 1] as usize;
        for _ in 0..n {
            out.push((c, bits));
            c += 1;
        }
        c <<= 1;
    }
    Some((out, max_len))
}

fn huff_lookup(codes: &[(u32, u8)], symbols: &[u8], sym: u8) -> Option<(u32, u8)> {
    symbols.iter().position(|s| *s == sym).map(|i| codes[i])
}

fn jpeg_sampling_components(seg: &[u8], ncomp: u8) -> Vec<(u8, u8)> {
    let mut out = Vec::new();
    for i in 0..ncomp as usize {
        let base = 10 + i * 3;
        if base + 2 < seg.len() {
            out.push((seg[base + 1] >> 4, seg[base + 1] & 0xF));
        } else {
            out.push((1, 1));
        }
    }
    out
}

fn jpeg_sos_components(payload: &[u8]) -> Vec<(u8, u8, u8)> {
    let mut out = Vec::new();
    if payload.len() < 3 {
        return out;
    }
    let n = payload[2] as usize;
    for i in 0..n {
        let base = 3 + i * 2;
        if base + 1 < payload.len() {
            out.push((
                payload[base],
                payload[base + 1] >> 4,
                payload[base + 1] & 0xF,
            ));
        } else {
            break;
        }
    }
    out
}

/// Extract a reusable structural template from a complete JPEG like a photo
/// from the reference camera (e.g. IMG_original.JPG).
fn extract_jpeg_template(data: &[u8]) -> Option<JpegTemplate> {
    let start = find_jpeg_start(data).unwrap_or(0);
    let parts = jpeg_parts(data, start);
    let sof_seg = parts.sof?;
    let (w, h, ncomp, prec) = parts.sof_dims?;
    let sos_payload = parts.sos_payload?;
    if ncomp == 0 || w == 0 || h == 0 {
        return None;
    }
    if parts.dqt.is_empty() || parts.dht.is_empty() {
        return None;
    }
    let sampling = jpeg_sampling_components(&sof_seg, ncomp);
    let baseline = sof_seg.get(1) == Some(&0xC0);
    let sos_comps = jpeg_sos_components(&sos_payload);
    if sos_comps.len() != ncomp as usize {
        return None;
    }
    let mut hmax = 0u8;
    let mut vmax = 0u8;
    for (sh, sv) in &sampling {
        hmax = hmax.max(*sh);
        vmax = vmax.max(*sv);
    }

    let dht_tables = parse_dht_tables(&parts.dht.concat());
    let mut dc_code0: Vec<Option<(u32, u8)>> = vec![None; 4];
    let mut eob_code: Vec<Option<(u32, u8)>> = vec![None; 4];
    for (tc, th, syms) in dht_tables {
        if th as usize >= 4 {
            continue;
        }
        let counts = dht_counts_for(&parts.dht.concat(), tc, th)?;
        let (codes, _max) = build_huffman_codes(&counts, &syms)?;
        if tc == 0 {
            dc_code0[th as usize] = huff_lookup(&codes, &syms, 0);
        } else {
            eob_code[th as usize] = huff_lookup(&codes, &syms, 0x00);
        }
    }

    Some(JpegTemplate {
        dqt: parts.dqt.concat(),
        dht: parts.dht.concat(),
        sof: sof_seg,
        sos_payload,
        width: w,
        height: h,
        ncomp,
        prec,
        sampling,
        sos_comps,
        hmax,
        vmax,
        baseline,
        dc_code0,
        eob_code,
    })
}

/// Recover the 16 symbol counts for the given DHT table (class, id), scanning
/// a concatenated DHT segment list.
fn dht_counts_for(dht: &[u8], class: u8, id: u8) -> Option<Vec<u8>> {
    let mut i = 0;
    while i + 2 <= dht.len() && dht[i] == 0xFF && dht[i + 1] == 0xC4 {
        if i + 4 > dht.len() {
            break;
        }
        let seg_len = u16::from_be_bytes([dht[i + 2], dht[i + 3]]) as usize;
        if seg_len < 2 || i + 2 + seg_len > dht.len() {
            break;
        }
        let mut p = i + 4;
        let end = i + 2 + seg_len;
        while p + 17 <= end {
            let tc = dht[p] >> 4;
            let th = dht[p] & 0xF;
            p += 1;
            if tc == class && th == id {
                return Some(dht[p..p + 16].to_vec());
            }
            let total = dht[p..p + 16].iter().map(|c| *c as usize).sum::<usize>();
            p += 16 + total;
        }
        i += 2 + seg_len;
    }
    None
}

fn jpeg_template_info(tpl: &JpegTemplate) -> String {
    format!(
        "{}x{}, {}-bit, {} components, {} DQT table(s), {} Huffman tables{}",
        tpl.width,
        tpl.height,
        tpl.prec,
        tpl.ncomp,
        dqt_table_count(&tpl.dqt),
        parse_dht_tables(&tpl.dht).len(),
        if tpl.baseline { "" } else { " (non-baseline)" }
    )
}

/// Encode entropy-coded scan data for a solid-color image using the
/// template's huffman tables. Every block emits only its DC category-0 code
/// (coefficient zero) followed by the EOB code, which is a valid, decodable
/// stream for *any* coefficients all being zero.
fn synth_jpeg_scan(tpl: &JpegTemplate, width: u32, height: u32) -> Option<Vec<u8>> {
    if !tpl.baseline {
        return None;
    }
    if tpl.hmax == 0 || tpl.vmax == 0 {
        return None;
    }
    let mcu_w = (width as u64 + 8 * tpl.hmax as u64 - 1) / (8 * tpl.hmax as u64);
    let mcu_h = (height as u64 + 8 * tpl.vmax as u64 - 1) / (8 * tpl.vmax as u64);
    if mcu_w == 0 || mcu_h == 0 {
        return None;
    }
    for (cid, dct, act) in &tpl.sos_comps {
        let _ = cid;
        if tpl.dc_code0.get(*dct as usize).and_then(|o| *o).is_none() {
            return None;
        }
        if tpl.eob_code.get(*act as usize).and_then(|o| *o).is_none() {
            return None;
        }
    }
    let mut bw = BitWriter::new();
    let total = mcu_w * mcu_h;
    for _ in 0..total {
        for (cid, dct, act) in &tpl.sos_comps {
            let (h, v) = tpl
                .sampling
                .get((*cid).saturating_sub(1) as usize)
                .copied()
                .unwrap_or((1, 1));
            for _ in 0..(h * v) {
                let (c, l) = tpl.dc_code0[*dct as usize].unwrap();
                bw.put_bits(c, l);
                let (c, l) = tpl.eob_code[*act as usize].unwrap();
                bw.put_bits(c, l);
            }
        }
    }
    bw.flush();
    Some(bw.out)
}

struct BitWriter {
    out: Vec<u8>,
    acc: u32,
    nbits: u32,
}

impl BitWriter {
    fn new() -> Self {
        BitWriter {
            out: Vec::new(),
            acc: 0,
            nbits: 0,
        }
    }
    fn put_bits(&mut self, code: u32, len: u8) {
        self.acc = (self.acc << len) | code;
        self.nbits += len as u32;
        while self.nbits >= 8 {
            let shift = self.nbits - 8;
            let b = ((self.acc >> shift) & 0xFF) as u8;
            self.acc &= if shift == 0 { 0 } else { (1u32 << shift) - 1 };
            self.nbits -= 8;
            self.emit(b);
        }
    }
    fn emit(&mut self, b: u8) {
        self.out.push(b);
        if b == 0xFF {
            self.out.push(0);
        }
    }
    fn flush(&mut self) {
        if self.nbits > 0 {
            let b = ((self.acc << (8 - self.nbits)) & 0xFF) as u8;
            self.emit(b);
        }
    }
}

fn rewrite_sof_dims(sof: &[u8], width: u32, height: u32) -> Vec<u8> {
    let mut out = sof.to_vec();
    if out.len() >= 9 {
        out[5] = ((height >> 8) & 0xFF) as u8;
        out[6] = (height & 0xFF) as u8;
        out[7] = ((width >> 8) & 0xFF) as u8;
        out[8] = (width & 0xFF) as u8;
    }
    out
}

fn tiff_u16(le: bool, t: &[u8], off: usize) -> u32 {
    if le {
        u16::from_le_bytes([t[off], t[off + 1]]) as u32
    } else {
        u16::from_be_bytes([t[off], t[off + 1]]) as u32
    }
}

fn tiff_u32(le: bool, t: &[u8], off: usize) -> u32 {
    if le {
        u32::from_le_bytes([t[off], t[off + 1], t[off + 2], t[off + 3]])
    } else {
        u32::from_be_bytes([t[off], t[off + 1], t[off + 2], t[off + 3]])
    }
}

/// Read one TIFF IFD entry value (tag/type/count at `val_field`). Handles the
/// common inline SHORT/LONG and out-of-line SHORT/LONG/RATIONAL layouts.
fn tiff_ifd_value(le: bool, t: &[u8], typ: u16, cnt: usize, val_field: usize) -> Option<u32> {
    if val_field + 4 > t.len() {
        return None;
    }
    let inline = match typ {
        1 => cnt <= 4,
        3 => cnt <= 2,
        4 | 9 => cnt <= 1,
        5 | 10 => cnt <= 0,
        _ => return None,
    };
    if inline {
        match typ {
            1 => Some(t[val_field] as u32),
            3 => Some(tiff_u16(le, t, val_field)),
            4 => Some(tiff_u32(le, t, val_field)),
            _ => None,
        }
    } else {
        let off = tiff_u32(le, t, val_field) as usize;
        if off + 4 > t.len() {
            return None;
        }
        match typ {
            3 => Some(tiff_u16(le, t, off)),
            4 | 5 => Some(tiff_u32(le, t, off)),
            _ => None,
        }
    }
}

/// Pull the pixel width/height out of a TIFF/EXIF body (`t` is everything
/// after the leading "Exif\0\0"). Looks at IFD0 and the Exif sub-IFD for the
/// PixelXDimension (0xA002) / PixelYDimension (0xA003) tags. Returns None when
/// the geometry is not present (e.g. damaged metadata).
fn tiff_dimensions(t: &[u8]) -> Option<(u32, u32)> {
    if t.len() < 8 {
        return None;
    }
    let le = match &t[0..2] {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    if tiff_u16(le, t, 2) != 42 {
        return None;
    }
    let ifd0 = tiff_u32(le, t, 4) as usize;
    if ifd0 + 2 > t.len() {
        return None;
    }
    let mut w = None;
    let mut h = None;
    let mut exif_ifd = None;
    let count = tiff_u16(le, t, ifd0) as usize;
    for e in 0..count {
        let off = ifd0 + 2 + e * 12;
        if off + 12 > t.len() {
            break;
        }
        let tag = tiff_u16(le, t, off) as u16;
        let typ = tiff_u16(le, t, off + 2) as u16;
        let cnt = tiff_u32(le, t, off + 4) as usize;
        match tag {
            0x8769 if typ == 4 && cnt == 1 => exif_ifd = Some(tiff_u32(le, t, off + 8) as usize),
            0xA002 => w = tiff_ifd_value(le, t, typ, cnt, off + 8),
            0xA003 => h = tiff_ifd_value(le, t, typ, cnt, off + 8),
            _ => {}
        }
    }
    if w.is_none() || h.is_none() {
        if let Some(ifd) = exif_ifd {
            if ifd + 2 <= t.len() {
                let count2 = tiff_u16(le, t, ifd) as usize;
                for e in 0..count2 {
                    let off = ifd + 2 + e * 12;
                    if off + 12 > t.len() {
                        break;
                    }
                    let tag = tiff_u16(le, t, off) as u16;
                    let typ = tiff_u16(le, t, off + 2) as u16;
                    let cnt = tiff_u32(le, t, off + 4) as usize;
                    match tag {
                        0xA002 => w = tiff_ifd_value(le, t, typ, cnt, off + 8),
                        0xA003 => h = tiff_ifd_value(le, t, typ, cnt, off + 8),
                        _ => {}
                    }
                }
            }
        }
    }
    Some((w?, h?))
}

/// Recover the real pixel dimensions of a damaged photo from its EXIF APP1
/// segment. Used when the SOF0 scan header is destroyed but the camera left
/// PixelXDimension/PixelYDimension behind, so a reference photo of a different
/// resolution still produces a correctly-sized result.
fn exif_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    let len = data.len();
    let mut pos = 0usize;
    while pos + 4 <= len {
        if data[pos] != 0xFF {
            return None;
        }
        let mut j = pos + 1;
        while j < len && data[j] == 0xFF {
            j += 1;
        }
        if j >= len {
            return None;
        }
        let code = data[j];
        if code == 0xD8 || code == 0x01 || (0xD0..=0xD7).contains(&code) {
            pos = j + 1;
            continue;
        }
        if j + 2 > len {
            return None;
        }
        let seg_len = u16::from_be_bytes([data[j + 1], data[j + 2]]) as usize;
        if seg_len < 2 || j + 1 + seg_len > len {
            return None;
        }
        if code == 0xE1 {
            let payload = &data[j + 3..j + 1 + seg_len];
            if payload.starts_with(b"Exif\0\0") {
                if let Some(d) = tiff_dimensions(&payload[6..]) {
                    return Some(d);
                }
            }
        }
        pos = j + 1 + seg_len;
    }
    None
}

/// Rebuild a viewer-openable JPEG for damaged data: keep any surviving
/// entropy-coded scan bytes and rebuild missing/destroyed headers, geometry
/// and tables from the reference template. If no scan data survives, emit
/// synthesized scan data so the result always decodes.
fn repair_jpeg_with_ref(data: &[u8], start: Option<usize>, tpl: &JpegTemplate) -> Option<Vec<u8>> {
    let start = start.or_else(|| find_jpeg_start(data)).unwrap_or(0);
    if start >= data.len() {
        return None;
    }
    let parts = jpeg_parts(data, start);

    let ncomp = parts.sof_dims.map(|(_, _, n, _)| n).unwrap_or(tpl.ncomp);
    if ncomp != tpl.ncomp {
        return None;
    }
    // The reference's SOF can only stand in for a destroyed scan header when
    // it is baseline/sequential; a progressive (SOF2) frame needs its multi-scan
    // progression to be valid, which a single rebuilt scan cannot provide.
    if parts.sof.is_none() && !tpl.baseline {
        return None;
    }
    // Prefer the damaged file's own geometry, then its EXIF metadata, and only
    // fall back to the reference photo's dimensions last. This keeps the repair
    // correct when the reference is a different-resolution photo from the same
    // camera.
    let (w, h) = parts
        .sof_dims
        .map(|(w, h, _, _)| (w, h))
        .or_else(|| exif_dimensions(data))
        .unwrap_or((tpl.width, tpl.height));
    if w == 0 || h == 0 {
        return None;
    }

    let sos_payload = match &parts.sos_payload {
        Some(p) if !p.is_empty() && p.len() >= 3 && p[2] as usize == ncomp as usize => p.clone(),
        _ => tpl.sos_payload.clone(),
    };

    // Only reuse the damaged file's own tables when they are genuinely usable;
    // otherwise fall back to the reference template's tables.
    let dqt = if dqt_has_valid_tables(&parts.dqt.concat(), ncomp) {
        parts.dqt.concat()
    } else {
        tpl.dqt.clone()
    };
    let dht = if dht_usable(&parts.dht.concat(), &sos_payload) {
        parts.dht.concat()
    } else {
        tpl.dht.clone()
    };

    let sof = match &parts.sof {
        Some(seg) => seg.clone(),
        None => rewrite_sof_dims(&tpl.sof, w, h),
    };

    let entropy = parts
        .entropy
        .map(|(es, ee)| data[es..ee].to_vec())
        .filter(|s| s.len() >= 8);

    let scan = match entropy {
        Some(s) => s,
        None => synth_jpeg_scan(tpl, w, h)?,
    };

    let mut out = Vec::with_capacity(64 + dqt.len() + dht.len() + sof.len() + scan.len() + 16);
    out.extend_from_slice(JPEG_SOI);
    out.extend_from_slice(&dqt);
    out.extend_from_slice(&sof);
    out.extend_from_slice(&dht);
    out.extend_from_slice(&[0xFF, 0xDA]);
    out.extend_from_slice(&sos_payload);
    out.extend_from_slice(&scan);
    out.extend_from_slice(JPEG_EOI);
    Some(out)
}

// ---------------------------------------------------------------------------
// BMP repair
// ---------------------------------------------------------------------------

const BMP_SIG: &[u8] = b"BM";

#[derive(Debug, Clone, Default)]
struct BmpInfo {
    has_header: bool,
    width: Option<u32>,
    height: Option<u32>,
    bpp: Option<u16>,
    data_offset: usize,
    declared_size: usize,
    pixel_end: Option<usize>,
    dib_size: u32,
}

fn scan_bmp(data: &[u8]) -> BmpInfo {
    let mut info = BmpInfo::default();
    if data.len() < 14 {
        return info;
    }
    info.has_header = data.starts_with(BMP_SIG);
    info.declared_size = u32::from_le_bytes([data[2], data[3], data[4], data[5]]) as usize;
    info.data_offset = u32::from_le_bytes([data[10], data[11], data[12], data[13]]) as usize;
    if data.len() < 18 {
        return info;
    }
    info.dib_size = u32::from_le_bytes([data[14], data[15], data[16], data[17]]);
    let dib = info.dib_size as usize;
    if dib >= 40 {
        if data.len() < 30 {
            return info;
        }
        let w = u32::from_le_bytes([data[18], data[19], data[20], data[21]]);
        let h = u32::from_le_bytes([data[22], data[23], data[24], data[25]]);
        info.bpp = Some(u16::from_le_bytes([data[28], data[29]]));
        if w > 0 && w <= 100_000 && h > 0 && h <= 100_000 {
            info.width = Some(w);
            info.height = Some(h);
        }
    } else if dib >= 12 {
        if data.len() < 26 {
            return info;
        }
        let w = u16::from_le_bytes([data[18], data[19]]) as u32;
        let h = u16::from_le_bytes([data[20], data[21]]) as u32;
        info.bpp = Some(u16::from_le_bytes([data[24], data[25]]));
        if w > 0 && w <= 100_000 && h > 0 && h <= 100_000 {
            info.width = Some(w);
            info.height = Some(h);
        }
    }
    if let (Some(w), Some(h), Some(bpp)) = (info.width, info.height, info.bpp) {
        let row = (w as u64 * bpp as u64).div_ceil(32) * 4;
        let img = row * h as u64;
        if info.data_offset > 0 && img <= u64::from(u32::MAX) {
            info.pixel_end = Some(info.data_offset + img as usize);
        }
    }
    info
}

fn bmp_details(info: &BmpInfo) -> String {
    let mut parts = Vec::new();
    if let (Some(w), Some(h)) = (info.width, info.height) {
        parts.push(format!("{}x{}", w, h));
    }
    if let Some(bpp) = info.bpp {
        parts.push(format!("{}bpp", bpp));
    }
    if info.has_header {
        parts.push(format!("pixel offset {}", info.data_offset));
    } else {
        parts.push("header missing".to_string());
    }
    if let Some(e) = info.pixel_end {
        parts.push(format!("pixel data ends at {}", e));
    }
    parts.push(format!("declared {} bytes", info.declared_size));
    parts.join(", ")
}

fn find_bmp_start(data: &[u8]) -> Option<usize> {
    if data.len() < 18 {
        return None;
    }
    for i in 0..=data.len().saturating_sub(18) {
        if &data[i..i + 2] != BMP_SIG {
            continue;
        }
        let off =
            u32::from_le_bytes([data[i + 10], data[i + 11], data[i + 12], data[i + 13]]) as usize;
        let dib = u32::from_le_bytes([data[i + 14], data[i + 15], data[i + 16], data[i + 17]]);
        if (14..=data.len()).contains(&off) && (12..=256).contains(&dib) {
            return Some(i);
        }
    }
    None
}

/// Rebuild a valid BMP: trims trailing garbage past the end of the pixel data
/// and rewrites the file-size field at offset 2 to match the actual length.
fn repair_bmp(data: &[u8], start: Option<usize>) -> Option<Vec<u8>> {
    let start = start.or_else(|| find_bmp_start(data)).unwrap_or(0);
    if start >= data.len() {
        return None;
    }
    let slice = &data[start..];
    if !slice.starts_with(BMP_SIG) || slice.len() < 14 {
        return None;
    }
    let info = scan_bmp(slice);
    let end = info
        .pixel_end
        .filter(|&e| (info.data_offset..=slice.len()).contains(&e))
        .or((info.data_offset..=slice.len())
            .contains(&info.declared_size)
            .then_some(info.declared_size));
    let mut out = match end {
        Some(e) => slice[..e].to_vec(),
        None => slice.to_vec(),
    };
    if out.len() >= 6 {
        let sz = out.len() as u32;
        out[2] = (sz & 0xFF) as u8;
        out[3] = ((sz >> 8) & 0xFF) as u8;
        out[4] = ((sz >> 16) & 0xFF) as u8;
        out[5] = ((sz >> 24) & 0xFF) as u8;
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// RIFF (WAV/AVI) repair
// ---------------------------------------------------------------------------

const RIFF_SIG: &[u8] = b"RIFF";

#[derive(Debug, Clone, Default)]
struct RiffInfo {
    has_header: bool,
    form_type: String,
    declared_size: usize,
    last_chunk_end: Option<usize>,
    partial_chunk: Option<usize>,
    chunk_count: usize,
}

fn scan_riff(data: &[u8]) -> RiffInfo {
    let mut info = RiffInfo::default();
    if data.len() < 12 {
        return info;
    }
    info.has_header = data.starts_with(RIFF_SIG);
    info.declared_size = u32::from_le_bytes([data[4], data[5], data[6], data[7]]) as usize;
    info.form_type = String::from_utf8_lossy(&data[8..12]).to_string();
    let mut pos = 12usize;
    let mut guard = 0u64;
    while pos + 8 <= data.len() && guard < 1_000_000 {
        guard += 1;
        let chunk_size =
            u32::from_le_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]])
                as usize;
        let chunk_end = pos + 8 + chunk_size + (chunk_size & 1);
        if chunk_end > data.len() {
            info.partial_chunk = Some(pos);
            break;
        }
        info.last_chunk_end = Some(chunk_end);
        info.chunk_count += 1;
        pos = chunk_end;
    }
    info
}

fn riff_details(info: &RiffInfo) -> String {
    let mut parts = Vec::new();
    let form = info.form_type.trim();
    if !form.is_empty() {
        parts.push(format!("{} container", form));
    } else {
        parts.push("RIFF container".to_string());
    }
    parts.push(format!("{} chunks", info.chunk_count));
    parts.push(format!("declared {} bytes", info.declared_size));
    parts.join(", ")
}

fn find_riff_start(data: &[u8]) -> Option<usize> {
    if data.len() < 12 {
        return None;
    }
    for i in 0..=data.len().saturating_sub(12) {
        if &data[i..i + 4] != RIFF_SIG {
            continue;
        }
        let sz = u32::from_le_bytes([data[i + 4], data[i + 5], data[i + 6], data[i + 7]]) as usize;
        if sz < 4 || i + 8 + sz > data.len() {
            continue;
        }
        let form = &data[i + 8..i + 12];
        if form.iter().all(|b| b.is_ascii_uppercase() || *b == b' ') {
            return Some(i);
        }
    }
    None
}

/// Rebuild a valid RIFF container. The RIFF size field at offset 4 is treated
/// as authoritative: trailing bytes past the declared end are dropped as
/// garbage. If the file is shorter than declared (truncated mid-chunk), the
/// partial trailing chunk is kept and its own size field is corrected so the
/// salvaged data remains readable.
fn repair_riff(data: &[u8], start: Option<usize>) -> Option<Vec<u8>> {
    let start = start.or_else(|| find_riff_start(data)).unwrap_or(0);
    if start >= data.len() {
        return None;
    }
    let slice = &data[start..];
    if !slice.starts_with(RIFF_SIG) || slice.len() < 12 {
        return None;
    }
    let len = slice.len();
    let info = scan_riff(slice);
    let declared_end = if info.declared_size >= 4 {
        info.declared_size + 8
    } else {
        0
    };

    let out_len = if declared_end >= 12 && declared_end <= len {
        declared_end
    } else {
        len
    };
    let mut out = slice[..out_len].to_vec();

    if out.len() >= 8 {
        let sz = (out.len() - 8) as u32;
        out[4] = (sz & 0xFF) as u8;
        out[5] = ((sz >> 8) & 0xFF) as u8;
        out[6] = ((sz >> 16) & 0xFF) as u8;
        out[7] = ((sz >> 24) & 0xFF) as u8;
    }

    if let Some(p) = info.partial_chunk {
        if p >= 12 && p + 8 <= out.len() {
            let avail = (out.len() - p - 8) & !1;
            let sz = avail as u32;
            out[p + 4] = (sz & 0xFF) as u8;
            out[p + 5] = ((sz >> 8) & 0xFF) as u8;
            out[p + 6] = ((sz >> 16) & 0xFF) as u8;
            out[p + 7] = ((sz >> 24) & 0xFF) as u8;
        }
    }
    Some(out)
}

fn riff_extension(data: &[u8], start: usize) -> String {
    if start + 12 <= data.len() {
        let form = &data[start + 8..start + 12];
        if form == b"WAVE" {
            return ".wav".to_string();
        }
        if form == b"AVI " {
            return ".avi".to_string();
        }
    }
    ".riff".to_string()
}

pub fn repair_file(
    path: String,
    output_dir: String,
    analysis: FileAnalysis,
    event_sender: Sender<RepairEvent>,
    stop_flag: Arc<AtomicBool>,
) {
    thread::spawn(move || {
        let _ = event_sender.send(RepairEvent::Started);
        send_log(&event_sender, "Reading file...");

        if !analysis.payload.is_recoverable() {
            let msg = match analysis.payload.warn_text() {
                Some(t) => t.to_string(),
                None => "Image data payload cannot be recovered".to_string(),
            };
            let _ = event_sender.send(RepairEvent::Error(msg));
            return;
        }

        let data = match fs::read(&path) {
            Ok(d) => d,
            Err(e) => {
                let _ = event_sender.send(RepairEvent::Error(format!("Cannot read file: {}", e)));
                return;
            }
        };
        send_progress(&event_sender, 10.0, "File read successfully");

        if stop_flag.load(Ordering::SeqCst) {
            return;
        }

        let ft = analysis.file_type;
        send_log(&event_sender, format!("Detected type: {}", ft.name()));
        send_log(&event_sender, format!("Input size: {} bytes", data.len()));
        send_progress(&event_sender, 15.0, "Analysis complete");

        if stop_flag.load(Ordering::SeqCst) {
            return;
        }

        send_log(&event_sender, "Reconstructing file data...");
        send_progress(&event_sender, 30.0, "Reconstructing...");

        let repaired = if ft == FileType::JPEG {
            repair_jpeg(&data, analysis.embedded_offset)
        } else if ft == FileType::PNG || ft == FileType::Unknown {
            repair_png(&data, analysis.embedded_offset)
        } else if ft == FileType::BMP {
            repair_bmp(&data, analysis.embedded_offset)
        } else if ft == FileType::RIFF {
            repair_riff(&data, analysis.embedded_offset)
        } else if ft.can_repair() && !analysis.has_footer {
            if let Some(footer) = ft.footer_bytes() {
                let mut d = data.clone();
                d.extend_from_slice(footer);
                Some(d)
            } else {
                Some(data.clone())
            }
        } else {
            Some(data.clone())
        };

        let repaired = match repaired {
            Some(r) => r,
            None => {
                let _ =
                    event_sender.send(RepairEvent::Error("Could not reconstruct file".to_string()));
                return;
            }
        };

        if stop_flag.load(Ordering::SeqCst) {
            return;
        }

        send_progress(&event_sender, 60.0, "Repair reconstruction done");

        let filename = Path::new(&path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "repaired.bin".to_string());

        let out_name = if ft == FileType::Unknown && analysis.embedded_offset.is_some() {
            let mut n = filename;
            if !n.to_lowercase().ends_with(".png") {
                n.push_str(".png");
            }
            n
        } else if ft == FileType::JPEG
            && analysis.embedded_offset.map_or(false, |o| o > 0)
            && !filename.to_lowercase().ends_with(".jpg")
            && !filename.to_lowercase().ends_with(".jpeg")
        {
            let mut n = filename;
            n.push_str(".jpg");
            n
        } else if ft == FileType::BMP
            && analysis.embedded_offset.map_or(false, |o| o > 0)
            && !filename.to_lowercase().ends_with(".bmp")
        {
            let mut n = filename;
            n.push_str(".bmp");
            n
        } else if ft == FileType::RIFF && analysis.embedded_offset.is_some() {
            let ext = analysis
                .embedded_offset
                .map(|o| riff_extension(&data, o))
                .unwrap_or_else(|| ".riff".to_string());
            let mut n = filename;
            if !n.to_lowercase().ends_with(&ext) {
                n.push_str(&ext);
            }
            n
        } else {
            filename
        };

        let out_dir = if output_dir.is_empty() {
            "/tmp/repair_output".to_string()
        } else {
            output_dir
        };

        send_log(
            &event_sender,
            format!("Writing to: {}/{}", out_dir, out_name),
        );
        send_progress(&event_sender, 80.0, "Writing repaired output...");

        if let Err(e) = fs::create_dir_all(&out_dir) {
            let _ = event_sender.send(RepairEvent::Error(format!(
                "Cannot create output directory: {}",
                e
            )));
            return;
        }

        let out_path = Path::new(&out_dir).join(&out_name);
        let out_path_str = out_path.to_string_lossy().to_string();

        if let Err(e) = fs::write(&out_path, &repaired) {
            let _ = event_sender.send(RepairEvent::Error(format!("Cannot write output: {}", e)));
            return;
        }

        if stop_flag.load(Ordering::SeqCst) {
            let _ = std::fs::remove_file(&out_path);
            return;
        }

        send_progress(&event_sender, 95.0, "Verifying output...");
        send_log(
            &event_sender,
            format!("Output size: {} bytes", repaired.len()),
        );

        send_progress(&event_sender, 100.0, "Repair complete!");
        send_log(&event_sender, "File repaired successfully.");

        let _ = event_sender.send(RepairEvent::Complete {
            output_path: out_path_str,
            size: repaired.len() as u64,
        });
    });
}

impl FileType {
    fn footer_bytes(&self) -> Option<&'static [u8]> {
        match self {
            Self::JPEG => Some(&[0xFF, 0xD9]),
            Self::PNG => Some(b"IEND\xae\x42\x60\x82"),
            Self::GIF => Some(&[0x00, 0x3B]),
            Self::PDF => Some(b"%%EOF"),
            _ => None,
        }
    }
}

fn send_progress(sender: &Sender<RepairEvent>, percent: f64, message: &str) {
    let _ = sender.send(RepairEvent::Progress(RepairProgress {
        percent,
        message: message.to_string(),
    }));
}

fn send_log(sender: &Sender<RepairEvent>, message: impl Into<String>) {
    let _ = sender.send(RepairEvent::Log(message.into()));
}

// ---------------------------------------------------------------------------
// Deep analysis & repair
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct DeepSignature {
    pub offset: usize,
    pub label: &'static str,
    pub file_type: FileType,
}

#[derive(Debug, Clone)]
pub struct EmbeddedCandidate {
    pub offset: usize,
    pub file_type: FileType,
    pub label: &'static str,
    pub size: Option<u64>,
    pub valid: bool,
}

#[derive(Debug, Clone)]
pub struct DeepAnalysis {
    pub total_size: u64,
    pub primary: FileAnalysis,
    pub signatures: Vec<DeepSignature>,
    pub embedded_files: Vec<EmbeddedCandidate>,
    pub color_metadata: String,
}

/// jpegrepair operation sequences tried automatically for each JPEG candidate
/// in deep repair. Each targets a common structural failure; attempts that fail
/// to decode are skipped by the FFI caller.
const JEPGREPAIR_AUTO_OPS: &[&[&str]] = &[
    &["dest", "0", "0", "insert", "1"],
    &["dest", "0", "0", "delete", "1"],
];

const DEEP_SIGNATURES: &[(&[u8], FileType, &'static str)] = &[
    (&[0xFF, 0xD8, 0xFF], FileType::JPEG, "JPEG image"),
    (PNG_SIG, FileType::PNG, "PNG image"),
    (b"GIF8", FileType::GIF, "GIF image"),
    (b"%PDF", FileType::PDF, "PDF document"),
    (
        &[0x50, 0x4B, 0x03, 0x04],
        FileType::ZIP,
        "ZIP archive (local header)",
    ),
    (
        &[0x50, 0x4B, 0x05, 0x06],
        FileType::ZIP,
        "ZIP archive (end of central dir)",
    ),
    (b"GIF8", FileType::GIF, "GIF image"),
    (b"%PDF", FileType::PDF, "PDF document"),
    (
        &[0x50, 0x4B, 0x03, 0x04],
        FileType::ZIP,
        "ZIP archive (local header)",
    ),
    (
        &[0x50, 0x4B, 0x05, 0x06],
        FileType::ZIP,
        "ZIP archive (end of central dir)",
    ),
    (b"RIFF", FileType::RIFF, "RIFF (AVI/WAV)"),
    (b"BM", FileType::BMP, "BMP image"),
    (&[0x7F, b'E', b'L', b'F'], FileType::ELF, "ELF binary"),
    (b"ID3", FileType::MP3, "MP3 audio (ID3 tag)"),
    (b"fLaC", FileType::FLAC, "FLAC audio"),
    (b"II\x2a\x00", FileType::TIFF, "TIFF image (little-endian)"),
    (b"MM\x00\x2a", FileType::TIFF, "TIFF image (big-endian)"),
    (b"8BPS", FileType::PSD, "Photoshop document"),
    (b"MZ", FileType::EXE, "Windows executable"),
    (
        &[0xCA, 0xFE, 0xBA, 0xBE],
        FileType::JavaClass,
        "Java class file",
    ),
    (b"OggS", FileType::Unknown, "Ogg container"),
    (
        &[0x1A, 0x45, 0xDF, 0xA3],
        FileType::Unknown,
        "WebM/Matroska",
    ),
];

fn plausible_bmp(data: &[u8], i: usize) -> bool {
    if i + 18 > data.len() {
        return false;
    }
    let off = u32::from_le_bytes([data[i + 10], data[i + 11], data[i + 12], data[i + 13]]) as usize;
    let dib = u32::from_le_bytes([data[i + 14], data[i + 15], data[i + 16], data[i + 17]]);
    (14..=data.len()).contains(&off) && (12..=256).contains(&dib)
}

fn plausible_mz(data: &[u8], i: usize) -> bool {
    let ptr_off = i + 0x3C;
    if ptr_off + 4 <= data.len() {
        let ptr = u32::from_le_bytes([
            data[ptr_off],
            data[ptr_off + 1],
            data[ptr_off + 2],
            data[ptr_off + 3],
        ]) as usize;
        ptr >= 0x40 && i + ptr + 4 <= data.len() && &data[i + ptr..i + ptr + 4] == b"PE\x00\x00"
    } else {
        i == 0
    }
}

/// Scan the entire byte buffer for every known file signature, returning all
/// hits (up to a cap per signature) sorted by offset.
pub fn deep_scan(data: &[u8]) -> Vec<DeepSignature> {
    let mut hits: Vec<DeepSignature> = Vec::new();
    for &(sig, ft, label) in DEEP_SIGNATURES {
        let mut found = 0usize;
        let limit = data.len().saturating_sub(sig.len().saturating_sub(1));
        let mut i = 0usize;
        while i < limit {
            if data[i..].starts_with(sig) {
                let ok = match ft {
                    FileType::BMP => plausible_bmp(data, i),
                    FileType::EXE => plausible_mz(data, i),
                    _ => true,
                };
                if ok {
                    hits.push(DeepSignature {
                        offset: i,
                        label,
                        file_type: ft,
                    });
                    found += 1;
                    if found >= 64 {
                        break;
                    }
                    i += sig.len();
                    continue;
                }
            }
            i += 1;
        }
    }
    hits.sort_by_key(|h| h.offset);
    hits.dedup_by_key(|h| h.offset);
    hits
}

fn signature_end(data: &[u8], off: usize, ft: FileType) -> Option<usize> {
    if off >= data.len() {
        return None;
    }
    match ft {
        FileType::JPEG => scan_jpeg(data, off).image_end,
        FileType::PNG => {
            let iend_sig = b"IEND";
            let mut i = off;
            while i + 4 <= data.len() {
                if &data[i..i + 4] == iend_sig {
                    return Some((i + 8).min(data.len()));
                }
                i += 1;
            }
            None
        }
        FileType::BMP => {
            let info = scan_bmp(&data[off..]);
            info.pixel_end.map(|e| off + e)
        }
        FileType::RIFF => {
            let info = scan_riff(&data[off..]);
            info.last_chunk_end.map(|e| off + e)
        }
        FileType::ZIP => find_zip_eocd(data).filter(|&e| e >= off),
        _ => None,
    }
}

fn candidate_valid(data: &[u8], off: usize, ft: FileType) -> bool {
    if off >= data.len() {
        return false;
    }
    match ft {
        FileType::PNG => has_valid_iend(&data[off..]),
        FileType::JPEG => scan_jpeg(data, off).terminated,
        FileType::BMP => scan_bmp(&data[off..]).pixel_end.is_some(),
        FileType::RIFF => scan_riff(&data[off..]).last_chunk_end.is_some(),
        FileType::ZIP => find_zip_eocd(data).filter(|&e| e >= off).is_some(),
        FileType::GIF => check_footer(&data[off..], ft),
        FileType::PDF => check_footer(&data[off..], ft),
        _ => true,
    }
}

/// Deep analysis: normal header/footer analysis plus a full signature scan that
/// reports every embedded file candidate found anywhere in the buffer.
pub fn analyze_file_deep(path: &str) -> Result<DeepAnalysis, String> {
    let data = fs::read(path).map_err(|e| format!("Cannot read file: {}", e))?;
    let total_size = data.len() as u64;
    let primary = analyze_bytes(&data);
    let signatures = deep_scan(&data);

    let sig_offsets: Vec<usize> = signatures.iter().map(|s| s.offset).collect();
    let embedded_files: Vec<EmbeddedCandidate> = signatures
        .iter()
        .enumerate()
        .map(|(idx, sig)| {
            let size = signature_end(&data, sig.offset, sig.file_type)
                .map(|e| (e - sig.offset) as u64)
                .or_else(|| {
                    sig_offsets
                        .get(idx + 1)
                        .copied()
                        .filter(|&next| next > sig.offset)
                        .map(|next| (next - sig.offset) as u64)
                });
            EmbeddedCandidate {
                offset: sig.offset,
                file_type: sig.file_type,
                label: sig.label,
                size,
                valid: candidate_valid(&data, sig.offset, sig.file_type),
            }
        })
        .collect();

    let color_metadata = color_metadata(&data, primary.file_type);
    Ok(DeepAnalysis {
        total_size,
        primary,
        signatures,
        embedded_files,
        color_metadata,
    })
}

// ---------------------------------------------------------------------------
// Color / geometry metadata analysis
// ---------------------------------------------------------------------------

fn color_type_name(ct: u8) -> &'static str {
    match ct {
        0 => "Grayscale",
        2 => "Truecolor RGB",
        3 => "Indexed (palette)",
        4 => "Grayscale + Alpha",
        6 => "Truecolor RGBA",
        _ => "Unknown",
    }
}

fn png_channels(ct: u8) -> u64 {
    match ct {
        0 => 1,
        2 => 3,
        3 => 1,
        4 => 2,
        6 => 4,
        _ => 3,
    }
}

fn png_color_metadata(data: &[u8]) -> String {
    let mut lines = Vec::new();
    let chunks = png_chunks(data);
    if let Some(c) = chunks.iter().find(|c| &c.typ == b"IHDR") {
        if let Some((w, h, bd, ct)) = try_extract_ihdr(data, c.start + 4) {
            lines.push(format!("Dimensions: {}x{} px", w, h));
            lines.push(format!("Bit depth: {}", bd));
            lines.push(format!("Color type: {} ({})", ct, color_type_name(ct)));
        }
    }
    for c in &chunks {
        let name = match std::str::from_utf8(&c.typ) {
            Ok(s) => s,
            Err(_) => continue,
        };
        match name {
            "gAMA" => {
                let p = c.start + 8;
                if p + 4 <= data.len() {
                    let v = u32::from_be_bytes([data[p], data[p + 1], data[p + 2], data[p + 3]]);
                    lines.push(format!("gAMA: gamma {:.4}", v as f64 / 100_000.0));
                }
            }
            "cHRM" => lines.push("cHRM: chromaticities present".to_string()),
            "sRGB" => lines.push("sRGB: standard RGB profile".to_string()),
            "iCCP" => lines.push("iCCP: embedded ICC color profile".to_string()),
            "pHYs" => lines.push("pHYs: pixel aspect ratio".to_string()),
            "tRNS" => lines.push("tRNS: transparency present".to_string()),
            "bKGD" => lines.push("bKGD: background color".to_string()),
            "PLTE" => lines.push(format!("PLTE: palette ({} entries)", c.len / 3)),
            "tEXt" | "zTXt" | "iTXt" => lines.push(format!("{}: text metadata", name)),
            _ => {}
        }
    }
    lines.join("\n")
}

fn jpeg_sof(data: &[u8], start: usize) -> Option<(u8, u32, u32, u8)> {
    let len = data.len();
    let mut pos = start;
    let mut guard = 0u64;
    while pos < len && guard < 1_000_000 {
        guard += 1;
        if data[pos] != 0xFF {
            return None;
        }
        let mut j = pos + 1;
        while j < len && data[j] == 0xFF {
            j += 1;
        }
        if j >= len {
            return None;
        }
        let code = data[j];
        if code == 0xD8 {
            pos = j + 1;
            continue;
        }
        if code == 0xD9 || code == 0xDA {
            return None;
        }
        if code == 0x01 || (0xD0..=0xD7).contains(&code) {
            pos = j + 1;
            continue;
        }
        if is_jpeg_sof(code) {
            let lp = j + 1;
            if lp + 2 <= len {
                let seg_len = u16::from_be_bytes([data[lp], data[lp + 1]]) as usize;
                if seg_len >= 8 && lp + seg_len <= len {
                    let precision = data[lp + 2];
                    let h = u16::from_be_bytes([data[lp + 3], data[lp + 4]]) as u32;
                    let w = u16::from_be_bytes([data[lp + 5], data[lp + 6]]) as u32;
                    let comps = data[lp + 7];
                    return Some((precision, w, h, comps));
                }
            }
            return None;
        }
        pos = j + 1;
        if pos + 2 > len {
            if let Some(np) = skip_to_next_marker(data, pos) {
                pos = np;
                continue;
            }
            break;
        }
        let seg_len = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
        if seg_len < 2 || pos + seg_len > len {
            if let Some(np) = skip_to_next_marker(data, pos) {
                pos = np;
                continue;
            }
            break;
        }
        pos += seg_len;
    }
    None
}

fn jpeg_app_markers(data: &[u8], start: usize) -> Vec<String> {
    let mut out = Vec::new();
    let len = data.len();
    let mut pos = start;
    let mut guard = 0u64;
    while pos < len && guard < 1_000_000 {
        guard += 1;
        if data[pos] != 0xFF {
            break;
        }
        let mut j = pos + 1;
        while j < len && data[j] == 0xFF {
            j += 1;
        }
        if j >= len {
            break;
        }
        let code = data[j];
        if code == 0xD9 || code == 0xDA {
            break;
        }
        if code == 0xD8 || code == 0x01 || (0xD0..=0xD7).contains(&code) {
            pos = j + 1;
            continue;
        }
        if (0xE0..=0xEF).contains(&code) {
            let lp = j + 1;
            if lp + 2 <= len {
                let seg_len = u16::from_be_bytes([data[lp], data[lp + 1]]) as usize;
                if seg_len >= 2 && lp + seg_len <= len {
                    let id = &data[lp + 2..lp + seg_len];
                    let label = if id.starts_with(b"JFIF") {
                        "JFIF (JPEG File Interchange)".to_string()
                    } else if id.starts_with(b"Exif") {
                        "Exif metadata".to_string()
                    } else if id.starts_with(b"ICC_PROFILE") {
                        "ICC color profile".to_string()
                    } else if id.starts_with(b"Adobe") {
                        "Adobe color transform".to_string()
                    } else if code == 0xE0 {
                        format!("APP0 ({})", String::from_utf8_lossy(id))
                    } else {
                        format!("APP{}", code - 0xE0)
                    };
                    out.push(label);
                }
                pos = lp + seg_len;
            } else {
                break;
            }
            continue;
        }
        pos = j + 1;
        if pos + 2 > len {
            break;
        }
        let seg_len = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
        if seg_len < 2 || pos + seg_len > len {
            break;
        }
        pos += seg_len;
    }
    out
}

fn jpeg_color_metadata(data: &[u8]) -> String {
    let mut lines = Vec::new();
    let start = find_jpeg_start(data).unwrap_or(0);
    if let Some((precision, w, h, comps)) = jpeg_sof(data, start) {
        lines.push(format!("Precision: {} bits/component", precision));
        lines.push(format!("Dimensions: {}x{} px", w, h));
        lines.push(format!(
            "Components: {} ({})",
            comps,
            jpeg_colorspace(comps)
        ));
    }
    let app = jpeg_app_markers(data, start);
    if !app.is_empty() {
        lines.push(format!("Color/metadata markers: {}", app.join(", ")));
    }
    lines.join("\n")
}

fn jpeg_colorspace(comps: u8) -> &'static str {
    match comps {
        1 => "Grayscale",
        3 => "YCbCr (color)",
        4 => "CMYK",
        _ => "Unknown",
    }
}

fn bmp_compression_name(c: u32) -> &'static str {
    match c {
        0 => "BI_RGB (none)",
        1 => "BI_RLE8",
        2 => "BI_RLE4",
        3 => "BI_BITFIELDS",
        4 => "BI_JPEG",
        5 => "BI_PNG",
        6 => "BI_ALPHABITFIELDS",
        _ => "Unknown",
    }
}

fn bmp_color_metadata(data: &[u8]) -> String {
    let mut lines = Vec::new();
    let info = scan_bmp(data);
    if let (Some(w), Some(h)) = (info.width, info.height) {
        lines.push(format!("Dimensions: {}x{} px", w, h));
    }
    if let Some(bpp) = info.bpp {
        lines.push(format!("Bits per pixel: {}", bpp));
    }
    if data.len() >= 34 {
        let planes = u16::from_le_bytes([data[26], data[27]]);
        lines.push(format!("Color planes: {}", planes));
        let comp = u32::from_le_bytes([data[30], data[31], data[32], data[33]]);
        lines.push(format!("Compression: {}", bmp_compression_name(comp)));
    }
    if data.len() >= 50 {
        let palette = u32::from_le_bytes([data[46], data[47], data[48], data[49]]);
        if palette > 0 {
            lines.push(format!("Palette colors: {}", palette));
        }
    }
    lines.join("\n")
}

fn gif_color_metadata(data: &[u8]) -> String {
    let mut lines = Vec::new();
    if data.len() >= 6 {
        let version = String::from_utf8_lossy(&data[3..6]).to_string();
        lines.push(format!("Version: GIF{}", version));
    }
    if data.len() >= 13 {
        let w = u16::from_le_bytes([data[6], data[7]]);
        let h = u16::from_le_bytes([data[8], data[9]]);
        lines.push(format!("Dimensions: {}x{} px", w, h));
        let packed = data[10];
        if packed & 0x80 != 0 {
            let colors = 2u32 << (packed & 0x07);
            lines.push(format!("Global color table: {} colors", colors));
        } else {
            lines.push("No global color table".to_string());
        }
        lines.push(format!("Background index: {}", data[11]));
    }
    lines.join("\n")
}

fn find_riff_chunk(data: &[u8], id: &[u8; 4]) -> Option<(usize, usize)> {
    let mut pos = 12usize;
    let mut guard = 0u64;
    while pos + 8 <= data.len() && guard < 1_000_000 {
        guard += 1;
        let sz = u32::from_le_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]])
            as usize;
        if &data[pos..pos + 4] == id {
            return Some((pos + 8, sz));
        }
        let chunk_end = pos + 8 + sz + (sz & 1);
        if chunk_end > data.len() {
            break;
        }
        pos = chunk_end;
    }
    None
}

fn wav_format_name(f: u16) -> &'static str {
    match f {
        1 => "PCM",
        3 => "IEEE float",
        6 => "A-law",
        7 => "mu-law",
        0x11 => "IMA ADPCM",
        0x55 => "MPEG Layer 3",
        0xFFFE => "WAVE_FORMAT_EXTENSIBLE",
        _ => "Unknown",
    }
}

fn riff_color_metadata(data: &[u8]) -> String {
    let mut lines = Vec::new();
    if data.len() >= 12 {
        let form = String::from_utf8_lossy(&data[8..12]).to_string();
        lines.push(format!("Container: {}", form.trim()));
        if form == "WAVE" {
            if let Some((p, sz)) = find_riff_chunk(data, b"fmt ") {
                if sz >= 16 && p + 16 <= data.len() {
                    let af = u16::from_le_bytes([data[p], data[p + 1]]);
                    let ch = u16::from_le_bytes([data[p + 2], data[p + 3]]);
                    let rate =
                        u32::from_le_bytes([data[p + 4], data[p + 5], data[p + 6], data[p + 7]]);
                    let bits = u16::from_le_bytes([data[p + 14], data[p + 15]]);
                    lines.push(format!(
                        "Audio: {} ({}), {} channel(s), {} Hz, {} bits",
                        af,
                        wav_format_name(af),
                        ch,
                        rate,
                        bits
                    ));
                }
            }
        }
    }
    lines.join("\n")
}

/// Extract human-readable color/geometry metadata for the given file type.
/// Returns an empty string when the type carries no such metadata.
pub fn color_metadata(data: &[u8], ft: FileType) -> String {
    match ft {
        FileType::PNG => png_color_metadata(data),
        FileType::JPEG => jpeg_color_metadata(data),
        FileType::BMP => bmp_color_metadata(data),
        FileType::GIF => gif_color_metadata(data),
        FileType::RIFF => riff_color_metadata(data),
        _ => String::new(),
    }
}

fn find_zip_eocd(data: &[u8]) -> Option<usize> {
    if data.len() < 4 {
        return None;
    }
    let sig = b"PK\x05\x06";
    let max = data.len().saturating_sub(sig.len());
    for i in (0..=max).rev() {
        if &data[i..i + sig.len()] == sig {
            return Some(i);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Salvage repair — prioritize "opens in a viewer" over pixel fidelity
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct PngChunkRef {
    start: usize,
    len: usize,
    typ: [u8; 4],
}

/// Parse a length/type/data/crc chunk table starting after the 8-byte PNG
/// signature. Stops at the first malformed or out-of-bounds chunk.
fn png_chunks(data: &[u8]) -> Vec<PngChunkRef> {
    let mut v = Vec::new();
    if data.len() < 8 {
        return v;
    }
    let mut pos = 8usize;
    while pos + 12 <= data.len() {
        let len =
            u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;
        if pos + 12 + len > data.len() {
            break;
        }
        let mut typ = [0u8; 4];
        typ.copy_from_slice(&data[pos + 4..pos + 8]);
        v.push(PngChunkRef {
            start: pos,
            len,
            typ,
        });
        pos += 12 + len;
        if &typ == b"IEND" {
            break;
        }
    }
    v
}

/// Guarantee the PNG is *decodable* by any image viewer:
/// - recomputes CRC on every preserved chunk,
/// - keeps the original IDAT only when it decompresses and matches the IHDR
///   scanline arithmetic,
/// - otherwise replaces it with a synthetically compressed solid-color image of
///   the same dimensions (colors/pixels are not preserved, but the file opens).
fn salvage_png(data: &[u8], offset: Option<usize>) -> Option<Vec<u8>> {
    let base = repair_png(data, offset)?;
    let chunks = png_chunks(&base);
    if chunks.is_empty() {
        return Some(base);
    }

    let mut ihdr: Option<(u32, u32, u8, u8)> = None;
    let mut idat_concat = Vec::new();
    let mut saw_idat = false;
    for c in &chunks {
        if &c.typ == b"IHDR" {
            ihdr = try_extract_ihdr(&base, c.start + 4);
        } else if &c.typ == b"IDAT" {
            saw_idat = true;
            idat_concat.extend_from_slice(&base[c.start + 8..c.start + 8 + c.len]);
        }
    }

    let (w, h, bd, ct) = match ihdr {
        Some(v) => v,
        None => decompress_to_vec_zlib(&idat_concat)
            .ok()
            .and_then(|d| Some(estimate_ihdr(&d)))
            .unwrap_or((1, 1, 8, 2)),
    };

    let idat_ok = if saw_idat {
        decompress_to_vec_zlib(&idat_concat)
            .map(|d| {
                let channels = png_channels(ct);
                let row = (w as u64 * channels as u64 * bd as u64).div_ceil(8).max(1);
                let expected = h as u64 * (row + 1);
                d.len() as u64 >= expected
            })
            .unwrap_or(false)
    } else {
        false
    };

    // Palette (color type 3) images require a PLTE chunk; avoid that constraint
    // when synthesizing placeholder data so the output always opens.
    let (w2, h2, bd2, ct2) = if idat_ok {
        (w, h, bd, ct)
    } else {
        let ct2 = if ct == 3 { 2 } else { ct };
        (w.max(1), h.max(1), bd, ct2)
    };

    let mut out = Vec::new();
    out.extend_from_slice(PNG_SIG);
    out.extend_from_slice(&build_ihdr(w2, h2, bd2, ct2));

    for c in &chunks {
        if c.typ == *b"IHDR" || c.typ == *b"IDAT" || c.typ == *b"IEND" {
            continue;
        }
        let end = c.start + 4 + 4 + c.len;
        if end > base.len() {
            continue;
        }
        let mut chunk = base[c.start..end].to_vec();
        let crc = calc_crc(&chunk[4..]);
        chunk.truncate(4 + 4 + c.len);
        chunk.extend_from_slice(&crc.to_be_bytes());
        out.extend_from_slice(&chunk);
    }

    let idat_data = if idat_ok {
        idat_concat
    } else {
        synth_idat(w2, h2, bd2, ct2)
    };

    let mut idat_chunk = Vec::with_capacity(12 + idat_data.len());
    idat_chunk.extend_from_slice(&(idat_data.len() as u32).to_be_bytes());
    idat_chunk.extend_from_slice(b"IDAT");
    idat_chunk.extend_from_slice(&idat_data);
    let crc = calc_crc(&idat_chunk[4..]);
    idat_chunk.extend_from_slice(&crc.to_be_bytes());
    out.extend_from_slice(&idat_chunk);

    out.extend_from_slice(&build_iend());
    Some(out)
}

/// Build a valid, decodable IDAT for a solid-color (dark gray) image matching
/// the given IHDR geometry. Always produces a well-formed zlib stream so the
/// PNG opens even when the original pixel data was destroyed.
fn synth_idat(width: u32, height: u32, bit_depth: u8, color_type: u8) -> Vec<u8> {
    let channels = png_channels(color_type);
    let row = ((width as u64 * channels as u64 * bit_depth as u64).div_ceil(8)).max(1);
    let mut raw = Vec::with_capacity(((row + 1) * height as u64) as usize);
    for _ in 0..height {
        raw.push(0); // filter type 0 (None)
        for _ in 0..row {
            raw.push(0x33);
        }
    }
    miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6)
}

/// Guarantee the GIF opens: ensure a valid header, a logical screen descriptor,
/// and a trailer byte (0x3B). Keeps whatever image data is salvageable.
fn salvage_gif(data: &[u8], off: usize) -> Option<Vec<u8>> {
    let slice = if off == 0 { data } else { &data[off..] };
    if slice.len() < 6 {
        return None;
    }
    let mut out = Vec::new();
    if &slice[0..3] == b"GIF"
        && slice.len() >= 6
        && (&slice[3..6] == b"87a" || &slice[3..6] == b"89a")
    {
        out.extend_from_slice(&slice[0..6]);
    } else {
        out.extend_from_slice(b"GIF89a");
    }

    if slice.len() >= 13 {
        out.extend_from_slice(&slice[6..13]);
        let packed = slice[10];
        let gct_size = if packed & 0x80 != 0 {
            3usize << ((packed & 0x07) + 1)
        } else {
            0
        };
        let after_lsd = 13 + gct_size;
        if slice.len() >= after_lsd {
            out.extend_from_slice(&slice[13..after_lsd]);
            let rest = &slice[after_lsd..];
            if let Some(t) = find_trailer(rest) {
                out.extend_from_slice(&rest[..t + 1]);
            } else {
                out.extend_from_slice(rest);
            }
        }
    } else {
        out.extend_from_slice(&[0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00]); // 1x1 screen
    }

    if !out.ends_with(&[0x3B]) {
        out.push(0x3B);
    }
    Some(out)
}

fn find_trailer(data: &[u8]) -> Option<usize> {
    (0..data.len()).rev().find(|&i| data[i] == 0x3B)
}

fn repair_zip_from(data: &[u8], off: usize) -> Option<Vec<u8>> {
    let eocd = find_zip_eocd(data)?;
    if eocd + 22 > data.len() {
        return None;
    }
    let comment_len = u16::from_le_bytes([data[eocd + 20], data[eocd + 21]]) as usize;
    let end = (eocd + 22 + comment_len).min(data.len());
    if end < off {
        return None;
    }
    let start = if off == 0 { 0 } else { off };
    Some(data[start..end].to_vec())
}

fn deep_repair_one(data: &[u8], off: usize, ft: FileType) -> Option<Vec<u8>> {
    match ft {
        FileType::PNG => salvage_png(data, Some(off)),
        FileType::JPEG => repair_jpeg(data, Some(off)),
        FileType::BMP => repair_bmp(data, Some(off)),
        FileType::RIFF => repair_riff(data, Some(off)),
        FileType::ZIP => repair_zip_from(data, off),
        FileType::GIF => salvage_gif(data, off),
        FileType::PDF => {
            let slice = if off == 0 { data } else { &data[off..] };
            if slice.is_empty() {
                return None;
            }
            if !candidate_valid(data, off, ft) {
                let mut out = slice.to_vec();
                if let Some(footer) = ft.footer_bytes() {
                    out.extend_from_slice(footer);
                }
                Some(out)
            } else {
                Some(slice.to_vec())
            }
        }
        _ => {
            if off == 0 {
                Some(data.to_vec())
            } else if off < data.len() {
                Some(data[off..].to_vec())
            } else {
                None
            }
        }
    }
}

fn deep_repair_valid(out: &[u8], ft: FileType) -> bool {
    match ft {
        FileType::PNG => out.starts_with(PNG_SIG) && has_valid_iend(out),
        FileType::JPEG => {
            out.starts_with(JPEG_SOI) && scan_jpeg(out, 0).terminated && scan_jpeg(out, 0).has_sos
        }
        FileType::BMP => out.starts_with(BMP_SIG),
        FileType::RIFF => out.starts_with(RIFF_SIG),
        FileType::ZIP => find_zip_eocd(out).is_some(),
        FileType::GIF => check_footer(out, ft),
        FileType::PDF => check_footer(out, ft),
        _ => !out.is_empty(),
    }
}

/// Count the number of SOS (FF DA) markers in an output buffer. More scans
/// means more surviving image data was preserved, which is the best proxy we
/// have for "this candidate actually contains color/image content" without
/// decoding it.
fn jpeg_scan_count(out: &[u8]) -> u32 {
    let mut n = 0u32;
    for i in 0..out.len().saturating_sub(1) {
        if out[i] == 0xFF && out[i + 1] == 0xDA {
            n += 1;
        }
    }
    n
}

fn attempt_score(
    offset: usize,
    ft: FileType,
    out: &[u8],
    primary_off: Option<usize>,
) -> (u8, u8, u32, usize) {
    let primary_bonus = if Some(offset) == primary_off { 1 } else { 0 };
    let valid = deep_repair_valid(out, ft);
    let tier = if ft == FileType::JPEG {
        let info = scan_jpeg(out, 0);
        if !info.has_sos {
            0
        } else {
            let mut t = 0u8;
            if info.width.is_some() && info.height.is_some() {
                t += 1;
            }
            if info.has_dqt {
                t += 1;
            }
            if info.has_dht {
                t += 1;
            }
            if info.has_sos {
                t += 1;
            }
            if info.terminated {
                t += 1;
            }
            t
        }
    } else if valid {
        1
    } else {
        0
    };
    let richness = if ft == FileType::JPEG {
        jpeg_scan_count(out)
    } else {
        0
    };
    (primary_bonus, tier, richness, out.len())
}

fn deep_output_name(filename: &str, ft: FileType, offset: usize, data: &[u8]) -> String {
    let lower = filename.to_lowercase();
    let mut n = filename.to_string();
    match ft {
        FileType::PNG if !lower.ends_with(".png") => n.push_str(".png"),
        FileType::JPEG if !lower.ends_with(".jpg") && !lower.ends_with(".jpeg") => {
            n.push_str(".jpg")
        }
        FileType::BMP if !lower.ends_with(".bmp") => n.push_str(".bmp"),
        FileType::GIF if !lower.ends_with(".gif") => n.push_str(".gif"),
        FileType::PDF if !lower.ends_with(".pdf") => n.push_str(".pdf"),
        FileType::ZIP if !lower.ends_with(".zip") => n.push_str(".zip"),
        FileType::RIFF => {
            let ext = riff_extension(data, offset);
            if !lower.ends_with(&ext) {
                n.push_str(&ext);
            }
        }
        _ => {}
    }
    n
}

/// Deep repair: attempt to reconstruct every embedded candidate found by the
/// deep scan, then write the best (most structurally valid) result.
pub fn repair_file_deep(
    path: String,
    output_dir: String,
    analysis: DeepAnalysis,
    event_sender: Sender<RepairEvent>,
    stop_flag: Arc<AtomicBool>,
    reference_path: String,
) {
    thread::spawn(move || {
        let _ = event_sender.send(RepairEvent::Started);
        send_log(&event_sender, "Deep repair: reading file...");

        let data = match fs::read(&path) {
            Ok(d) => d,
            Err(e) => {
                let _ = event_sender.send(RepairEvent::Error(format!("Cannot read file: {}", e)));
                return;
            }
        };
        send_progress(&event_sender, 8.0, "File read");

        let template = if reference_path.is_empty() {
            None
        } else {
            send_log(
                &event_sender,
                format!("Loading reference file: {}", reference_path),
            );
            match fs::read(&reference_path) {
                Ok(ref_data) => match extract_jpeg_template(&ref_data) {
                    Some(tpl) => {
                        send_log(
                            &event_sender,
                            format!("Reference template ready: {}", jpeg_template_info(&tpl)),
                        );
                        Some(tpl)
                    }
                    None => {
                        send_log(
                            &event_sender,
                            "Reference file is not a usable JPEG template — ignoring it",
                        );
                        None
                    }
                },
                Err(e) => {
                    send_log(
                        &event_sender,
                        format!("Cannot read reference file: {} — ignoring it", e),
                    );
                    None
                }
            }
        };

        if stop_flag.load(Ordering::SeqCst) {
            return;
        }

        send_log(
            &event_sender,
            format!(
                "Found {} embedded candidates",
                analysis.embedded_files.len()
            ),
        );

        let mut attempts: Vec<(usize, FileType, &'static str, Vec<u8>)> = Vec::new();
        for cand in &analysis.embedded_files {
            if cand.offset >= data.len() {
                continue;
            }
            if stop_flag.load(Ordering::SeqCst) {
                return;
            }
            send_log(
                &event_sender,
                format!("  → Attempting {} at offset {}...", cand.label, cand.offset),
            );
            if let Some(out) = deep_repair_one(&data, cand.offset, cand.file_type) {
                attempts.push((cand.offset, cand.file_type, cand.label, out));
            }
            if cand.file_type == FileType::JPEG {
                if let Some(ref tpl) = template {
                    if let Some(out) = repair_jpeg_with_ref(&data, Some(cand.offset), tpl) {
                        attempts.push((cand.offset, cand.file_type, "JPEG (reference)", out));
                    }
                }
                // jpegrepair block-level structural ops: attempt to fix
                // misaligned / sliced / duplicated-scanline corruption by
                // shifting the DCT block grid. Requires a decodable
                // coefficient stream; corrupted entropy simply fails here.
                let slice = &data[cand.offset..];
                for ops in JEPGREPAIR_AUTO_OPS {
                    if let Some(out) = crate::backend::jpegrepair_ffi::jpegrepair_mem(slice, ops) {
                        attempts.push((cand.offset, cand.file_type, ops[0], out));
                    }
                }
            }
        }

        if attempts.is_empty() {
            send_log(
                &event_sender,
                "No embedded candidates — falling back to classic repair.",
            );
            let ft = analysis.primary.file_type;
            let mut fallback = Vec::new();
            let primary_off = analysis.primary.embedded_offset;
            if ft == FileType::JPEG {
                if let Some(out) = repair_jpeg(&data, primary_off) {
                    fallback.push((0, ft, analysis.primary.file_type.name(), out));
                }
                if let Some(ref tpl) = template {
                    if let Some(out) = repair_jpeg_with_ref(&data, primary_off, tpl) {
                        fallback.push((0, ft, "JPEG (reference)", out));
                    }
                }
                let off = primary_off.unwrap_or(0);
                for ops in JEPGREPAIR_AUTO_OPS {
                    if let Some(out) =
                        crate::backend::jpegrepair_ffi::jpegrepair_mem(&data[off..], ops)
                    {
                        fallback.push((off, ft, ops[0], out));
                    }
                }
            } else if ft == FileType::PNG || ft == FileType::Unknown {
                if let Some(out) = salvage_png(&data, primary_off) {
                    fallback.push((0, ft, analysis.primary.file_type.name(), out));
                }
            } else if ft == FileType::GIF {
                if let Some(out) = salvage_gif(&data, 0) {
                    fallback.push((0, ft, analysis.primary.file_type.name(), out));
                }
            } else if ft == FileType::BMP {
                if let Some(out) = repair_bmp(&data, primary_off) {
                    fallback.push((0, ft, analysis.primary.file_type.name(), out));
                }
            } else if ft == FileType::RIFF {
                if let Some(out) = repair_riff(&data, primary_off) {
                    fallback.push((0, ft, analysis.primary.file_type.name(), out));
                }
            } else if ft != FileType::JPEG && ft.can_repair() && !analysis.primary.has_footer {
                let mut d = data.clone();
                if let Some(footer) = ft.footer_bytes() {
                    d.extend_from_slice(footer);
                }
                if !d.is_empty() {
                    fallback.push((0, ft, analysis.primary.file_type.name(), d));
                }
            } else {
                fallback.push((0, ft, analysis.primary.file_type.name(), data.clone()));
            }
            attempts.extend(fallback);
        }

        // A JPEG whose scan structure (SOF/DHT/DQT/SOS) is destroyed cannot be
        // reconstructed without a reference template. Fail with a helpful error
        // instead of writing an undecodable file.
        let primary_is_jpeg = analysis.primary.file_type == FileType::JPEG;
        let jpeg_sos_ok = attempts
            .iter()
            .any(|(_, ft, _, out)| *ft == FileType::JPEG && scan_jpeg(out, 0).has_sos);
        if attempts.is_empty() || (primary_is_jpeg && !jpeg_sos_ok) {
            let _ = event_sender.send(RepairEvent::Error(if primary_is_jpeg {
                "Could not reconstruct a viewable JPEG: its scan headers \
                     (SOF/DHT/DQT/SOS) are destroyed. Choose any JPEG photo taken \
                     with the same camera model as the reference and retry."
                    .to_string()
            } else {
                "Could not reconstruct file".to_string()
            }));
            return;
        }

        let primary_off = analysis.primary.embedded_offset;
        let best = attempts
            .into_iter()
            .max_by(|a, b| {
                attempt_score(a.0, a.1, &a.3, primary_off).cmp(&attempt_score(
                    b.0,
                    b.1,
                    &b.3,
                    primary_off,
                ))
            })
            .unwrap();
        let (chosen_offset, chosen_ft, chosen_label, repaired) = best;

        if stop_flag.load(Ordering::SeqCst) {
            return;
        }

        send_progress(&event_sender, 55.0, "Best candidate selected");
        send_log(
            &event_sender,
            format!("Selected: {} at offset {}", chosen_label, chosen_offset),
        );
        send_log(
            &event_sender,
            format!("Output size: {} bytes", repaired.len()),
        );
        if chosen_ft == FileType::JPEG && jpeg_parts(&data, chosen_offset).entropy.is_none() {
            send_log(
                &event_sender,
                "Warning: no recoverable image data was found — the repaired JPEG shows a \
                 plain gray placeholder (only the photo structure was rebuilt).",
            );
        }

        let filename = Path::new(&path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "repaired.bin".to_string());

        let out_name = deep_output_name(&filename, chosen_ft, chosen_offset, &repaired);

        let out_dir = if output_dir.is_empty() {
            "/tmp/repair_output".to_string()
        } else {
            output_dir
        };

        send_log(
            &event_sender,
            format!("Writing to: {}/{}", out_dir, out_name),
        );
        send_progress(&event_sender, 80.0, "Writing repaired output...");

        if let Err(e) = fs::create_dir_all(&out_dir) {
            let _ = event_sender.send(RepairEvent::Error(format!(
                "Cannot create output directory: {}",
                e
            )));
            return;
        }

        let out_path = Path::new(&out_dir).join(&out_name);
        let out_path_str = out_path.to_string_lossy().to_string();

        if let Err(e) = fs::write(&out_path, &repaired) {
            let _ = event_sender.send(RepairEvent::Error(format!("Cannot write output: {}", e)));
            return;
        }

        if stop_flag.load(Ordering::SeqCst) {
            let _ = std::fs::remove_file(&out_path);
            return;
        }

        send_progress(&event_sender, 95.0, "Verifying output...");
        send_log(
            &event_sender,
            format!("Output size: {} bytes", repaired.len()),
        );

        send_progress(&event_sender, 100.0, "Deep repair complete!");
        send_log(&event_sender, "File repaired successfully.");

        let _ = event_sender.send(RepairEvent::Complete {
            output_path: out_path_str,
            size: repaired.len() as u64,
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_repair_png_partial_sig() {
        let data = std::fs::read("png.png").unwrap();
        let result = repair_png(&data, None);
        assert!(
            result.is_some(),
            "repair_png should return Some for png.png"
        );
        let out = result.unwrap();
        assert!(
            out.starts_with(PNG_SIG),
            "output must start with PNG signature"
        );
        assert!(
            out.len() > data.len() - 8,
            "output should be at least as large as input minus zeroed sig"
        );
        let ihdr_pos = find_chunk(&out, b"IHDR").expect("output must contain IHDR");
        let (w, h, _bd, _ct) = try_extract_ihdr(&out, ihdr_pos).expect("IHDR must be valid");
        assert_eq!(w, 500, "width should be 500");
        assert_eq!(h, 500, "height should be 500");
        assert!(has_valid_iend(&out), "output must have valid IEND");
        // Write repaired output for manual inspection
        let _ = std::fs::write("/tmp/test_png_repaired.png", &out);
    }

    #[test]
    fn test_repair_png_no_ihdr() {
        let data = std::fs::read("png2.png").unwrap();
        let result = repair_png(&data, None);
        assert!(
            result.is_some(),
            "repair_png should return Some for png2.png"
        );
        let out = result.unwrap();
        assert!(
            out.starts_with(PNG_SIG),
            "output must start with PNG signature"
        );
        let ihdr_pos = find_chunk(&out, b"IHDR").expect("output must contain IHDR");
        let (w, h, _bd, _ct) = try_extract_ihdr(&out, ihdr_pos).expect("IHDR must be valid");
        assert_eq!(w, 1920, "width should be 1920");
        assert_eq!(h, 1080, "height should be 1080");
        assert!(has_valid_iend(&out), "output must have valid IEND");
        let _ = std::fs::write("/tmp/test_png2_repaired.png", &out);
    }

    #[test]
    fn test_find_embedded_png() {
        let data = std::fs::read("png.png").unwrap();
        let off = find_embedded_png(&data);
        assert_eq!(off, Some(8), "should find embedded at offset 8");

        let data2 = std::fs::read("png2.png").unwrap();
        let off2 = find_embedded_png(&data2);
        assert_eq!(off2, Some(1646), "should find embedded at IDAT offset");
    }

    #[test]
    fn test_extract_idat_and_estimate() {
        let data = std::fs::read("png2.png").unwrap();
        let decompressed = extract_idat_data(&data, 1646);
        assert!(decompressed.is_some(), "should decompress IDAT");
        let d = decompressed.unwrap();
        assert_eq!(
            d.len(),
            6221880,
            "decompressed size should match 1920x1080 RGB"
        );

        let (w, h, bd, ct) = estimate_ihdr(&d);
        assert_eq!(w, 1920, "estimated width should be 1920");
        assert_eq!(h, 1080, "estimated height should be 1080");
        assert_eq!(ct, 2, "color_type should be 2 (RGB)");
        assert_eq!(bd, 8, "bit_depth should be 8");
    }

    #[test]
    fn test_png_metadata_preservation() {
        let data = std::fs::read("png2.png").unwrap();
        let result = repair_png(&data, None);
        let out = result.unwrap();
        // Check metadata chunks are preserved
        assert!(
            find_chunk(&out, b"pHYs").is_some(),
            "output should contain pHYs"
        );
        // iTXt contains 'XML:com.adobe.xmp'
        let out_str = String::from_utf8_lossy(&out);
        assert!(
            out_str.contains("XML:com.adobe.xmp"),
            "output should contain iTXt metadata"
        );
    }

    #[test]
    fn test_detect_jpeg() {
        let data = std::fs::read("jpg").unwrap();
        assert_eq!(detect_type(&data), FileType::JPEG);
    }

    #[test]
    fn test_repair_jpeg_intact_roundtrip() {
        let data = std::fs::read("jpg").unwrap();
        let out = repair_jpeg(&data, Some(0)).expect("valid jpg should repair");
        assert_eq!(out, data, "intact jpeg should pass through unchanged");
    }

    #[test]
    fn test_ref_repair_recovers_entropy_after_destroyed_headers() {
        // Regression: a photo whose DQT/DHT header markers are destroyed must
        // still have its surviving scan data recovered by the reference repair,
        // instead of falling back to the synthesized gray placeholder.
        let data = std::fs::read("jpg").unwrap();
        let tpl = extract_jpeg_template(&data).expect("fixture must yield a template");

        let mut damaged = data.clone();
        for i in 0..damaged.len() - 1 {
            if damaged[i] == 0xFF && (damaged[i + 1] == 0xDB || damaged[i + 1] == 0xC4) {
                damaged[i + 1] = 0x00;
            }
        }
        // jpeg_parts must now look past the destroyed markers and find entropy.
        let parts = jpeg_parts(&damaged, 0);
        assert!(
            parts.entropy.is_some(),
            "entropy must be recovered despite destroyed header markers"
        );
        let out =
            repair_jpeg_with_ref(&damaged, Some(0), &tpl).expect("reference repair must succeed");
        assert!(out.starts_with(JPEG_SOI) && out.ends_with(JPEG_EOI));
        let info = scan_jpeg(&out, 0);
        assert!(
            info.has_sos && info.terminated,
            "repaired JPEG must be a valid frame"
        );
    }

    #[test]
    fn test_repair_jpeg_truncated() {
        let data = std::fs::read("jpg").unwrap();
        assert!(data.ends_with(JPEG_EOI), "fixture must end with EOI");
        let truncated = &data[..data.len() - 2];
        let out = repair_jpeg(truncated, None).expect("truncated jpeg should repair");
        assert!(out.starts_with(JPEG_SOI), "output must start with SOI");
        assert!(out.ends_with(JPEG_EOI), "output must end with EOI");
        let info = scan_jpeg(&out, 0);
        assert!(info.terminated, "repaired output must terminate cleanly");
        let orig_info = scan_jpeg(&data, 0);
        assert_eq!(info.width, orig_info.width, "width must be preserved");
        assert_eq!(info.height, orig_info.height, "height must be preserved");
        let _ = std::fs::write("/tmp/test_jpg_truncated_repaired.jpg", &out);
    }

    #[test]
    fn test_repair_jpeg_trailing_garbage() {
        let data = std::fs::read("jpg").unwrap();
        let mut carved = data.clone();
        carved.extend_from_slice(b"\x00\xde\xad\xbe\xef\x01\x02garbage-after-eoi");
        let out = repair_jpeg(&carved, None).expect("carved jpeg should repair");
        assert!(out.ends_with(JPEG_EOI), "output must end with EOI");
        assert!(
            out.len() <= carved.len().saturating_sub(b"garbage-after-eoi".len()),
            "trailing garbage must be stripped"
        );
        let info = scan_jpeg(&out, 0);
        assert!(
            info.has_sos && info.terminated,
            "repaired must be a valid JPEG"
        );
        let orig = scan_jpeg(&data, 0);
        assert_eq!(info.width, orig.width);
        assert_eq!(info.height, orig.height);
    }

    #[test]
    fn test_repair_jpeg_zeroed_soi() {
        let data = std::fs::read("jpg").unwrap();
        let mut no_soi = data.clone();
        no_soi[0] = 0x00;
        no_soi[1] = 0x00;
        assert_eq!(
            detect_type(&no_soi),
            FileType::Unknown,
            "zeroed SOI must not detect as JPEG"
        );
        let start = find_jpeg_start(&no_soi);
        assert_eq!(
            start,
            Some(2),
            "should locate first segment marker at offset 2"
        );
        let out = repair_jpeg(&no_soi, start).expect("should repair with restored SOI");
        assert!(
            out.starts_with(JPEG_SOI),
            "output must start with restored SOI"
        );
        assert!(out.ends_with(JPEG_EOI), "output must end with EOI");
        let info = scan_jpeg(&out, 0);
        assert!(
            info.has_sos && info.terminated,
            "repaired must be valid JPEG"
        );
        let orig = scan_jpeg(&data, 0);
        assert_eq!(info.width, orig.width, "width preserved");
        assert_eq!(info.height, orig.height, "height preserved");
        let _ = std::fs::write("/tmp/test_jpg_zeroed_soi_repaired.jpg", &out);
    }

    #[test]
    fn test_analyze_embedded_jpeg() {
        let data = std::fs::read("jpg").unwrap();
        let mut unknown = vec![0u8; 64];
        unknown.extend_from_slice(&data);
        let a = analyze_bytes(&unknown);
        assert_eq!(
            a.file_type,
            FileType::JPEG,
            "embedded jpeg should be detected"
        );
        assert_eq!(
            a.embedded_offset,
            Some(64),
            "embedded offset should be correct"
        );
        assert!(a.has_footer, "embedded jpeg has valid EOI");
    }

    #[test]
    fn test_analyze_jpeg_file() {
        let a = analyze_file("jpg").expect("analyze should succeed");
        assert_eq!(a.file_type, FileType::JPEG);
        assert!(a.has_header);
        assert!(a.has_footer);
        assert_eq!(a.embedded_offset, Some(0));
    }

    #[test]
    fn test_repair_jpeg_zeroed_prefix() {
        // Real-world carve: SOI + APP0 marker/length/JFIF id are zeroed, so the
        // first intact marker is FF DB at offset 0x14.
        let jpg = std::fs::read("jpg").unwrap();
        let mut prefix = vec![0u8; 11];
        prefix.extend_from_slice(&[0x01, 0x01, 0x01, 0x00, 0x48, 0x00, 0x48, 0x00, 0x00]);
        let mut data = prefix;
        data.extend_from_slice(&jpg[0x14..]);
        assert_eq!(data[0x14], 0xFF);
        assert_eq!(data[0x15], 0xDB);

        let a = analyze_bytes(&data);
        assert_eq!(
            a.file_type,
            FileType::JPEG,
            "zeroed-prefix jpeg must be detected"
        );
        assert!(!a.has_header, "SOI is missing");
        assert!(a.has_footer, "image still terminates with EOI");
        assert_eq!(
            a.embedded_offset,
            Some(0x14),
            "start marker located at offset 0x14"
        );

        let out = repair_jpeg(&data, a.embedded_offset).expect("should repair");
        assert!(out.starts_with(JPEG_SOI), "SOI must be restored");
        let info = scan_jpeg(&out, 0);
        assert!(info.terminated, "repaired output must terminate with EOI");
        assert_eq!((info.width, info.height), (Some(736), Some(460)));
        let _ = std::fs::write("/tmp/test_jpg_zeroed_prefix_repaired.jpg", &out);
    }

    #[test]
    fn test_extract_jpeg_template_from_reference() {
        if !std::path::Path::new("IMG_original.JPG").exists() {
            eprintln!("IMG_original.JPG not present; skipping reference test");
            return;
        }
        let data = std::fs::read("IMG_original.JPG").unwrap();
        let tpl = extract_jpeg_template(&data).expect("reference must yield a template");
        assert_eq!(tpl.width, 5184);
        assert_eq!(tpl.height, 3456);
        assert_eq!(tpl.ncomp, 3);
        assert_eq!(tpl.prec, 8);
        assert!(tpl.baseline, "Canon 4000D photos are baseline JPEGs");
        assert_eq!(
            dqt_table_count(&tpl.dqt),
            2,
            "luma + chroma quantization tables"
        );
        assert_eq!(parse_dht_tables(&tpl.dht).len(), 4, "DC0/DC1/AC0/AC1");
        assert_eq!(tpl.sos_comps.len(), 3);
        assert_eq!(tpl.hmax, 2);
        assert_eq!(tpl.vmax, 1);
        for id in [0usize, 1] {
            assert!(
                tpl.dc_code0[id].is_some(),
                "DC table {} must have a category-0 code",
                id
            );
            assert!(
                tpl.eob_code[id].is_some(),
                "AC table {} must have an EOB code",
                id
            );
        }
    }

    #[test]
    fn test_repair_jpeg_with_ref_header_destroyed() {
        if !std::path::Path::new("IMG_original.JPG").exists() {
            eprintln!("IMG_original.JPG not present; skipping reference test");
            return;
        }
        let data = std::fs::read("IMG_original.JPG").unwrap();
        let tpl = extract_jpeg_template(&data).unwrap();

        // Locate the real SOS marker by walking the marker stream.
        let mut sos = None;
        let mut pos = 0usize;
        while pos + 3 < data.len() {
            if data[pos] != 0xFF {
                pos += 1;
                continue;
            }
            let mut j = pos + 1;
            while j < data.len() && data[j] == 0xFF {
                j += 1;
            }
            if j >= data.len() {
                break;
            }
            let code = data[j];
            if code == 0xDA {
                sos = Some(pos);
                break;
            }
            if code == 0xD8 || code == 0x01 || (0xD0..=0xD7).contains(&code) {
                pos = j + 1;
                continue;
            }
            if code == 0xD9 || code == 0x00 {
                break;
            }
            let seg_len = u16::from_be_bytes([data[j + 1], data[j + 2]]) as usize;
            if seg_len < 2 || j + 1 + seg_len > data.len() {
                break;
            }
            pos = j + 1 + seg_len;
        }
        let sos = sos.expect("reference must contain SOS");

        // Destroy the header/table PAYLOADS (APP1/DQT/SOF/DHT contents) but keep
        // the marker bytes and lengths so the marker stream still walks and the
        // entropy-coded scan data survives intact.
        let mut damaged = data.clone();
        let zero_ranges: Vec<(usize, usize)> = vec![
            (6, 24280),     // APP1 Exif payload
            (24284, 26842), // APP1 XMP payload
            (26846, 26976), // DQT payload
            (26980, 26995), // SOF0 payload
            (26999, sos),   // DHT payload
        ];
        for (st, en) in zero_ranges {
            for b in damaged.iter_mut().take(en).skip(st) {
                *b = 0;
            }
        }

        let out =
            repair_jpeg_with_ref(&damaged, Some(0), &tpl).expect("must rebuild from template");
        assert!(out.starts_with(JPEG_SOI));
        let info = scan_jpeg(&out, 0);
        assert!(info.terminated, "output must terminate with EOI");
        assert_eq!((info.width, info.height), (Some(5184), Some(3456)));
        assert!(
            info.has_dqt && info.has_dht && info.has_sos,
            "tables restored from reference"
        );

        // The surviving entropy must be preserved verbatim.
        let sos_len = u16::from_be_bytes([data[sos + 2], data[sos + 3]]) as usize;
        let entropy_start = sos + 2 + sos_len;
        let orig_scan = &data[entropy_start..data.len() - 2];
        let out_scan_end = out.len() - 2;
        assert_eq!(
            &out[out_scan_end - orig_scan.len()..out_scan_end],
            orig_scan
        );
        let _ = std::fs::write("/tmp/test_ref_header_destroyed.jpg", &out);
    }

    #[test]
    fn test_repair_jpeg_with_ref_synthesizes_scan() {
        if !std::path::Path::new("IMG_original.JPG").exists() {
            eprintln!("IMG_original.JPG not present; skipping reference test");
            return;
        }
        let data = std::fs::read("IMG_original.JPG").unwrap();
        let tpl = extract_jpeg_template(&data).unwrap();

        // No SOS/scan data survives at all — repair must synthesize valid entropy.
        let destroyed = vec![0u8; 64];
        let out = repair_jpeg_with_ref(&destroyed, None, &tpl).expect("must synthesize scan");
        assert!(out.starts_with(JPEG_SOI));
        let info = scan_jpeg(&out, 0);
        assert!(
            info.terminated,
            "synthesized output must terminate with EOI"
        );
        assert_eq!((info.width, info.height), (Some(5184), Some(3456)));
        assert!(info.has_dqt && info.has_dht && info.has_sos);

        // MCU grid: 2x1 luma sampling over 16x8 MCUs -> 324x432 MCUs.
        // Bits per MCU: Y(2 blocks) = 2*(dc2+eob4)=12, Cb/Cr = 2*(2+2)=8 -> 20.
        let mcu_w = 324u64;
        let mcu_h = 432u64;
        let expected_scan = mcu_w * mcu_h * 20 / 8;
        let scan_start = out.len() - expected_scan as usize - 2;
        assert_eq!(out.len() - 2 - scan_start, expected_scan as usize);
        assert!(
            !out[scan_start..scan_start + 4096].contains(&0xFF),
            "no marker bytes inside entropy"
        );
        let _ = std::fs::write("/tmp/test_ref_synth.jpg", &out);
    }

    #[test]
    fn test_repair_jpeg_with_ref_rejects_mismatched_components() {
        if !std::path::Path::new("IMG_original.JPG").exists() {
            eprintln!("IMG_original.JPG not present; skipping reference test");
            return;
        }
        let data = std::fs::read("IMG_original.JPG").unwrap();
        let tpl = extract_jpeg_template(&data).unwrap();
        // A grayscale-ish damaged file (SOF with 1 component) must be refused.
        let mut damaged = vec![0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x0B, 0x08];
        damaged.extend_from_slice(&3456u16.to_be_bytes());
        damaged.extend_from_slice(&5184u16.to_be_bytes());
        damaged.push(1);
        damaged.push(1);
        damaged.push(0x11);
        damaged.push(0);
        assert!(
            repair_jpeg_with_ref(&damaged, Some(0), &tpl).is_none(),
            "mismatched component count must not use the reference template"
        );
    }

    #[test]
    fn test_exif_dimensions_from_canon() {
        if !std::path::Path::new("IMG_original.JPG").exists() {
            eprintln!("IMG_original.JPG not present; skipping test");
            return;
        }
        let data = std::fs::read("IMG_original.JPG").unwrap();
        assert_eq!(exif_dimensions(&data), Some((5184, 3456)));
        // Corrupt tail after the EXIF segment must not break dimension recovery.
        let mut truncated = data[..24300].to_vec();
        truncated.extend_from_slice(&[0xFF, 0xD9]);
        assert_eq!(exif_dimensions(&truncated), Some((5184, 3456)));
    }

    #[test]
    fn test_repair_jpeg_with_ref_different_resolution_reference() {
        if !std::path::Path::new("IMG_original.JPG").exists() {
            eprintln!("IMG_original.JPG not present; skipping test");
            return;
        }
        let data = std::fs::read("IMG_original.JPG").unwrap();
        let mut tpl = extract_jpeg_template(&data).unwrap();

        // Simulate a reference photo from the same camera but at a different
        // resolution (e.g. a lower-quality mode).
        tpl.width = 736;
        tpl.height = 460;
        tpl.sof = rewrite_sof_dims(&tpl.sof, 736, 460);

        // Locate SOS.
        let mut sos = None;
        let mut pos = 0usize;
        while pos + 3 < data.len() {
            let mut j = pos + 1;
            while j < data.len() && data[j] == 0xFF {
                j += 1;
            }
            if j >= data.len() {
                break;
            }
            let code = data[j];
            if code == 0xDA {
                sos = Some(pos);
                break;
            }
            if code == 0xD8 || code == 0x01 || (0xD0..=0xD7).contains(&code) {
                pos = j + 1;
                continue;
            }
            if code == 0xD9 || code == 0x00 {
                break;
            }
            let seg_len = u16::from_be_bytes([data[j + 1], data[j + 2]]) as usize;
            if seg_len < 2 || j + 1 + seg_len > data.len() {
                break;
            }
            pos = j + 1 + seg_len;
        }
        let sos = sos.expect("reference must contain SOS");

        // Destroy the scan headers but KEEP the EXIF APP1 so the real geometry
        // is still recoverable.
        let mut damaged = data.clone();
        for (st, en) in [
            (24284usize, 26842usize),
            (26846, 26976),
            (26980, 26995),
            (26999, sos),
        ] {
            for b in damaged.iter_mut().take(en).skip(st) {
                *b = 0;
            }
        }

        let out = repair_jpeg_with_ref(&damaged, Some(0), &tpl)
            .expect("same-camera reference at a different resolution must work");
        let info = scan_jpeg(&out, 0);
        // Geometry must come from the damaged file's EXIF, not the 736x460 ref.
        assert_eq!((info.width, info.height), (Some(5184), Some(3456)));
        assert!(info.has_sos && info.terminated);
        // The surviving entropy must still be preserved verbatim.
        let sos_len = u16::from_be_bytes([data[sos + 2], data[sos + 3]]) as usize;
        let orig_scan = &data[sos + 2 + sos_len..data.len() - 2];
        let out_scan_end = out.len() - 2;
        assert_eq!(
            &out[out_scan_end - orig_scan.len()..out_scan_end],
            orig_scan
        );
        let _ = std::fs::write("/tmp/test_ref_different_res.jpg", &out);
    }

    #[test]
    fn test_repair_jpeg_with_ref_rejects_progressive_reference() {
        if !std::path::Path::new("jpgoriginal.jpg").exists() {
            eprintln!("jpgoriginal.jpg not present; skipping test");
            return;
        }
        let tpl = extract_jpeg_template(&std::fs::read("jpgoriginal.jpg").unwrap());
        assert!(
            tpl.is_some(),
            "progressive JPEG must still parse as a template"
        );
        let tpl = tpl.unwrap();
        assert!(
            !tpl.baseline,
            "jpgoriginal.jpg is a progressive (SOF2) JPEG"
        );

        if std::path::Path::new("IMG_original.JPG").exists() {
            let data = std::fs::read("IMG_original.JPG").unwrap();
            // A damaged file with a destroyed scan header cannot borrow a scan
            // structure from a progressive reference: the single rebuilt scan
            // would be invalid without the progressive multi-scan progression.
            let mut damaged = data.clone();
            for (st, en) in [
                (26846usize, 26976usize),
                (26980, 26995),
                (26999, damaged.len() - 2),
            ] {
                for b in damaged.iter_mut().take(en).skip(st) {
                    *b = 0;
                }
            }
            assert!(
                repair_jpeg_with_ref(&damaged, Some(0), &tpl).is_none(),
                "a non-baseline reference must not stand in for a destroyed scan header"
            );
        }
    }

    #[test]
    fn test_deep_repair_jpeg_with_reference_end_to_end() {
        if !std::path::Path::new("IMG_original.JPG").exists() {
            eprintln!("IMG_original.JPG not present; skipping reference test");
            return;
        }
        use std::time::Duration;

        let data = std::fs::read("IMG_original.JPG").unwrap();
        // Find the real SOS marker position.
        let mut sos = 0usize;
        let mut pos = 0usize;
        while pos + 3 < data.len() {
            let mut j = pos + 1;
            while j < data.len() && data[j] == 0xFF {
                j += 1;
            }
            if j >= data.len() {
                break;
            }
            if data[j] == 0xDA {
                sos = pos;
                break;
            }
            if !(data[j] == 0xD8
                || data[j] == 0xD9
                || data[j] == 0x01
                || (0xD0..=0xD7).contains(&data[j]))
            {
                let seg_len = u16::from_be_bytes([data[j + 1], data[j + 2]]) as usize;
                if seg_len < 2 {
                    break;
                }
                pos = j + 1 + seg_len;
            } else {
                pos = j + 1;
            }
        }
        assert!(sos > 0, "SOS must be found");

        let mut damaged = data.clone();
        for (st, en) in [
            (6usize, 24280usize),
            (24284, 26842),
            (26846, 26976),
            (26980, 26995),
            (26999, sos),
        ] {
            for b in damaged.iter_mut().take(en).skip(st) {
                *b = 0;
            }
        }
        let src = "/tmp/ref_e2e_damaged.jpg";
        std::fs::write(src, &damaged).unwrap();

        let analysis = analyze_file_deep(src).expect("deep analysis must succeed");
        let out_dir = "/tmp/ref_e2e_out";
        let _ = std::fs::create_dir_all(out_dir);

        let (tx, rx) = crossbeam_channel::unbounded::<RepairEvent>();
        let stop = Arc::new(AtomicBool::new(false));
        repair_file_deep(
            src.to_string(),
            out_dir.to_string(),
            analysis,
            tx,
            stop,
            "IMG_original.JPG".to_string(),
        );

        let mut done = false;
        for _ in 0..100 {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(RepairEvent::Complete { .. }) => {
                    done = true;
                    break;
                }
                Ok(RepairEvent::Error(e)) => panic!("deep repair errored: {}", e),
                _ => {}
            }
        }
        assert!(done, "deep repair must complete");

        let out_path = format!("{}/ref_e2e_damaged.jpg", out_dir);
        let out = std::fs::read(&out_path).expect("output file must exist");
        assert!(out.starts_with(JPEG_SOI));
        let info = scan_jpeg(&out, 0);
        assert!(info.terminated);
        assert_eq!((info.width, info.height), (Some(5184), Some(3456)));
        assert!(info.has_dqt && info.has_dht && info.has_sos);
        assert!(
            out.len() > 5_000_000,
            "entropy should be preserved, not synthesized"
        );
    }

    #[test]
    fn test_detect_bmp() {
        let data = std::fs::read("bmp.bmp").unwrap();
        assert_eq!(detect_type(&data), FileType::BMP);
    }

    #[test]
    fn test_repair_bmp_intact_roundtrip() {
        let data = std::fs::read("bmp.bmp").unwrap();
        let out = repair_bmp(&data, Some(0)).expect("valid bmp should repair");
        assert_eq!(out, data, "intact bmp should pass through unchanged");
    }

    #[test]
    fn test_repair_bmp_truncated() {
        let data = std::fs::read("bmp.bmp").unwrap();
        let truncated = &data[..data.len() - 100];
        let out = repair_bmp(truncated, None).expect("truncated bmp should repair");
        assert_eq!(out.len(), truncated.len(), "output keeps available data");
        assert_eq!(&out[..2], &truncated[..2], "BM signature preserved");
        assert_eq!(&out[6..], &truncated[6..], "pixel data must not be altered");
        let fixed = u32::from_le_bytes([out[2], out[3], out[4], out[5]]) as usize;
        assert_eq!(
            fixed,
            truncated.len(),
            "file-size field must match actual length"
        );
        let info = scan_bmp(&out);
        assert_eq!(info.width, Some(320));
        assert_eq!(info.height, Some(200));
        let _ = std::fs::write("/tmp/test_bmp_truncated_repaired.bmp", &out);
    }

    #[test]
    fn test_repair_bmp_trailing_garbage() {
        let data = std::fs::read("bmp.bmp").unwrap();
        let mut carved = data.clone();
        carved.extend_from_slice(b"\x00\xde\xad\xbe\xefgarbage-after-pixels");
        let out = repair_bmp(&carved, None).expect("carved bmp should repair");
        assert_eq!(out, data, "trailing garbage must be truncated to pixel end");
    }

    #[test]
    fn test_analyze_bmp_file() {
        let a = analyze_file("bmp.bmp").expect("analyze should succeed");
        assert_eq!(a.file_type, FileType::BMP);
        assert!(a.has_header);
        assert!(a.has_footer, "intact bmp is complete");
        assert_eq!(a.embedded_offset, Some(0));
        assert!(
            a.details.contains("320x200"),
            "details should include dimensions"
        );
    }

    #[test]
    fn test_analyze_truncated_bmp_reports_incomplete() {
        let data = std::fs::read("bmp.bmp").unwrap();
        let truncated = &data[..data.len() - 100];
        let a = analyze_bytes(truncated);
        assert_eq!(a.file_type, FileType::BMP);
        assert!(!a.has_footer, "truncated bmp must be flagged as incomplete");
    }

    #[test]
    fn test_detect_riff() {
        let data = std::fs::read("wav.wav").unwrap();
        assert_eq!(detect_type(&data), FileType::RIFF);
    }

    #[test]
    fn test_analyze_carved_riff_reports_incomplete() {
        let data = std::fs::read("wav.wav").unwrap();
        let mut carved = data.clone();
        carved.extend_from_slice(b"junk-after-eof");
        let a = analyze_bytes(&carved);
        assert_eq!(a.file_type, FileType::RIFF);
        assert!(!a.has_footer, "carved wav must be flagged as incomplete");
        assert!(
            a.details.contains("WAVE"),
            "details should include form type"
        );
    }

    #[test]
    fn test_repair_riff_intact_roundtrip() {
        let data = std::fs::read("wav.wav").unwrap();
        let out = repair_riff(&data, Some(0)).expect("valid wav should repair");
        assert_eq!(out, data, "intact wav should pass through unchanged");
    }

    #[test]
    fn test_repair_riff_trailing_garbage() {
        let data = std::fs::read("wav.wav").unwrap();
        let mut carved = data.clone();
        carved.extend_from_slice(b"\x00\xde\xad\xbe\xefgarbage-after-eof");
        let out = repair_riff(&carved, None).expect("carved wav should repair");
        assert_eq!(
            out, data,
            "trailing garbage must be truncated to last chunk"
        );
    }

    #[test]
    fn test_repair_riff_truncated() {
        let data = std::fs::read("wav.wav").unwrap();
        let truncated = &data[..data.len() - 100];
        let out = repair_riff(truncated, None).expect("truncated wav should repair");
        assert_eq!(
            out.len(),
            truncated.len(),
            "output keeps all available bytes"
        );
        let fixed = u32::from_le_bytes([out[4], out[5], out[6], out[7]]) as usize;
        assert_eq!(
            fixed,
            out.len() - 8,
            "RIFF size field must match actual length"
        );
        assert_eq!(&out[36..40], b"data", "partial data chunk must be salvaged");
        let data_size = u32::from_le_bytes([out[40], out[41], out[42], out[43]]) as usize;
        assert_eq!(
            data_size,
            out.len() - 36 - 8,
            "partial data chunk size must be fixed"
        );
        let info = scan_riff(&out);
        assert_eq!(
            info.last_chunk_end,
            Some(out.len()),
            "repaired output ends at chunk boundary"
        );
        let _ = std::fs::write("/tmp/test_wav_truncated_repaired.wav", &out);
    }

    #[test]
    fn test_analyze_riff_file() {
        let a = analyze_file("wav.wav").expect("analyze should succeed");
        assert_eq!(a.file_type, FileType::RIFF);
        assert!(a.has_header);
        assert!(a.has_footer, "intact wav is complete");
        assert_eq!(a.embedded_offset, Some(0));
        assert!(
            a.details.contains("WAVE"),
            "details should include form type"
        );
    }

    #[test]
    fn test_riff_extension() {
        let data = std::fs::read("wav.wav").unwrap();
        assert_eq!(riff_extension(&data, 0), ".wav");
    }

    #[test]
    fn test_deep_scan_finds_signatures() {
        let data = std::fs::read("pngorigin.png").unwrap();
        let hits = deep_scan(&data);
        assert!(
            hits.iter()
                .any(|h| h.file_type == FileType::PNG && h.offset == 0),
            "deep scan must find PNG at offset 0"
        );
        assert!(
            hits.windows(2).all(|w| w[0].offset <= w[1].offset),
            "signatures must be sorted by offset"
        );

        let jpg = std::fs::read("jpg").unwrap();
        let hits = deep_scan(&jpg);
        assert!(
            hits.iter()
                .any(|h| h.file_type == FileType::JPEG && h.offset == 0),
            "deep scan must find JPEG at offset 0"
        );
    }

    #[test]
    fn test_deep_scan_bmp_plausibility() {
        let bmp = std::fs::read("bmp.bmp").unwrap();
        let hits = deep_scan(&bmp);
        assert!(
            hits.iter()
                .any(|h| h.file_type == FileType::BMP && h.offset == 0),
            "valid BMP must be detected by deep scan"
        );
    }

    #[test]
    fn test_analyze_file_deep() {
        let a = analyze_file_deep("jpg").expect("deep analyze should succeed");
        assert_eq!(a.primary.file_type, FileType::JPEG);
        assert_eq!(a.primary.embedded_offset, Some(0));
        assert!(
            a.embedded_files
                .iter()
                .any(|c| c.file_type == FileType::JPEG && c.offset == 0),
            "embedded file list must include the primary JPEG"
        );
    }

    #[test]
    fn test_deep_repair_zip_truncation() {
        // Build a tiny zip: dummy local header + complete EOCD (with comment).
        let mut data = Vec::new();
        data.extend_from_slice(&[0x50, 0x4B, 0x03, 0x04, 0x00, 0x00, 0x00, 0x00]);
        data.extend_from_slice(b"garbage-before-eocd");
        data.extend_from_slice(b"PK\x05\x06");
        data.extend_from_slice(&[0u8; 16]);
        data.extend_from_slice(&2u16.to_le_bytes()); // comment length
        data.extend_from_slice(b"xt"); // comment
        let trailing: &[u8] = b"junk-after-zip";
        data.extend_from_slice(trailing);

        let out = repair_zip_from(&data, 0).expect("zip should repair");
        let expected = data[..data.len() - trailing.len()].to_vec();
        assert_eq!(
            out, expected,
            "trailing garbage after EOCD + comment must be dropped"
        );
        assert_eq!(
            data.len(),
            expected.len() + trailing.len(),
            "sanity: only garbage removed"
        );
    }

    #[test]
    fn test_deep_repair_embedded_png() {
        let png = std::fs::read("pngorigin.png").unwrap();
        let mut container = b"\x00\x01\x02\x03 -> ".to_vec();
        let expected = container.len() as usize;
        container.extend_from_slice(&png);
        container.extend_from_slice(b"trailing-carve-garbage");

        let a = analyze_bytes(&container);
        assert_eq!(a.file_type, FileType::Unknown);
        assert_eq!(
            a.embedded_offset,
            Some(expected),
            "embedded PNG must be located"
        );

        let out = deep_repair_one(&container, expected, FileType::PNG).expect("should repair");
        assert!(
            out.starts_with(PNG_SIG),
            "output must start with PNG signature"
        );
        assert!(has_valid_iend(&out), "output must terminate with IEND");
        assert!(
            out.len() < container.len(),
            "output must drop container prefix"
        );
    }

    #[test]
    fn test_color_metadata_png() {
        let data = std::fs::read("pngorigin.png").unwrap();
        let meta = color_metadata(&data, FileType::PNG);
        assert!(meta.contains("Dimensions"), "must report dimensions");
        assert!(meta.contains("Bit depth"), "must report bit depth");
        assert!(meta.contains("Color type"), "must report color type");
    }

    #[test]
    fn test_color_metadata_jpeg() {
        let data = std::fs::read("jpg").unwrap();
        let meta = color_metadata(&data, FileType::JPEG);
        assert!(meta.contains("Dimensions"), "must report dimensions");
        assert!(meta.contains("Components"), "must report color components");
        assert!(meta.contains("JFIF"), "must report JFIF marker");
    }

    #[test]
    fn test_color_metadata_bmp_and_wav() {
        let bmp = std::fs::read("bmp.bmp").unwrap();
        let meta = color_metadata(&bmp, FileType::BMP);
        assert!(meta.contains("Bits per pixel"), "must report bpp");

        let wav = std::fs::read("wav.wav").unwrap();
        let meta = color_metadata(&wav, FileType::RIFF);
        assert!(meta.contains("PCM"), "must report PCM audio format");
        assert!(meta.contains("Hz"), "must report sample rate");
    }

    #[test]
    fn test_deep_analysis_includes_color_metadata() {
        let a = analyze_file_deep("pngorigin.png").expect("deep analyze");
        assert!(
            a.color_metadata.contains("Color type"),
            "DeepAnalysis must carry color metadata"
        );
    }

    #[test]
    fn test_salvage_png_keeps_good_idat() {
        // png.png has a zeroed signature but intact pixel data.
        let data = std::fs::read("png.png").unwrap();
        let out = salvage_png(&data, None).expect("salvage should succeed");
        assert!(out.starts_with(PNG_SIG), "signature restored");
        let chunks = png_chunks(&out);
        let idat: Vec<&PngChunkRef> = chunks.iter().filter(|c| c.typ == *b"IDAT").collect();
        assert!(!idat.is_empty(), "output must contain IDAT");
        let mut concat = Vec::new();
        for c in &idat {
            concat.extend_from_slice(&out[c.start + 8..c.start + 8 + c.len]);
        }
        let d = decompress_to_vec_zlib(&concat).expect("IDAT must be a valid zlib stream");
        let (w, h, bd, ct) = try_extract_ihdr(
            &out,
            chunks.iter().find(|c| c.typ == *b"IHDR").unwrap().start + 4,
        )
        .expect("IHDR must parse");
        let channels = png_channels(ct);
        let row = (w as u64 * channels as u64 * bd as u64).div_ceil(8).max(1);
        assert_eq!(
            d.len() as u64,
            h as u64 * (row + 1),
            "IDAT decompresses to full image"
        );
    }

    #[test]
    fn test_salvage_png_synthesizes_idat_when_corrupt() {
        let data = std::fs::read("pngorigin.png").unwrap();
        // Destroy the IDAT payload so the original pixels are unrecoverable.
        let mut corrupt = data.clone();
        let chunks = png_chunks(&corrupt);
        let idat = chunks
            .iter()
            .find(|c| c.typ == *b"IDAT")
            .expect("fixture has IDAT");
        for i in (idat.start + 8)..(idat.start + 8 + idat.len) {
            corrupt[i] ^= 0xA5;
        }
        let out = salvage_png(&corrupt, None).expect("salvage must succeed");
        assert!(out.starts_with(PNG_SIG), "signature preserved");
        let chunks = png_chunks(&out);
        let ihdr = try_extract_ihdr(
            &out,
            chunks.iter().find(|c| c.typ == *b"IHDR").unwrap().start + 4,
        )
        .expect("IHDR must parse");
        let idat: Vec<&PngChunkRef> = chunks.iter().filter(|c| c.typ == *b"IDAT").collect();
        let mut concat = Vec::new();
        for c in &idat {
            concat.extend_from_slice(&out[c.start + 8..c.start + 8 + c.len]);
        }
        let d =
            decompress_to_vec_zlib(&concat).expect("synthesized IDAT must be a valid zlib stream");
        let (w, h, bd, ct) = ihdr;
        let channels = png_channels(ct);
        let row = (w as u64 * channels as u64 * bd as u64).div_ceil(8).max(1);
        assert_eq!(
            d.len() as u64,
            h as u64 * (row + 1),
            "placeholder fills the whole image"
        );
    }

    #[test]
    fn test_salvage_png_recomputes_crc() {
        let data = std::fs::read("pngorigin.png").unwrap();
        let out = salvage_png(&data, None).expect("salvage should succeed");
        for c in png_chunks(&out) {
            let typ = &out[c.start + 4..c.start + 8];
            let data_start = c.start + 8;
            let expected = calc_crc(&out[data_start - 4..data_start + c.len]);
            let actual = u32::from_be_bytes([
                out[data_start + c.len],
                out[data_start + c.len + 1],
                out[data_start + c.len + 2],
                out[data_start + c.len + 3],
            ]);
            assert_eq!(actual, expected, "CRC must be recomputed for {:?}", typ);
        }
    }

    #[test]
    fn test_salvage_gif_adds_trailer() {
        let mut data = b"GIF89a".to_vec();
        data.extend_from_slice(&[0x10, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00]); // LSD
                                                                             // truncated image data, no trailer
        data.extend_from_slice(&[0x2C, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x10, 0x00, 0x00]);

        let out = salvage_gif(&data, 0).expect("salvage should succeed");
        assert!(out.starts_with(b"GIF89a"), "header preserved");
        assert!(out.ends_with(&[0x3B]), "trailer must be present");
    }

    #[test]
    fn test_salvage_gif_repairs_bad_header() {
        let mut data = vec![0u8; 4];
        data.extend_from_slice(b"GIF-bad\x01\x00\x01\x00\x00\x00\x00");
        let out = salvage_gif(&data, 0).expect("salvage should succeed");
        assert!(out.starts_with(b"GIF89a"), "header must be restored");
        assert!(out.ends_with(&[0x3B]), "trailer must be present");
    }

    #[test]
    fn test_repair_jpeg_rejects_missing_sos() {
        // SOI + APP1 + APP1 + EOI with no scan structure: not a viewable JPEG.
        let mut data = JPEG_SOI.to_vec();
        for (payload, id) in [
            (&[0u8; 64][..], b"Exif\0\0".as_slice()),
            (&[0u8; 32][..], b"XMP\0".as_slice()),
        ] {
            data.push(0xFF);
            data.push(0xE1);
            let len = (payload.len() + 2) as u16;
            data.extend_from_slice(&len.to_be_bytes());
            data.extend_from_slice(id);
            data.extend_from_slice(payload);
        }
        data.extend_from_slice(JPEG_EOI);
        assert!(
            repair_jpeg(&data, Some(0)).is_none(),
            "repair_jpeg must not emit a JPEG without an SOS segment"
        );
    }

    #[test]
    fn test_repair_jpeg_keeps_truncated_sos() {
        // A JPEG truncated mid-scan (SOS present) must still be repaired.
        let data = std::fs::read("IMG_original.JPG").unwrap();
        let truncated = &data[..27415 + 12 + 1000]; // SOS header + short entropy
        let out = repair_jpeg(truncated, Some(0)).expect("SOS present → repairable");
        let info = scan_jpeg(&out, 0);
        assert!(
            info.has_sos && info.terminated,
            "must retain SOS and terminate"
        );
    }

    #[test]
    fn test_deep_repair_jpeg_without_reference_errors() {
        if !std::path::Path::new("IMG.JPG").exists()
            || !std::path::Path::new("IMG_original.JPG").exists()
        {
            eprintln!("IMG.JPG/IMG_original.JPG not present; skipping test");
            return;
        }
        use std::time::Duration;

        let analysis = analyze_file_deep("IMG.JPG").expect("deep analysis");
        let (tx, rx) = crossbeam_channel::unbounded::<RepairEvent>();
        let stop = Arc::new(AtomicBool::new(false));
        repair_file_deep(
            "IMG.JPG".to_string(),
            "/tmp/repair_noref".to_string(),
            analysis,
            tx,
            stop,
            String::new(),
        );
        let mut outcome = None;
        for _ in 0..50 {
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(RepairEvent::Error(e)) => {
                    outcome = Some(format!("error: {}", e));
                    break;
                }
                Ok(RepairEvent::Complete { output_path, .. }) => {
                    outcome = Some(format!("file: {}", output_path));
                    break;
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
        let outcome = outcome.expect("repair must terminate");
        assert!(
            outcome.starts_with("error:"),
            "SOS-less JPEG without a reference must error, got: {}",
            outcome
        );
    }

    #[test]
    fn test_jpegrepair_ffi_roundtrip() {
        if !std::path::Path::new("good.jpg").exists() {
            eprintln!("good.jpg not present; skipping test");
            return;
        }
        // A clean JPEG must survive a no-op style op through the C port.
        let data = std::fs::read("good.jpg").unwrap();
        let out = crate::backend::jpegrepair_ffi::jpegrepair_mem(
            &data,
            &["dest", "0", "0", "insert", "1"],
        )
        .expect("clean JPEG must decode and re-encode");
        assert!(out.starts_with(&[0xFF, 0xD8]), "output must start with SOI");
        let info = scan_jpeg(&out, 0);
        assert!(info.has_sos, "re-encoded output must contain SOS");
    }

    #[test]
    fn test_jpegrepair_ffi_rejects_garbage() {
        // Random / non-JPEG input must fail gracefully through the C port.
        let garbage: Vec<u8> = (0..4096).map(|i| (i * 7 % 256) as u8).collect();
        let out = crate::backend::jpegrepair_ffi::jpegrepair_mem(&garbage, &["delete", "1"]);
        assert!(out.is_none(), "garbage must not produce output");
    }

    #[test]
    fn test_jpegrepair_ffi_corrupt_deep_repair() {
        // A JPEG corrupted in the entropy region still yields a structure-level
        // deep repair via the ported jpegrepair ops when coefficients decode.
        if !std::path::Path::new("corrupt.jpg").exists() {
            eprintln!("corrupt.jpg not present; skipping test");
            return;
        }
        use std::time::Duration;
        let analysis = analyze_file_deep("corrupt.jpg").expect("deep analysis");
        let (tx, rx) = crossbeam_channel::unbounded::<RepairEvent>();
        let stop = Arc::new(AtomicBool::new(false));
        repair_file_deep(
            "corrupt.jpg".to_string(),
            "/tmp/repair_corrupt".to_string(),
            analysis,
            tx,
            stop,
            String::new(),
        );
        let mut outcome = None;
        for _ in 0..50 {
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(RepairEvent::Error(e)) => {
                    outcome = Some(format!("error: {}", e));
                    break;
                }
                Ok(RepairEvent::Complete { output_path, size }) => {
                    outcome = Some(format!("ok:{}:{}", output_path, size));
                    break;
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
        assert!(
            outcome.as_ref().map_or(false, |o| o.starts_with("ok:")),
            "structural deep repair should complete, got: {:?}",
            outcome
        );
    }
}

#[cfg(test)]
mod payload_tests {
    use super::*;

    fn zlib_bytes(data: &[u8]) -> Vec<u8> {
        let c = miniz_oxide::deflate::compress_to_vec_zlib(data, 6);
        c
    }

    fn png_chunk(typ: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let len = (data.len() as u32).to_be_bytes();
        let mut v = Vec::new();
        v.extend_from_slice(&len);
        v.extend_from_slice(typ);
        v.extend_from_slice(data);
        let mut crc_in = Vec::new();
        crc_in.extend_from_slice(typ);
        crc_in.extend_from_slice(data);
        v.extend_from_slice(&calc_crc(&crc_in).to_be_bytes());
        v
    }

    fn ihdr() -> Vec<u8> {
        let mut d = Vec::new();
        d.extend_from_slice(&(8u32).to_be_bytes());
        d.extend_from_slice(&(8u32).to_be_bytes());
        d.push(8);
        d.push(6);
        d.push(0);
        d.push(0);
        d.push(0);
        png_chunk(b"IHDR", &d)
    }

    #[test]
    fn png_ok() {
        let mut f = Vec::new();
        f.extend_from_slice(PNG_SIG);
        f.extend_from_slice(&ihdr());
        f.extend_from_slice(&png_chunk(b"IDAT", &zlib_bytes(&[1, 2, 3, 4])));
        f.extend_from_slice(&png_chunk(b"IEND", &[]));
        assert_eq!(png_payload_status(&f, None), PayloadStatus::Ok);
    }

    #[test]
    fn png_missing() {
        let mut f = Vec::new();
        f.extend_from_slice(PNG_SIG);
        f.extend_from_slice(&ihdr());
        f.extend_from_slice(&png_chunk(b"IEND", &[]));
        assert_eq!(png_payload_status(&f, None), PayloadStatus::Missing);
    }

    #[test]
    fn png_corrupt() {
        let mut f = Vec::new();
        f.extend_from_slice(PNG_SIG);
        f.extend_from_slice(&ihdr());
        f.extend_from_slice(&png_chunk(b"IDAT", &[0x78, 0x9c, 0xde, 0xad, 0xbe, 0xef]));
        f.extend_from_slice(&png_chunk(b"IEND", &[]));
        assert_eq!(png_payload_status(&f, None), PayloadStatus::Corrupt);
    }

    #[test]
    fn png_shifted() {
        let mut z = zlib_bytes(&[1, 2, 3, 4]);
        z.insert(0, 0x00);
        let mut f = Vec::new();
        f.extend_from_slice(PNG_SIG);
        f.extend_from_slice(&ihdr());
        f.extend_from_slice(&png_chunk(b"IDAT", &z));
        f.extend_from_slice(&png_chunk(b"IEND", &[]));
        assert_eq!(png_payload_status(&f, None), PayloadStatus::Shifted);
    }

    fn sos() -> Vec<u8> {
        Vec::from(
            &[
                0xFF, 0xDA, 0x00, 0x0C, 0x03, 0x01, 0x00, 0x02, 0x11, 0x03, 0x11, 0x00, 0x3F, 0x00,
            ][..],
        )
    }

    fn jpeg_base() -> Vec<u8> {
        let mut f = Vec::new();
        f.extend_from_slice(&[0xFF, 0xD8]);
        f.extend_from_slice(&[0xFF, 0xDB, 0x00, 0x04, 0x00, 0x01, 0x02, 0x03]);
        f.extend_from_slice(&sos());
        f
    }

    #[test]
    fn jpeg_ok() {
        let mut f = jpeg_base();
        f.extend_from_slice(&[0x01, 0x02, 0x03, 0x04, 0x05]);
        f.extend_from_slice(&[0xFF, 0xD9]);
        assert_eq!(jpeg_payload_from_scan(&f, 0), PayloadStatus::Ok);
    }

    #[test]
    fn jpeg_missing_entropy() {
        let mut f = jpeg_base();
        f.extend_from_slice(&[0xFF, 0xD9]);
        assert_eq!(jpeg_payload_from_scan(&f, 0), PayloadStatus::Missing);
    }

    #[test]
    fn jpeg_broken_header_but_entropy_survives() {
        let mut f = Vec::new();
        f.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
        f.extend_from_slice(&sos());
        f.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
        f.extend_from_slice(&[0xFF, 0xD9]);
        assert_eq!(
            jpeg_payload_from_scan(&f, 0),
            PayloadStatus::Ok,
            "intact scan data after a destroyed header should count as recoverable"
        );
    }
}
