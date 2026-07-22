use std::time::Duration;

use gst::prelude::*;

use crate::NetworkSourceOptions;

pub(crate) fn configure_playbin_network(playbin: &gst::Element, options: &NetworkSourceOptions) {
    if let Some(buffer_duration) = options.buffer_duration() {
        let nanoseconds = i64::try_from(buffer_duration.as_nanos()).unwrap_or(i64::MAX);
        playbin.set_property("buffer-duration", nanoseconds);
    }
    if let Some(buffer_size) = options.buffer_size() {
        playbin.set_property(
            "buffer-size",
            i32::try_from(buffer_size).unwrap_or(i32::MAX),
        );
    }
    if let Some(connection_speed_kbps) = options.connection_speed_kbps() {
        playbin.set_property("connection-speed", connection_speed_kbps);
    }

    let options_for_source = options.clone();
    playbin.connect("source-setup", false, move |values| {
        if let Some(source) = values
            .get(1)
            .and_then(|value| value.get::<gst::Element>().ok())
        {
            apply_network_source_options(&source, &options_for_source);
        }
        None
    });

    // Adaptive demuxers can create additional URI source elements for media
    // segments after the manifest source has already been configured. Apply
    // the same options to every element whose plugin exposes compatible HTTP
    // properties so HLS/DASH segment requests inherit authentication too.
    let options_for_elements = options.clone();
    playbin.connect("element-setup", false, move |values| {
        if let Some(element) = values
            .get(1)
            .and_then(|value| value.get::<gst::Element>().ok())
        {
            apply_network_source_options(&element, &options_for_elements);
        }
        None
    });
}

fn apply_network_source_options(source: &gst::Element, options: &NetworkSourceOptions) {
    if !options.headers().is_empty() && source.find_property("extra-headers").is_some() {
        source.set_property("extra-headers", extra_headers(options));
    }
    set_string_property(source, "user-id", options.user_id());
    set_string_property(source, "user-pw", options.user_password());
    set_string_property(source, "user-agent", options.user_agent());
    set_string_property(source, "proxy", options.proxy());

    if let Some(timeout) = options.timeout()
        && source.find_property("timeout").is_some()
    {
        source.set_property("timeout", duration_seconds_ceil(timeout).min(3600));
    }
    if let Some(retries) = options.retry_count()
        && source.find_property("retries").is_some()
    {
        source.set_property("retries", i32::try_from(retries).unwrap_or(i32::MAX));
    }
    if let Some(backoff) = options.retry_backoff_factor()
        && source.find_property("retry-backoff-factor").is_some()
    {
        source.set_property("retry-backoff-factor", backoff.as_secs_f64());
    }
    if let Some(max_backoff) = options.retry_backoff_max()
        && source.find_property("retry-backoff-max").is_some()
    {
        source.set_property("retry-backoff-max", max_backoff.as_secs_f64());
    }
    set_bool_property(source, "automatic-redirect", options.automatic_redirect());
    set_bool_property(source, "keep-alive", options.keep_alive());
    set_bool_property(source, "ssl-strict", options.strict_tls());
}

fn set_string_property(source: &gst::Element, name: &str, value: Option<&str>) {
    if let Some(value) = value
        && source.find_property(name).is_some()
    {
        source.set_property(name, value);
    }
}

fn set_bool_property(source: &gst::Element, name: &str, value: Option<bool>) {
    if let Some(value) = value
        && source.find_property(name).is_some()
    {
        source.set_property(name, value);
    }
}

fn extra_headers(options: &NetworkSourceOptions) -> gst::Structure {
    let mut headers = gst::Structure::new_empty("extra-headers");
    for (name, value) in options.headers() {
        headers.set(name, value.as_str());
    }
    headers
}

fn duration_seconds_ceil(duration: Duration) -> u32 {
    let seconds = duration.as_secs();
    let seconds = seconds.saturating_add(u64::from(duration.subsec_nanos() > 0));
    u32::try_from(seconds).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use gst::prelude::*;

    use super::{apply_network_source_options, duration_seconds_ceil, extra_headers};
    use crate::NetworkSourceOptions;

    #[test]
    fn header_structure_preserves_custom_request_headers() {
        crate::init().unwrap();
        let options = NetworkSourceOptions::default()
            .with_header("Authorization", "Bearer secret")
            .unwrap()
            .with_header("Referer", "https://example.com/")
            .unwrap();
        let headers = extra_headers(&options);

        assert_eq!(
            headers.get::<String>("Authorization").unwrap(),
            "Bearer secret"
        );
        assert_eq!(
            headers.get::<String>("Referer").unwrap(),
            "https://example.com/"
        );
    }

    #[test]
    fn subsecond_network_timeout_rounds_up() {
        assert_eq!(duration_seconds_ceil(Duration::ZERO), 0);
        assert_eq!(duration_seconds_ceil(Duration::from_millis(1)), 1);
        assert_eq!(duration_seconds_ceil(Duration::from_millis(1500)), 2);
    }

    #[test]
    fn options_are_applied_to_soup_http_sources() {
        crate::init().unwrap();
        let source = gst::ElementFactory::make("souphttpsrc").build().unwrap();
        let options = NetworkSourceOptions::default()
            .with_header("X-Playback-Token", "secret")
            .unwrap()
            .with_basic_auth("webdav-user", "webdav-password")
            .with_user_agent("gpui-video-test")
            .with_timeout(Duration::from_millis(1500))
            .with_retry_count(4)
            .with_retry_backoff(Duration::from_millis(250), Duration::from_secs(3))
            .with_automatic_redirect(false)
            .with_keep_alive(false)
            .with_strict_tls(true);

        apply_network_source_options(&source, &options);

        let headers = source.property::<gst::Structure>("extra-headers");
        assert_eq!(headers.get::<String>("X-Playback-Token").unwrap(), "secret");
        assert_eq!(source.property::<String>("user-agent"), "gpui-video-test");
        assert_eq!(source.property::<String>("user-id"), "webdav-user");
        assert_eq!(source.property::<String>("user-pw"), "webdav-password");
        assert_eq!(source.property::<u32>("timeout"), 2);
        assert_eq!(source.property::<i32>("retries"), 4);
        assert_eq!(source.property::<f64>("retry-backoff-factor"), 0.25);
        assert_eq!(source.property::<f64>("retry-backoff-max"), 3.0);
        assert!(!source.property::<bool>("automatic-redirect"));
        assert!(!source.property::<bool>("keep-alive"));
        assert!(source.property::<bool>("ssl-strict"));
    }
}
