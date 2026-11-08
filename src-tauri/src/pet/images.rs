use std::{io, path::Path};

/// Imported artwork keeps its alpha and smooth edges. Pixel art is no longer forced.
pub fn prepare_pet_image(source: &Path, output: &Path) -> io::Result<()> {
    let (width, height) = image::image_dimensions(source).map_err(io::Error::other)?;
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > 32_000_000 {
        return Err(io::Error::other(
            "pet image must contain 1–32 million pixels",
        ));
    }
    let image = image::open(source).map_err(io::Error::other)?;
    let prepared = if width > 1024 || height > 1024 {
        image.resize(1024, 1024, image::imageops::FilterType::Lanczos3)
    } else {
        image
    };
    prepared
        .to_rgba8()
        .save_with_format(output, image::ImageFormat::Png)
        .map_err(io::Error::other)
}
