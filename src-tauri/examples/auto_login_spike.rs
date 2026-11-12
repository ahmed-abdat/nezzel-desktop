//! Opt-in, credential-free macOS spike. No app commands, plugins, or capabilities.
use std::{sync::mpsc, time::Duration};
use tauri::{webview::NewWindowResponse, WebviewUrl, WebviewWindowBuilder};

fn main() {
  // Bound startup too: setup may never run if WindowServer/LaunchServices is unavailable.
  std::thread::spawn(|| {
    let (_tx, rx) = mpsc::channel::<()>();
    let _ = rx.recv_timeout(Duration::from_secs(100));
    eprintln!("SPIKE_DEADLINE");
    std::process::exit(124);
  });
  let site = std::env::args()
    .nth(1)
    .expect("facebook | instagram | google");
  let (host, path, store) = match site.as_str() {
    "facebook" => ("www.facebook.com", "/login/", 1),
    "instagram" => ("www.instagram.com", "/accounts/login/", 2),
    "google" => ("accounts.google.com", "/ServiceLogin", 3),
    _ => panic!("unsupported spike site"),
  };
  let url: tauri::Url = format!("https://{host}{path}").parse().unwrap();
  let mut context = tauri::test::mock_context(tauri::test::noop_assets());
  context.config_mut().identifier = "dev.ahmed.nezzel.login-spike".into();
  tauri::Builder::default()
    .setup(move |app| {
      eprintln!("SETUP_STARTED");
      let window = WebviewWindowBuilder::new(app, "login-spike", WebviewUrl::External(url.clone()))
        .title(format!("Nezzel login spike: {site}"))
        .inner_size(1000.0, 800.0)
        .data_store_identifier([store; 16])
        .on_navigation(move |url| {
          let base = host
            .trim_start_matches("www.")
            .trim_start_matches("accounts.");
          let h = url.host_str().unwrap_or("");
          let allowed = url.scheme() == "https"
            && (h == base || h.ends_with(&format!(".{base}")))
            && url.username().is_empty()
            && url.password().is_none()
            && url.port_or_known_default() == Some(443);
          if !allowed {
            eprintln!(
              "NAVIGATION_BLOCKED {}{}",
              url.host_str().unwrap_or("?"),
              url.path()
            );
          }
          allowed
        })
        .on_new_window(|_, _| NewWindowResponse::Deny)
        .on_page_load(|_, event| {
          eprintln!(
            "PAGE_LOAD {:?} {}{}",
            event.event(),
            event.url().host_str().unwrap_or("?"),
            event.url().path()
          )
        })
        .build()?;
      eprintln!("WINDOW_CREATED");
      let app = app.handle().clone();
      // Never call the synchronous WebView cookie API on the event-loop thread.
      std::thread::spawn(move || {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
          let result = (|| {
            let cookie = tauri::webview::Cookie::build(("nezzel_spike_httponly", "synthetic"))
              .domain(host)
              .path("/")
              .secure(true)
              .http_only(true)
              .build();
            window
              .set_cookie(cookie.clone())
              .map_err(|_| "set_cookie")?;
            let cookies = window.cookies_for_url(url).map_err(|_| "cookies_for_url")?;
            let found = cookies.iter().any(|c| {
              c.name() == cookie.name()
                && c.value() == cookie.value()
                && c.http_only() == Some(true)
            });
            window.delete_cookie(cookie).map_err(|_| "delete_cookie")?;
            Ok::<bool, &str>(found)
          })();
          let _ = tx.send(result);
        });
        match rx.recv_timeout(Duration::from_secs(5)) {
          Ok(Ok(found)) => eprintln!("HTTPONLY_ROUNDTRIP {found}"),
          Ok(Err(stage)) => eprintln!("COOKIE_API_FAILED {stage}"),
          Err(_) => eprintln!("COOKIE_API_TIMEOUT"),
        }
        // A bounded observation run; no credentials are entered by the harness.
        let (_tx, rx) = mpsc::channel::<()>();
        let _ = rx.recv_timeout(Duration::from_secs(90));
        app.exit(0);
      });
      Ok(())
    })
    .run(context)
    .expect("spike runtime failed");
}
