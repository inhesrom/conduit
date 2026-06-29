/** Minimal unified-diff parser for `git diff` / `git show` output — enough
 * for the TUI-parity diff pane (colored lines + line numbers), no rendering
 * framework attached. */

export interface DiffLine {
  kind: "context" | "add" | "del" | "meta";
  text: string;
  oldNo?: number;
  newNo?: number;
}

export interface DiffHunk {
  header: string;
  lines: DiffLine[];
}

export interface DiffFile {
  oldPath: string;
  newPath: string;
  hunks: DiffHunk[];
  isBinary: boolean;
  isRename: boolean;
}

const HUNK_RE = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/;

export function parseUnifiedDiff(text: string): DiffFile[] {
  const files: DiffFile[] = [];
  let file: DiffFile | null = null;
  let hunk: DiffHunk | null = null;
  let oldNo = 0;
  let newNo = 0;

  for (const line of text.split("\n")) {
    if (line.startsWith("diff --git ")) {
      file = { oldPath: "", newPath: "", hunks: [], isBinary: false, isRename: false };
      files.push(file);
      hunk = null;
      continue;
    }
    if (file === null) {
      // `git show <hash>:<file>` and friends emit raw content with no diff
      // header — treat the whole input as one context-only pseudo-file.
      file = { oldPath: "", newPath: "", hunks: [], isBinary: false, isRename: false };
      files.push(file);
      hunk = { header: "", lines: [] };
      file.hunks.push(hunk);
      oldNo = 1;
      newNo = 1;
    }

    const hunkMatch = HUNK_RE.exec(line);
    if (hunkMatch) {
      oldNo = parseInt(hunkMatch[1]!, 10);
      newNo = parseInt(hunkMatch[2]!, 10);
      hunk = { header: line, lines: [] };
      file.hunks.push(hunk);
      continue;
    }

    if (hunk === null) {
      if (line.startsWith("Binary files ")) file.isBinary = true;
      else if (line.startsWith("rename from ")) {
        file.isRename = true;
        file.oldPath = line.slice("rename from ".length);
      } else if (line.startsWith("rename to ")) file.newPath = line.slice("rename to ".length);
      else if (line.startsWith("--- ")) file.oldPath = stripPrefix(line.slice(4));
      else if (line.startsWith("+++ ")) file.newPath = stripPrefix(line.slice(4));
      continue;
    }

    if (line.startsWith("+")) {
      hunk.lines.push({ kind: "add", text: line.slice(1), newNo: newNo++ });
    } else if (line.startsWith("-")) {
      hunk.lines.push({ kind: "del", text: line.slice(1), oldNo: oldNo++ });
    } else if (line.startsWith("\\")) {
      hunk.lines.push({ kind: "meta", text: line });
    } else {
      hunk.lines.push({
        kind: "context",
        text: line.startsWith(" ") ? line.slice(1) : line,
        oldNo: oldNo++,
        newNo: newNo++,
      });
    }
  }

  // Drop the trailing empty context line produced by the final "\n".
  for (const f of files) {
    const last = f.hunks.at(-1)?.lines.at(-1);
    if (last && last.kind === "context" && last.text === "") f.hunks.at(-1)!.lines.pop();
  }
  return files;
}

function stripPrefix(path: string): string {
  if (path === "/dev/null") return path;
  return path.replace(/^[ab]\//, "");
}

/** The new-file line span covered by a selection, as a bare label ("40" or
 * "40–48"). Falls back to the old-file span for a pure-deletion selection,
 * flagged via `removed`. Empty range when nothing numbered is selected. */
export function diffLineRange(lines: DiffLine[]): { range: string; removed: boolean } {
  const newNos = lines.map((l) => l.newNo).filter((n): n is number => n != null);
  const removed = newNos.length === 0;
  const pool = removed ? lines.map((l) => l.oldNo).filter((n): n is number => n != null) : newNos;
  if (pool.length === 0) return { range: "", removed: false };
  const lo = Math.min(...pool);
  const hi = Math.max(...pool);
  return { range: lo === hi ? `${lo}` : `${lo}–${hi}`, removed };
}

/** Compose a Diff Question: a `file <range>` header, the literal selected lines
 * as a fenced block (each prefixed with its line number and +/-/space marker so
 * the agent sees what changed), then the user's question. New-file line numbers
 * locate the code in the live worktree; deleted lines keep their old numbers. */
export function buildDiffQuestion(file: string, lines: DiffLine[], question: string): string {
  const { range, removed } = diffLineRange(lines);
  const label = !range
    ? "selection"
    : `${removed ? "removed " : ""}line${range.includes("–") ? "s" : ""} ${range}`;
  const body = lines
    .map((l) => {
      const no = l.newNo ?? l.oldNo ?? "";
      const mark = l.kind === "add" ? "+" : l.kind === "del" ? "-" : " ";
      return `${String(no).padStart(4)} ${mark} ${l.text}`;
    })
    .join("\n");
  return `Re: ${file} ${label}\n\n\`\`\`\n${body}\n\`\`\`\n\n${question.trim()}`;
}
