//! System regional date/clock preferences, independent of the WebView language.
use chrono::{DateTime, Local};

pub fn format_local(time: &str, locale: Option<&str>) -> String {
    let Ok(time) = DateTime::parse_from_rfc3339(time) else {
        return String::new();
    };
    let local = time.with_timezone(&Local);
    platform(&local, locale).unwrap_or_else(|| local.format("%Y-%m-%d %H:%M").to_string())
}

#[cfg(not(any(windows, target_os = "macos")))]
fn platform(_: &DateTime<Local>, _: Option<&str>) -> Option<String> {
    None
}

#[cfg(windows)]
fn platform(time: &DateTime<Local>, locale: Option<&str>) -> Option<String> {
    use chrono::{Datelike, Timelike};
    use windows_sys::Win32::{
        Foundation::SYSTEMTIME,
        Globalization::{GetDateFormatEx, GetTimeFormatEx, DATE_SHORTDATE, TIME_NOSECONDS},
    };
    let locale = locale.map(|s| s.encode_utf16().chain(Some(0)).collect::<Vec<_>>());
    let name = locale.as_ref().map_or(std::ptr::null(), |s| s.as_ptr());
    let time = SYSTEMTIME {
        wYear: time.year() as u16,
        wMonth: time.month() as u16,
        wDayOfWeek: time.weekday().num_days_from_sunday() as u16,
        wDay: time.day() as u16,
        wHour: time.hour() as u16,
        wMinute: time.minute() as u16,
        wSecond: time.second() as u16,
        wMilliseconds: 0,
    };
    // Null locale selects the user default; omit LOCALE_NOUSEROVERRIDE so
    // regional customisations (including the hour cycle) remain effective.
    unsafe {
        let date_len = GetDateFormatEx(
            name,
            DATE_SHORTDATE,
            &time,
            std::ptr::null(),
            std::ptr::null_mut(),
            0,
            std::ptr::null(),
        );
        let clock_len = GetTimeFormatEx(
            name,
            TIME_NOSECONDS,
            &time,
            std::ptr::null(),
            std::ptr::null_mut(),
            0,
        );
        if date_len <= 0 || clock_len <= 0 {
            return None;
        }
        let mut date = vec![0; date_len as usize];
        let mut clock = vec![0; clock_len as usize];
        if GetDateFormatEx(
            name,
            DATE_SHORTDATE,
            &time,
            std::ptr::null(),
            date.as_mut_ptr(),
            date_len,
            std::ptr::null(),
        ) == 0
            || GetTimeFormatEx(
                name,
                TIME_NOSECONDS,
                &time,
                std::ptr::null(),
                clock.as_mut_ptr(),
                clock_len,
            ) == 0
        {
            return None;
        }
        Some(format!(
            "{} {}",
            String::from_utf16_lossy(&date[..date.len() - 1]),
            String::from_utf16_lossy(&clock[..clock.len() - 1])
        ))
    }
}

#[cfg(target_os = "macos")]
fn platform(time: &DateTime<Local>, locale: Option<&str>) -> Option<String> {
    use core_foundation_sys::{base::*, date_formatter::*, locale::*, string::*};
    // Own every Create/Copy result, including early-return paths.
    struct Owned(CFTypeRef);
    impl Drop for Owned {
        fn drop(&mut self) {
            unsafe { CFRelease(self.0) }
        }
    }
    fn own(pointer: CFTypeRef) -> Option<Owned> {
        (!pointer.is_null()).then_some(Owned(pointer))
    }
    unsafe {
        let locale = if let Some(name) = locale {
            let name = std::ffi::CString::new(name).ok()?;
            let id = own(CFStringCreateWithCString(
                std::ptr::null(),
                name.as_ptr(),
                kCFStringEncodingUTF8,
            )
            .cast())?;
            own(CFLocaleCreate(std::ptr::null(), id.0.cast()).cast())?
        } else {
            own(CFLocaleCopyCurrent().cast())?
        };
        let formatter = own(CFDateFormatterCreate(
            std::ptr::null(),
            locale.0.cast(),
            kCFDateFormatterShortStyle,
            kCFDateFormatterShortStyle,
        )
        .cast())?;
        // CF accepts an absolute instant (epoch 2001-01-01) and converts it to
        // its default local timezone. Do not apply the local offset twice.
        let absolute = time.timestamp_millis() as f64 / 1000.0 - 978_307_200.0;
        let text = own(CFDateFormatterCreateStringWithAbsoluteTime(
            std::ptr::null(),
            formatter.0.cast_mut().cast(),
            absolute,
        )
        .cast())?;
        let length = CFStringGetMaximumSizeForEncoding(
            CFStringGetLength(text.0.cast()),
            kCFStringEncodingUTF8,
        ) + 1;
        let mut buffer = vec![0u8; length as usize];
        if CFStringGetCString(
            text.0.cast(),
            buffer.as_mut_ptr().cast(),
            length,
            kCFStringEncodingUTF8,
        ) == 0
        {
            return None;
        }
        Some(
            std::ffi::CStr::from_ptr(buffer.as_ptr().cast())
                .to_string_lossy()
                .into_owned(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    #[test]
    fn date_order_hour_cycle_and_no_seconds() {
        let time = Local
            .with_ymd_and_hms(2026, 11, 4, 15, 4, 37)
            .single()
            .unwrap()
            .to_rfc3339();
        for locale in ["en-US", "en-GB"] {
            let text = format_local(&time, Some(locale));
            #[cfg(any(windows, target_os = "macos"))]
            {
                let numbers: Vec<_> = text
                    .split(|c: char| !c.is_ascii_digit())
                    .filter(|s| !s.is_empty())
                    .collect();
                let (first, second) = if locale == "en-US" { (11, 4) } else { (4, 11) };
                assert_eq!(numbers[0].parse::<u32>().unwrap(), first, "{text}");
                assert_eq!(numbers[1].parse::<u32>().unwrap(), second, "{text}");
                if locale == "en-US" {
                    assert!(
                        text.contains("3:04") && text.to_uppercase().contains("PM"),
                        "{text}"
                    );
                    assert!(!text.contains("15:04"), "{text}");
                } else {
                    assert!(text.contains("15:04"), "{text}");
                    assert!(!text.to_uppercase().contains("PM"), "{text}");
                }
                assert!(!text.contains(":37"), "{text}");
                assert_eq!(text.matches(':').count(), 1, "{text}");
            }
            #[cfg(not(any(windows, target_os = "macos")))]
            assert_eq!(text, "2026-11-04 15:04");
        }
        assert_eq!(format_local("invalid", None), "");
    }
}
