//! Nextcloud connection (fork SHP-ART/deckcraft): `?nc=<path>` opens a presentation from Nextcloud
//! through the `nc/` passage of the office server, and Save under the same name writes it back
//! there instead of downloading. Other names and exports still download.
//!
//! Only pure helpers live here so native tests can check them; the fetch calls are in `web.rs`.

/// The passage, relative to the app's own URL (`/folien/` → `/nc/`).
pub(crate) const PASSAGE: &str = "../nc/";

/// Presentation types that may be opened from and saved to Nextcloud.
const EXTS: &[&str] = &["deckcraft", "pptx", "potx", "ppsx", "txt", "md"];

/// The Nextcloud path from the page's query (`?nc=Folder%2FSlides.pptx`), or `None` if absent or
/// not a plain relative path to a presentation (no `..`, no leading `/`, no backslash or control
/// characters).
pub(crate) fn path_from_query(query: &str) -> Option<String> {
    let raw = query.trim_start_matches('?').split('&').find_map(|part| part.strip_prefix("nc="))?;
    let path = percent_decode(raw)?;
    let ok = !path.is_empty()
        && path.len() <= 1024
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path.chars().any(char::is_control)
        && path.split('/').all(|seg| !seg.is_empty() && seg != "." && seg != "..")
        && extension(&path).is_some_and(|ext| EXTS.contains(&ext.as_str()));
    ok.then_some(path)
}

/// The URL of `path` behind the passage, each segment percent-encoded.
pub(crate) fn url_for(path: &str) -> String {
    let encoded: Vec<String> = path.split('/').map(percent_encode).collect();
    format!("{PASSAGE}{}", encoded.join("/"))
}

/// Where a save goes: back to Nextcloud when a Nextcloud file is open and the saved name is that
/// file's name (File › Save); `None` for any other name (Save As, Export), which downloads.
pub(crate) fn save_target(open: Option<&str>, saved_path: &str) -> Option<String> {
    let open = open?;
    let saved = saved_path.rsplit(['/', '\\']).next()?;
    (file_name(open) == saved).then(|| open.to_string())
}

/// The last component of a Nextcloud path.
pub(crate) fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn extension(path: &str) -> Option<String> {
    let name = file_name(path);
    let (stem, ext) = name.rsplit_once('.')?;
    (!stem.is_empty()).then(|| ext.to_ascii_lowercase())
}

fn percent_encode(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for b in segment.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let hex = s.get(i + 1..i + 3)?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_path_from_the_query() {
        assert_eq!(path_from_query("?nc=Ausarbeitungen%2FVortrag%20%C3%9Cbung.pptx").as_deref(), Some("Ausarbeitungen/Vortrag Übung.pptx"));
        assert_eq!(path_from_query("?webgl&nc=a.deckcraft").as_deref(), Some("a.deckcraft"));
        assert_eq!(path_from_query("?sample"), None);
    }

    #[test]
    fn rejects_paths_that_leave_the_folder_or_are_no_presentations() {
        for bad in [
            "?nc=..%2Fgeheim.pptx",
            "?nc=%2Fetc%2Fpasswd.txt",
            "?nc=a%2F..%2Fb.deckcraft",
            "?nc=a%5Cb.pptx",
            "?nc=bild.png",
            "?nc=",
            "?nc=%ZZ.pptx",
            "?nc=.pptx",
            "?nc=a%0A.pptx",
        ] {
            assert_eq!(path_from_query(bad), None, "{bad}");
        }
    }

    #[test]
    fn encodes_each_segment_behind_the_passage() {
        assert_eq!(url_for("Aus arbeitungen/Übung #1.pptx"), "../nc/Aus%20arbeitungen/%C3%9Cbung%20%231.pptx");
    }

    #[test]
    fn saves_back_only_under_the_opened_name() {
        assert_eq!(save_target(Some("A/Bericht.pptx"), "Bericht.pptx").as_deref(), Some("A/Bericht.pptx"));
        assert_eq!(save_target(Some("A/Bericht.pptx"), "/tmp/x/Bericht.pptx").as_deref(), Some("A/Bericht.pptx"));
        assert_eq!(save_target(Some("A/Bericht.pptx"), "Bericht.pdf"), None);
        assert_eq!(save_target(Some("A/Bericht.pptx"), "Bericht Kopie.pptx"), None);
        assert_eq!(save_target(None, "Bericht.pptx"), None);
    }
}
