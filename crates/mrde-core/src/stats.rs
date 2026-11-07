//! Change statistics and the "disktree" treemap layout. Pure functions, no I/O.
use crate::models::{ChangedFile, FileStat};

/// Line counts from GitLab's diff. `None` when GitLab truncated any file, because the
/// estimate would silently under-count; the Statistics action computes exact numbers.
pub fn gitlab_stats(files: &[ChangedFile]) -> Option<Vec<FileStat>> {
    if files.iter().any(|f| f.too_large || f.collapsed) {
        return None;
    }
    Some(files.iter().map(file_stat).collect())
}

/// Only lines inside hunks count, so removed content starting with `--` is not mistaken
/// for a file header.
pub fn file_stat(file: &ChangedFile) -> FileStat {
    let (mut added, mut removed, mut in_hunk) = (0, 0, false);
    for line in file.diff.lines() {
        if line.starts_with("@@") {
            in_hunk = true;
        } else if in_hunk && line.starts_with('+') {
            added += 1;
        } else if in_hunk && line.starts_with('-') {
            removed += 1;
        }
    }
    FileStat {
        path: file.new_path.clone(),
        added,
        removed,
        binary: false,
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Zone {
    /// Directory prefix shared by the zone's files, or `(root)`.
    pub name: String,
    pub changed: u64,
    pub files: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tile {
    /// The file drawn by this tile, or `None` for a zone's grouped smaller files.
    pub file: Option<usize>,
    /// Number of files represented: 1, or the size of the group.
    pub files: usize,
    pub changed: u64,
    pub zone: usize,
    pub rect: Rect,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Treemap {
    pub zones: Vec<Zone>,
    pub zone_rects: Vec<Rect>,
    pub tiles: Vec<Tile>,
}

/// At most one color per zone in the disktree palette.
const MAX_ZONES: usize = 8;

fn directories(path: &str) -> Vec<&str> {
    let mut parts: Vec<&str> = path.split('/').collect();
    parts.pop();
    parts
}

/// Splits the most changed zone into its subdirectories, repeatedly, while the map stays
/// within eight zones. Files directly inside a split directory keep their own zone.
pub fn zones(files: &[FileStat]) -> Vec<Zone> {
    struct Group {
        depth: usize,
        prefix: Vec<String>,
        files: Vec<usize>,
        changed: u64,
    }
    let dirs: Vec<Vec<&str>> = files.iter().map(|f| directories(&f.path)).collect();
    let mut groups = vec![Group {
        depth: 0,
        prefix: vec![],
        files: (0..files.len()).collect(),
        changed: files.iter().map(FileStat::changed).sum(),
    }];
    loop {
        let mut order: Vec<usize> = (0..groups.len()).collect();
        order.sort_by(|&a, &b| groups[b].changed.cmp(&groups[a].changed));
        let mut split = None;
        for index in order {
            let group = &groups[index];
            let mut children: Vec<Group> = Vec::new();
            for &file in &group.files {
                let (prefix, depth) = match dirs[file].get(group.depth) {
                    Some(part) => {
                        let mut prefix = group.prefix.clone();
                        prefix.push(part.to_string());
                        (prefix, group.depth + 1)
                    }
                    // Already at the file's own directory: it cannot go deeper.
                    None => (group.prefix.clone(), usize::MAX),
                };
                match children
                    .iter_mut()
                    .find(|c| c.prefix == prefix && c.depth == depth)
                {
                    Some(child) => {
                        child.files.push(file);
                        child.changed += files[file].changed();
                    }
                    None => children.push(Group {
                        depth,
                        prefix,
                        files: vec![file],
                        changed: files[file].changed(),
                    }),
                }
            }
            let splittable = group.depth != usize::MAX;
            // A single child is a shared prefix: descend without counting a new zone.
            if splittable && children.len() == 1 && children[0].depth != usize::MAX {
                split = Some((index, children));
                break;
            }
            if splittable && children.len() > 1 && groups.len() - 1 + children.len() <= MAX_ZONES {
                split = Some((index, children));
                break;
            }
        }
        let Some((index, children)) = split else {
            break;
        };
        groups.splice(index..=index, children);
    }
    let mut zones: Vec<Zone> = groups
        .into_iter()
        .map(|group| Zone {
            name: if group.prefix.is_empty() {
                "(root)".into()
            } else {
                group.prefix.join("/")
            },
            changed: group.changed,
            files: group.files,
        })
        .collect();
    zones.sort_by(|a, b| b.changed.cmp(&a.changed).then_with(|| a.name.cmp(&b.name)));
    zones
}

/// Every file gets a minimum weight so binary and rename-only files stay visible.
fn weight(file: &FileStat) -> f64 {
    file.changed().max(1) as f64
}

/// Two-level squarified treemap: zones first, then files inside each zone. At most
/// `max_tiles` files get their own tile; each zone's remaining files share one tile, so
/// large MRs stay readable and cheap to draw.
pub fn treemap(files: &[FileStat], width: f64, height: f64, max_tiles: usize) -> Treemap {
    let zones = zones(files);
    let bounds = Rect {
        x: 0.,
        y: 0.,
        w: width,
        h: height,
    };
    let zone_weights: Vec<f64> = zones
        .iter()
        .map(|zone| zone.files.iter().map(|&i| weight(&files[i])).sum())
        .collect();
    let zone_rects = squarify(&zone_weights, bounds);
    let mut ranked: Vec<usize> = (0..files.len()).collect();
    ranked.sort_by(|&a, &b| weight(&files[b]).total_cmp(&weight(&files[a])));
    let mut individual = vec![false; files.len()];
    for &index in ranked
        .iter()
        .take(max_tiles.saturating_sub(zones.len()).max(1))
    {
        individual[index] = true;
    }
    let mut tiles = Vec::new();
    for (zone_index, (zone, rect)) in zones.iter().zip(&zone_rects).enumerate() {
        let mut members: Vec<Tile> = Vec::new();
        let (mut grouped, mut grouped_changed, mut grouped_weight) = (0, 0, 0.);
        for &file in &zone.files {
            if individual[file] {
                members.push(Tile {
                    file: Some(file),
                    files: 1,
                    changed: files[file].changed(),
                    zone: zone_index,
                    rect: Rect::default(),
                });
            } else {
                grouped += 1;
                grouped_changed += files[file].changed();
                grouped_weight += weight(&files[file]);
            }
        }
        let mut weights: Vec<f64> = members
            .iter()
            .map(|t| weight(&files[t.file.unwrap_or_default()]))
            .collect();
        if grouped > 0 {
            members.push(Tile {
                file: None,
                files: grouped,
                changed: grouped_changed,
                zone: zone_index,
                rect: Rect::default(),
            });
            weights.push(grouped_weight);
        }
        for (mut tile, rect) in members.into_iter().zip(squarify(&weights, *rect)) {
            tile.rect = rect;
            tiles.push(tile);
        }
    }
    Treemap {
        zones,
        zone_rects,
        tiles,
    }
}

/// Worst aspect ratio of a row laid along `side` (Bruls, Huizing, van Wijk).
fn worst(row: &[f64], side: f64) -> f64 {
    let sum: f64 = row.iter().sum();
    let max = row.iter().cloned().fold(f64::MIN, f64::max);
    let min = row.iter().cloned().fold(f64::MAX, f64::min);
    let (sum2, side2) = (sum * sum, side * side);
    (side2 * max / sum2).max(sum2 / (side2 * min))
}

/// Rectangles in input order whose areas are proportional to `values`.
pub fn squarify(values: &[f64], bounds: Rect) -> Vec<Rect> {
    let mut out = vec![Rect::default(); values.len()];
    let total: f64 = values.iter().filter(|v| **v > 0.).sum();
    if total <= 0. || bounds.w <= 0. || bounds.h <= 0. {
        return out;
    }
    let mut order: Vec<usize> = (0..values.len()).filter(|&i| values[i] > 0.).collect();
    order.sort_by(|&a, &b| values[b].total_cmp(&values[a]));
    let scale = bounds.w * bounds.h / total;
    let areas: Vec<f64> = order.iter().map(|&i| values[i] * scale).collect();
    let mut free = bounds;
    let mut start = 0;
    while start < areas.len() {
        let side = free.w.min(free.h).max(f64::EPSILON);
        let mut end = start + 1;
        let mut best = worst(&areas[start..end], side);
        while end < areas.len() {
            let next = worst(&areas[start..=end], side);
            if next > best {
                break;
            }
            best = next;
            end += 1;
        }
        let row: f64 = areas[start..end].iter().sum();
        if free.w >= free.h {
            let width = (row / free.h.max(f64::EPSILON)).min(free.w);
            let mut y = free.y;
            for k in start..end {
                let h = areas[k] / width.max(f64::EPSILON);
                out[order[k]] = Rect {
                    x: free.x,
                    y,
                    w: width,
                    h,
                };
                y += h;
            }
            free.x += width;
            free.w = (free.w - width).max(0.);
        } else {
            let height = (row / free.w.max(f64::EPSILON)).min(free.h);
            let mut x = free.x;
            for k in start..end {
                let w = areas[k] / height.max(f64::EPSILON);
                out[order[k]] = Rect {
                    x,
                    y: free.y,
                    w,
                    h: height,
                };
                x += w;
            }
            free.y += height;
            free.h = (free.h - height).max(0.);
        }
        start = end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    fn stat(path: &str, added: u64, removed: u64) -> FileStat {
        FileStat {
            path: path.into(),
            added,
            removed,
            binary: false,
        }
    }
    #[test]
    fn gitlab_stats_count_hunk_lines_and_refuse_truncated_diffs() {
        let file = |diff: &str, too_large| ChangedFile {
            old_path: "a.sql".into(),
            new_path: "a.sql".into(),
            diff: diff.into(),
            too_large,
            collapsed: false,
        };
        let stats = gitlab_stats(&[file("@@ -1,2 +1,1 @@\n--- old\n+++ new\n ctx\n", false)]);
        assert_eq!(stats.unwrap()[0], stat("a.sql", 1, 1));
        assert!(gitlab_stats(&[file("", true)]).is_none());
    }
    #[test]
    fn zones_split_the_most_changed_directories_first() {
        let files = [
            stat("backend/api/a.go", 10, 0),
            stat("backend/api/b.go", 5, 5),
            stat("backend/db/m.sql", 1, 0),
            stat("README.md", 1, 0),
        ];
        let names: Vec<_> = zones(&files).into_iter().map(|z| z.name).collect();
        assert_eq!(names, ["backend/api", "(root)", "backend/db"]);
        let nested = [stat("src/ui/a.rs", 3, 0), stat("src/core/b.rs", 9, 0)];
        let zones = zones(&nested);
        assert_eq!(zones[0].name, "src/core");
        assert_eq!(zones[0].changed, 9);
    }
    #[test]
    fn zones_stay_within_the_palette_and_skip_shared_prefixes() {
        let mut files = Vec::new();
        for area in [
            "domain",
            "application",
            "api",
            "migrations",
            "notifications",
            "cli",
            "server",
            "docs",
            "scripts",
            "bin",
        ] {
            for i in 0..3 {
                files.push(stat(&format!("backend/src/{area}/f{i}.go"), 10, 0));
            }
        }
        files.push(stat("web/src/features/schedules/ScheduleDialog.tsx", 50, 0));
        let zones = zones(&files);
        assert!(zones.len() <= 8, "{} zones", zones.len());
        assert!(
            zones
                .iter()
                .all(|z| z.name.starts_with("backend/src") || z.name.starts_with("web"))
        );
        assert_eq!(
            zones.iter().map(|z| z.files.len()).sum::<usize>(),
            files.len()
        );
    }
    #[test]
    fn treemap_tiles_cover_the_area_proportionally_without_overlap() {
        let files = [
            stat("a/x.rs", 60, 0),
            stat("a/y.rs", 20, 0),
            stat("b/z.rs", 15, 5),
            stat("b/bin.png", 0, 0),
        ];
        let map = treemap(&files, 3., 1., 100);
        assert_eq!(map.tiles.len(), 4);
        let total: f64 = map.tiles.iter().map(|t| t.rect.w * t.rect.h).sum();
        assert!((total - 3.).abs() < 1e-9);
        let x = map.tiles.iter().find(|t| t.file == Some(0)).unwrap().rect;
        assert!((x.w * x.h - 3. * 60. / 101.).abs() < 1e-9);
        for tile in &map.tiles {
            let r = tile.rect;
            assert!(
                r.x >= -1e-9 && r.y >= -1e-9 && r.x + r.w <= 3. + 1e-9 && r.y + r.h <= 1. + 1e-9
            );
        }
        for (i, a) in map.tiles.iter().enumerate() {
            for b in &map.tiles[i + 1..] {
                let overlap_w =
                    (a.rect.x + a.rect.w).min(b.rect.x + b.rect.w) - a.rect.x.max(b.rect.x);
                let overlap_h =
                    (a.rect.y + a.rect.h).min(b.rect.y + b.rect.h) - a.rect.y.max(b.rect.y);
                assert!(overlap_w <= 1e-9 || overlap_h <= 1e-9, "tiles overlap");
            }
        }
    }
    #[test]
    fn treemap_groups_smaller_files_per_zone_beyond_the_tile_limit() {
        let mut files: Vec<FileStat> = (0..50)
            .map(|i| stat(&format!("a/f{i}.rs"), 100 - i, 0))
            .collect();
        files.extend((0..50).map(|i| stat(&format!("b/g{i}.rs"), 10, 0)));
        let map = treemap(&files, 3., 1., 12);
        assert!(map.tiles.len() <= 12);
        let grouped: Vec<_> = map.tiles.iter().filter(|t| t.file.is_none()).collect();
        assert_eq!(
            grouped.iter().map(|t| t.files).sum::<usize>() + map.tiles.len() - grouped.len(),
            100
        );
        assert!(
            map.tiles.iter().any(|t| t.file == Some(0)),
            "largest file keeps its tile"
        );
        let area: f64 = map.tiles.iter().map(|t| t.rect.w * t.rect.h).sum();
        assert!((area - 3.).abs() < 1e-9);
    }
    #[test]
    fn squarify_handles_empty_and_zero_inputs() {
        let bounds = Rect {
            x: 0.,
            y: 0.,
            w: 1.,
            h: 1.,
        };
        assert!(squarify(&[], bounds).is_empty());
        assert_eq!(squarify(&[0., 0.], bounds), vec![Rect::default(); 2]);
    }
}
