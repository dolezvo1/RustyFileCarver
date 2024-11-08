use clap::Parser;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::sync::LazyLock;

trait SizeRange {
    fn max(&self) -> usize;
}
impl SizeRange for std::ops::RangeFull {
    fn max(&self) -> usize { usize::MAX }
}
impl SizeRange for std::ops::RangeInclusive<usize> {
    fn max(&self) -> usize { *self.end() }
}
impl SizeRange for std::ops::RangeToInclusive<usize> {
    fn max(&self) -> usize { self.end }
}

enum Footer<'a> {
    None, // Take maximum size allowed by the range
    Inclusive(&'a [u8]), // Take until end of footer or maximum allowed by the range
    Exclusive(&'a [u8]), // Take until beginning of footer or maximum allowed by the range
}

impl<'a> Footer<'a> {
    fn file_size_after_footer_pos(&self) -> usize {
        match self {
            Footer::None | Footer::Exclusive(_) => 0,
            Footer::Inclusive(data) => data.len(),
        }
    }
}

// Define file signatures with extension, valid size range, header, and footer
const FILE_SIGNATURES: LazyLock<Vec<(&str, Box<dyn SizeRange>, &[u8], Footer)>> = LazyLock::new(|| vec![
    // Archive/Binary files
    ("zip", Box::new(..=10_000_000usize), b"PK\x03\x04", Footer::Inclusive(b"\x50\x4B\x05\x06")), // ZIP
    ("rar", Box::new(..=10_000_000usize), b"Rar!", Footer::Inclusive(b"\x00\x00\x00\x00")), // RAR
    ("7z", Box::new(..=10_000_000usize), b"7z\xBC\xAF\x27\x1C", Footer::Inclusive(b"\x00\x00\x00\x00")), // 7Z
    ("tar", Box::new(..=10_000_000usize), b"ustar", Footer::Inclusive(b"\x00\x00\x00\x00")), // TAR
    ("iso", Box::new(..=10_000_000usize), b"CD001", Footer::Inclusive(b"\x00\x00\x00\x00")), // ISO

    // Documents
    ("doc", Box::new(..=10_000_000usize), b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1\x00\x00", Footer::Exclusive(b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1\x00\x00")), // DOC
    ("doc", Box::new(..=10_000_000usize), b"\xd0\xcf\x11\xe0\xa1\xb1", Footer::None), // DOC
    ("html", Box::new(..=10_000_000usize), b"<html", Footer::Inclusive(b"</html>")), // HTML
    ("html", Box::new(..=10_000_000usize), b"<!DOCTYPE html", Footer::Inclusive(b"</html>")), // HTML
    ("pdf", Box::new(..=10_000_000usize), b"%PDF-", Footer::Inclusive(b"%%EOF")), // PDF
    ("rtf", Box::new(..=10_000_000usize), b"{\\rtf1", Footer::Inclusive(b"}")), // RTF

    // Image files
    // TODO: BMP could have less false positives if regexes were used?
    ("bmp", Box::new(..=10_000_000usize), b"\x42\x4D", Footer::None), // BMP
    ("gif", Box::new(..=5_000_000usize), b"\x47\x49\x46\x38\x37\x61", Footer::Inclusive(b"\x00\x3b")), // GIF
    ("gif", Box::new(..=5_000_000usize), b"\x47\x49\x46\x38\x39\x61", Footer::Inclusive(b"\x00\x00\x3b")), // GIF
    ("jpg", Box::new(..=200_000_000usize), b"\xff\xd8\xff\xe0\x00\x10", Footer::Inclusive(b"\xFF\xD9")), // JPEG
    ("jpg", Box::new(..=200_000_000usize), b"\xff\xd8\xff\xe1", Footer::Inclusive(b"\xFF\xD9")), // JPEG
    ("png", Box::new(..=10_000_000usize), b"\x89PNG\r\n\x1A\n", Footer::Inclusive(b"\xFF\xFC\xFD\xFE")), // PNG
    ("tif", Box::new(..=10_000_000usize), b"\x49\x49\x2a\x00", Footer::None), // TIFF
    ("tif", Box::new(..=10_000_000usize), b"\x4D\x4D\x00\x2A", Footer::None), // TIFF

    // Audio/Video
    ("avi", Box::new(..=10_000_000usize), b"RIFF\x00\x00\x00AVI ", Footer::None), // AVI
    ("mov", Box::new(..=10_000_000usize), b"\x00\x00\x00\x20ftyp", Footer::None), // MOV
    ("mp3", Box::new(..=10_000_000usize), b"\x57\x41\x56\\45", Footer::Inclusive(b"\x00\x00\xFF")), // MP3
    ("mp3", Box::new(..=10_000_000usize), b"\xFF\xFB\xD0\\", Footer::Inclusive(b"\xD1\x35\x51\xCC")), // MP3
    ("mp3", Box::new(..=10_000_000usize), b"\x4C\x41\x4D\x45\\", Footer::None), // MP3
    ("mp4", Box::new(..=10_000_000usize), b"\x00\x00\x00\x20ftyp", Footer::None), // MP4
    ("wav", Box::new(..=10_000_000usize), b"RIFF\x00\x00\x00WAVE", Footer::None), // WAV
]);

#[derive(Parser)]
struct CliArgs {
    /// Input .img/.dd/.raw file
    #[arg(long)]
    input_file: String,

    /// Directory to save recovered files to
    #[arg(long)]
    output_directory: String,
}

/// NOTE: does not read FAT/recover allocated files
fn main() -> io::Result<()> {
    let cli_args = CliArgs::parse();

    // Create output directory
    std::fs::create_dir_all(&cli_args.output_directory)?;

    // Read the entire input file into memory
    // TODO: would memory mapping be beneficial here?
    let mut file = File::open(&cli_args.input_file)?;
    let mut file_data = Vec::new();
    file.read_to_end(&mut file_data)?;

    let file_size = file_data.len();

    // Check for each signature in the loaded file data
    for (idx, (extension, size_range, header, footer)) in FILE_SIGNATURES.iter().enumerate() {
        for pos in (0..file_size - header.len())
                .filter(|ii| file_data[(*ii)..*ii + header.len()] == **header)
        {
            println!("Found {} signature at offset {}", extension, pos);

            let file_name = format!("{}/recovered_{}_{}.{}", cli_args.output_directory, pos, idx, extension);
            let mut output_file = OpenOptions::new()
                .write(true)
                .create(true)
                .open(file_name)?;

            // Check for footer in the remaining data
            let footer_signature_pos = match footer {
                Footer::Inclusive(f) | Footer::Exclusive(f) => Some(f),
                Footer::None => None,
            }.and_then(|e| find_signature(&file_data[(pos+header.len())..file_size], e));

            if let Some(footer_pos) = footer_signature_pos {
                output_file.write_all(&file_data[pos..pos + footer_pos + footer.file_size_after_footer_pos()])?;
            } else {
                output_file.write_all(&file_data[pos..pos + size_range.max()])?;
            }
        }
    }

    Ok(())
}

fn find_signature(buffer: &[u8], signature: &[u8]) -> Option<usize> {
    for ii in 0..buffer.len() - signature.len() {
        if &buffer[ii..ii + signature.len()] == signature {
            return Some(ii);
        }
    }
    None
}
