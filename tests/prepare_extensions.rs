mod support;
use apollo_artifactd::{Limits, Store};
use std::{
    io::{Cursor, Read},
    os::unix::fs::PermissionsExt,
};
use support::{image, layer, platform};

fn extended_layer(kind: &str) -> (Vec<u8>, String, Vec<u8>) {
    let path = format!("{kind}-{}", "n".repeat(180));
    let payload = if kind == "pax-size" {
        vec![b'p'; 600]
    } else {
        b"extended payload".to_vec()
    };
    if kind == "gnu" {
        return (
            layer(&[(&path, &payload), ("following", b"tail")]),
            path,
            payload,
        );
    }
    let mut builder = tar::Builder::new(Vec::new());
    let mut header = tar::Header::new_gnu();
    header.set_mode(0o644);
    header.set_size(payload.len() as u64);
    if kind.starts_with("pax") {
        let mut extensions = vec![("path", path.as_bytes())];
        if kind == "pax-size" {
            extensions.push(("size", b"600"));
            // PAX's effective size controls framing, including the next header.
            header.set_size(0);
        }
        builder.append_pax_extensions(extensions).unwrap();
        header.set_path("placeholder").unwrap();
        header.set_cksum();
        builder.append(&header, payload.as_slice()).unwrap();
    } else {
        header.set_cksum();
        builder
            .append_data(&mut header, &path, payload.as_slice())
            .unwrap();
    }
    let mut following = tar::Header::new_gnu();
    following.set_mode(0o644);
    following.set_size(4);
    following.set_cksum();
    builder
        .append_data(&mut following, "following", b"tail".as_slice())
        .unwrap();
    (builder.into_inner().unwrap(), path, payload)
}

#[test]
fn prepare_standard_extensions_preserves_effective_paths_sizes_and_framing() {
    let mut failures = Vec::new();
    for kind in ["gnu", "pax", "pax-size"] {
        let root = tempfile::tempdir().unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut store = Store::open(root.path(), Limits::default()).unwrap();
        let (layer, path, payload) = extended_layer(kind);
        let manifest = image(&mut store, "amd64", &[layer], false);
        let prepared = match store.prepare(&manifest, &platform("amd64")) {
            Ok(prepared) => prepared,
            Err(error) => {
                failures.push(format!("{kind}: {error:#}"));
                continue;
            }
        };
        let lease = store.lease(&manifest).unwrap();
        let directory = cap_std::fs::Dir::from_std_file(
            store.open_prepared(&prepared, lease.as_str()).unwrap(),
        );
        let mut actual = Vec::new();
        directory
            .open(path)
            .unwrap()
            .read_to_end(&mut actual)
            .unwrap();
        assert_eq!(actual, payload, "{kind} effective payload");
        assert_eq!(
            directory.read("following").unwrap(),
            b"tail",
            "{kind} framing"
        );
        assert_eq!(
            store.prepare(&manifest, &platform("amd64")).unwrap(),
            prepared
        );
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn prepare_rejects_malformed_and_ambiguous_pax_metadata() {
    let mut accepted = Vec::new();
    for kind in [
        "malformed",
        "blank",
        "unterminated",
        "size",
        "duplicate",
        "oversized",
    ] {
        let mut builder = tar::Builder::new(Vec::new());
        match kind {
            "malformed" | "blank" | "unterminated" => {
                let bytes: &[u8] = match kind {
                    "blank" => b"16 path=ignored\n\n999 linkpath=ignored\n",
                    "unterminated" => b"15 path=ignored",
                    _ => b"999 path=ignored\n",
                };
                let mut extension = tar::Header::new_gnu();
                extension.set_entry_type(tar::EntryType::XHeader);
                extension.set_size(bytes.len() as u64);
                extension.set_mode(0o644);
                extension.set_cksum();
                builder
                    .append_data(&mut extension, "metadata", bytes)
                    .unwrap();
            }
            "size" => builder
                .append_pax_extensions([("size", b"not-a-size".as_slice())])
                .unwrap(),
            "duplicate" => builder
                .append_pax_extensions([
                    ("path", b"first".as_slice()),
                    ("path", b"second".as_slice()),
                ])
                .unwrap(),
            "oversized" => builder
                .append_pax_extensions([("comment", vec![b'x'; 65537].as_slice())])
                .unwrap(),
            _ => unreachable!(),
        }
        let mut header = tar::Header::new_gnu();
        header.set_size(4);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, "fallback", b"data".as_slice())
            .unwrap();
        let root = tempfile::tempdir().unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut store = Store::open(root.path(), Limits::default()).unwrap();
        let manifest = image(&mut store, "amd64", &[builder.into_inner().unwrap()], false);
        match store.prepare(&manifest, &platform("amd64")) {
            Ok(_) => accepted.push(kind),
            Err(error) => {
                if kind == "oversized" {
                    assert!(format!("{error:#}").contains("extension exceeds configured limit"));
                }
            }
        }
    }
    assert!(accepted.is_empty(), "accepted ambiguous PAX: {accepted:?}");
}

#[test]
fn prepare_bounds_decompressed_bytes_across_the_whole_image() {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut store = Store::open(
        root.path(),
        Limits {
            max_output: 3072,
            ..Limits::default()
        },
    )
    .unwrap();
    let layers = [layer(&[("first", b"a")]), layer(&[("second", b"b")])];
    assert!(layers.iter().all(|bytes| bytes.len() < 3072));
    assert!(layers.iter().map(Vec::len).sum::<usize>() > 3072);
    let manifest = image(&mut store, "amd64", &layers, false);
    let error = store.prepare(&manifest, &platform("amd64")).unwrap_err();
    assert!(format!("{error:#}").contains("decompression budget exceeded"));
}

#[test]
fn prepare_normalizes_current_directory_members_and_rejects_alias_collisions() {
    // Public prepared output owns normalization; no private path-policy seam.
    for collision in [false, true] {
        let root = tempfile::tempdir().unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut store = Store::open(root.path(), Limits::default()).unwrap();
        let mut archive = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Directory);
        header.set_size(0);
        header.set_mode(0o755);
        header.set_cksum();
        archive
            .append_data(&mut header, "./", std::io::empty())
            .unwrap();
        for path in if collision {
            vec!["./dir/file", "dir/file"]
        } else {
            vec!["./dir/file"]
        } {
            let mut header = tar::Header::new_gnu();
            header.set_size(7);
            header.set_mode(0o644);
            header.set_cksum();
            archive
                .append_data(&mut header, path, b"payload".as_slice())
                .unwrap();
        }
        let manifest = image(&mut store, "amd64", &[archive.into_inner().unwrap()], false);
        if collision {
            let error = store.prepare(&manifest, &platform("amd64")).unwrap_err();
            assert!(
                error.to_string().contains("duplicate layer path"),
                "{error:#}"
            );
        } else {
            let prepared = store.prepare(&manifest, &platform("amd64")).unwrap();
            let lease = store.lease(&manifest).unwrap();
            let directory = cap_std::fs::Dir::from_std_file(
                store.open_prepared(&prepared, lease.as_str()).unwrap(),
            );
            assert_eq!(directory.read("dir/file").unwrap(), b"payload");
            assert_eq!(directory.read_dir(".").unwrap().count(), 1);
        }
    }
}

#[test]
fn prepare_normalization_keeps_root_type_and_traversal_guards() {
    for (path, expected) in [
        (".", "root archive member must be an empty directory"),
        ("./../escape", "absolute/traversing archive path"),
        ("/escape", "invalid archive path"),
    ] {
        let root = tempfile::tempdir().unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut store = Store::open(root.path(), Limits::default()).unwrap();
        let mut archive = tar::Builder::new(Vec::new());
        archive
            .append_pax_extensions([("path", path.as_bytes())])
            .unwrap();
        let mut header = tar::Header::new_gnu();
        header.set_size(1);
        header.set_mode(0o644);
        header.set_cksum();
        archive
            .append_data(&mut header, "placeholder", b"x".as_slice())
            .unwrap();
        let manifest = image(&mut store, "amd64", &[archive.into_inner().unwrap()], false);
        let error = store.prepare(&manifest, &platform("amd64")).unwrap_err();
        assert!(error.to_string().contains(expected), "{path}: {error:#}");
        for entry in std::fs::read_dir(root.path().join("prepared")).unwrap() {
            assert!(
                entry
                    .unwrap()
                    .file_name()
                    .to_str()
                    .unwrap()
                    .ends_with(".staging")
            );
        }
    }
}

#[test]
fn prepare_rewrites_in_root_absolute_symlinks_and_rejects_escape() {
    for (target, expected) in [
        ("/bin/busybox", "busybox"),
        ("../lib/ld-musl", "../lib/ld-musl"),
        ("a/../b", "a/../b"),
        ("/a/../b", "../a/../b"),
    ] {
        let root = tempfile::tempdir().unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut store = Store::open(root.path(), Limits::default()).unwrap();
        let mut archive = tar::Builder::new(Vec::new());
        let mut directory = tar::Header::new_gnu();
        directory.set_entry_type(tar::EntryType::Directory);
        directory.set_size(0);
        directory.set_mode(0o755);
        directory.set_cksum();
        archive
            .append_data(&mut directory, "bin", Cursor::new([]))
            .unwrap();
        let mut link = tar::Header::new_gnu();
        link.set_entry_type(tar::EntryType::Symlink);
        link.set_size(0);
        link.set_link_name(target).unwrap();
        link.set_cksum();
        archive
            .append_data(&mut link, "bin/sh", Cursor::new([]))
            .unwrap();
        let manifest = image(&mut store, "amd64", &[archive.into_inner().unwrap()], false);
        let prepared = store.prepare(&manifest, &platform("amd64")).unwrap();
        let lease = store.lease(&manifest).unwrap();
        let directory = cap_std::fs::Dir::from_std_file(
            store.open_prepared(&prepared, lease.as_str()).unwrap(),
        );
        assert_eq!(
            directory.read_link("bin/sh").unwrap().to_str().unwrap(),
            expected
        );
    }

    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut store = Store::open(root.path(), Limits::default()).unwrap();
    let mut archive = tar::Builder::new(Vec::new());
    let mut link = tar::Header::new_gnu();
    link.set_entry_type(tar::EntryType::Symlink);
    link.set_size(0);
    link.set_link_name("../../etc/passwd").unwrap();
    link.set_cksum();
    archive
        .append_data(&mut link, "bin/sh", Cursor::new([]))
        .unwrap();
    let manifest = image(&mut store, "amd64", &[archive.into_inner().unwrap()], false);
    let error = store.prepare(&manifest, &platform("amd64")).unwrap_err();
    assert!(error.to_string().contains("symlink escape"), "{error:#}");
}

#[test]
fn prepare_preserves_intermediate_symlink_resolution_for_relative_and_absolute_targets() {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut store = Store::open(root.path(), Limits::default()).unwrap();
    let mut archive = tar::Builder::new(Vec::new());
    for path in ["bin", "dir", "dir/sub"] {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Directory);
        header.set_size(0);
        header.set_mode(0o755);
        header.set_cksum();
        archive
            .append_data(&mut header, path, Cursor::new([]))
            .unwrap();
    }
    for (path, contents) in [
        ("b", b"normalized".as_slice()),
        ("dir/b", b"literal".as_slice()),
    ] {
        let mut header = tar::Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        archive
            .append_data(&mut header, path, Cursor::new(contents))
            .unwrap();
    }
    for (path, target) in [
        ("a", "dir/sub"),
        ("bin/relative", "../a/../b"),
        ("bin/absolute", "/a/../b"),
    ] {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        header.set_link_name(target).unwrap();
        header.set_cksum();
        archive
            .append_data(&mut header, path, Cursor::new([]))
            .unwrap();
    }
    let manifest = image(&mut store, "amd64", &[archive.into_inner().unwrap()], false);
    let prepared = store.prepare(&manifest, &platform("amd64")).unwrap();
    let lease = store.lease(&manifest).unwrap();
    let directory =
        cap_std::fs::Dir::from_std_file(store.open_prepared(&prepared, lease.as_str()).unwrap());
    for path in ["bin/relative", "bin/absolute"] {
        let mut resolved = Vec::new();
        directory
            .open(path)
            .unwrap()
            .read_to_end(&mut resolved)
            .unwrap();
        assert_eq!(resolved, b"literal", "{path} was lexically normalized");
    }
}

#[test]
fn prepare_preserves_pax_linkpath_dot_suffix() {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut store = Store::open(root.path(), Limits::default()).unwrap();
    let mut archive = tar::Builder::new(Vec::new());
    let mut directory = tar::Header::new_gnu();
    directory.set_entry_type(tar::EntryType::Directory);
    directory.set_size(0);
    directory.set_mode(0o755);
    directory.set_cksum();
    archive
        .append_data(&mut directory, "bin", Cursor::new([]))
        .unwrap();
    archive
        .append_pax_extensions([("linkpath", b"/bin/busybox/.".as_slice())])
        .unwrap();
    let mut link = tar::Header::new_gnu();
    link.set_entry_type(tar::EntryType::Symlink);
    link.set_size(0);
    link.set_link_name("fallback").unwrap();
    link.set_cksum();
    archive
        .append_data(&mut link, "bin/dot", Cursor::new([]))
        .unwrap();
    let manifest = image(&mut store, "amd64", &[archive.into_inner().unwrap()], false);
    let prepared = store.prepare(&manifest, &platform("amd64")).unwrap();
    let lease = store.lease(&manifest).unwrap();
    let directory =
        cap_std::fs::Dir::from_std_file(store.open_prepared(&prepared, lease.as_str()).unwrap());
    assert_eq!(
        directory.read_link("bin/dot").unwrap().to_str().unwrap(),
        "../bin/busybox/."
    );
}
