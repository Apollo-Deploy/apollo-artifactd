use std::io::{Cursor, Read};
use tar::{Archive, Builder, Header};

const LIMIT: u64 = 64 * 1024;

fn archive_with_pax(headers: &[(&str, &[u8])]) -> Vec<u8> {
    let mut builder = Builder::new(Vec::new());
    builder
        .append_pax_extensions(headers.iter().copied())
        .unwrap();
    let mut header = Header::new_ustar();
    header.set_path("short").unwrap();
    header.set_size(1);
    header.set_cksum();
    builder.append(&header, &b"x"[..]).unwrap();
    builder.into_inner().unwrap()
}

#[test]
fn configured_limit_bounds_extension_buffer_before_read_all() {
    let value = vec![b'x'; LIMIT as usize + 1];
    let bytes = archive_with_pax(&[("comment", value.as_slice())]);
    let mut archive = Archive::new(Cursor::new(bytes));
    archive.set_extension_size_limit(LIMIT);
    let mut entries = archive.entries().unwrap();
    assert!(entries.next().unwrap().is_err());
}

#[test]
fn effective_pax_path_and_size_preserve_following_entry_framing() {
    let mut builder = Builder::new(Vec::new());
    let long_path = format!("root/{}/file", "segment".repeat(30));
    builder
        .append_pax_extensions([("path", long_path.as_bytes()), ("size", b"513")])
        .unwrap();
    let mut first = Header::new_ustar();
    first.set_path("short").unwrap();
    first.set_size(512);
    first.set_cksum();
    builder.append(&first, vec![b'a'; 512].as_slice()).unwrap();
    let mut second = Header::new_ustar();
    second.set_path("following").unwrap();
    second.set_size(1);
    second.set_cksum();
    builder.append(&second, &b"y"[..]).unwrap();
    builder.finish().unwrap();
    let mut bytes = builder.into_inner().unwrap();
    // The PAX size makes the first data member span one additional tar block.
    // Insert that block before the following header while keeping the header's
    // ordinary size field at 512 bytes.
    let following_header = 2048;
    bytes.splice(following_header..following_header, [0u8; 512]);

    let mut archive = Archive::new(Cursor::new(bytes));
    archive.set_extension_size_limit(LIMIT);
    let mut entries = archive.entries().unwrap();
    let mut first = entries.next().unwrap().unwrap();
    assert_eq!(first.path().unwrap().to_string_lossy(), long_path);
    let mut first_data = Vec::new();
    first.read_to_end(&mut first_data).unwrap();
    assert_eq!(first_data.len(), 513);
    assert_eq!(
        entries
            .next()
            .unwrap()
            .unwrap()
            .path()
            .unwrap()
            .to_string_lossy(),
        "following"
    );
}

#[test]
fn gnu_long_path_is_exposed_as_effective_entry_path() {
    let mut builder = Builder::new(Vec::new());
    let long_path = format!("root/{}/file", "segment".repeat(30));
    let mut header = Header::new_gnu();
    header.set_size(1);
    header.set_cksum();
    builder
        .append_data(&mut header, &long_path, &b"x"[..])
        .unwrap();
    let mut archive = Archive::new(Cursor::new(builder.into_inner().unwrap()));
    archive.set_extension_size_limit(LIMIT);
    let entry = archive.entries().unwrap().next().unwrap().unwrap();
    assert_eq!(entry.path().unwrap().to_string_lossy(), long_path);
}

#[test]
fn malformed_pax_record_is_reported_by_extension_iterator() {
    let mut bytes = archive_with_pax(&[("path", b"valid")]);
    let marker = bytes
        .windows(5)
        .position(|window| window == b"path=")
        .unwrap();
    bytes[marker - 2] = b'0';
    let mut archive = Archive::new(Cursor::new(bytes));
    let mut entry = archive.entries().unwrap().next().unwrap().unwrap();
    assert!(entry
        .pax_extensions()
        .unwrap()
        .unwrap()
        .next()
        .unwrap()
        .is_err());
}

#[test]
fn empty_interior_record_is_not_a_silent_end() {
    let mut extensions = tar::PaxExtensions::new(b"10 path=x\n\n10 path=y\n");
    assert!(extensions.next().unwrap().is_ok());
    assert!(extensions.next().unwrap().is_err());
}

#[test]
fn unterminated_record_is_rejected() {
    let mut extensions = tar::PaxExtensions::new(b"10 path=x");
    assert!(extensions.next().unwrap().is_err());
}

#[test]
fn aggregate_extension_limit_bounds_many_small_members() {
    let mut builder = Builder::new(Vec::new());
    for index in 0..3 {
        builder
            .append_pax_extensions([("comment", b"x".as_slice())])
            .unwrap();
        let mut header = Header::new_ustar();
        header.set_path(format!("entry-{index}")).unwrap();
        header.set_size(1);
        header.set_cksum();
        builder.append(&header, &b"x"[..]).unwrap();
    }
    let mut archive = Archive::new(Cursor::new(builder.into_inner().unwrap()));
    archive.set_extension_size_limit(64);
    archive.set_extension_total_limit(20);
    let mut entries = archive.entries().unwrap();
    assert!(entries.next().unwrap().is_ok());
    assert!(entries.next().unwrap().is_err());
}
