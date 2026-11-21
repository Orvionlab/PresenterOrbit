use cosmic_text::FontSystem;

pub fn installed_font_families() -> Vec<String> {
    let font_system = FontSystem::new();
    let names = font_system
        .db()
        .faces()
        .flat_map(|face| face.families.iter().map(|(name, _)| name.clone()));
    normalize_families(names)
}

fn normalize_families(names: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut names: Vec<String> = names
        .into_iter()
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .collect();
    names.sort_by(|left, right| {
        left.to_lowercase()
            .cmp(&right.to_lowercase())
            .then(left.cmp(right))
    });
    names.dedup_by(|left, right| left.to_lowercase() == right.to_lowercase());
    names
}
