//! FAT-chain integrity analysis and fragmentation measurement.
//!
//! Builds a per-cluster reference count by walking every file's and
//! subdirectory's cluster chain from the root down (mirroring
//! [`crate::fs::Volume`]'s directory traversal), then derives:
//! - **lost** clusters: allocated in the FAT but referenced by no chain,
//! - **cross-linked** clusters: referenced by two or more chains,
//! - **bad pointers**: next-pointers outside the valid cluster range,
//! - **fragmentation**: the share of files whose chain is non-contiguous.
//!
//! Pure over the already-parsed `(buf, &Bpb, FatType)`, so it serves both the
//! floppy and hard-disk-partition paths.

use std::collections::HashSet;

use super::{BadPointer, CrossLink, FatIntegrity};
use crate::fs::map::{self, Bpb, DirLocation, FatType, Pointer};
use crate::image::geometry::SECTOR_SIZE;

/// Largest directory nesting depth followed; guards pathological structures.
const MAX_DEPTH: usize = 64;

/// Analyze the FAT of a volume with `cluster_count` data clusters, returning the
/// integrity findings and the fragmentation percentage (`0.0..=100.0`).
pub(crate) fn analyze(
    buf: &[u8],
    bpb: &Bpb,
    fat_type: FatType,
    cluster_count: usize,
) -> (FatIntegrity, f64) {
    let mut refs = vec![0u32; cluster_count + 2];
    let mut bad_pointers = Vec::new();
    let mut file_count = 0usize;
    let mut fragmented = 0usize;

    let mut visited_dirs = Vec::new();
    walk_dir(
        buf,
        bpb,
        fat_type,
        &DirLocation::Root,
        cluster_count,
        0,
        &mut refs,
        &mut bad_pointers,
        &mut file_count,
        &mut fragmented,
        &mut visited_dirs,
    );

    let fat_start_byte = bpb.fat_start() * SECTOR_SIZE;
    let mut lost_clusters = Vec::new();
    let mut cross_linked = Vec::new();
    for (c, &count) in refs.iter().enumerate().skip(2).take(cluster_count) {
        if count >= 2 {
            cross_linked.push(CrossLink {
                cluster: c as u16,
                references: count as usize,
            });
        }
        if count == 0 {
            // Allocated-but-unreferenced clusters are lost. Free, bad-marked, and
            // reserved clusters are intentional, not lost.
            let entry = map::fat_entry(buf, fat_start_byte, fat_type, c);
            if !matches!(
                map::classify_pointer(fat_type, entry, cluster_count),
                Pointer::Free | Pointer::Bad | Pointer::Reserved
            ) {
                lost_clusters.push(c as u16);
            }
        }
    }

    let fragmentation_pct = if file_count == 0 {
        0.0
    } else {
        100.0 * fragmented as f64 / file_count as f64
    };

    (
        FatIntegrity {
            lost_clusters,
            cross_linked,
            bad_pointers,
        },
        fragmentation_pct,
    )
}

/// Visit every entry under `dir`, counting file and subdirectory chains, then
/// recurse into subdirectories. Subdirectories are collected first so the
/// directory-walk closure does not hold the mutable accumulators across the
/// recursive call.
#[allow(clippy::too_many_arguments)]
fn walk_dir(
    buf: &[u8],
    bpb: &Bpb,
    fat_type: FatType,
    dir: &DirLocation,
    cluster_count: usize,
    depth: usize,
    refs: &mut [u32],
    bad_pointers: &mut Vec<BadPointer>,
    file_count: &mut usize,
    fragmented: &mut usize,
    visited_dirs: &mut Vec<u16>,
) {
    if depth >= MAX_DEPTH {
        return;
    }
    let mut subdirs: Vec<u16> = Vec::new();
    map::for_each_entry(buf, bpb, fat_type, dir, |entry| {
        let name = map::entry_name(entry);
        if name == "." || name == ".." {
            return false;
        }
        let first = u16::from_le_bytes([entry[26], entry[27]]);
        let is_dir = entry[11] & 0x10 != 0;
        let chain = mark_chain(buf, bpb, fat_type, first, cluster_count, refs, bad_pointers);
        if is_dir {
            subdirs.push(first);
        } else {
            *file_count += 1;
            if is_fragmented(&chain) {
                *fragmented += 1;
            }
        }
        false
    });

    for first in subdirs {
        if !visited_dirs.contains(&first) {
            visited_dirs.push(first);
            walk_dir(
                buf,
                bpb,
                fat_type,
                &DirLocation::Cluster(first),
                cluster_count,
                depth + 1,
                refs,
                bad_pointers,
                file_count,
                fragmented,
                visited_dirs,
            );
            visited_dirs.pop();
        }
    }
}

/// Follow the chain starting at `first`, incrementing `refs[c]` for each visited
/// cluster and recording any out-of-range next-pointer. Returns the cluster
/// numbers in order. A self-referential cycle terminates at the repeat.
fn mark_chain(
    buf: &[u8],
    bpb: &Bpb,
    fat_type: FatType,
    first: u16,
    cluster_count: usize,
    refs: &mut [u32],
    bad_pointers: &mut Vec<BadPointer>,
) -> Vec<u16> {
    let fat_start_byte = bpb.fat_start() * SECTOR_SIZE;
    let mut chain: Vec<u16> = Vec::new();
    let mut seen: HashSet<u16> = HashSet::new();
    let mut cluster = first as usize;
    while cluster >= 2 && cluster < refs.len() {
        if !seen.insert(cluster as u16) {
            break; // cyclic chain (O(1) revisit check)
        }
        let entry = map::fat_entry(buf, fat_start_byte, fat_type, cluster);
        match map::classify_pointer(fat_type, entry, cluster_count) {
            Pointer::Next(next) => {
                refs[cluster] += 1;
                chain.push(cluster as u16);
                cluster = next as usize;
            }
            Pointer::End => {
                refs[cluster] += 1;
                chain.push(cluster as u16);
                break;
            }
            Pointer::OutOfRange(next) => {
                refs[cluster] += 1;
                chain.push(cluster as u16);
                bad_pointers.push(BadPointer {
                    from_cluster: cluster as u16,
                    points_to: next,
                });
                break;
            }
            // A free, bad-marked, or reserved entry where a chain should
            // continue: stop.
            Pointer::Free | Pointer::Bad | Pointer::Reserved => break,
        }
    }
    chain
}

/// Whether a cluster chain has any non-contiguous step.
fn is_fragmented(chain: &[u16]) -> bool {
    chain.windows(2).any(|w| w[1] != w[0] + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal hand-laid FAT16 volume for integrity tests: 8000 sectors,
    /// spf=64 (forces FAT16), spc=1, 2 FATs, root at a fixed region.
    struct Builder {
        data: Vec<u8>,
        spf: usize,
        root_entries: usize,
        num_fats: usize,
        reserved: usize,
        spc: usize,
    }

    impl Builder {
        fn new() -> Builder {
            let mut b = Builder {
                data: vec![0u8; 8000 * SECTOR_SIZE],
                spf: 64,
                root_entries: 512,
                num_fats: 2,
                reserved: 1,
                spc: 1,
            };
            let d = &mut b.data;
            d[0] = 0xEB;
            d[11..13].copy_from_slice(&(SECTOR_SIZE as u16).to_le_bytes());
            d[13] = b.spc as u8;
            d[14..16].copy_from_slice(&(b.reserved as u16).to_le_bytes());
            d[16] = b.num_fats as u8;
            d[17..19].copy_from_slice(&(b.root_entries as u16).to_le_bytes());
            d[22..24].copy_from_slice(&(b.spf as u16).to_le_bytes());
            d[510] = 0x55;
            d[511] = 0xAA;
            b
        }

        fn root_start(&self) -> usize {
            self.reserved + self.num_fats * self.spf
        }

        fn set_fat16(&mut self, cluster: usize, value: u16) {
            for fat in 0..self.num_fats {
                let off = (self.reserved + fat * self.spf) * SECTOR_SIZE + cluster * 2;
                self.data[off..off + 2].copy_from_slice(&value.to_le_bytes());
            }
        }

        fn write_dir_entry(
            &mut self,
            slot: usize,
            name: &[u8; 11],
            attr: u8,
            first: u16,
            size: u32,
        ) {
            let off = self.root_start() * SECTOR_SIZE + slot * 32;
            self.data[off..off + 11].copy_from_slice(name);
            self.data[off + 11] = attr;
            self.data[off + 26..off + 28].copy_from_slice(&first.to_le_bytes());
            self.data[off + 28..off + 32].copy_from_slice(&size.to_le_bytes());
        }

        fn mount(self) -> (Vec<u8>, Bpb, FatType, usize) {
            let bpb = Bpb::parse(&self.data).expect("bpb");
            let total = self.data.len() / SECTOR_SIZE;
            let fat_type = bpb.fat_type(total);
            let cluster_count = bpb.cluster_count(total);
            (self.data, bpb, fat_type, cluster_count)
        }
    }

    #[test]
    fn clean_volume_has_no_findings() {
        let mut b = Builder::new();
        // FILE.TXT -> cluster 2 (single cluster, EOC).
        b.write_dir_entry(0, b"FILE    TXT", 0x20, 2, 100);
        b.set_fat16(2, 0xFFFF);
        let (buf, bpb, ft, cc) = b.mount();
        let (integ, frag) = analyze(&buf, &bpb, ft, cc);
        assert!(integ.is_clean());
        assert_eq!(frag, 0.0);
    }

    #[test]
    fn detects_lost_cluster() {
        let mut b = Builder::new();
        b.write_dir_entry(0, b"FILE    TXT", 0x20, 2, 100);
        b.set_fat16(2, 0xFFFF);
        // Cluster 7 is allocated (EOC) but no directory entry references it.
        b.set_fat16(7, 0xFFFF);
        let (buf, bpb, ft, cc) = b.mount();
        let (integ, _) = analyze(&buf, &bpb, ft, cc);
        assert_eq!(integ.lost_clusters, vec![7]);
        assert!(integ.cross_linked.is_empty());
    }

    #[test]
    fn reserved_cluster_value_is_not_lost() {
        let mut b = Builder::new();
        b.write_dir_entry(0, b"FILE    TXT", 0x20, 2, 100);
        b.set_fat16(2, 0xFFFF);
        // Cluster 6 holds a reserved FAT16 value (0xFFF0..=0xFFF6) and is
        // referenced by no directory entry: intentional, not lost.
        b.set_fat16(6, 0xFFF0);
        let (buf, bpb, ft, cc) = b.mount();
        let (integ, _) = analyze(&buf, &bpb, ft, cc);
        assert!(
            integ.lost_clusters.is_empty(),
            "reserved value must not be reported as lost: {:?}",
            integ.lost_clusters
        );
    }

    #[test]
    fn detects_cross_linked_cluster() {
        let mut b = Builder::new();
        // Two files whose chains both pass through cluster 4.
        // A: 2 -> 4 -> EOC ; B: 3 -> 4 -> EOC.
        b.write_dir_entry(0, b"A       TXT", 0x20, 2, 600);
        b.write_dir_entry(1, b"B       TXT", 0x20, 3, 600);
        b.set_fat16(2, 4);
        b.set_fat16(3, 4);
        b.set_fat16(4, 0xFFFF);
        let (buf, bpb, ft, cc) = b.mount();
        let (integ, _) = analyze(&buf, &bpb, ft, cc);
        assert!(integ
            .cross_linked
            .iter()
            .any(|x| x.cluster == 4 && x.references == 2));
    }

    #[test]
    fn detects_bad_pointer() {
        let mut b = Builder::new();
        // FILE -> 2 -> 9000 (out of range; volume has ~7900 clusters).
        b.write_dir_entry(0, b"FILE    TXT", 0x20, 2, 600);
        b.set_fat16(2, 9000);
        let (buf, bpb, ft, cc) = b.mount();
        let (integ, _) = analyze(&buf, &bpb, ft, cc);
        assert!(integ
            .bad_pointers
            .iter()
            .any(|p| p.from_cluster == 2 && p.points_to == 9000));
    }

    #[test]
    fn measures_fragmentation() {
        let mut b = Builder::new();
        // A is contiguous (2 -> 3 -> EOC); B is fragmented (5 -> 8 -> EOC).
        b.write_dir_entry(0, b"A       TXT", 0x20, 2, 600);
        b.set_fat16(2, 3);
        b.set_fat16(3, 0xFFFF);
        b.write_dir_entry(1, b"B       TXT", 0x20, 5, 600);
        b.set_fat16(5, 8);
        b.set_fat16(8, 0xFFFF);
        let (buf, bpb, ft, cc) = b.mount();
        let (_, frag) = analyze(&buf, &bpb, ft, cc);
        assert_eq!(frag, 50.0); // 1 of 2 files fragmented
    }

    #[test]
    fn cyclic_chain_terminates() {
        let mut b = Builder::new();
        b.write_dir_entry(0, b"LOOP    TXT", 0x20, 2, 600);
        // 2 -> 3 -> 2 (cycle).
        b.set_fat16(2, 3);
        b.set_fat16(3, 2);
        let (buf, bpb, ft, cc) = b.mount();
        let (integ, _) = analyze(&buf, &bpb, ft, cc); // must not hang
                                                      // Both clusters referenced exactly once, so neither is cross-linked.
        assert!(integ.cross_linked.is_empty());
    }
}
