use std::path::{Path, PathBuf};

fn development_marker(executable: &Path) -> Option<PathBuf> {
    let macos = executable.parent()?;
    let contents = macos.parent()?;
    if macos.file_name()? != "MacOS" || contents.file_name()? != "Contents" {
        return None;
    }
    Some(contents.join("Resources/Development"))
}

pub fn is_development() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|path| development_marker(&path))
        .is_some_and(|path| path.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn development_marker_is_scoped_to_an_app_bundle() {
        assert_eq!(
            development_marker(Path::new("/apps/Gander Dev.app/Contents/MacOS/gander")),
            Some(PathBuf::from(
                "/apps/Gander Dev.app/Contents/Resources/Development"
            ))
        );
        assert!(development_marker(Path::new("/repo/target/debug/gander")).is_none());
    }
}
