use std::{
    collections::HashSet,
    io::{self, Read, Seek, SeekFrom, Write},
};
use zip::{CompressionMethod, ZipArchive};

const MAX_ENTRIES: usize = 16;
const MAX_CENTRAL_DIRECTORY_BYTES: u64 = 64 * 1024;
const MAX_EXPANDED_SIZE: u64 = 64 * 1024 * 1024;
const MAX_TEXT_SIZE: u64 = 1024 * 1024;
const ZIP_END_LEN: usize = 22;
const ZIP_CENTRAL_ENTRY_LEN: usize = 46;

/// The caller supplies a locked, fixed staging file and removes it on failure.
/// No archive member name is ever used as a filesystem path.
pub(super) fn extract_installer(
    version: &str,
    mut reader: impl Read + Seek,
    output: &mut impl Write,
) -> io::Result<()> {
    if version.len() > 32
        || !version
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return Err(invalid("Invalid installer version"));
    }
    let expected_name = format!("campus-network-guardian-{version}-windows-x64-setup.exe");
    check_directory(&mut reader)?;
    let mut archive = ZipArchive::new(reader).map_err(io::Error::other)?;
    if archive.has_duplicate_names() {
        return Err(invalid("Duplicate ZIP member"));
    }
    let mut names = HashSet::new();
    let mut installer = None;
    let mut total_size = 0u64;
    for index in 0..archive.len() {
        let entry = archive.by_index_data(index).map_err(io::Error::other)?;
        let name = entry.name().map_err(io::Error::other)?;
        let unsafe_byte = |byte: u8| matches!(byte, b'/' | b'\\' | b':' | 0);
        if name.is_empty()
            || name.bytes().any(unsafe_byte)
            || entry.name_raw().iter().copied().any(unsafe_byte)
            || !entry.is_file()
            || entry
                .unix_mode()
                .is_some_and(|mode| !matches!(mode & 0o170000, 0 | 0o100000))
            || entry.encrypted()
            || !matches!(
                entry.compression(),
                CompressionMethod::Stored | CompressionMethod::Deflated
            )
        {
            return Err(invalid("Unsafe ZIP member"));
        }
        if !names.insert(name.to_ascii_lowercase()) {
            return Err(invalid("Duplicate ZIP member"));
        }
        let member_limit = if name == expected_name {
            installer = Some(index);
            MAX_EXPANDED_SIZE
        } else if auxiliary_name(&name) {
            MAX_TEXT_SIZE
        } else {
            return Err(invalid("Unexpected ZIP member"));
        };
        total_size = total_size
            .checked_add(entry.size())
            .ok_or_else(|| invalid("ZIP expanded size overflow"))?;
        if entry.size() > member_limit || total_size > MAX_EXPANDED_SIZE {
            return Err(invalid("ZIP expanded size exceeds limit"));
        }
    }
    let installer = installer.ok_or_else(|| invalid("ZIP installer is missing"))?;
    archive
        .validate_header_consistency()
        .map_err(io::Error::other)?;

    // Read auxiliary files to EOF as well, so their CRC and text contents are checked.
    for index in 0..archive.len() {
        if index == installer {
            continue;
        }
        let mut entry = archive.by_index(index).map_err(io::Error::other)?;
        let mut text = Vec::with_capacity(entry.size() as usize);
        let expected_size = entry.size();
        copy_to_eof(&mut entry, &mut text, expected_size)?;
        let text = std::str::from_utf8(&text).map_err(|_| invalid("ZIP text is not UTF-8"))?;
        if text
            .chars()
            .any(|ch| ch.is_control() && !matches!(ch, '\t' | '\r' | '\n'))
        {
            return Err(invalid("ZIP auxiliary member is not text"));
        }
    }

    let mut entry = archive.by_index(installer).map_err(io::Error::other)?;
    let size = entry.size();
    if size < 90 {
        return Err(invalid("Installer PE header is truncated"));
    }
    let mut dos = [0; 64];
    entry.read_exact(&mut dos)?;
    let pe_offset = u64::from(u32::from_le_bytes(dos[0x3c..0x40].try_into().unwrap()));
    if &dos[..2] != b"MZ" || pe_offset < 64 || pe_offset > size - 26 {
        return Err(invalid("Installer DOS header is invalid"));
    }
    output.write_all(&dos)?;
    let gap = pe_offset - 64;
    if io::copy(&mut entry.by_ref().take(gap), output)? != gap {
        return Err(invalid("Installer PE header is truncated"));
    }
    let mut pe = [0; 26];
    entry.read_exact(&mut pe)?;
    let optional_header_size = u64::from(word_at(&pe, 20));
    // The x64 release uses an I386 Inno Setup launcher with a PE32 optional header.
    if &pe[..4] != b"PE\0\0"
        || !matches!(
            (word_at(&pe, 4), word_at(&pe, 24)),
            (0x014c, 0x010b) | (0x8664, 0x020b)
        )
        || optional_header_size < 2
        || optional_header_size > size - pe_offset - 24
    {
        return Err(invalid("Installer must be an x86 or AMD64 PE executable"));
    }
    output.write_all(&pe)?;
    // An explicit final EOF read is required to trigger zip's CRC verification.
    copy_to_eof(&mut entry, output, size - pe_offset - pe.len() as u64)
}

fn check_directory(reader: &mut (impl Read + Seek)) -> io::Result<()> {
    // Bound the classic single-disk directory before ZipArchive builds its index.
    let size = reader.seek(SeekFrom::End(0))?;
    let tail_len = size.min((ZIP_END_LEN + u16::MAX as usize) as u64) as usize;
    let tail_start = size - tail_len as u64;
    reader.seek(SeekFrom::Start(tail_start))?;
    let mut tail = vec![0; tail_len];
    reader.read_exact(&mut tail)?;
    let end_index = tail
        .windows(ZIP_END_LEN)
        .rposition(|record| record.starts_with(b"PK\x05\x06"))
        .ok_or_else(|| invalid("ZIP end record is missing"))?;
    let end = tail_start + end_index as u64;
    let footer = &tail[end_index..][..ZIP_END_LEN];
    let count = usize::from(word_at(footer, 10));
    if count == 0 || count > MAX_ENTRIES {
        return Err(invalid("ZIP directory entry count exceeds limit"));
    }
    if footer[4..8] != [0; 4]
        || usize::from(word_at(footer, 8)) != count
        || usize::from(word_at(footer, 20)) != tail_len - end_index - ZIP_END_LEN
    {
        return Err(invalid("Invalid single-disk ZIP end record"));
    }
    let directory_size = u64::from(u32::from_le_bytes(footer[12..16].try_into().unwrap()));
    let mut position = u64::from(u32::from_le_bytes(footer[16..20].try_into().unwrap()));
    if directory_size > MAX_CENTRAL_DIRECTORY_BYTES || position + directory_size != end {
        return Err(invalid("Invalid ZIP central directory size"));
    }

    let mut entry = [0; ZIP_CENTRAL_ENTRY_LEN];
    for _ in 0..count {
        if end.saturating_sub(position) < ZIP_CENTRAL_ENTRY_LEN as u64 {
            return Err(invalid("ZIP central directory is truncated"));
        }
        reader.seek(SeekFrom::Start(position))?;
        reader.read_exact(&mut entry)?;
        if &entry[..4] != b"PK\x01\x02" || word_at(&entry, 34) != 0 {
            return Err(invalid("Invalid ZIP central directory entry"));
        }
        position += ZIP_CENTRAL_ENTRY_LEN as u64
            + u64::from(word_at(&entry, 28))
            + u64::from(word_at(&entry, 30))
            + u64::from(word_at(&entry, 32));
    }
    if position != end {
        return Err(invalid(
            "ZIP central directory does not match its end record",
        ));
    }
    Ok(())
}

fn word_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn auxiliary_name(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "readme"
            | "readme.txt"
            | "readme.md"
            | "license"
            | "license.txt"
            | "license.md"
            | "copying"
            | "copying.txt"
            | "copying.md"
            | "notice"
            | "notice.txt"
            | "notice.md"
            | "安装说明.txt"
            | "安装说明.md"
            | "使用说明.txt"
            | "使用说明.md"
            | "许可证.txt"
    )
}

fn copy_to_eof(
    reader: &mut impl Read,
    output: &mut impl Write,
    expected_size: u64,
) -> io::Result<()> {
    let mut remaining = expected_size;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let capacity = (remaining + 1).min(buffer.len() as u64) as usize;
        let count = match reader.read(&mut buffer[..capacity]) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            result => result?,
        };
        if count == 0 {
            return if remaining == 0 {
                Ok(())
            } else {
                Err(invalid("ZIP member size differs from metadata"))
            };
        }
        if count as u64 > remaining {
            return Err(invalid("ZIP member exceeds declared size"));
        }
        output.write_all(&buffer[..count])?;
        remaining -= count as u64;
    }
}

#[cfg(test)]
mod tests {
    use super::extract_installer;
    use flate2::{write::DeflateEncoder, Compression};
    use std::io::{Cursor, Write};

    const VERSION: &str = "0.6.8";
    const INSTALLER: &str = "campus-network-guardian-0.6.8-windows-x64-setup.exe";
    const MIB: u32 = 1024 * 1024;

    struct Entry {
        name: String,
        data: Vec<u8>,
        method: u16,
        flags: u16,
        mode: u32,
        claimed_size: Option<u32>,
        comment: Vec<u8>,
    }

    impl Entry {
        fn new(name: &str, data: Vec<u8>) -> Self {
            Self {
                name: name.into(),
                data,
                method: 0,
                flags: 0x800,
                mode: 0o100644,
                claimed_size: None,
                comment: Vec::new(),
            }
        }
    }

    fn pe() -> Vec<u8> {
        let mut data = vec![0; 256];
        data[0..2].copy_from_slice(b"MZ");
        data[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        data[0x80..0x84].copy_from_slice(b"PE\0\0");
        data[0x84..0x86].copy_from_slice(&0x8664u16.to_le_bytes());
        data[0x94..0x96].copy_from_slice(&2u16.to_le_bytes());
        data[0x98..0x9a].copy_from_slice(&0x020bu16.to_le_bytes());
        data
    }

    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = !0u32;
        for byte in bytes {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xedb8_8320 & (0u32.wrapping_sub(crc & 1)));
            }
        }
        !crc
    }

    fn word(out: &mut Vec<u8>, value: u16) {
        out.extend(value.to_le_bytes());
    }

    fn dword(out: &mut Vec<u8>, value: u32) {
        out.extend(value.to_le_bytes());
    }

    // Build real ZIP records so duplicate names, flags and hostile paths survive intact.
    fn zip(entries: Vec<Entry>) -> Vec<u8> {
        let count = entries.len() as u16;
        let mut output = Vec::new();
        let mut central = Vec::new();
        for entry in entries {
            let offset = output.len() as u32;
            let checksum = crc32(&entry.data);
            let size = entry.claimed_size.unwrap_or(entry.data.len() as u32);
            let compressed = if entry.method == 8 {
                let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
                encoder.write_all(&entry.data).unwrap();
                encoder.finish().unwrap()
            } else {
                entry.data
            };
            dword(&mut output, 0x0403_4b50);
            word(&mut output, 20);
            word(&mut output, entry.flags);
            word(&mut output, entry.method);
            dword(&mut output, 0);
            dword(&mut output, checksum);
            dword(&mut output, compressed.len() as u32);
            dword(&mut output, size);
            word(&mut output, entry.name.len() as u16);
            word(&mut output, 0);
            output.extend(entry.name.as_bytes());
            output.extend(compressed.iter());

            dword(&mut central, 0x0201_4b50);
            word(&mut central, 0x0314);
            word(&mut central, 20);
            word(&mut central, entry.flags);
            word(&mut central, entry.method);
            dword(&mut central, 0);
            dword(&mut central, checksum);
            dword(&mut central, compressed.len() as u32);
            dword(&mut central, size);
            word(&mut central, entry.name.len() as u16);
            word(&mut central, 0);
            word(&mut central, entry.comment.len() as u16);
            word(&mut central, 0);
            word(&mut central, 0);
            dword(&mut central, entry.mode << 16);
            dword(&mut central, offset);
            central.extend(entry.name.as_bytes());
            central.extend(entry.comment);
        }
        let central_offset = output.len() as u32;
        let central_size = central.len() as u32;
        output.extend(central);
        dword(&mut output, 0x0605_4b50);
        word(&mut output, 0);
        word(&mut output, 0);
        word(&mut output, count);
        word(&mut output, count);
        dword(&mut output, central_size);
        dword(&mut output, central_offset);
        word(&mut output, 0);
        output
    }

    fn reject(entries: Vec<Entry>) {
        let result = extract_installer(VERSION, Cursor::new(zip(entries)), &mut Vec::new());
        assert!(result.is_err(), "unsafe ZIP must fail closed");
    }

    #[test]
    fn extracts_stored_amd64_installer_with_chinese_readme() {
        let installer = pe();
        let archive = zip(vec![
            Entry::new("安装说明.txt", "先解压，再安装。".as_bytes().to_vec()),
            Entry::new(INSTALLER, installer.clone()),
        ]);
        let mut output = Vec::new();
        extract_installer(VERSION, Cursor::new(archive), &mut output).unwrap();
        assert_eq!(output, installer);
    }

    #[test]
    fn extracts_deflated_installer() {
        let installer = pe();
        let mut entry = Entry::new(INSTALLER, installer.clone());
        entry.method = 8;
        let mut output = Vec::new();
        extract_installer(VERSION, Cursor::new(zip(vec![entry])), &mut output).unwrap();
        assert_eq!(output, installer);
    }

    #[test]
    fn extracts_x64_release_packaged_with_inno_pe32_setup_stub() {
        let mut installer = pe();
        installer[0x84..0x86].copy_from_slice(&0x014cu16.to_le_bytes());
        installer[0x98..0x9a].copy_from_slice(&0x010bu16.to_le_bytes());
        let mut output = Vec::new();
        let bytes = zip(vec![Entry::new(INSTALLER, installer.clone())]);
        extract_installer(VERSION, Cursor::new(bytes), &mut output).unwrap();
        assert_eq!(output, installer);
    }

    #[test]
    fn rejects_empty_archive_and_missing_installer() {
        reject(vec![]);
        reject(vec![Entry::new("README.md", b"read me".to_vec())]);
        reject(vec![Entry::new(
            "campus-network-guardian-0.6.7-windows-x64-setup.exe",
            pe(),
        )]);
    }

    #[test]
    fn rejects_duplicate_member_names() {
        reject(vec![
            Entry::new(INSTALLER, pe()),
            Entry::new(INSTALLER, pe()),
        ]);
        reject(vec![
            Entry::new(INSTALLER, pe()),
            Entry::new("README.md", vec![]),
            Entry::new("README.md", vec![]),
        ]);
        reject(vec![
            Entry::new(INSTALLER, pe()),
            Entry::new("README.md", vec![]),
            Entry::new("readme.MD", vec![]),
        ]);
    }

    #[test]
    fn rejects_paths_directories_and_unexpected_members() {
        for name in [
            "../README.md",
            "/README.md",
            "C:/README.md",
            "C:README.md",
            "folder/README.md",
            "folder\\README.md",
            "README.md:ads",
            "README.md/",
            "install.cmd",
            "unexpected.exe",
        ] {
            reject(vec![Entry::new(INSTALLER, pe()), Entry::new(name, vec![])]);
        }
        let mut directory = Entry::new("README.md", vec![]);
        directory.mode = 0o040755;
        reject(vec![Entry::new(INSTALLER, pe()), directory]);
    }

    #[test]
    fn rejects_symbolic_links() {
        let mut entry = Entry::new(INSTALLER, pe());
        entry.mode = 0o120777;
        reject(vec![entry]);
    }

    #[test]
    fn rejects_encrypted_and_unsupported_compression() {
        let mut encrypted = Entry::new(INSTALLER, pe());
        encrypted.flags |= 1;
        reject(vec![encrypted]);
        let mut unsupported = Entry::new(INSTALLER, pe());
        unsupported.method = 12;
        reject(vec![unsupported]);
    }

    #[test]
    fn rejects_excessive_entry_count() {
        let mut entries = vec![Entry::new(INSTALLER, pe())];
        entries.extend((0..16).map(|_| Entry::new("README.md", vec![])));
        let bytes = zip(entries);
        let error = extract_installer(VERSION, Cursor::new(bytes), &mut Vec::new()).unwrap_err();
        assert_eq!(error.to_string(), "ZIP directory entry count exceeds limit");
    }

    #[test]
    fn rejects_oversized_central_directory_even_if_its_size_is_understated() {
        let mut installer = Entry::new(INSTALLER, pe());
        installer.comment = vec![b' '; u16::MAX as usize];
        let mut bytes = zip(vec![installer]);
        assert!(extract_installer(VERSION, Cursor::new(&bytes), &mut Vec::new()).is_err());

        let size_offset = bytes.len() - 10;
        let short_size = (46 + INSTALLER.len()) as u32;
        bytes[size_offset..size_offset + 4].copy_from_slice(&short_size.to_le_bytes());
        assert!(extract_installer(VERSION, Cursor::new(bytes), &mut Vec::new()).is_err());
    }

    #[test]
    fn rejects_central_entry_whose_fields_extend_into_footer() {
        let mut bytes = zip(vec![Entry::new(INSTALLER, pe())]);
        let central = 30 + INSTALLER.len() + pe().len();
        let comment_len = central + 32;
        bytes[comment_len..comment_len + 2].copy_from_slice(&1u16.to_le_bytes());
        assert!(extract_installer(VERSION, Cursor::new(bytes), &mut Vec::new()).is_err());
    }

    #[test]
    fn rejects_installer_text_and_total_size_over_limits() {
        let mut installer = Entry::new(INSTALLER, pe());
        installer.claimed_size = Some(64 * MIB + 1);
        reject(vec![installer]);
        let mut text = Entry::new("README.md", b"text".to_vec());
        text.claimed_size = Some(MIB + 1);
        reject(vec![Entry::new(INSTALLER, pe()), text]);
        let mut installer = Entry::new(INSTALLER, pe());
        installer.claimed_size = Some(64 * MIB);
        reject(vec![installer, Entry::new("README.md", vec![b'a'])]);
    }

    #[test]
    fn rejects_damaged_installer_and_auxiliary_crc() {
        for corrupt_installer in [true, false] {
            let mut archive = zip(vec![
                Entry::new(INSTALLER, pe()),
                Entry::new("README.md", b"readme".to_vec()),
            ]);
            if corrupt_installer {
                archive[30 + INSTALLER.len() + 200] ^= 1;
            } else {
                archive[30 + INSTALLER.len() + pe().len() + 30 + "README.md".len()] ^= 1;
            }
            assert!(extract_installer(VERSION, Cursor::new(archive), &mut Vec::new()).is_err());
        }
    }

    #[test]
    fn rejects_non_text_auxiliary_files() {
        for data in [vec![0], vec![0xff, 0xfe], b"README\x01".to_vec()] {
            reject(vec![
                Entry::new(INSTALLER, pe()),
                Entry::new("README.md", data),
            ]);
        }
    }

    #[test]
    fn rejects_declared_sizes_that_disagree_with_decompressed_bytes() {
        for size in [255, 257] {
            let mut entry = Entry::new(INSTALLER, pe());
            entry.method = 8;
            entry.claimed_size = Some(size);
            reject(vec![entry]);
        }
    }

    #[test]
    fn rejects_local_and_central_header_disagreement() {
        let mut archive = zip(vec![Entry::new(INSTALLER, pe())]);
        archive[30] = b'x';
        assert!(extract_installer(VERSION, Cursor::new(archive), &mut Vec::new()).is_err());
    }

    #[test]
    fn propagates_output_write_failures() {
        struct Unwritable;
        impl Write for Unwritable {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "locked",
                ))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let result = extract_installer(
            VERSION,
            Cursor::new(zip(vec![Entry::new(INSTALLER, pe())])),
            &mut Unwritable,
        );
        assert_eq!(
            result.unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
    }

    #[test]
    fn rejects_unsupported_or_malformed_pe_headers() {
        let mut no_mz = pe();
        no_mz[0] = 0;
        let mut no_pe = pe();
        no_pe[0x80] = 0;
        let mut mismatched = pe();
        mismatched[0x84..0x86].copy_from_slice(&0x014cu16.to_le_bytes());
        let mut arm64 = pe();
        arm64[0x84..0x86].copy_from_slice(&0xaa64u16.to_le_bytes());
        let mut bad_offset = pe();
        bad_offset[0x3c..0x40].copy_from_slice(&u32::MAX.to_le_bytes());
        for data in [
            vec![],
            b"MZ".to_vec(),
            no_mz,
            no_pe,
            mismatched,
            arm64,
            bad_offset,
        ] {
            reject(vec![Entry::new(INSTALLER, data)]);
        }
    }

    #[test]
    fn rejects_untrusted_version_text() {
        for version in ["../0.6.8", "0.6.8\\x", "", "0.6.8:ads"] {
            assert!(extract_installer(
                version,
                Cursor::new(zip(vec![Entry::new(INSTALLER, pe())])),
                &mut Vec::new()
            )
            .is_err());
        }
    }
}
