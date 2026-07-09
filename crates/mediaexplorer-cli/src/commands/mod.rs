//! One module per subcommand. Each exposes a pure function over the
//! normalized sector buffer; all host I/O stays in `main.rs`.

pub mod add;
pub mod bootsector;
pub mod convert;
pub mod extract;
pub mod ls;
pub mod mkdir;
pub mod mv;
pub mod new;
pub mod rm;

#[cfg(test)]
pub(crate) mod testdisk {
    use msx_disk::fs::write;
    use msx_disk::image::geometry::DiskFormat;

    /// A blank 720KB disk for command tests.
    pub fn blank() -> Vec<u8> {
        write::create_blank(DiskFormat::Ds720).unwrap()
    }

    /// A disk with `HELLO.TXT` and `UTILS/GAME.COM`.
    pub fn populated() -> Vec<u8> {
        let disk = blank();
        let disk = write::create_dir(&disk, "UTILS").unwrap();
        write::add_files(
            &disk,
            &[
                ("HELLO.TXT", b"hi there".as_ref()),
                ("UTILS/GAME.COM", &[0xC9; 16]),
            ],
        )
        .unwrap()
    }
}
