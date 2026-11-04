import { describe, expect, it } from 'vitest';
import { calendarDateToIso, isoToCalendarDate } from '../../src/components/ui/date-picker/value';

describe('date picker value', () => {
  it.each(['2026-10-28', '2024-02-29', '1999-01-01', '2026-12-31'])('round-trips %s unchanged', (iso) => {
    expect(calendarDateToIso(isoToCalendarDate(iso))).toBe(iso);
  });

  it.each([null, '', '2026/28/10', '28-10-2026', '2026-13-01', 'nope'])('treats %j as empty', (value) => {
    expect(calendarDateToIso(isoToCalendarDate(value))).toBeNull();
  });
});
