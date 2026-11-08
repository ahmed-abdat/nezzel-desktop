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

/// Physical pixels. Called by close_to_tray's worker; COM never runs on Tauri's UI thread.
#[cfg(windows)]
pub(super) fn hidden_icons_rect() -> Option<Area> {
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
          CUIAutomation8, IUIAutomation2, IUIAutomationCondition, PropertyConditionFlags,
          PropertyConditionFlags_IgnoreCase, PropertyConditionFlags_MatchSubstring,
          TreeScope_Descendants, UIA_AutomationIdPropertyId, UIA_BoundingRectanglePropertyId,
          UIA_ClassNamePropertyId, UIA_IsOffscreenPropertyId, UIA_NamePropertyId,
        },
        WindowsAndMessaging::{FindWindowW, GetWindowRect},
      },
    },
  };

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
      (|| -> windows::core::Result<Option<Area>> {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        let _apartment = ComApartment;
        let automation: IUIAutomation2 =
          CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER)?;
        automation.SetConnectionTimeout(100)?;
        automation.SetTransactionTimeout(100)?;
        let deadline = Instant::now() + Duration::from_millis(350);
        let hwnd = FindWindowW(w!("Shell_TrayWnd"), None)?;
        let mut bar = RECT::default();
        GetWindowRect(hwnd, &mut bar)?;
        let area = |r: RECT| Area {
          x: f64::from(r.left),
          y: f64::from(r.top),
          w: f64::from(r.right) - f64::from(r.left),
          h: f64::from(r.bottom) - f64::from(r.top),
        };
        let root = automation.ElementFromHandle(hwnd)?;
        let cache = automation.CreateCacheRequest()?;
        cache.AddProperty(UIA_BoundingRectanglePropertyId)?;
        let visible =
          automation.CreatePropertyCondition(UIA_IsOffscreenPropertyId, &VARIANT::from(false))?;
        let flags = PropertyConditionFlags(
          PropertyConditionFlags_IgnoreCase.0 | PropertyConditionFlags_MatchSubstring.0,
        );
        // SystemTrayIcon alone is shared by clock/volume; match the chevron's class or specific ID.
        let identity = automation.CreateOrCondition(
          &automation.CreatePropertyConditionEx(
            UIA_ClassNamePropertyId,
            &VARIANT::from("Chevron"),
            flags,
          )?,
          &automation.CreatePropertyConditionEx(
            UIA_AutomationIdPropertyId,
            &VARIANT::from("Chevron"),
            flags,
          )?,
        )?;
        let find = |condition: &IUIAutomationCondition| {
          root
            .FindFirstBuildCache(TreeScope_Descendants, condition, &cache)
            .ok()
            .and_then(|element| element.CachedBoundingRectangle().ok())
            .map(area)
            .filter(|rect| contains(area(bar), *rect))
        };
        if let Some(rect) = find(&automation.CreateAndCondition(&identity, &visible)?) {
          return Ok(Some(rect));
        }
        if Instant::now() >= deadline {
          return Ok(None);
        }
        // Only a failed identity lookup permits the localized name fallback; never enumerate the tree.
        let mut names = automation.CreateFalseCondition()?;
        for name in [
          "Show hidden icons",
          "Notification Chevron",
          "NotificationChevron",
          "Afficher les icônes cachées",
          "Afficher les icônes masquées",
          "إظهار الأيقونات المخفية",
        ] {
          let name = automation.CreatePropertyConditionEx(
            UIA_NamePropertyId,
            &VARIANT::from(name),
            PropertyConditionFlags_IgnoreCase,
          )?;
          names = automation.CreateOrCondition(&names, &name)?;
        }
        Ok(find(&automation.CreateAndCondition(&names, &visible)?))
      })()
    }
    .ok()
    .flatten();
    let _ = send.send(found);
  });
  // A broken provider must not delay closing the app. Per-call UIA timeouts also bound the worker.
  receive
    .recv_timeout(Duration::from_millis(400))
    .ok()
    .flatten()
}

#[cfg(test)]
mod tests {
  use super::*;

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
