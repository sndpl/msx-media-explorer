//! Command-line argument definitions (clap derive).

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use msx_disk::charset::MsxCharset;
use msx_disk::fs::DosVersion;
use msx_disk::image::geometry::DiskFormat;

#[derive(Parser)]
#[command(
    name = "mediaexplorer-cli",
    version,
    about = "Browse and edit MSX disk images from the command line",
    after_help = "Edits to a compressed .xsa image are recompressed in place. Raw-track .dmk \
                  containers and partitioned hard-disk images cannot be modified; use `convert` \
                  to turn a .dmk into an editable .dsk first."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// List the contents of a disk image
    #[command(visible_alias = "l")]
    Ls(LsArgs),
    /// Add (or update) host files on a disk image
    #[command(visible_alias = "a")]
    Add(AddArgs),
    /// Extract files from a disk image to the host filesystem
    #[command(visible_alias = "e")]
    Extract(ExtractArgs),
    /// Remove files or directories from a disk image
    #[command(visible_alias = "r")]
    Rm(RmArgs),
    /// Rename or move a file or directory inside a disk image
    #[command(visible_alias = "m")]
    Mv(MvArgs),
    /// Create directories on a disk image
    #[command(visible_alias = "d")]
    Mkdir(MkdirArgs),
    /// Create a new blank, formatted disk image
    #[command(visible_alias = "n")]
    New(NewArgs),
    /// Show or install the MSX-DOS 1/2 boot sector of a disk image
    #[command(visible_alias = "b")]
    Bootsector(BootsectorArgs),
    /// Convert between disk-image containers (.dsk, .img, .msx, .ddi, .xsa, .dmk, .sav)
    #[command(visible_alias = "c")]
    Convert(ConvertArgs),
    /// Split a multi-disk image (several whole disks in one file) into one .dsk per disk
    #[command(visible_alias = "s")]
    Split(SplitArgs),
}

#[derive(Args)]
pub struct SplitArgs {
    /// Multi-disk image to split
    pub image: PathBuf,
    /// Directory to write the per-disk images into (default: the image's own
    /// directory)
    #[arg(long, value_name = "DIR")]
    pub out_dir: Option<PathBuf>,
    /// Overwrite existing output files
    #[arg(short, long)]
    pub force: bool,
}

#[derive(Args)]
pub struct LsArgs {
    /// Disk image to read
    pub image: PathBuf,
    /// Directory (or file) inside the image; defaults to the root
    pub path: Option<String>,
    /// Long listing: attributes, size, date, plus a volume header line
    #[arg(short, long)]
    pub long: bool,
    /// Recurse into subdirectories
    #[arg(short = 'R', long)]
    pub recursive: bool,
    /// Code page for filenames (default: auto-detect)
    #[arg(long, value_enum)]
    pub charset: Option<CharsetArg>,
}

#[derive(Args)]
pub struct AddArgs {
    /// Disk image to modify
    pub image: PathBuf,
    /// Host files to add
    #[arg(required = true)]
    pub files: Vec<PathBuf>,
    /// Existing directory inside the image to add into
    #[arg(long, value_name = "DIR")]
    pub dest: Option<String>,
    /// Name to use inside the image (single host file only)
    #[arg(long = "as", value_name = "NAME")]
    pub as_name: Option<String>,
    /// Overwrite existing files inside the image
    #[arg(short, long)]
    pub force: bool,
    /// Code page for non-ASCII names in --dest/--as (default: auto-detect)
    #[arg(long, value_enum)]
    pub charset: Option<CharsetArg>,
}

#[derive(Args)]
pub struct ExtractArgs {
    /// Disk image to read
    pub image: PathBuf,
    /// File or directory inside the image; omit to extract everything
    pub path: Option<String>,
    /// Host directory to extract into (default: current directory)
    #[arg(long, value_name = "DIR", default_value = ".")]
    pub out: PathBuf,
    /// Required to extract a directory
    #[arg(short = 'R', long)]
    pub recursive: bool,
    /// Code page used to decode filenames for the host (default: auto-detect)
    #[arg(long, value_enum)]
    pub charset: Option<CharsetArg>,
}

#[derive(Args)]
pub struct RmArgs {
    /// Disk image to modify
    pub image: PathBuf,
    /// Files or directories inside the image
    #[arg(required = true)]
    pub paths: Vec<String>,
    /// Remove directories and their contents recursively
    #[arg(short, long)]
    pub recursive: bool,
    /// Ignore missing paths
    #[arg(short, long)]
    pub force: bool,
}

#[derive(Args)]
pub struct MvArgs {
    /// Disk image to modify
    pub image: PathBuf,
    /// Existing file or directory inside the image
    pub from: String,
    /// New name, or an existing directory to move into
    pub to: String,
    /// Replace an existing target file
    #[arg(short, long)]
    pub force: bool,
}

#[derive(Args)]
pub struct MkdirArgs {
    /// Disk image to modify
    pub image: PathBuf,
    /// Directories to create inside the image
    #[arg(required = true)]
    pub paths: Vec<String>,
    /// Create missing parent directories; no error if the directory exists
    #[arg(short, long)]
    pub parents: bool,
}

#[derive(Args)]
pub struct NewArgs {
    /// Path of the image to create (a raw .dsk-family container)
    pub image: PathBuf,
    /// Disk format
    #[arg(long, value_enum, default_value = "720")]
    pub format: FormatArg,
    /// Boot sector generation: 2 enables MSX-DOS 2 UNDEL and disk cache
    #[arg(long, value_enum, default_value = "2")]
    pub dos: DosArg,
    /// Overwrite an existing file
    #[arg(short, long)]
    pub force: bool,
}

#[derive(Args)]
pub struct BootsectorArgs {
    /// Disk image to inspect or modify
    pub image: PathBuf,
    /// Install this boot sector generation (omit to just show the current one)
    #[arg(long, value_enum)]
    pub dos: Option<DosArg>,
}

#[derive(Args)]
pub struct ConvertArgs {
    /// Source image (.dsk, .img, .msx, .ddi, .xsa, .dmk, .sav)
    pub input: PathBuf,
    /// Destination image; the extension picks the format (.dsk, .xsa, or .sav)
    pub output: PathBuf,
    /// Overwrite an existing output file
    #[arg(short, long)]
    pub force: bool,
}

/// `--charset` values, mapped onto [`MsxCharset`].
#[derive(Clone, Copy, ValueEnum)]
pub enum CharsetArg {
    International,
    Japanese,
    Russian,
    Korean,
    Arabic,
    Brazilian,
}

impl From<CharsetArg> for MsxCharset {
    fn from(arg: CharsetArg) -> MsxCharset {
        match arg {
            CharsetArg::International => MsxCharset::International,
            CharsetArg::Japanese => MsxCharset::Japanese,
            CharsetArg::Russian => MsxCharset::Russian,
            CharsetArg::Korean => MsxCharset::Korean,
            CharsetArg::Arabic => MsxCharset::Arabic,
            CharsetArg::Brazilian => MsxCharset::Brazilian,
        }
    }
}

/// `--format` values for `new`, mapped onto [`DiskFormat`].
#[derive(Clone, Copy, ValueEnum)]
pub enum FormatArg {
    /// 3.5" double-sided 720 kB (2DD)
    #[value(name = "720")]
    Ds720,
    /// 3.5" single-sided 360 kB (1DD)
    #[value(name = "360ss")]
    Ss360,
    /// 5.25" double-sided 360 kB
    #[value(name = "360ds")]
    Ds360,
    /// 5.25" single-sided 180 kB
    #[value(name = "180")]
    Ss180,
}

impl From<FormatArg> for DiskFormat {
    fn from(arg: FormatArg) -> DiskFormat {
        match arg {
            FormatArg::Ds720 => DiskFormat::Ds720,
            FormatArg::Ss360 => DiskFormat::Ss360,
            FormatArg::Ds360 => DiskFormat::Ds360,
            FormatArg::Ss180 => DiskFormat::Ss180,
        }
    }
}

/// `--dos` values, mapped onto [`DosVersion`].
#[derive(Clone, Copy, ValueEnum)]
pub enum DosArg {
    #[value(name = "1")]
    Dos1,
    #[value(name = "2")]
    Dos2,
}

impl From<DosArg> for DosVersion {
    fn from(arg: DosArg) -> DosVersion {
        match arg {
            DosArg::Dos1 => DosVersion::Dos1,
            DosArg::Dos2 => DosVersion::Dos2,
        }
    }
}
