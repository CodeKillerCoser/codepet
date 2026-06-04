use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubjectCutoutResult {
    pub source_path: String,
    pub output_path: String,
    pub width: u32,
    pub height: u32,
    pub mime_type: String,
}

pub fn cut_out_subject(source_path: String, output_path: Option<String>) -> Result<SubjectCutoutResult, String> {
    let source = PathBuf::from(&source_path);
    if !source.exists() {
        return Err(format!("image not found: {source_path}"));
    }

    let output = output_path
        .map(PathBuf::from)
        .unwrap_or_else(|| default_subject_cutout_path(&source));
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }

    platform_cut_out_subject(&source, &output)?;
    let (width, height) = image::image_dimensions(&output).map_err(|error| error.to_string())?;

    Ok(SubjectCutoutResult {
        source_path,
        output_path: output.to_string_lossy().to_string(),
        width,
        height,
        mime_type: "image/png".to_string(),
    })
}

pub fn default_subject_cutout_path(source: &Path) -> PathBuf {
    let stem = source
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("subject");
    source.with_file_name(format!("{stem}-subject.png"))
}

#[cfg(target_os = "macos")]
fn platform_cut_out_subject(source: &Path, output: &Path) -> Result<(), String> {
    macos::cut_out_subject(source, output)
}

#[cfg(not(target_os = "macos"))]
fn platform_cut_out_subject(_source: &Path, _output: &Path) -> Result<(), String> {
    Err("subject cutout is only supported on macOS right now".to_string())
}

#[cfg(target_os = "macos")]
mod macos {
    use objc2::AnyThread;
    use objc2::rc::autoreleasepool;
    use objc2::runtime::AnyObject;
    use objc2_core_graphics::CGColorSpace;
    use objc2_core_image::{kCIFormatRGBA8, CIContext, CIImage, CIImageRepresentationOption};
    use objc2_foundation::{NSArray, NSDictionary, NSURL};
    use objc2_vision::{VNGenerateForegroundInstanceMaskRequest, VNImageRequestHandler, VNImageOption, VNRequest};
    use std::fs;
    use std::path::Path;

    pub fn cut_out_subject(source: &Path, output: &Path) -> Result<(), String> {
        autoreleasepool(|_| unsafe { cut_out_subject_inner(source, output) })
    }

    unsafe fn cut_out_subject_inner(source: &Path, output: &Path) -> Result<(), String> {
        let source_url = NSURL::from_file_path(source)
            .ok_or_else(|| format!("invalid image path: {}", source.to_string_lossy()))?;
        let options = NSDictionary::<VNImageOption, AnyObject>::new();
        let handler = VNImageRequestHandler::initWithURL_options(
            VNImageRequestHandler::alloc(),
            &source_url,
            &options,
        );
        let request = VNGenerateForegroundInstanceMaskRequest::new();
        let foreground_requests = NSArray::<VNGenerateForegroundInstanceMaskRequest>::from_slice(&[&request]);
        let requests = foreground_requests.cast_unchecked::<VNRequest>();

        handler
            .performRequests_error(requests)
            .map_err(|error| format!("Vision subject cutout failed: {}", error.localizedDescription()))?;

        let observation = request
            .results()
            .and_then(|results| results.firstObject())
            .ok_or_else(|| "Vision did not find a foreground subject".to_string())?;
        let instances = observation.allInstances();
        let masked_buffer = observation
            .generateMaskedImageOfInstances_fromRequestHandler_croppedToInstancesExtent_error(
                &instances,
                &handler,
                true,
            )
            .map_err(|error| format!("Vision masked image generation failed: {}", error.localizedDescription()))?;

        let image = CIImage::imageWithCVPixelBuffer(&masked_buffer);
        let context = CIContext::context();
        let color_space = CGColorSpace::new_device_rgb()
            .ok_or_else(|| "failed to create RGB color space".to_string())?;
        let png_options = NSDictionary::<CIImageRepresentationOption, AnyObject>::new();
        let png_data = context
            .PNGRepresentationOfImage_format_colorSpace_options(
                &image,
                kCIFormatRGBA8,
                &color_space,
                &png_options,
            )
            .ok_or_else(|| "Core Image failed to encode subject cutout as PNG".to_string())?;

        fs::write(output, png_data.to_vec()).map_err(|error| error.to_string())
    }
}
