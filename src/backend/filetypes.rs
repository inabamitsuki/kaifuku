//! File type categories used to limit which files PhotoRec recovers.
//!
//! The scan engine knows ~350 distinct file formats. When a scan should only
//! recover certain kinds of files (photos, videos, documents, ...) the GUI maps
//! those categories onto the engine's extension list and enables only the
//! matching file types. Anything not explicitly classified falls into "Other".

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileCategory {
    Photo,
    Video,
    Document,
    Audio,
    Archive,
    Other,
}

pub const ALL_CATEGORIES: [FileCategory; 6] = [
    FileCategory::Photo,
    FileCategory::Video,
    FileCategory::Document,
    FileCategory::Audio,
    FileCategory::Archive,
    FileCategory::Other,
];

impl FileCategory {
    pub fn label(self) -> &'static str {
        match self {
            Self::Photo => "Photos",
            Self::Video => "Videos",
            Self::Document => "Documents",
            Self::Audio => "Audio",
            Self::Archive => "Archives",
            Self::Other => "Other",
        }
    }

    /// Classify a file extension (e.g. "jpg", ".Jpg") into a category.
    /// Unrecognized extensions always fall back to `Other`.
    pub fn classify(ext: &str) -> FileCategory {
        let e = ext.trim().trim_start_matches('.').to_ascii_lowercase();
        for cat in ALL_CATEGORIES {
            if cat.extensions().iter().any(|&known| known == e) {
                return cat;
            }
        }
        Self::Other
    }

    /// Canonical extensions that belong to this category.
    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            Self::Photo => &[
                "bmp", "bpg", "cam", "cpi", "crw", "dpx", "exr", "fits", "gif", "hdr", "icns",
                "ico", "jpg", "mrw", "nd2", "orf", "pct", "pcx", "png", "pnm", "psb", "psd", "raf",
                "raw", "rw2", "tif", "tiff", "x3f", "xcf",
            ],
            Self::Video => &[
                "3gp", "asf", "avi", "dv", "flv", "m2ts", "mkv", "mlv", "mov", "mp4", "mpg",
                "mpeg", "mpl", "mts", "mxf", "r3d", "rm", "swf", "ts", "vob", "wdp", "wmv", "wtv",
            ],
            Self::Document => &[
                "chm", "cwk", "doc", "dsc", "dvi", "dwg", "dxf", "emf", "lit", "lnk", "mobi",
                "one", "pdf", "ps", "txt", "wks", "wmf", "wpd", "xml", "xpt",
            ],
            Self::Audio => &[
                "aif", "aiff", "amr", "ape", "au", "caf", "flac", "gsm", "itu", "mid", "mp3",
                "mus", "ogg", "paf", "ra", "shn", "spe", "sp3", "wav", "wv",
            ],
            Self::Archive => &[
                "7z", "ace", "arj", "bz2", "cab", "dar", "ddf", "gz", "lz", "lzh", "lzo", "lso",
                "pa", "par2", "rar", "sit", "tar", "tg", "tz", "xz", "zip", "zpr",
            ],
            Self::Other => &[],
        }
    }
}

/// Bitmask of selected categories. Defaults to every category enabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CategoryMask {
    pub photo: bool,
    pub video: bool,
    pub document: bool,
    pub audio: bool,
    pub archive: bool,
    pub other: bool,
}

impl Default for CategoryMask {
    fn default() -> Self {
        Self::all()
    }
}

impl CategoryMask {
    pub fn all() -> Self {
        Self {
            photo: true,
            video: true,
            document: true,
            audio: true,
            archive: true,
            other: true,
        }
    }

    pub fn is_all(self) -> bool {
        self.photo && self.video && self.document && self.audio && self.archive && self.other
    }

    pub fn includes(self, category: FileCategory) -> bool {
        match category {
            FileCategory::Photo => self.photo,
            FileCategory::Video => self.video,
            FileCategory::Document => self.document,
            FileCategory::Audio => self.audio,
            FileCategory::Archive => self.archive,
            FileCategory::Other => self.other,
        }
    }
}
