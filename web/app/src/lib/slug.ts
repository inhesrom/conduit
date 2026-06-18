/** Mirror of protocol::branch_slug (crates/protocol/src/lib.rs): lowercase,
 * runs of non-alphanumerics collapse to a single dash, trimmed, max 50 chars.
 * Used for the create-workspace slug preview and to correlate the initial
 * prompt with the WorkspaceCreated event's slug. */
export function branchSlug(name: string): string {
  let out = "";
  let prevDash = false;
  for (const c of name.trim()) {
    if (/[a-z0-9]/i.test(c)) {
      out += c.toLowerCase();
      prevDash = false;
    } else if (out.length > 0 && !prevDash) {
      out += "-";
      prevDash = true;
    }
  }
  while (out.endsWith("-")) out = out.slice(0, -1);
  if (out.length > 50) {
    out = out.slice(0, 50);
    while (out.endsWith("-")) out = out.slice(0, -1);
  }
  return out;
}
