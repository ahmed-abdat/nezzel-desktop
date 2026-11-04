import { CalendarDate, parseDate } from '@internationalized/date';

const ISO_DATE = /^\d{4}-\d{2}-\d{2}$/;

/** `YYYY-MM-DD` (what the filter stores) → calendar date; anything else → undefined. */
export function isoToCalendarDate(value: string | null | undefined): CalendarDate | undefined {
  if (!value || !ISO_DATE.test(value)) return undefined;
  try {
    return parseDate(value);
  } catch {
    return undefined;
  }
}

/** Calendar date → `YYYY-MM-DD`, the same string a native `<input type="date">` produced. */
export function calendarDateToIso(value: { year: number; month: number; day: number } | null | undefined): string | null {
  if (!value) return null;
  return new CalendarDate(value.year, value.month, value.day).toString();
}

/** App locale → Intl locale with Latin digits (same rule as helpers/localeNumber). */
export function dateLocale(locale: string): string {
  return `${locale}-u-nu-latn`;
}
