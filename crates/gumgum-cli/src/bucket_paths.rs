use gumgum_core::{ErrorCode, GumgumError, Subsystem};

pub fn is_local_bucket_path(value: &str) -> bool {
    value.starts_with("./")
        || value.starts_with("../")
        || value.starts_with('/')
        || std::path::Path::new(value).exists()
}

pub fn split_remote_bucket_path(value: &str) -> gumgum_core::Result<(String, String)> {
    let (bucket, path) = value
        .trim_start_matches('/')
        .split_once('/')
        .ok_or_else(|| {
            GumgumError::structured(
                Subsystem::Cli,
                ErrorCode::InvalidArgs,
                format!("bucket object path must be bucket/path: {value}"),
            )
            .build()
        })?;
    if bucket.is_empty() || path.is_empty() {
        return Err(GumgumError::structured(
            Subsystem::Cli,
            ErrorCode::InvalidArgs,
            format!("bucket object path must be bucket/path: {value}"),
        )
        .build());
    }
    Ok((bucket.to_owned(), path.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_bucket_paths_require_bucket_and_key() {
        assert_eq!(
            split_remote_bucket_path("assets/images/logo.png").unwrap(),
            ("assets".to_owned(), "images/logo.png".to_owned())
        );
        assert!(split_remote_bucket_path("assets").is_err());
        assert!(split_remote_bucket_path("assets/").is_err());
        assert!(split_remote_bucket_path("/assets/key.txt").is_ok());
    }

    #[test]
    fn local_bucket_paths_are_explicit_or_existing_paths() {
        assert!(is_local_bucket_path("./file.txt"));
        assert!(is_local_bucket_path("../file.txt"));
        assert!(is_local_bucket_path("/tmp/file.txt"));
        assert!(!is_local_bucket_path("bucket/file.txt"));
    }
}
