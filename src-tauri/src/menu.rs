use crate::models::payloads::NavigatePayload;
use tauri::menu::{MenuBuilder, MenuItemBuilder, SubmenuBuilder};
use tauri::{AppHandle, Emitter};

pub fn setup_menu(handle: &AppHandle) {
  #[cfg(target_os = "macos")]
  {
    setup_macos(handle);
  }
  #[cfg(target_os = "windows")]
  {
    setup_windows(handle);
  }
  #[cfg(target_os = "linux")]
  {
    setup_linux(handle);
  }
}

#[allow(dead_code)]
fn setup_macos(handle: &AppHandle) {
  let settings_item = MenuItemBuilder::new("Settings…")
    .id("settings")
    .accelerator("CmdOrCtrl+,")
    .build(handle)
    .expect("Could not build settings item.");

  // Opens Settings → About in the app (owner: one About, no native panel).
  let about_item = MenuItemBuilder::new("About Nezzel")
    .id("about")
    .build(handle)
    .expect("Could not build about item.");

  let app_submenu = SubmenuBuilder::new(handle, "App")
    .item(&about_item)
    .separator()
    .item(&settings_item)
    .separator()
    .services()
    .separator()
    .hide()
    .hide_others()
    .separator()
    .quit()
    .build()
    .expect("Could not build app submenu.");

  let edit_submenu = SubmenuBuilder::new(handle, "Edit")
    .undo()
    .redo()
    .separator()
    .cut()
    .copy()
    .paste()
    .select_all()
    .build()
    .expect("Could not build edit submenu.");

  let menu = MenuBuilder::new(handle)
    .items(&[&app_submenu, &edit_submenu])
    .build()
    .expect("Could not attach submenus to menu.");

  handle.set_menu(menu).expect("Could not attach menu.");
  handle.on_menu_event(move |app, event| {
    let route = if event.id() == settings_item.id() {
      "settings.general"
    } else if event.id() == about_item.id() {
      "settings.about"
    } else {
      return;
    };
    crate::background::show_main(app);
    let _ = app.emit(
      "navigate",
      NavigatePayload {
        route: route.into(),
      },
    );
  });
}

#[allow(dead_code)]
fn setup_windows(handle: &AppHandle) {
  let menu = MenuBuilder::new(handle)
    .items(&[])
    .build()
    .expect("Could not attach submenus to menu.");

  handle.set_menu(menu).expect("Could not attach menu.");
}

#[allow(dead_code)]
fn setup_linux(handle: &AppHandle) {
  let menu = MenuBuilder::new(handle)
    .items(&[])
    .build()
    .expect("Could not attach submenus to menu.");

  handle.set_menu(menu).expect("Could not attach menu.");
}
