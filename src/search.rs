//! Filters the app list by what was typed in the panel's search box (pure).
use crate::apps::AppEntry;

/// Lowercase, without the Portuguese/Spanish accents.
pub fn normalize(s: &str) -> String {
    s.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
            'é' | 'ê' | 'è' | 'ë' => 'e',
            'í' | 'î' | 'ì' | 'ï' => 'i',
            'ó' | 'ô' | 'õ' | 'ò' | 'ö' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            'ñ' => 'n',
            c => c,
        })
        .collect()
}

/// How well a normalized name matches a normalized query (lower is better).
fn rank(name: &str, query: &str) -> Option<u8> {
    if name.starts_with(query) {
        Some(0)
    } else if name.split(|c: char| !c.is_alphanumeric()).any(|word| word.starts_with(query)) {
        Some(1)
    } else if name.contains(query) {
        Some(2)
    } else {
        None
    }
}

/// Up to `max` apps matching `query`: names that start with it first, then
/// names with a word that starts with it, then names that merely contain it;
/// alphabetical within each group.
pub fn filter<'a>(apps: &'a [AppEntry], query: &str, max: usize) -> Vec<&'a AppEntry> {
    let query = normalize(query.trim());
    if query.is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<(u8, String, &AppEntry)> = apps
        .iter()
        .filter_map(|app| {
            let name = normalize(&app.name);
            rank(&name, &query).map(|r| (r, name, app))
        })
        .collect();
    hits.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
    hits.into_iter().take(max).map(|(_, _, app)| app).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apps() -> Vec<AppEntry> {
        ["Câmera", "Calculadora", "Visual Studio Code", "Discord", "Steam", "Spotify", "OBS Studio"]
            .iter()
            .map(|&name| AppEntry { name: name.into(), id: name.into(), icon: None })
            .collect()
    }

    fn names(apps: &[AppEntry], query: &str, max: usize) -> Vec<String> {
        filter(apps, query, max).iter().map(|a| a.name.clone()).collect()
    }

    #[test]
    fn accent_insensitive() {
        assert_eq!(names(&apps(), "camera", 8), ["Câmera"]);
    }

    #[test]
    fn prefix_before_word_before_contains() {
        assert_eq!(names(&apps(), "st", 8), ["Steam", "OBS Studio", "Visual Studio Code"]);
    }

    #[test]
    fn case_insensitive() {
        assert_eq!(names(&apps(), "DIS", 8), ["Discord"]);
    }

    #[test]
    fn no_match_is_empty() {
        assert!(names(&apps(), "zzz", 8).is_empty());
    }

    #[test]
    fn max_limits() {
        assert_eq!(names(&apps(), "s", 2).len(), 2);
    }
}
