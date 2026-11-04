/** Deep-assigns plain-object `partial` into `target` in place (arrays and values replace). */
export function mergeInto<T extends object>(target: T, partial: object): T {
  for (const [key, value] of Object.entries(partial)) {
    const current = (target as Record<string, unknown>)[key];
    if (value && typeof value === 'object' && !Array.isArray(value) && current && typeof current === 'object' && !Array.isArray(current)) {
      mergeInto(current as object, value as object);
    } else {
      (target as Record<string, unknown>)[key] = value;
    }
  }
  return target;
}

const isPlainObject = (value: unknown): value is Record<string, unknown> => !!value && typeof value === 'object' && !Array.isArray(value);
const sameValue = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);

/**
 * Undoes one failed optimistic `mergeInto`, field by field: each leaf `partial` wrote goes back to
 * its value in `before`, but only while it still holds what this patch wrote. A newer patch to the
 * same field, or patches to other fields, are left alone.
 */
export function rollbackInto(target: object, partial: object, before: object) {
  for (const [key, value] of Object.entries(partial)) {
    const t = target as Record<string, unknown>;
    const b = (before as Record<string, unknown>)[key];
    if (isPlainObject(value) && isPlainObject(t[key])) {
      rollbackInto(t[key] as object, value, isPlainObject(b) ? b : {});
    } else if (sameValue(t[key], value)) {
      t[key] = structuredClone(b);
    }
  }
}
