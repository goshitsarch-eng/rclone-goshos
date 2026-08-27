//! MIME / extension → Adwaita icon names, matching the Angular icon maps.

use crate::operations::FileTypeCategory;

const MIME_ICONS: &[(&str, &str)] = &[
    ("application/zip", "package-x-generic"),
    ("application/x-tar", "package-x-generic"),
    ("application/gzip", "package-x-generic"),
    ("application/x-7z-compressed", "package-x-generic"),
    ("application/x-rar-compressed", "package-x-generic"),
    ("application/x-bzip2", "package-x-generic"),
    ("application/x-xz", "package-x-generic"),
    (
        "application/vnd.android.package-archive",
        "android-package-archive",
    ),
    ("application/vnd.appimage", "application-x-iso9600-appimage"),
    ("application/x-deb", "application-x-deb"),
    ("application/x-rpm", "application-x-rpm"),
    ("application/java-archive", "application-x-java-archive"),
    ("application/x-java-archive", "application-x-java-archive"),
    ("application/x-iso9660-image", "application-x-cd-image"),
    ("application/x-cd-image", "application-x-cd-image"),
    ("text/x-python", "text-x-python"),
    ("application/x-python", "text-x-python"),
    ("text/x-java", "text-x-java"),
    ("text/x-java-source", "text-x-java"),
    ("application/javascript", "text-x-javascript"),
    ("text/javascript", "text-x-javascript"),
    ("text/x-typescript", "text-x-typescript"),
    ("text/x-c", "text-x-c"),
    ("text/x-cpp", "text-x-cpp"),
    ("text/x-csharp", "text-x-csharp"),
    ("text/x-go", "text-x-go"),
    ("text/x-rust", "text-rust"),
    ("text/x-ruby", "text-x-ruby"),
    ("text/x-php", "application-x-php"),
    ("text/x-perl", "application-x-perl"),
    ("text/x-lua", "text-x-lua"),
    ("text/x-shellscript", "application-x-shellscript"),
    ("text/html", "application-xml"),
    ("text/xml", "application-xml"),
    ("application/xml", "application-xml"),
    ("text/x-yaml", "application-x-yaml"),
    ("application/json", "application-json"),
    ("text/markdown", "text-x-markdown"),
    ("text/x-makefile", "text-x-makefile"),
    ("text/css", "text-css"),
    ("application/pdf", "application-pdf"),
    ("application/msword", "x-office-document"),
    (
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "x-office-document",
    ),
    ("application/vnd.oasis.opendocument.text", "oasis-text"),
    (
        "application/vnd.oasis.opendocument.spreadsheet",
        "oasis-spreadsheet",
    ),
    (
        "application/vnd.oasis.opendocument.presentation",
        "oasis-presentation",
    ),
    ("image/jpeg", "image-x-generic"),
    ("image/png", "image-x-generic"),
    ("image/gif", "image-x-generic"),
    ("image/svg+xml", "image-x-generic"),
    ("audio/mpeg", "audio-x-generic"),
    ("audio/ogg", "audio-x-generic"),
    ("audio/wav", "audio-x-generic"),
    ("video/mp4", "video-x-generic"),
    ("video/mpeg", "video-x-generic"),
    ("video/webm", "video-x-generic"),
    ("font/ttf", "font-x-generic"),
    ("font/otf", "font-x-generic"),
    ("application/x-font-ttf", "font-x-generic"),
    ("application/x-virtualbox-vdi", "virtualbox-vdi"),
    ("application/x-virtualbox-vbox", "virtualbox-vbox"),
];

const EXT_ICONS: &[(&str, &str)] = &[
    ("7z", "package-x-generic"),
    ("apk", "android-package-archive"),
    ("bz2", "package-x-generic"),
    ("deb", "application-x-deb"),
    ("gz", "package-x-generic"),
    ("iso", "application-x-cd-image"),
    ("jar", "application-x-java-archive"),
    ("rar", "package-x-generic"),
    ("rpm", "application-x-rpm"),
    ("tar", "package-x-generic"),
    ("tgz", "package-x-generic"),
    ("zip", "package-x-generic"),
    ("xz", "package-x-generic"),
    ("c", "text-x-c"),
    ("cc", "text-x-cpp"),
    ("cpp", "text-x-cpp"),
    ("cs", "text-x-csharp"),
    ("css", "text-css"),
    ("sass", "text-css"),
    ("scss", "text-css"),
    ("go", "text-x-go"),
    ("h", "text-x-chdr"),
    ("html", "application-xml"),
    ("java", "text-x-java"),
    ("js", "text-x-javascript"),
    ("mjs", "text-x-javascript"),
    ("cjs", "text-x-javascript"),
    ("json", "application-json"),
    ("md", "text-x-markdown"),
    ("markdown", "text-x-markdown"),
    ("sql", "text-x-generic"),
    ("php", "application-x-php"),
    ("py", "text-x-python"),
    ("rb", "text-x-ruby"),
    ("rs", "text-rust"),
    ("sh", "application-x-shellscript"),
    ("bash", "application-x-shellscript"),
    ("zsh", "application-x-shellscript"),
    ("ts", "text-x-typescript"),
    ("xml", "application-xml"),
    ("yaml", "application-x-yaml"),
    ("yml", "application-x-yaml"),
    ("doc", "x-office-document"),
    ("docx", "x-office-document"),
    ("odt", "oasis-text"),
    ("ods", "oasis-spreadsheet"),
    ("odp", "oasis-presentation"),
    ("pdf", "application-pdf"),
    ("ppt", "oasis-presentation"),
    ("pptx", "oasis-presentation"),
    ("xls", "oasis-spreadsheet"),
    ("xlsx", "oasis-spreadsheet"),
    ("aac", "audio-x-generic"),
    ("flac", "audio-x-generic"),
    ("m4a", "audio-x-generic"),
    ("mp3", "audio-x-generic"),
    ("ogg", "audio-x-generic"),
    ("wav", "audio-x-generic"),
    ("avi", "video-x-generic"),
    ("mkv", "video-x-generic"),
    ("mov", "video-x-generic"),
    ("mp4", "video-x-generic"),
    ("webm", "video-x-generic"),
    ("gif", "image-x-generic"),
    ("jpeg", "image-x-generic"),
    ("jpg", "image-x-generic"),
    ("png", "image-x-generic"),
    ("svg", "image-x-generic"),
    ("webp", "image-x-generic"),
];

pub fn normalize_mime(mime: &str) -> String {
    mime.to_ascii_lowercase()
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_string()
}

pub fn icon_for_mime(mime: &str) -> Option<&'static str> {
    let normalized = normalize_mime(mime);
    if normalized.is_empty() {
        return None;
    }
    MIME_ICONS
        .iter()
        .find(|(key, _)| *key == normalized)
        .map(|(_, icon)| *icon)
}

pub fn generic_icon_for_mime(mime: &str) -> &'static str {
    let normalized = normalize_mime(mime);
    match normalized.split('/').next().unwrap_or_default() {
        "text" => "text-x-generic",
        "image" => "image-x-generic",
        "audio" => "audio-x-generic",
        "video" => "video-x-generic",
        "font" => "font-x-generic",
        "application" => "package-x-generic",
        "model" => "application-x-model",
        _ => "text-x-generic",
    }
}

pub fn icon_for_extension(name: &str) -> Option<&'static str> {
    let ext = std::path::Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ext.is_empty() {
        return None;
    }
    EXT_ICONS
        .iter()
        .find(|(key, _)| *key == ext)
        .map(|(_, icon)| *icon)
}

pub fn folder_icon(name: &str) -> &'static str {
    match name.to_ascii_lowercase().as_str() {
        "downloads" | "download" => "folder-download",
        "movies" | "videos" | "video" => "folder-videos",
        "pictures" | "photos" | "images" => "folder-pictures",
        "music" | "audio" => "folder-music",
        "documents" | "docs" => "folder-documents",
        "desktop" => "user-desktop",
        "home" => "user-home",
        "node_modules" => "folder",
        _ => "folder",
    }
}

pub fn icon_for_entry(name: &str, is_dir: bool, mime: &str) -> String {
    if is_dir {
        return folder_icon(name).to_string();
    }
    if let Some(icon) = icon_for_extension(name) {
        return icon.to_string();
    }
    if let Some(icon) = icon_for_mime(mime) {
        return icon.to_string();
    }
    if !normalize_mime(mime).is_empty() {
        return generic_icon_for_mime(mime).to_string();
    }
    FileTypeCategory::from_name(name, false)
        .icon_name()
        .to_string()
}

pub fn category_for_entry(name: &str, is_dir: bool, mime: &str) -> FileTypeCategory {
    FileTypeCategory::from_entry(name, is_dir, mime)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_mime_and_extension() {
        assert_eq!(
            icon_for_mime("application/pdf; charset=binary"),
            Some("application-pdf")
        );
        assert_eq!(icon_for_mime("IMAGE/PNG"), Some("image-x-generic"));
        assert_eq!(icon_for_mime(""), None);
        assert_eq!(icon_for_extension("notes.md"), Some("text-x-markdown"));
        assert_eq!(icon_for_extension("query.sql"), Some("text-x-generic"));
        assert_eq!(
            icon_for_extension("init.zsh"),
            Some("application-x-shellscript")
        );
        assert_eq!(icon_for_extension("theme.sass"), Some("text-css"));
        assert_eq!(icon_for_extension("archive.ZIP"), Some("package-x-generic"));
        assert_eq!(icon_for_extension("README"), None);
        assert_eq!(generic_icon_for_mime("video/unknown"), "video-x-generic");
        assert_eq!(generic_icon_for_mime(""), "text-x-generic");
    }

    #[test]
    fn entry_icons_and_folders() {
        assert_eq!(icon_for_entry("Downloads", true, ""), "folder-download");
        assert_eq!(icon_for_entry("movies", true, ""), "folder-videos");
        assert_eq!(icon_for_entry("home", true, ""), "user-home");
        assert_eq!(icon_for_entry("clip.mp4", false, ""), "video-x-generic");
        assert_eq!(
            icon_for_entry("blob", false, "image/jpeg"),
            "image-x-generic"
        );
        assert_eq!(
            icon_for_entry("blob", false, "model/gltf+json"),
            "application-x-model"
        );
        assert_eq!(
            category_for_entry("shot", false, "image/png"),
            FileTypeCategory::Image
        );
        assert_eq!(
            category_for_entry("pack.zip", false, ""),
            FileTypeCategory::Archive
        );
    }
}
