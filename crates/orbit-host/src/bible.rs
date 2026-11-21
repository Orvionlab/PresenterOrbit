use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

const PACKAGE_SCHEMA: &str = "presenter-orbit.bible-package";
const PACKAGE_VERSION: u32 = 1;
const MAX_PACKAGE_BYTES: u64 = 20 * 1024 * 1024;
const MAX_ARCHIVE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_TRANSLATION_BYTES: u64 = 16 * 1024 * 1024;
const SEARCH_LIMIT: u32 = 100;
const MAX_PARALLEL_TRANSLATIONS: usize = 8;
const WEBP_VPL_URL: &str = "https://ebible.org/Scriptures/engwebp_vpl.zip";
const WEBP_SOURCE_URL: &str = "https://ebible.org/details.php?id=engwebp";
const WEBP_LICENSE_URL: &str = "https://ebible.org/engwebp/copyright.htm";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct License {
    pub name: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notice: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Translation {
    pub id: String,
    pub name: String,
    pub abbreviation: String,
    pub language: String,
    pub source_url: String,
    pub license: License,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChapterSummary {
    pub number: u32,
    pub verses: Vec<u32>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BookSummary {
    pub id: String,
    pub name: String,
    pub chapters: Vec<ChapterSummary>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Verse {
    pub book_id: String,
    pub book_name: String,
    pub chapter: u32,
    pub verse: u32,
    pub text: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PassageRequest {
    pub translation_ids: Vec<String>,
    pub book_id: String,
    pub chapter_start: u32,
    pub chapter_end: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verse_start: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verse_end: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PassageTranslation {
    pub translation: Translation,
    pub verses: Vec<Verse>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Passage {
    pub book_id: String,
    pub book_name: String,
    pub chapter_start: u32,
    pub chapter_end: u32,
    pub verse_start: u32,
    pub verse_end: u32,
    pub reference: String,
    pub translations: Vec<PassageTranslation>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SearchRequest {
    pub translation_id: String,
    pub query: String,
    pub offset: u32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SearchResults {
    pub results: Vec<Verse>,
    pub total: u32,
    pub offset: u32,
    pub limit: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BiblePackage {
    schema: String,
    version: u32,
    translation: Translation,
    books: Vec<Book>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Book {
    id: String,
    name: String,
    chapters: Vec<Chapter>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Chapter {
    number: u32,
    verses: Vec<VerseText>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct VerseText {
    number: u32,
    text: String,
}

#[derive(Clone, Debug)]
pub struct Store {
    root: PathBuf,
}

impl Store {
    pub fn open(root: &Path) -> Result<Self, String> {
        fs::create_dir_all(root)
            .map_err(|error| format!("Bible package folder could not be created: {error}"))?;
        let root = root
            .canonicalize()
            .map_err(|error| format!("Bible package folder could not be resolved: {error}"))?;
        if !root.is_dir() {
            return Err("Bible package path must be a folder.".into());
        }
        Ok(Self { root })
    }

    pub fn list(&self) -> Result<Vec<Translation>, String> {
        let mut translations = Vec::new();
        for entry in fs::read_dir(&self.root)
            .map_err(|error| format!("Bible packages could not be listed: {error}"))?
        {
            let entry =
                entry.map_err(|error| format!("Bible package could not be read: {error}"))?;
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let metadata = fs::symlink_metadata(&path)
                .map_err(|error| format!("Bible package could not be inspected: {error}"))?;
            if !metadata.file_type().is_file() {
                return Err("Bible package entries must be regular files.".into());
            }
            let package = read_package(&path)?;
            if path.file_stem().and_then(|value| value.to_str())
                != Some(package.translation.id.as_str())
            {
                return Err("Bible package file name does not match its translation ID.".into());
            }
            translations.push(package.translation);
        }
        translations.sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)));
        Ok(translations)
    }

    pub fn install(&self, path: &Path) -> Result<Translation, String> {
        let package = read_package(path)?;
        self.install_package(package)
    }

    pub fn install_web(&self) -> Result<Translation, String> {
        let package = download_web_package()?;
        self.install_package(package)
    }

    pub fn remove(&self, id: &str) -> Result<(), String> {
        validate_translation_id(id)?;
        let path = self.package_path(id)?;
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("Bible translation is unavailable: {error}"))?;
        if !metadata.file_type().is_file() {
            return Err("Bible translation package must be a regular file.".into());
        }
        let canonical = path
            .canonicalize()
            .map_err(|error| format!("Bible translation could not be resolved: {error}"))?;
        if !canonical.starts_with(&self.root) {
            return Err("Bible translation package escapes its package folder.".into());
        }
        fs::remove_file(canonical)
            .map_err(|error| format!("Bible translation could not be removed: {error}"))
    }

    pub fn books(&self, id: &str) -> Result<Vec<BookSummary>, String> {
        let package = self.load(id)?;
        Ok(package
            .books
            .iter()
            .map(|book| BookSummary {
                id: book.id.clone(),
                name: book.name.clone(),
                chapters: book
                    .chapters
                    .iter()
                    .map(|chapter| ChapterSummary {
                        number: chapter.number,
                        verses: chapter.verses.iter().map(|verse| verse.number).collect(),
                    })
                    .collect(),
            })
            .collect())
    }

    pub fn resolve_reference(
        &self,
        translation_id: &str,
        reference: &str,
    ) -> Result<PassageRequest, String> {
        let package = self.load(translation_id)?;
        let input = reference.trim();
        if input.is_empty() || input.len() > 300 {
            return Err("Bible reference must contain no more than 300 characters.".into());
        }
        let separator = input
            .rfind(char::is_whitespace)
            .ok_or("Bible reference must include a book and chapter.")?;
        let book_label = input[..separator].trim();
        let range = input[separator..].trim();
        let book = package
            .books
            .iter()
            .find(|book| {
                book.id.eq_ignore_ascii_case(book_label)
                    || book.name.eq_ignore_ascii_case(book_label)
            })
            .ok_or_else(|| format!("Book {book_label:?} is not available in this translation."))?;
        let (start_chapter, start_verse, end_chapter, end_verse) = parse_reference_range(range)?;
        if start_chapter == 0
            || start_chapter > 200
            || end_chapter == 0
            || end_chapter > 200
            || end_chapter < start_chapter
            || end_chapter - start_chapter > 10
            || start_verse.is_some_and(|verse| verse == 0 || verse > 200)
            || end_verse.is_some_and(|verse| verse == 0 || verse > 200)
            || (start_chapter == end_chapter
                && start_verse
                    .zip(end_verse)
                    .is_some_and(|(start, end)| end < start))
        {
            return Err("Bible reference range is invalid.".into());
        }
        Ok(PassageRequest {
            translation_ids: vec![translation_id.into()],
            book_id: book.id.clone(),
            chapter_start: start_chapter,
            chapter_end: end_chapter,
            verse_start: start_verse,
            verse_end: end_verse,
        })
    }

    pub fn passage(&self, request: &PassageRequest) -> Result<Passage, String> {
        validate_book_id(&request.book_id)?;
        if request.translation_ids.is_empty()
            || request.translation_ids.len() > MAX_PARALLEL_TRANSLATIONS
            || request
                .translation_ids
                .iter()
                .any(|id| validate_translation_id(id).is_err())
        {
            return Err(format!(
                "A passage requires 1 to {MAX_PARALLEL_TRANSLATIONS} valid translation IDs."
            ));
        }
        if request.chapter_start == 0
            || request.chapter_start > 200
            || request.chapter_end > 200
            || request.chapter_end < request.chapter_start
            || request.chapter_end - request.chapter_start > 10
            || request
                .verse_start
                .is_some_and(|verse| verse == 0 || verse > 200)
            || request
                .verse_end
                .is_some_and(|verse| verse == 0 || verse > 200)
            || (request.chapter_start == request.chapter_end
                && request
                    .verse_start
                    .zip(request.verse_end)
                    .is_some_and(|(start, end)| end < start))
        {
            return Err("Bible passage range is invalid.".into());
        }
        let unique_ids = request.translation_ids.iter().collect::<HashSet<_>>();
        if unique_ids.len() != request.translation_ids.len() {
            return Err("Bible passage contains a duplicate translation ID.".into());
        }

        let mut results = Vec::with_capacity(request.translation_ids.len());
        for id in &request.translation_ids {
            let package = self.load(id)?;
            results.push(passage_in_package(&package, request)?);
        }
        let expected = results[0]
            .verses
            .iter()
            .map(|verse| (verse.chapter, verse.verse))
            .collect::<Vec<_>>();
        if results.iter().skip(1).any(|result| {
            result
                .verses
                .iter()
                .map(|verse| (verse.chapter, verse.verse))
                .collect::<Vec<_>>()
                != expected
        }) {
            return Err(
                "Selected translations use different verse numbering for this passage.".into(),
            );
        }
        let first_verse = results[0]
            .verses
            .first()
            .ok_or("Bible passage contains no verses.")?;
        let last_verse = results[0]
            .verses
            .last()
            .ok_or("Bible passage contains no verses.")?;
        let book_name = first_verse.book_name.clone();
        let reference = format_reference(
            &book_name,
            first_verse.chapter,
            first_verse.verse,
            last_verse.chapter,
            last_verse.verse,
        );
        Ok(Passage {
            book_id: request.book_id.clone(),
            book_name,
            chapter_start: first_verse.chapter,
            chapter_end: last_verse.chapter,
            verse_start: first_verse.verse,
            verse_end: last_verse.verse,
            reference,
            translations: results,
        })
    }

    pub fn search(&self, request: &SearchRequest) -> Result<SearchResults, String> {
        validate_translation_id(&request.translation_id)?;
        if request.query.len() > 512 {
            return Err("Bible search query must be no longer than 512 bytes.".into());
        }
        let terms = request
            .query
            .split_whitespace()
            .map(str::to_lowercase)
            .collect::<Vec<_>>();
        if terms.is_empty() {
            return Err("Bible search query must not be empty.".into());
        }
        if terms.iter().any(|term| term.len() > 128) {
            return Err("Bible search terms must be no longer than 128 bytes.".into());
        }
        let package = self.load(&request.translation_id)?;
        let mut matches = Vec::new();
        for book in &package.books {
            for chapter in &book.chapters {
                for verse in &chapter.verses {
                    let text = verse.text.to_lowercase();
                    if terms.iter().all(|term| text.contains(term)) {
                        matches.push(Verse {
                            book_id: book.id.clone(),
                            book_name: book.name.clone(),
                            chapter: chapter.number,
                            verse: verse.number,
                            text: verse.text.clone(),
                        });
                    }
                }
            }
        }
        let total = u32::try_from(matches.len())
            .map_err(|_| "Bible search returned too many results.".to_string())?;
        let start = (request.offset as usize).min(matches.len());
        let end = start
            .saturating_add(SEARCH_LIMIT as usize)
            .min(matches.len());
        Ok(SearchResults {
            results: matches[start..end].to_vec(),
            total,
            offset: request.offset,
            limit: SEARCH_LIMIT,
        })
    }

    fn install_package(&self, package: BiblePackage) -> Result<Translation, String> {
        let package = validate_package(package)?;
        let id = &package.translation.id;
        let path = self.package_path(id)?;
        match fs::symlink_metadata(&path) {
            Ok(_) => return Err(format!("Bible translation '{id}' is already installed.")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "Bible translation path could not be inspected: {error}"
                ))
            }
        }
        let bytes = serde_json::to_vec_pretty(&package)
            .map_err(|error| format!("Bible translation could not be encoded: {error}"))?;
        if bytes.len() as u64 > MAX_PACKAGE_BYTES {
            return Err("Bible package exceeds the 20 MiB size limit.".into());
        }
        let temporary = self
            .root
            .join(format!(".{id}-{}.tmp", uuid::Uuid::new_v4()));
        let result: Result<(), String> = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
                .map_err(|error| format!("Bible package could not be staged: {error}"))?;
            file.write_all(&bytes)
                .and_then(|_| file.sync_all())
                .map_err(|error| format!("Bible package could not be saved: {error}"))?;
            fs::hard_link(&temporary, &path)
                .map_err(|error| format!("Bible translation could not be installed: {error}"))?;
            fs::remove_file(&temporary).map_err(|error| {
                format!("Bible package staging file could not be removed: {error}")
            })?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result?;
        Ok(package.translation)
    }

    fn load(&self, id: &str) -> Result<BiblePackage, String> {
        validate_translation_id(id)?;
        let path = self.package_path(id)?;
        let package = read_package(&path)?;
        if package.translation.id != id {
            return Err("Bible package file name does not match its translation ID.".into());
        }
        Ok(package)
    }

    fn package_path(&self, id: &str) -> Result<PathBuf, String> {
        validate_translation_id(id)?;
        let path = self.root.join(format!("{id}.json"));
        if path.exists() {
            let canonical = path
                .canonicalize()
                .map_err(|error| format!("Bible package path could not be resolved: {error}"))?;
            if !canonical.starts_with(&self.root) {
                return Err("Bible package path escapes its package folder.".into());
            }
        }
        Ok(path)
    }
}

fn passage_in_package(
    package: &BiblePackage,
    request: &PassageRequest,
) -> Result<PassageTranslation, String> {
    let book = package
        .books
        .iter()
        .find(|book| book.id == request.book_id)
        .ok_or_else(|| {
            format!(
                "{} is not available in {}.",
                request.book_id, package.translation.abbreviation
            )
        })?;
    let mut verses = Vec::new();
    for chapter_number in request.chapter_start..=request.chapter_end {
        let chapter = book
            .chapters
            .iter()
            .find(|chapter| chapter.number == chapter_number)
            .ok_or_else(|| {
                format!(
                    "{} {} is unavailable in {}.",
                    book.name, chapter_number, package.translation.abbreviation
                )
            })?;
        let first_requested = if chapter_number == request.chapter_start {
            request.verse_start
        } else {
            None
        };
        let last_requested = if chapter_number == request.chapter_end {
            request.verse_end
        } else {
            None
        };
        let first_available = chapter
            .verses
            .first()
            .ok_or_else(|| {
                format!(
                    "{} {} contains no verses in {}.",
                    book.name, chapter_number, package.translation.abbreviation
                )
            })?
            .number;
        let last_available = chapter
            .verses
            .last()
            .ok_or_else(|| {
                format!(
                    "{} {} contains no verses in {}.",
                    book.name, chapter_number, package.translation.abbreviation
                )
            })?
            .number;
        let start = first_requested.unwrap_or(first_available);
        let end = last_requested.unwrap_or(last_available);
        if end < start {
            return Err("Bible passage verse range is invalid.".into());
        }
        let selected = chapter
            .verses
            .iter()
            .filter(|verse| (start..=end).contains(&verse.number))
            .collect::<Vec<_>>();
        if request.verse_start.is_some() || request.verse_end.is_some() {
            for number in start..=end {
                if !selected.iter().any(|verse| verse.number == number) {
                    return Err(format!(
                        "Requested verse {} {}:{} is unavailable in {}.",
                        book.name, chapter_number, number, package.translation.abbreviation
                    ));
                }
            }
        }
        for text in selected {
            verses.push(Verse {
                book_id: book.id.clone(),
                book_name: book.name.clone(),
                chapter: chapter_number,
                verse: text.number,
                text: text.text.clone(),
            });
        }
    }
    Ok(PassageTranslation {
        translation: package.translation.clone(),
        verses,
    })
}

fn read_package(path: &Path) -> Result<BiblePackage, String> {
    let file =
        File::open(path).map_err(|error| format!("Bible package could not be opened: {error}"))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("Bible package could not be inspected: {error}"))?;
    if !metadata.is_file() {
        return Err("Bible package must be a regular file.".into());
    }
    if metadata.len() > MAX_PACKAGE_BYTES {
        return Err("Bible package exceeds the 20 MiB size limit.".into());
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_PACKAGE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Bible package could not be read: {error}"))?;
    if bytes.len() as u64 > MAX_PACKAGE_BYTES {
        return Err("Bible package exceeds the 20 MiB size limit.".into());
    }
    let package: BiblePackage = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Bible package JSON is invalid: {error}"))?;
    validate_package(package)
}

fn parse_reference_range(value: &str) -> Result<(u32, Option<u32>, u32, Option<u32>), String> {
    let mut parts = value.split('-');
    let start = parse_reference_point(parts.next().unwrap_or_default())?;
    let end = match parts.next() {
        Some(value) => {
            if parts.next().is_some() {
                return Err("Bible reference may contain only one range separator.".into());
            }
            if value.contains(':') {
                parse_reference_point(value)?
            } else if start.1.is_some() {
                let verse = value
                    .parse::<u32>()
                    .map_err(|_| "Bible reference ending verse is invalid.")?;
                (start.0, Some(verse))
            } else {
                return Err("A chapter range must include a verse on each end.".into());
            }
        }
        None => match start.1 {
            Some(verse) => (start.0, Some(verse)),
            None => (start.0, None),
        },
    };
    Ok((start.0, start.1, end.0, end.1))
}

fn parse_reference_point(value: &str) -> Result<(u32, Option<u32>), String> {
    let mut parts = value.split(':');
    let chapter = parts
        .next()
        .and_then(|value| value.parse::<u32>().ok())
        .ok_or("Bible reference chapter is invalid.")?;
    let verse = match parts.next() {
        Some(value) => Some(
            value
                .parse::<u32>()
                .map_err(|_| "Bible reference verse is invalid.")?,
        ),
        None => None,
    };
    if parts.next().is_some() {
        return Err("Bible reference point is invalid.".into());
    }
    Ok((chapter, verse))
}

fn validate_package(mut package: BiblePackage) -> Result<BiblePackage, String> {
    if package.schema != PACKAGE_SCHEMA || package.version != PACKAGE_VERSION {
        return Err(format!(
            "Unsupported Bible package schema or version; expected {PACKAGE_SCHEMA} version {PACKAGE_VERSION}."
        ));
    }
    validate_translation(&package.translation)?;
    if package.books.is_empty() || package.books.len() > 200 {
        return Err("Bible package must contain 1 to 200 books.".into());
    }
    let mut book_ids = HashSet::new();
    let mut book_names = HashSet::new();
    let mut verse_count = 0_usize;
    for book in &mut package.books {
        validate_book_id(&book.id)?;
        if !book_ids.insert(book.id.clone()) {
            return Err(format!(
                "Bible package contains duplicate book {}.",
                book.id
            ));
        }
        validate_label(&book.name, "Bible book name", 100)?;
        if !book_names.insert(book.name.to_lowercase()) {
            return Err(format!(
                "Bible package contains duplicate book name {:?}.",
                book.name
            ));
        }
        if book.chapters.is_empty() || book.chapters.len() > 200 {
            return Err(format!(
                "Bible book {} has an invalid chapter list.",
                book.id
            ));
        }
        book.chapters.sort_by_key(|chapter| chapter.number);
        let mut chapter_numbers = HashSet::new();
        for chapter in &mut book.chapters {
            if chapter.number == 0
                || chapter.number > 200
                || !chapter_numbers.insert(chapter.number)
            {
                return Err(format!(
                    "Bible book {} has an invalid or duplicate chapter number.",
                    book.id
                ));
            }
            if chapter.verses.is_empty() {
                return Err(format!(
                    "Bible chapter {} {} contains no verses.",
                    book.id, chapter.number
                ));
            }
            chapter.verses.sort_by_key(|verse| verse.number);
            let mut verse_numbers = HashSet::new();
            for verse in &chapter.verses {
                if verse.number == 0 || verse.number > 200 || !verse_numbers.insert(verse.number) {
                    return Err(format!(
                        "Bible chapter {} {} has an invalid or duplicate verse number.",
                        book.id, chapter.number
                    ));
                }
                validate_label(&verse.text, "Bible verse text", 20_000)?;
                verse_count += 1;
            }
        }
    }
    if verse_count > 100_000 {
        return Err("Bible package contains too many verses.".into());
    }
    package.books.sort_by(|left, right| {
        book_order(&left.id)
            .unwrap_or(usize::MAX)
            .cmp(&book_order(&right.id).unwrap_or(usize::MAX))
            .then(left.id.cmp(&right.id))
    });
    Ok(package)
}

fn validate_translation(translation: &Translation) -> Result<(), String> {
    validate_translation_id(&translation.id)?;
    validate_label(&translation.name, "Bible translation name", 200)?;
    validate_label(
        &translation.abbreviation,
        "Bible translation abbreviation",
        32,
    )?;
    validate_label(&translation.language, "Bible translation language", 35)?;
    validate_https_url(&translation.source_url, "Bible source URL")?;
    validate_label(&translation.license.name, "Bible license name", 100)?;
    validate_https_url(&translation.license.url, "Bible license URL")?;
    if let Some(notice) = &translation.license.notice {
        validate_label(notice, "Bible license notice", 512)?;
    }
    Ok(())
}

fn validate_label(value: &str, label: &str, max_bytes: usize) -> Result<(), String> {
    if value.trim().is_empty()
        || value.len() > max_bytes
        || value.chars().any(|character| character == '\0')
    {
        return Err(format!("{label} is invalid."));
    }
    Ok(())
}

fn validate_https_url(value: &str, label: &str) -> Result<(), String> {
    let parsed = url::Url::parse(value).map_err(|_| format!("{label} is invalid."))?;
    if parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err(format!("{label} must be an HTTPS URL without credentials."));
    }
    Ok(())
}

fn validate_translation_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        || id.starts_with('-')
        || id.ends_with('-')
    {
        return Err(
            "Bible translation ID must use lowercase letters, digits, and interior hyphens.".into(),
        );
    }
    Ok(())
}

fn validate_book_id(id: &str) -> Result<(), String> {
    // Validate the stable code shape rather than a Protestant-canon allowlist.
    let bytes = id.as_bytes();
    if bytes.len() != 3
        || !(bytes[0].is_ascii_uppercase() || matches!(bytes[0], b'1'..=b'9'))
        || !bytes[1..].iter().all(u8::is_ascii_uppercase)
    {
        return Err(format!(
            "Bible book ID {id:?} must be a three-character USFM-style code (uppercase letters, optionally preceded by a digit)."
        ));
    }
    Ok(())
}

fn book_order(id: &str) -> Option<usize> {
    CANONICAL_BOOKS.iter().position(|book| book.id == id)
}

fn download_web_package() -> Result<BiblePackage, String> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(REQUEST_TIMEOUT))
        .http_status_as_error(false)
        .max_redirects(5)
        .max_redirects_will_error(true)
        .user_agent("Presenter Orbit Bible/1.0")
        .build()
        .new_agent();
    let mut response = agent
        .get(WEBP_VPL_URL)
        .call()
        .map_err(|error| format!("World English Bible download failed: {error}"))?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(format!(
            "World English Bible download returned HTTP {status}."
        ));
    }
    let bytes = response
        .body_mut()
        .with_config()
        .limit(MAX_ARCHIVE_BYTES)
        .read_to_vec()
        .map_err(|error| format!("World English Bible archive could not be read: {error}"))?;
    parse_web_archive(&bytes)
}

fn parse_web_archive(bytes: &[u8]) -> Result<BiblePackage, String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| format!("World English Bible archive is invalid: {error}"))?;
    if archive.len() > 256 {
        return Err("World English Bible archive contains too many files.".into());
    }
    let entry = archive
        .by_name("engwebp_vpl.txt")
        .map_err(|_| "World English Bible archive does not contain engwebp_vpl.txt.".to_string())?;
    if entry.size() > MAX_TRANSLATION_BYTES {
        return Err("World English Bible text exceeds the 16 MiB size limit.".into());
    }
    let mut bytes = Vec::with_capacity(entry.size() as usize);
    entry
        .take(MAX_TRANSLATION_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("World English Bible VPL entry could not be read: {error}"))?;
    if bytes.len() as u64 > MAX_TRANSLATION_BYTES {
        return Err("World English Bible text exceeds the 16 MiB size limit.".into());
    }
    let text = String::from_utf8(bytes)
        .map_err(|_| "World English Bible VPL text is not valid UTF-8.".to_string())?;
    if text.contains('\0') {
        return Err("World English Bible VPL text contains null characters.".into());
    }
    let books = rows_to_books(parse_vpl(&text)?)?;
    validate_package(BiblePackage {
        schema: PACKAGE_SCHEMA.into(),
        version: PACKAGE_VERSION,
        translation: Translation {
            id: "engwebp".into(),
            name: "World English Bible".into(),
            abbreviation: "WEBP".into(),
            language: "en".into(),
            source_url: WEBP_SOURCE_URL.into(),
            license: License {
                name: "Public Domain".into(),
                url: WEBP_LICENSE_URL.into(),
                notice: Some(
                    "World English Bible is a trademark and identifies faithful copies of the translation.".into(),
                ),
            },
        },
        books,
    })
}

fn parse_vpl(text: &str) -> Result<Vec<(String, u32, u32, String)>, String> {
    let mut rows = Vec::new();
    for (line_number, raw_line) in text.lines().enumerate() {
        let line = raw_line.trim_start_matches('\u{feff}').trim_start();
        if line.is_empty() {
            continue;
        }
        let mut fields = line.splitn(2, char::is_whitespace);
        let source_book_id = fields.next().ok_or_else(|| {
            format!(
                "World English Bible VPL line {} has no book ID.",
                line_number + 1
            )
        })?;
        let remainder = fields
            .next()
            .ok_or_else(|| {
                format!(
                    "World English Bible VPL line {} has no verse reference.",
                    line_number + 1
                )
            })?
            .trim_start();
        let mut fields = remainder.splitn(2, char::is_whitespace);
        let reference = fields.next().ok_or_else(|| {
            format!(
                "World English Bible VPL line {} has no verse reference.",
                line_number + 1
            )
        })?;
        let body = fields.next().unwrap_or_default().trim_start().trim_end();
        let book_id = webp_book_id(source_book_id).ok_or_else(|| {
            format!(
                "World English Bible VPL line {} has unsupported book ID {source_book_id:?}.",
                line_number + 1
            )
        })?;
        let mut reference = reference.split(':');
        let chapter = parse_positive_number(reference.next(), "chapter", line_number + 1)?;
        let verse = parse_positive_number(reference.next(), "verse", line_number + 1)?;
        if reference.next().is_some() || chapter == 0 || verse == 0 {
            return Err(format!(
                "World English Bible VPL line {} has an invalid verse reference.",
                line_number + 1
            ));
        }
        // eBible's VPL reserves blank rows for verses omitted from the source translation.
        if body.is_empty() {
            continue;
        }
        validate_label(&body, "Bible verse text", 20_000).map_err(|_| {
            format!(
                "World English Bible VPL line {} has empty or oversized verse text.",
                line_number + 1
            )
        })?;
        rows.push((book_id.to_string(), chapter, verse, body.to_string()));
        if rows.len() > 100_000 {
            return Err("World English Bible text contains too many verses.".into());
        }
    }
    if rows.is_empty() {
        return Err("World English Bible VPL text contains no verses.".into());
    }
    Ok(rows)
}

fn parse_positive_number(value: Option<&str>, label: &str, line: usize) -> Result<u32, String> {
    let parsed = value
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|value| *value <= 200);
    parsed.ok_or_else(|| {
        format!("World English Bible VPL line {line} has an invalid {label} number.")
    })
}

fn webp_book_id(source_id: &str) -> Option<&'static str> {
    CANONICAL_BOOKS
        .iter()
        .find(|book| book.webp_code == source_id)
        .map(|book| book.id)
}

fn rows_to_books(rows: Vec<(String, u32, u32, String)>) -> Result<Vec<Book>, String> {
    let mut books = BTreeMap::<usize, (String, BTreeMap<u32, BTreeMap<u32, String>>)>::new();
    for (book_id, chapter_number, verse_number, text) in rows {
        let order = book_order(&book_id)
            .ok_or_else(|| "VPL contained an unsupported book ID.".to_string())?;
        let book = books
            .entry(order)
            .or_insert_with(|| (book_id.clone(), BTreeMap::new()));
        if book
            .1
            .entry(chapter_number)
            .or_default()
            .insert(verse_number, text)
            .is_some()
        {
            return Err(format!(
                "World English Bible VPL repeats {book_id} {chapter_number}:{verse_number}."
            ));
        }
    }
    Ok(books
        .into_iter()
        .map(|(order, (id, chapters))| Book {
            id,
            name: CANONICAL_BOOKS[order].name.into(),
            chapters: chapters
                .into_iter()
                .map(|(number, verses)| Chapter {
                    number,
                    verses: verses
                        .into_iter()
                        .map(|(number, text)| VerseText { number, text })
                        .collect(),
                })
                .collect(),
        })
        .collect())
}

fn format_reference(
    book_name: &str,
    chapter_start: u32,
    verse_start: u32,
    chapter_end: u32,
    verse_end: u32,
) -> String {
    if chapter_start == chapter_end && verse_start == verse_end {
        format!("{book_name} {chapter_start}:{verse_start}")
    } else if chapter_start == chapter_end {
        format!("{book_name} {chapter_start}:{verse_start}-{verse_end}")
    } else {
        format!("{book_name} {chapter_start}:{verse_start}-{chapter_end}:{verse_end}")
    }
}

struct BookDefinition {
    id: &'static str,
    name: &'static str,
    webp_code: &'static str,
}

macro_rules! book {
    ($id:literal, $name:literal, $webp_code:literal) => {
        BookDefinition {
            id: $id,
            name: $name,
            webp_code: $webp_code,
        }
    };
}

const CANONICAL_BOOKS: [BookDefinition; 66] = [
    book!("GEN", "Genesis", "GEN"),
    book!("EXO", "Exodus", "EXO"),
    book!("LEV", "Leviticus", "LEV"),
    book!("NUM", "Numbers", "NUM"),
    book!("DEU", "Deuteronomy", "DEU"),
    book!("JOS", "Joshua", "JOS"),
    book!("JDG", "Judges", "JDG"),
    book!("RUT", "Ruth", "RUT"),
    book!("1SA", "1 Samuel", "1SA"),
    book!("2SA", "2 Samuel", "2SA"),
    book!("1KI", "1 Kings", "1KI"),
    book!("2KI", "2 Kings", "2KI"),
    book!("1CH", "1 Chronicles", "1CH"),
    book!("2CH", "2 Chronicles", "2CH"),
    book!("EZR", "Ezra", "EZR"),
    book!("NEH", "Nehemiah", "NEH"),
    book!("EST", "Esther", "EST"),
    book!("JOB", "Job", "JOB"),
    book!("PSA", "Psalms", "PSA"),
    book!("PRO", "Proverbs", "PRO"),
    book!("ECC", "Ecclesiastes", "ECC"),
    book!("SNG", "Song of Solomon", "SOL"),
    book!("ISA", "Isaiah", "ISA"),
    book!("JER", "Jeremiah", "JER"),
    book!("LAM", "Lamentations", "LAM"),
    book!("EZK", "Ezekiel", "EZE"),
    book!("DAN", "Daniel", "DAN"),
    book!("HOS", "Hosea", "HOS"),
    book!("JOL", "Joel", "JOE"),
    book!("AMO", "Amos", "AMO"),
    book!("OBA", "Obadiah", "OBA"),
    book!("JON", "Jonah", "JON"),
    book!("MIC", "Micah", "MIC"),
    book!("NAM", "Nahum", "NAH"),
    book!("HAB", "Habakkuk", "HAB"),
    book!("ZEP", "Zephaniah", "ZEP"),
    book!("HAG", "Haggai", "HAG"),
    book!("ZEC", "Zechariah", "ZEC"),
    book!("MAL", "Malachi", "MAL"),
    book!("MAT", "Matthew", "MAT"),
    book!("MRK", "Mark", "MAR"),
    book!("LUK", "Luke", "LUK"),
    book!("JHN", "John", "JOH"),
    book!("ACT", "Acts", "ACT"),
    book!("ROM", "Romans", "ROM"),
    book!("1CO", "1 Corinthians", "1CO"),
    book!("2CO", "2 Corinthians", "2CO"),
    book!("GAL", "Galatians", "GAL"),
    book!("EPH", "Ephesians", "EPH"),
    book!("PHP", "Philippians", "PHI"),
    book!("COL", "Colossians", "COL"),
    book!("1TH", "1 Thessalonians", "1TH"),
    book!("2TH", "2 Thessalonians", "2TH"),
    book!("1TI", "1 Timothy", "1TI"),
    book!("2TI", "2 Timothy", "2TI"),
    book!("TIT", "Titus", "TIT"),
    book!("PHM", "Philemon", "PHM"),
    book!("HEB", "Hebrews", "HEB"),
    book!("JAS", "James", "JAM"),
    book!("1PE", "1 Peter", "1PE"),
    book!("2PE", "2 Peter", "2PE"),
    book!("1JN", "1 John", "1JO"),
    book!("2JN", "2 John", "2JO"),
    book!("3JN", "3 John", "3JO"),
    book!("JUD", "Jude", "JUD"),
    book!("REV", "Revelation", "REV"),
];
