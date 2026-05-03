use image::DynamicImage;

pub async fn is_available() -> bool {
    // Phase 4: ashpd portal screencast for GNOME/KDE
    false
}

pub async fn capture() -> Result<DynamicImage, Box<dyn std::error::Error + Send + Sync>> {
    Err("ashpd portal capture not yet implemented (Phase 4)".into())
}
