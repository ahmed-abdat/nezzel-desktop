//! Windows tray geometry and a best-effort, bounded Explorer UI Automation lookup.
use super::{Area, Edge};

/// The same physical anchor is consumed by the main window and note, which can have different DPIs.
pub(super) fn logical_point(point: (f64, f64), window_scale: f64) -> (f64, f64) {
  (point.0 / window_scale, point.1 / window_scale)
}

/// Last estimate in physical pixels, mirrored for the taskbar's layout direction.
pub(super) fn fallback_anchor(
  screen: Area,
  work: Area,
  monitor_scale: f64,
  rtl: bool,
) -> (f64, f64, Edge) {
  let bar_top = work.y + work.h;
  let bar_h = (screen.y + screen.h - bar_top).max(40. * monitor_scale);
  let offset = 150. * monitor_scale;
  (
    if rtl {
      screen.x + offset
    } else {
      screen.x + screen.w - offset
    },
    bar_top + bar_h / 2.,
    Edge::Bottom,
  )
}

fn contains(outer: Area, inner: Area) -> bool {
  inner.w > 0.
    && inner.h > 0.
    && inner.x >= outer.x
    && inner.y >= outer.y
    && inner.x + inner.w <= outer.x + outer.w
    && inner.y + inner.h <= outer.y + outer.h
}

// Explorer's accessible names, not app copy. Match the noun phrase so verb/case changes work.
const HIDDEN_ICONS_NAMES: &[&str] = &[
  "hidden icons",
  "notification chevron",
  "notificationchevron",
  "icônes cachées",
  "icônes masquées",
  "الأيقونات المخفية",
  "الأيقونات المستترة",
  "الرموز المخفية",
];

/// SystemTrayIcon IDs and SystemTray.NormalButton classes are shared with clock/volume buttons.
/// Only a specific Chevron identity or a localized hidden-icons button name is safe to anchor to.
fn hidden_icons_source(id: &str, class: &str, name: &str, button: bool) -> Option<&'static str> {
  let name = name.to_lowercase();
  if id.to_lowercase().contains("chevron") || class.to_lowercase().contains("chevron") {
    Some("uia_identity")
  } else if button
    && HIDDEN_ICONS_NAMES
      .iter()
      .any(|fragment| name.contains(fragment))
  {
    Some("uia_name")
  } else {
    None
  }
}

/// Use the actual taskbar bounds, including an auto-hide taskbar overlaid on the work area.
pub(super) fn taskbar_anchor(screen: Area, taskbar: Area, rect: Area) -> Option<(f64, f64, Edge)> {
  if !contains(screen, rect) || !contains(taskbar, rect) {
    return None;
  }
  let x = rect.x + rect.w / 2.;
  let y = rect.y + rect.h / 2.;
  Some((
    x,
    y,
    if y < screen.y + screen.h / 2. {
      Edge::Top
    } else {
      Edge::Bottom
    },
  ))
}

#[cfg(windows)]
pub(super) fn taskbar_rect() -> Option<Area> {
  use windows::{
    core::w,
    Win32::{
      Foundation::RECT,
      UI::WindowsAndMessaging::{FindWindowW, GetWindowRect},
    },
  };
  // SAFETY: class name is NUL-terminated and the output rectangle is a valid stack allocation.
  unsafe {
    let hwnd = FindWindowW(w!("Shell_TrayWnd"), None).ok()?;
    let mut rect = RECT::default();
    GetWindowRect(hwnd, &mut rect).ok()?;
    Some(Area {
      x: f64::from(rect.left),
      y: f64::from(rect.top),
      w: f64::from(rect.right) - f64::from(rect.left),
      h: f64::from(rect.bottom) - f64::from(rect.top),
    })
  }
}

#[cfg(windows)]
pub(super) fn taskbar_is_rtl() -> bool {
  use windows::{
    core::w,
    Win32::UI::WindowsAndMessaging::{FindWindowW, GetWindowLongW, GWL_EXSTYLE, WS_EX_LAYOUTRTL},
  };
  // SAFETY: class name is NUL-terminated; the returned handle is only used to read its styles.
  unsafe {
    FindWindowW(w!("Shell_TrayWnd"), None)
      .ok()
      .is_some_and(|hwnd| (GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_LAYOUTRTL.0) != 0)
  }
}

/// Physical pixels and the matching path. COM never runs on Tauri's UI thread.
#[cfg(windows)]
pub(super) fn hidden_icons_rect() -> Option<(Area, &'static str)> {
  use std::{
    sync::mpsc,
    time::{Duration, Instant},
  };
  use windows::{
    core::w,
    Win32::{
      Foundation::RECT,
      System::{
        Com::{
          CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
          COINIT_MULTITHREADED,
        },
        Variant::VARIANT,
      },
      UI::{
        Accessibility::{
          CUIAutomation8, IUIAutomation2, PropertyConditionFlags,
          PropertyConditionFlags_IgnoreCase, PropertyConditionFlags_MatchSubstring,
          TreeScope_Descendants, UIA_AutomationIdPropertyId, UIA_BoundingRectanglePropertyId,
          UIA_ButtonControlTypeId, UIA_ClassNamePropertyId, UIA_ControlTypePropertyId,
          UIA_IsOffscreenPropertyId, UIA_NamePropertyId,
        },
        WindowsAndMessaging::{FindWindowW, GetWindowRect},
      },
    },
  };

  // Cold Explorer/XAML providers can take >1 s. The close worker waits before showing the note;
  // the receiver remains bounded even if COM initialization or a provider ignores its timeout.
  const WAIT: Duration = Duration::from_millis(1500);
  let started = Instant::now();
  macro_rules! uia {
    ($stage:literal, $call:expr) => {
      $call.inspect_err(|error| {
        tracing::info!(
          stage = $stage,
          hresult = format_args!("{:#010x}", error.code().0 as u32),
          elapsed_ms = started.elapsed().as_millis() as u64,
          "tray anchor UIA error"
        );
      })?
    };
  }
  struct ComApartment;
  impl Drop for ComApartment {
    fn drop(&mut self) {
      // SAFETY: created only after a successful CoInitializeEx on this same worker.
      unsafe { CoUninitialize() };
    }
  }
  let (send, receive) = mpsc::sync_channel(1);
  std::thread::spawn(move || {
    // SAFETY: COM objects stay on this MTA worker and drop before its apartment is uninitialized.
    let found = unsafe {
      (|| -> windows::core::Result<Option<(Area, &'static str)>> {
        uia!(
          "CoInitializeEx",
          CoInitializeEx(None, COINIT_MULTITHREADED).ok()
        );
        let _apartment = ComApartment;
        let automation: IUIAutomation2 = uia!(
          "CoCreateInstance",
          CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER)
        );
        uia!(
          "SetConnectionTimeout",
          automation.SetConnectionTimeout(WAIT.as_millis() as u32)
        );
        uia!(
          "SetTransactionTimeout",
          automation.SetTransactionTimeout(WAIT.as_millis() as u32)
        );
        let hwnd = uia!("FindWindowW", FindWindowW(w!("Shell_TrayWnd"), None));
        let mut bar = RECT::default();
        uia!("GetWindowRect", GetWindowRect(hwnd, &mut bar));
        let area = |r: RECT| Area {
          x: f64::from(r.left),
          y: f64::from(r.top),
          w: f64::from(r.right) - f64::from(r.left),
          h: f64::from(r.bottom) - f64::from(r.top),
        };
        tracing::info!(taskbar_rect = ?area(bar), "tray anchor UIA taskbar");
        // Descendants crosses hosted UIA fragments, including Win11's
        // Windows.UI.Composition.DesktopWindowContentBridge child of Shell_TrayWnd.
        // Do not restrict this to immediate children or the legacy TrayNotifyWnd subtree.
        let root = uia!("ElementFromHandle", automation.ElementFromHandle(hwnd));
        let cache = uia!("CreateCacheRequest", automation.CreateCacheRequest());
        uia!(
          "SetTreeFilter",
          cache.SetTreeFilter(&uia!(
            "CreateTrueCondition",
            automation.CreateTrueCondition()
          ))
        );
        for property in [
          UIA_BoundingRectanglePropertyId,
          UIA_AutomationIdPropertyId,
          UIA_ClassNamePropertyId,
          UIA_NamePropertyId,
          UIA_ControlTypePropertyId,
        ] {
          uia!("AddProperty", cache.AddProperty(property));
        }
        let visible = uia!(
          "visible condition",
          automation.CreatePropertyCondition(UIA_IsOffscreenPropertyId, &VARIANT::from(false))
        );
        let flags = PropertyConditionFlags(
          PropertyConditionFlags_IgnoreCase.0 | PropertyConditionFlags_MatchSubstring.0,
        );
        let class = uia!(
          "class condition",
          automation.CreatePropertyConditionEx(
            UIA_ClassNamePropertyId,
            &VARIANT::from("Chevron"),
            flags
          )
        );
        let id = uia!(
          "identity condition",
          automation.CreatePropertyConditionEx(
            UIA_AutomationIdPropertyId,
            &VARIANT::from("Chevron"),
            flags
          )
        );
        let identity = uia!("identity OR", automation.CreateOrCondition(&class, &id));
        let mut names = uia!("CreateFalseCondition", automation.CreateFalseCondition());
        for name in HIDDEN_ICONS_NAMES {
          let name = uia!(
            "name condition",
            automation.CreatePropertyConditionEx(UIA_NamePropertyId, &VARIANT::from(*name), flags)
          );
          names = uia!("names OR", automation.CreateOrCondition(&names, &name));
        }
        // 24H2's button may have a generic SystemTrayIcon-style ID and
        // SystemTray.NormalButton class. Its localized name disambiguates clock/volume.
        let button = uia!(
          "button condition",
          automation.CreatePropertyCondition(
            UIA_ControlTypePropertyId,
            &VARIANT::from(UIA_ButtonControlTypeId.0)
          )
        );
        let named_button = uia!(
          "named button AND",
          automation.CreateAndCondition(&names, &button)
        );
        let candidate = uia!(
          "candidate OR",
          automation.CreateOrCondition(&identity, &named_button)
        );
        let condition = uia!(
          "visible AND",
          automation.CreateAndCondition(&candidate, &visible)
        );
        let remaining = WAIT.saturating_sub(started.elapsed());
        if remaining.is_zero() {
          return Ok(None);
        }
        uia!(
          "search timeout",
          automation.SetTransactionTimeout(remaining.as_millis().max(1) as u32)
        );
        // One search covers both identities and localized names within the same cold-start budget.
        let element = uia!(
          "FindFirstBuildCache",
          root.FindFirstBuildCache(TreeScope_Descendants, &condition, &cache)
        );
        let rect = area(uia!(
          "CachedBoundingRectangle",
          element.CachedBoundingRectangle()
        ));
        let id = uia!("CachedAutomationId", element.CachedAutomationId());
        let class = uia!("CachedClassName", element.CachedClassName());
        let name = uia!("CachedName", element.CachedName());
        let button =
          uia!("CachedControlType", element.CachedControlType()) == UIA_ButtonControlTypeId;
        let source = hidden_icons_source(
          &id.to_string(),
          &class.to_string(),
          &name.to_string(),
          button,
        );
        let accepted = contains(area(bar), rect) && source.is_some();
        // Never log provider strings: only their classification and physical geometry.
        tracing::info!(
          source = source.unwrap_or("unmatched"),
          candidate_rect = ?rect,
          taskbar_rect = ?area(bar),
          accepted,
          elapsed_ms = started.elapsed().as_millis() as u64,
          "tray anchor UIA candidate"
        );
        Ok(source.filter(|_| accepted).map(|source| (rect, source)))
      })()
    }
    .unwrap_or_default();
    tracing::info!(
      found = found.is_some(),
      elapsed_ms = started.elapsed().as_millis() as u64,
      "tray anchor UIA finished"
    );
    let _ = send.send(found);
  });
  match receive.recv_timeout(WAIT) {
    Ok(found) => found,
    Err(error) => {
      tracing::info!(
        timeout = matches!(error, mpsc::RecvTimeoutError::Timeout),
        elapsed_ms = started.elapsed().as_millis() as u64,
        "tray anchor UIA unavailable"
      );
      None
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn xaml_hidden_icons_button_matches_localized_substrings_without_matching_other_tray_buttons() {
    for name in [
      "Show Hidden Icons",
      "SHOW HIDDEN ICONS (3)",
      "Afficher les icônes cachées (3)",
      "AFFICHER LES ICÔNES MASQUÉES",
      "إظهار الأيقونات المخفية (3)",
      "إظهار الرموز المخفية",
    ] {
      assert_eq!(
        hidden_icons_source("SystemTrayIcon", "SystemTray.NormalButton", name, true),
        Some("uia_name"),
        "{name}"
      );
    }
    for (name, button) in [("Volume", true), ("Clock", true), ("Hidden icons", false)] {
      assert_eq!(
        hidden_icons_source("SystemTrayIcon", "SystemTray.NormalButton", name, button),
        None,
        "{name}"
      );
    }
    for (id, class) in [
      ("SystemTrayIcon.Chevron", "SystemTray.NormalButton"),
      ("", "Chevron"),
    ] {
      assert_eq!(
        hidden_icons_source(id, class, "", true),
        Some("uia_identity")
      );
    }
  }

  #[test]
  fn mixed_dpi_consumers_reach_the_same_physical_anchor() {
    let physical = (1560., 1020.);
    let main = logical_point(physical, 1.5);
    let note = logical_point(physical, 2.);
    assert_eq!(main, (1040., 680.));
    assert_eq!(note, (780., 510.));
    assert_eq!((main.0 * 1.5, main.1 * 1.5), physical);
    assert_eq!((note.0 * 2., note.1 * 2.), physical);
  }

  #[test]
  fn fallback_mirrors_the_physical_taskbar_offset_in_rtl() {
    let screen = Area {
      x: -2560.,
      y: 0.,
      w: 2560.,
      h: 1440.,
    };
    let work = Area { h: 1360., ..screen };
    assert_eq!(
      fallback_anchor(screen, work, 2., false),
      (-300., 1400., Edge::Bottom)
    );
    assert_eq!(
      fallback_anchor(screen, work, 2., true),
      (-2260., 1400., Edge::Bottom)
    );
  }

  #[test]
  fn pinned_icon_is_centered_but_overflow_and_stale_rects_are_rejected() {
    let screen = Area {
      x: -1280.,
      y: 0.,
      w: 1280.,
      h: 720.,
    };
    let bar = Area {
      y: 680.,
      h: 40.,
      ..screen
    };
    let icon = Area {
      x: -80.,
      y: 686.,
      w: 24.,
      h: 24.,
    };
    assert_eq!(
      taskbar_anchor(screen, bar, icon),
      Some((-68., 698., Edge::Bottom))
    );
    assert_eq!(taskbar_anchor(screen, bar, Area { y: 600., ..icon }), None);
    assert_eq!(taskbar_anchor(screen, bar, Area { x: 10., ..icon }), None);
    assert_eq!(taskbar_anchor(screen, bar, Area { w: 0., ..icon }), None);
  }

  #[test]
  fn top_taskbar_uses_the_top_edge() {
    let screen = Area {
      x: 0.,
      y: 0.,
      w: 1280.,
      h: 720.,
    };
    let bar = Area { h: 40., ..screen };
    let icon = Area {
      x: 1200.,
      y: 8.,
      w: 24.,
      h: 24.,
    };
    assert_eq!(
      taskbar_anchor(screen, bar, icon),
      Some((1212., 20., Edge::Top))
    );
  }
}
