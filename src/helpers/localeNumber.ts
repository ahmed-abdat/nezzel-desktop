/**
 * Numbers in the UI always go through Intl with Latin digits (`-u-nu-latn`): Arabic users in
 * Mauritania and the Maghreb read 0-9, and a raw `ar` locale would print ٠١٢.
 */
function intlLocale(locale: string): string {
  return `${locale}-u-nu-latn`;
}

export function formatCount(value: number, locale: string): string {
  return new Intl.NumberFormat(intlLocale(locale), { maximumFractionDigits: 0 }).format(value);
}

const BYTE_UNITS = ['byte', 'kilobyte', 'megabyte', 'gigabyte', 'terabyte'] as const;

function scaleBytes(bytes: number): { value: number; unit: typeof BYTE_UNITS[number] } {
  let value = Math.max(0, bytes);
  let index = 0;
  while (value >= 1000 && index < BYTE_UNITS.length - 1) {
    value /= 1000;
    index++;
  }
  return { value, unit: BYTE_UNITS[index] };
}

/** Arabic CLDR "short" units are cryptic abbreviations (م.ب); use the full word ("ميغابايت"). */
const unitDisplayFor = (locale: string) => (locale.startsWith('ar') ? 'long' : 'short');

/** File size with the locale's own unit words ("412 MB", "412 ميغابايت"), Latin digits. */
export function formatSize(bytes: number, locale: string): string {
  const { value, unit } = scaleBytes(bytes);
  return new Intl.NumberFormat(intlLocale(locale), {
    style: 'unit', unit, unitDisplay: unitDisplayFor(locale), maximumFractionDigits: value < 10 && unit !== 'byte' ? 1 : 0,
  }).format(value);
}

/** Transfer speed, e.g. "21.4 MB/s" ("21.4 ميغابايت/ث": the long "per second" form is too wide for a row). */
export function formatSpeed(bytesPerSecond: number, locale: string): string {
  const { value, unit } = scaleBytes(bytesPerSecond);
  if (locale.startsWith('ar')) {
    return `${new Intl.NumberFormat(intlLocale(locale), { style: 'unit', unit, unitDisplay: 'long', maximumFractionDigits: 1 }).format(value)}/ث`;
  }
  return new Intl.NumberFormat(intlLocale(locale), {
    style: 'unit', unit: `${unit}-per-second`, unitDisplay: 'short', maximumFractionDigits: 1,
  }).format(value);
}

/** `value` is 0–100. */
export function formatPercent(value: number, locale: string, fractionDigits = 0): string {
  return new Intl.NumberFormat(intlLocale(locale), {
    style: 'percent',
    minimumFractionDigits: fractionDigits,
    maximumFractionDigits: fractionDigits,
  }).format(Math.max(0, Math.min(100, value)) / 100);
}
