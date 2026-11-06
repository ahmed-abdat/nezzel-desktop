import { PERMANENT_CODES } from '@/generated/contract';

const PERMANENT: ReadonlySet<string> = new Set(PERMANENT_CODES);

/** True when retrying cannot help until something on the site changes (Rust `extract::outcome::PERMANENT_CODES`). */
export function isPermanentError(code: string | undefined): boolean {
  return !!code && PERMANENT.has(code);
}
