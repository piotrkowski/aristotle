// SPDX-License-Identifier: AGPL-3.0-or-later
//! The reader's own fonts: TTF and OTF files in `fonts/` at the root of the
//! USB drive (`/mnt/us/fonts`, the folder the stock reader takes its custom
//! fonts from).
//!
//! - **Families.** Files are grouped by the family and style names FreeType
//!   reads from their name tables, the same names Plato matches when it
//!   loads a family. A style name the table gives but Plato's list does not
//!   know (as "Normal" or "Oblique") is read from the file name instead
//!   (`Family-Regular.ttf`, `Family-BoldItalic.otf`). Files FreeType cannot
//!   open, files over [`MAX_BYTES`] and families without a regular face
//!   are left out.
//! - **Lazily, cached.** The folder is read when the font list opens, not at
//!   start. The list is kept in `fonts.json` in the library data directory
//!   with the folder's mtime and file names, and read again only when they
//!   change.
//! - **A folder per family.** Plato loads a family by opening every font
//!   file in a directory and keeping the ones whose family name matches.
//!   Pointed at the user's folder it would open all of them for every book,
//!   on every thread that lays out. So the chosen family's faces (four at
//!   most) are copied to `reading-fonts/<name>-<stamp>/` in the library data
//!   directory and Plato is pointed there. FAT has no links, so a copy it
//!   is. The stamp hashes the sources' paths, sizes and mtimes: a changed
//!   font gets a new folder, and the folder's path is part of the style, so
//!   saved page numbers and snapshots of the old one are not used.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use aristotle_i18n::{tr, Msg};
use aristotle_render::plato_core::anyhow::Error;
use aristotle_render::plato_core::font::FontOpener;
use aristotle_ui::text::FamilyId;
use aristotle_ui::tree::Refresh;
use serde::{Deserialize, Serialize};

use crate::reading::Overlay;
use crate::{perf, App, Screen};

/// The built-in reading font.
pub const DEFAULT: &str = "Literata";
/// Where the stock reader and Aristotle look for the user's fonts.
pub const DEFAULT_DIR: &str = "/mnt/us/fonts";
/// Larger files are skipped: CJK superfamilies run to tens of megabytes and
/// would cost a long copy and memory for little use in this reader.
pub const MAX_BYTES: u64 = 20 * 1024 * 1024;
/// The family list, in the library data directory.
const LIST_FILE: &str = "fonts.json";
/// Copies of chosen families, in the library data directory.
pub const COPIES_DIR: &str = "reading-fonts";

/// Face slots, in the order Plato and the UI use: regular, italic, bold,
/// bold italic.
pub const SLOTS: [&str; 4] = ["Regular", "Italic", "Bold", "BoldItalic"];

/// Style names Plato's `FontFamily::from_name` takes for each slot, in its
/// order of preference.
const PLATO_STYLES: [&[&str]; 4] = [
    &["Regular", "Roman", "Book"],
    &["Italic", "Book Italic", "Regular Italic"],
    &["Bold", "Semibold", "SemiBold", "Medium"],
    &["Bold Italic", "SemiBold Italic", "Medium Italic"],
];

/// One font file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Face {
    pub path: PathBuf,
    /// The style name from the name table, as FreeType reports it.
    pub style: String,
    pub size: u64,
    pub mtime: i64,
}

/// A family with its faces by slot ([`SLOTS`]); the regular one is always
/// there.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Family {
    pub name: String,
    pub faces: [Option<Face>; 4],
}

impl Family {
    pub fn regular(&self) -> &Face {
        self.faces[0].as_ref().expect("a family has a regular face")
    }

    /// The faces copied for Plato. Plato needs a face named as in its list
    /// for each style it should use, and a regular one among them, unless
    /// the folder holds one file, which then serves every style. So a
    /// regular face with another name goes alone.
    fn for_plato(&self) -> Vec<(usize, &Face)> {
        let named = |slot: usize, f: &Face| PLATO_STYLES[slot].contains(&f.style.as_str());
        if !named(0, self.regular()) {
            return vec![(0, self.regular())];
        }
        self.faces
            .iter()
            .enumerate()
            .filter_map(|(slot, f)| f.as_ref().map(|f| (slot, f)))
            .filter(|(slot, f)| named(*slot, f))
            .collect()
    }

    /// A hash of the sources, for the copy's folder name.
    fn stamp(&self) -> u64 {
        let mut h = DefaultHasher::new();
        self.name.hash(&mut h);
        for (slot, f) in self.for_plato() {
            slot.hash(&mut h);
            f.path.hash(&mut h);
            f.size.hash(&mut h);
            f.mtime.hash(&mut h);
        }
        h.finish()
    }
}

/// What a scan learnt about one file.
#[derive(Clone, Debug)]
pub struct Scanned {
    pub path: PathBuf,
    pub size: u64,
    pub mtime: i64,
    pub family: String,
    pub style: String,
}

/// The slot a style name stands for: Plato's names first, then common
/// variants.
pub fn slot_of_style(style: &str) -> Option<usize> {
    if let Some(slot) = PLATO_STYLES.iter().position(|names| names.contains(&style)) {
        return Some(slot);
    }
    let s: String = style
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase();
    match s.as_str() {
        "regular" | "roman" | "book" | "normal" | "plain" | "text" | "" => Some(0),
        "italic" | "oblique" | "bookitalic" | "regularitalic" | "textitalic" => Some(1),
        "bold" | "semibold" | "demibold" | "medium" => Some(2),
        "bolditalic" | "boldoblique" | "semibolditalic" | "demibolditalic" | "mediumitalic" => {
            Some(3)
        }
        _ => None,
    }
}

/// The slot a file name gives, as in `Family-BoldItalic.ttf`.
pub fn slot_of_file_name(path: &Path) -> Option<usize> {
    let stem = path.file_stem()?.to_str()?;
    let (_, style) = stem.rsplit_once(['-', '_'])?;
    slot_of_style(style)
}

/// Group scanned files into families: by family name, each style to its
/// slot (from the style name, else the file name), the first of Plato's
/// names winning where two files claim a slot. A family keeps a regular
/// face, or is a single file of any style, which then serves as regular.
/// Sorted by name.
pub fn group(files: Vec<Scanned>) -> Vec<Family> {
    let mut by_name: std::collections::BTreeMap<String, Vec<Scanned>> = Default::default();
    for f in files {
        by_name.entry(f.family.clone()).or_default().push(f);
    }
    let mut out = Vec::new();
    for (name, mut files) in by_name {
        files.sort_by(|a, b| a.path.cmp(&b.path));
        let mut faces: [Option<(usize, Face)>; 4] = Default::default();
        let single = files.len() == 1;
        for f in files {
            let rank = |slot: usize| {
                PLATO_STYLES[slot]
                    .iter()
                    .position(|s| *s == f.style)
                    .unwrap_or(PLATO_STYLES[slot].len())
            };
            let slot = slot_of_style(&f.style)
                .or_else(|| slot_of_file_name(&f.path))
                .or(single.then_some(0));
            let Some(slot) = slot else { continue };
            let r = rank(slot);
            let face = Face {
                path: f.path,
                style: f.style,
                size: f.size,
                mtime: f.mtime,
            };
            match &faces[slot] {
                Some((best, _)) if *best <= r => {}
                _ => faces[slot] = Some((r, face)),
            }
        }
        let faces = faces.map(|f| f.map(|(_, f)| f));
        if faces[0].is_some() {
            out.push(Family { name, faces });
        }
    }
    out
}

/// Whether `path` looks like a font file.
fn is_font(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("ttf") || e.eq_ignore_ascii_case("otf"))
}

/// The font files in `dir` (not its subfolders), sorted, with their
/// metadata. Hidden files (macOS leaves `._Name.ttf` next to each) are
/// skipped.
fn list(dir: &Path) -> Vec<(PathBuf, std::fs::Metadata)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<_> = entries
        .flatten()
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .map(|e| e.path())
        .filter(|p| is_font(p))
        .filter_map(|p| {
            let m = std::fs::metadata(&p).ok()?;
            m.is_file().then_some((p, m))
        })
        .collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

/// Read every font file in `dir` with FreeType.
pub fn scan(dir: &Path) -> Vec<Family> {
    let Ok(opener) = FontOpener::new() else {
        return Vec::new();
    };
    let mut scanned = Vec::new();
    for (path, meta) in list(dir) {
        if meta.len() > MAX_BYTES || meta.len() == 0 {
            continue;
        }
        let Ok(font) = opener.open(&path) else {
            continue;
        };
        // Plato matches families by this name; a face without one could
        // not be loaded.
        let Some(family) = font.family_name().filter(|n| !n.trim().is_empty()) else {
            continue;
        };
        scanned.push(Scanned {
            family: family.to_string(),
            style: font.style_name().unwrap_or("Regular").to_string(),
            size: meta.len(),
            mtime: meta.mtime(),
            path,
        });
    }
    group(scanned)
}

/// The cached list: the folder's mtime and file names it was made from.
#[derive(Serialize, Deserialize)]
struct List {
    dir: PathBuf,
    key: u64,
    families: Vec<Family>,
}

/// What decides whether a cached list is still good: the folder's mtime
/// and its font files' names, sizes and mtimes (FAT keeps mtimes to 2 s,
/// and replacing a file need not touch the folder).
fn folder_key(dir: &Path) -> Option<u64> {
    let meta = std::fs::metadata(dir).ok()?;
    let mut h = DefaultHasher::new();
    (meta.mtime(), meta.mtime_nsec()).hash(&mut h);
    for (p, m) in list(dir) {
        (p, m.len(), m.mtime()).hash(&mut h);
    }
    Some(h.finish())
}

/// The families in `dir`, from the cache in `data_dir` when the folder has
/// not changed. An absent folder has none.
pub fn families(data_dir: &Path, dir: &Path) -> Vec<Family> {
    let Some(key) = folder_key(dir) else {
        return Vec::new();
    };
    let path = data_dir.join(LIST_FILE);
    if let Some(cached) = std::fs::read(&path)
        .ok()
        .and_then(|d| serde_json::from_slice::<List>(&d).ok())
        .filter(|l| l.dir == dir && l.key == key)
    {
        return cached.families;
    }
    let families = scan(dir);
    let list = List {
        dir: dir.to_path_buf(),
        key,
        families,
    };
    if let Ok(data) = serde_json::to_vec_pretty(&list) {
        if let Err(e) = aristotle_library::index::write_atomic(&path, &data) {
            eprintln!("aristotle: can't save the font list: {e}");
        }
    }
    list.families
}

/// A file-name-safe form of a family name.
fn slug(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    s.trim_matches('-').to_string()
}

/// The folder Plato loads `family` from: its copy in `data_dir`, made now if
/// missing. Older copies of the same family are removed.
pub fn prepare(data_dir: &Path, family: &Family) -> std::io::Result<PathBuf> {
    let root = data_dir.join(COPIES_DIR);
    let base = slug(&family.name);
    let name = format!("{base}-{:016x}", family.stamp());
    let dir = root.join(&name);
    if dir.is_dir() {
        return Ok(dir);
    }
    std::fs::create_dir_all(&root)?;
    // Copy into a fresh folder and rename it into place, so a copy cut
    // short by a crash or a full drive is never used.
    let part = root.join(format!("{name}.part"));
    if part.exists() {
        std::fs::remove_dir_all(&part)?;
    }
    std::fs::create_dir(&part)?;
    for (slot, f) in family.for_plato() {
        let ext = f.path.extension().and_then(|e| e.to_str()).unwrap_or("ttf");
        let to = part.join(format!(
            "{base}-{}.{}",
            SLOTS[slot],
            ext.to_ascii_lowercase()
        ));
        std::fs::copy(&f.path, &to)?;
    }
    std::fs::rename(&part, &dir)?;
    if let Ok(entries) = std::fs::read_dir(&root) {
        for e in entries.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            let stale = n != name
                && n.strip_prefix(&base)
                    .and_then(|r| r.strip_prefix('-'))
                    .is_some_and(|r| {
                        r.len() >= 16 && r[..16].bytes().all(|b| b.is_ascii_hexdigit())
                    });
            if stale {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }
    Ok(dir)
}

/// The folder for the family called `name` in `dir`, prepared; `None` if
/// it is not there (any more) or cannot be copied.
#[cfg(test)]
fn resolve(data_dir: &Path, dir: &Path, name: &str) -> Option<PathBuf> {
    let families = families(data_dir, dir);
    let family = families.iter().find(|f| f.name == name)?;
    match prepare(data_dir, family) {
        Ok(p) => Some(p),
        Err(e) => {
            eprintln!("aristotle: can't prepare the font {name}: {e}");
            None
        }
    }
}

/// A row of the font list.
pub struct FontRow {
    pub name: String,
    pub meta: Option<String>,
    /// The UI family the name is set in; `None` for the hint.
    pub ui: Option<FamilyId>,
    /// What picking the row chooses; `None` for the hint.
    pub pick: Option<Pick>,
}

pub enum Pick {
    Default,
    Family(Box<Family>),
}

/// Regular faces up to this size are read to show the family's name in its
/// own face; larger ones show it in the UI font.
const PREVIEW_BYTES: u64 = 2 * 1024 * 1024;

impl App {
    /// At start: find the folder of the reading font in the preferences.
    /// A family no longer there reads in Literata, and comes back when its
    /// files do.
    pub(crate) fn load_font(&mut self) {
        let t = std::time::Instant::now();
        self.prefs.font_dir = None;
        self.prefs.font_ui = 0;
        if self.prefs.font == DEFAULT {
            return;
        }
        let data = self.lib.data_dir().to_path_buf();
        let families = families(&data, &self.fonts_dir);
        let Some(family) = families.iter().find(|f| f.name == self.prefs.font) else {
            perf!(
                "fonts: {} is not in {}",
                self.prefs.font,
                self.fonts_dir.display()
            );
            return;
        };
        match prepare(&data, family) {
            Ok(dir) => {
                self.prefs.font_dir = Some(dir.to_string_lossy().into_owned());
                self.prefs.font_ui = self.ui_family(family).0 as usize;
            }
            Err(e) => eprintln!("aristotle: can't prepare the font {}: {e}", family.name),
        }
        perf!(
            "fonts: {} ready in {:.0} ms",
            self.prefs.font,
            crate::ms(t.elapsed())
        );
    }

    /// The UI family showing `family`'s name: its regular face, read once,
    /// or the UI font when that is large or unreadable.
    fn ui_family(&mut self, family: &Family) -> FamilyId {
        let regular = family.regular();
        if let Some(id) = self.font_faces.get(&regular.path) {
            return *id;
        }
        let id = (regular.size <= PREVIEW_BYTES)
            .then(|| std::fs::read(&regular.path).ok())
            .flatten()
            .and_then(|bytes| {
                let fonts = &mut self.ui.text.fonts;
                let face = fonts
                    .add_face(aristotle_ui::text::FontData::Shared(std::sync::Arc::new(
                        bytes,
                    )))
                    .ok()?;
                fonts.add_family(&family.name, [Some(face), None, None, None])
            })
            .unwrap_or(FamilyId(0));
        self.font_faces.insert(regular.path.clone(), id);
        id
    }

    /// The list of fonts, from the menu's font row: Literata, then the
    /// user's families. One full update.
    pub(crate) fn open_fonts(&mut self) -> Result<(), Error> {
        let t = std::time::Instant::now();
        let data = self.lib.data_dir().to_path_buf();
        let families: Vec<Family> = families(&data, &self.fonts_dir)
            .into_iter()
            .filter(|f| f.name != DEFAULT)
            .collect();
        let in_use = self.prefs.font_in_use().to_string();
        let mark = |name: &str, meta: Option<&str>| {
            let used = (name == in_use).then(|| tr(Msg::ReaderFontInUse));
            let parts: Vec<&str> = meta.into_iter().chain(used).collect();
            (!parts.is_empty()).then(|| parts.join(" · "))
        };
        let mut rows = vec![FontRow {
            name: DEFAULT.into(),
            meta: mark(DEFAULT, Some(tr(Msg::ReaderFontBuiltIn))),
            ui: Some(FamilyId(0)),
            pick: Some(Pick::Default),
        }];
        for f in &families {
            let ui = self.ui_family(f);
            rows.push(FontRow {
                name: f.name.clone(),
                meta: mark(&f.name, None),
                ui: Some(ui),
                pick: Some(Pick::Family(Box::new(f.clone()))),
            });
        }
        if families.is_empty() {
            rows.push(FontRow {
                name: tr(Msg::ReaderFontsHint).into(),
                meta: None,
                ui: None,
                pick: None,
            });
        }
        perf!(
            "fonts: {} families listed in {:.0} ms",
            families.len(),
            crate::ms(t.elapsed())
        );
        let Screen::Reader(r) = &mut self.screen else {
            return Ok(());
        };
        r.overlay = Overlay::Fonts { rows, page: 0 };
        r.menu_open = false;
        self.rebuild_reader(Refresh::Auto)?;
        self.present_full()
    }

    /// A row of the font list tapped: lay the book out in that font and show
    /// it under the menu, in one full update. The tapped row's outline is
    /// the acknowledgement meanwhile.
    pub(crate) fn pick_font(&mut self, i: usize) -> Result<(), Error> {
        let Screen::Reader(r) = &mut self.screen else {
            return Ok(());
        };
        let Overlay::Fonts { rows, .. } = &r.overlay else {
            return Ok(());
        };
        let Some(pick) = rows.get(i).and_then(|row| row.pick.as_ref()) else {
            return Ok(());
        };
        let data = self.lib.data_dir().to_path_buf();
        let (name, dir, ui) = match pick {
            Pick::Default => (DEFAULT.to_string(), None, FamilyId(0)),
            Pick::Family(f) => {
                let f = (**f).clone();
                match prepare(&data, &f) {
                    Ok(dir) => {
                        let ui = self.ui_family(&f);
                        (f.name, Some(dir.to_string_lossy().into_owned()), ui)
                    }
                    Err(e) => {
                        eprintln!("aristotle: can't prepare the font {}: {e}", f.name);
                        return self.fonts_back();
                    }
                }
            }
        };
        if name == self.prefs.font_in_use() {
            return self.fonts_back();
        }
        self.prefs.font = name;
        self.prefs.font_dir = dir;
        self.prefs.font_ui = ui.0 as usize;
        self.save_prefs();
        let Screen::Reader(r) = &mut self.screen else {
            return Ok(());
        };
        r.overlay = Overlay::None;
        r.nodes = Default::default();
        r.menu_open = true;
        r.gestures.reset();
        let mut settings = r.settings;
        settings.family = self.prefs.font_ui;
        self.relayout(settings)
    }

    /// "Back" from the font list: the menu over the page, in one full
    /// update.
    pub(crate) fn fonts_back(&mut self) -> Result<(), Error> {
        let Screen::Reader(r) = &mut self.screen else {
            return Ok(());
        };
        r.overlay = Overlay::None;
        r.menu_open = true;
        r.gestures.reset();
        self.rebuild_reader(Refresh::Auto)?;
        let Screen::Reader(r) = &mut self.screen else {
            return Ok(());
        };
        let page = r.session.page(r.session.current)?;
        crate::fit_menu(
            &mut self.ui,
            &self.styles,
            &r.session.title,
            &r.settings,
            &page,
            &mut r.menu,
            &mut r.page_node,
            &mut r.nav,
        );
        crate::update_menu_labels(&mut self.ui, r);
        self.present_full()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ASSETS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../aristotle-ui/assets/fonts");

    fn scanned(path: &str, family: &str, style: &str) -> Scanned {
        Scanned {
            path: PathBuf::from(path),
            size: 1,
            mtime: 0,
            family: family.into(),
            style: style.into(),
        }
    }

    fn slots(f: &Family) -> Vec<Option<&str>> {
        f.faces
            .iter()
            .map(|f| f.as_ref().map(|f| f.path.to_str().unwrap()))
            .collect()
    }

    #[test]
    fn styles_and_file_names_give_slots() {
        assert_eq!(slot_of_style("Regular"), Some(0));
        assert_eq!(slot_of_style("Bold Italic"), Some(3));
        assert_eq!(slot_of_style("Oblique"), Some(1));
        assert_eq!(slot_of_style("Light"), None);
        assert_eq!(
            slot_of_file_name(Path::new("a/Foo-BoldItalic.otf")),
            Some(3)
        );
        assert_eq!(slot_of_file_name(Path::new("Foo_Italic.ttf")), Some(1));
        assert_eq!(slot_of_file_name(Path::new("Foo.ttf")), None);
    }

    #[test]
    fn grouping_by_name_table() {
        let fams = group(vec![
            scanned("x/b.ttf", "Foo", "Bold"),
            scanned("x/a.ttf", "Foo", "Regular"),
            scanned("x/m.ttf", "Foo", "Medium"),
            scanned("x/i.ttf", "Foo", "Italic"),
            scanned("x/l.ttf", "Foo", "Light"),
            // A style name only the file name explains.
            scanned("x/Foo-BoldItalic.ttf", "Foo", "Heavy Slanted"),
            // No regular face among several: left out.
            scanned("x/bar-i.ttf", "Bar", "Italic"),
            scanned("x/bar-b.ttf", "Bar", "Bold"),
            // A single file of any style serves as regular.
            scanned("x/one.ttf", "Solo", "Light"),
        ]);
        let names: Vec<_> = fams.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["Foo", "Solo"]);
        assert_eq!(
            slots(&fams[0]),
            [
                Some("x/a.ttf"),
                Some("x/i.ttf"),
                Some("x/b.ttf"),
                Some("x/Foo-BoldItalic.ttf")
            ]
        );
        // Plato would not take "Heavy Slanted" as bold italic: not copied.
        let plato: Vec<usize> = fams[0].for_plato().iter().map(|(s, _)| *s).collect();
        assert_eq!(plato, [0, 1, 2]);
        // A regular Plato would not find goes alone.
        let normal = group(vec![
            scanned("y/n.ttf", "Baz", "Normal"),
            scanned("y/i.ttf", "Baz", "Italic"),
        ]);
        assert_eq!(normal[0].for_plato().len(), 1);
    }

    /// The OFL fonts in the repository, copied under other names, a broken
    /// file, an italic without its regular and a file that is too large.
    #[test]
    fn scanning_a_folder_of_real_fonts() {
        let dir = crate::testdir("fonts-scan");
        let user = dir.join("fonts");
        std::fs::create_dir_all(&user).unwrap();
        let assets = Path::new(ASSETS);
        for (from, to) in [
            ("Charis-Regular.ttf", "c1.ttf"),
            ("Charis-Italic.ttf", "c2.TTF"),
            ("GentiumBook-Italic.ttf", "gentium.otf"),
            ("Literata-Regular.ttf", "lit.ttf"),
            ("Literata-Bold.ttf", "lit-b.ttf"),
            ("Literata-BoldItalic.ttf", "lit-bi.ttf"),
        ] {
            std::fs::copy(assets.join(from), user.join(to)).unwrap();
        }
        std::fs::write(user.join("broken.ttf"), b"not a font").unwrap();
        std::fs::write(user.join("notes.txt"), b"x").unwrap();
        std::fs::copy(assets.join("Charis-Regular.ttf"), user.join("._c1.ttf")).unwrap();
        let big = std::fs::File::create(user.join("huge.ttf")).unwrap();
        big.set_len(MAX_BYTES + 1).unwrap();

        let fams = families(&dir, &user);
        let names: Vec<_> = fams.iter().map(|f| f.name.as_str()).collect();
        // Gentium Book has only its italic here.
        assert_eq!(names, ["Charis", "Literata"]);
        let charis = &fams[0];
        assert!(charis.faces[0].as_ref().unwrap().path.ends_with("c1.ttf"));
        assert!(charis.faces[1].as_ref().unwrap().path.ends_with("c2.TTF"));
        assert!(charis.faces[2].is_none());
        let lit = &fams[1];
        assert!(lit.faces[1].is_none() && lit.faces[2].is_some() && lit.faces[3].is_some());

        // The list is cached until the folder changes.
        assert!(dir.join(LIST_FILE).exists());
        assert_eq!(families(&dir, &user), fams);
        std::fs::copy(
            assets.join("GentiumBook-Regular.ttf"),
            user.join("gentium-r.ttf"),
        )
        .unwrap();
        let names: Vec<_> = families(&dir, &user).into_iter().map(|f| f.name).collect();
        assert_eq!(names, ["Charis", "Gentium Book", "Literata"]);
        assert!(families(&dir, &dir.join("absent")).is_empty());
    }

    /// The copy holds the faces under fixed names; Plato loads the family
    /// from it with its italic; a changed source gets a new folder and the
    /// old one goes.
    #[test]
    fn prepared_folder_loads_in_plato() {
        use aristotle_render::plato_core::font::FontFamily;
        let dir = crate::testdir("fonts-prepare");
        let user = dir.join("fonts");
        std::fs::create_dir_all(&user).unwrap();
        let assets = Path::new(ASSETS);
        std::fs::copy(assets.join("Charis-Regular.ttf"), user.join("one.ttf")).unwrap();
        std::fs::copy(assets.join("Charis-Italic.ttf"), user.join("two.ttf")).unwrap();
        let p = resolve(&dir, &user, "Charis").unwrap();
        let mut names: Vec<_> = std::fs::read_dir(&p)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(names, ["Charis-Italic.ttf", "Charis-Regular.ttf"]);
        let fam = FontFamily::from_name("Charis", &p).expect("Plato loads the copy");
        assert_eq!(fam.italic.style_name(), Some("Italic"));
        assert_eq!(resolve(&dir, &user, "Charis").unwrap(), p, "made once");
        assert!(resolve(&dir, &user, "Nothing").is_none());

        // Replace the italic: a new folder, the old one removed.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::copy(assets.join("Charis-Regular.ttf"), user.join("two.ttf")).unwrap();
        let q = resolve(&dir, &user, "Charis").unwrap();
        assert_ne!(p, q);
        assert!(!p.exists());
    }
}
