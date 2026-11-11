/** Live entries use the same separator as the saved Rust log, even without a trailing newline. */
export function appendLogLine(current: string, line: string): string {
  return current + (current && !current.endsWith('\n') ? '\n' : '') + line;
}
