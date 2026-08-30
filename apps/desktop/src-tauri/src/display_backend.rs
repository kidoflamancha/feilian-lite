fn selected_backend(override_backend: Option<&str>) -> Option<String> {
    override_backend
        .filter(|backend| !backend.trim().is_empty())
        .map(|backend| backend.trim().to_string())
}

fn effective_backend<'a>(
    selected_backend: Option<&'a str>,
    current_backend: Option<&'a str>,
    wayland_display: Option<&str>,
    x11_display: Option<&str>,
) -> Option<&'a str> {
    selected_backend
        .or_else(|| current_backend.and_then(|backends| backends.split(',').next().map(str::trim)))
        .or_else(|| {
            wayland_display
                .filter(|display| !display.is_empty())
                .map(|_| "wayland")
        })
        .or_else(|| {
            x11_display
                .filter(|display| !display.is_empty())
                .map(|_| "x11")
        })
}

fn should_disable_dmabuf(effective_backend: Option<&str>, has_explicit_setting: bool) -> bool {
    !has_explicit_setting && matches!(effective_backend, Some("wayland" | "x11"))
}

pub fn configure() {
    let override_backend = std::env::var("FEILIAN_GDK_BACKEND").ok();
    let current_backend = std::env::var("GDK_BACKEND").ok();
    let wayland_display = std::env::var("WAYLAND_DISPLAY").ok();
    let x11_display = std::env::var("DISPLAY").ok();
    let backend = selected_backend(override_backend.as_deref());
    let effective = effective_backend(
        backend.as_deref(),
        current_backend.as_deref(),
        wayland_display.as_deref(),
        x11_display.as_deref(),
    );
    if should_disable_dmabuf(
        effective,
        std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_some(),
    ) {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }
    if let Some(backend) = backend {
        std::env::set_var("GDK_BACKEND", backend);
    }
}

#[cfg(test)]
mod tests {
    use super::{effective_backend, selected_backend, should_disable_dmabuf};

    #[test]
    fn preserves_native_wayland_when_xwayland_is_available() {
        assert_eq!(selected_backend(None), None);
        assert_eq!(
            effective_backend(None, Some("wayland"), Some("wayland-0"), Some(":0")),
            Some("wayland")
        );
    }

    #[test]
    fn explicit_feilian_override_wins() {
        let selected = selected_backend(Some(" x11 "));

        assert_eq!(selected, Some("x11".to_string()));
        assert_eq!(
            effective_backend(
                selected.as_deref(),
                Some("wayland"),
                Some("wayland-0"),
                Some(":0")
            ),
            Some("x11")
        );
    }

    #[test]
    fn infers_backend_when_gdk_backend_is_unset() {
        assert_eq!(
            effective_backend(None, None, Some("wayland-0"), Some(":0")),
            Some("wayland")
        );
        assert_eq!(effective_backend(None, None, None, Some(":0")), Some("x11"));
        assert_eq!(effective_backend(None, None, None, None), None);
    }

    #[test]
    fn disables_dmabuf_for_graphical_backends_unless_explicitly_configured() {
        assert!(should_disable_dmabuf(Some("wayland"), false));
        assert!(should_disable_dmabuf(Some("x11"), false));
        assert!(!should_disable_dmabuf(Some("wayland"), true));
        assert!(!should_disable_dmabuf(None, false));
    }
}
