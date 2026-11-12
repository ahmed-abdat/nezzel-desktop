# Local cookie patch

Source: cookie 0.18.2, https://github.com/SergioBenitez/cookie-rs (MIT OR Apache-2.0).
Copied from the published crate; source and licences retained.

Only code change: `Cookie::domain_original()` exposes the stored domain, including
its leading dot. Both upstream public domain getters strip that dot. Tauri/Wry
retains the native domain internally, so this accessor lets the sign-in exporter
preserve host-only and domain cookie scope without changing upstream getters.

Remove this patch when upstream offers an equivalent accessor.
