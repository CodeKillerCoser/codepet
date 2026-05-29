use code_pet_lib::subject_cutout::{cut_out_subject, default_subject_cutout_path};
use std::path::Path;

#[test]
fn default_subject_cutout_path_writes_png_next_to_source() {
    let output = default_subject_cutout_path(Path::new("/tmp/pets/source.photo.jpeg"));

    assert_eq!(output, Path::new("/tmp/pets/source.photo-subject.png"));
}

#[test]
fn subject_cutout_rejects_missing_source_before_platform_work() {
    let error = cut_out_subject("/tmp/code-pet/missing-photo.png".to_string(), None).unwrap_err();

    assert!(error.contains("image not found"));
}
