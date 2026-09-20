// Upload feedback (board #214, owner 2026-09-20: "文件上传要有个进度或者提示，
// 让我知道传上去了没有"). One file is ONE atomic `fs_upload` RPC — its base64
// body rides in a single WebSocket message and nothing observes the send —
// so the only honest progress is per file: the LOCAL read (FileReader reports
// bytes) then a discrete "sending" beat that ends when the server answers.
// `uploaded` is the RPC resolving, never the send returning.

import type { FeedbackValue } from '../ui/feedback-lifetime.ts';

/** The server refuses a WebSocket message above 80 MB (`WS_MAX_MESSAGE_BYTES`,
 * connection.rs); base64 grows a file by 4/3, so ~60 MB of file is the most
 * one `fs_upload` can carry. Refusing here, up front, is a clear "too large"
 * instead of a 60 s request timeout that then tears the connection down. */
export const UPLOAD_MAX_BYTES = 60 * 1024 * 1024;
export const UPLOAD_MAX_MB = UPLOAD_MAX_BYTES / 1024 / 1024;

export type UploadPhase = 'reading' | 'sending';

export interface UploadStep {
  name: string;
  /** 1-based position in the batch. */
  index: number;
  total: number;
  phase: UploadPhase;
  /** Bytes of THIS file read so far; only meaningful while `reading`. */
  loaded: number;
  size: number;
}

export interface UploadOutcome {
  ok: string[];
  failed: { name: string; error: string }[];
}

export interface UploadStrings {
  uploading: string;      // 'Uploading {name} ({index}/{total})'
  uploadSending: string;  // 'Sending'
  uploadReading: string;  // 'Reading'
  uploaded: string;       // 'Uploaded {name}'
  uploadedMany: string;   // 'Uploaded {count} files'
  uploadFailed: string;   // 'Upload failed: {names}'
  uploadPartial: string;  // '{ok} uploaded, {failed} failed: {names}'
}

const fill = (template: string, values: Record<string, string | number>) =>
  template.replace(/\{(\w+)\}/g, (_, key: string) => String(values[key] ?? ''));

/** The in-flight line. Reading shows a real percentage (FileReader bytes);
 * sending shows the phase with an indeterminate spinner — a percentage there
 * would be invented. */
export function uploadProgress(step: UploadStep, dir: string, s: UploadStrings): FeedbackValue {
  const message = fill(s.uploading, { name: step.name, index: step.index, total: step.total });
  if (step.phase === 'reading' && step.size > 0) {
    return { kind: 'progress', message, detail: `${s.uploadReading} · ${dir}`, progress: Math.min(100, (step.loaded / step.size) * 100) };
  }
  return { kind: 'progress', message, detail: `${s.uploadSending} · ${dir}`, progress: null };
}

/** The closing line. Any failure names the files that failed (owner: know
 * WHICH one did not land); a clean batch says how many landed. */
export function uploadSummary(outcome: UploadOutcome, dir: string, s: UploadStrings): FeedbackValue {
  const names = outcome.failed.map((f) => f.name).join(', ');
  if (outcome.failed.length && outcome.ok.length) {
    return { kind: 'error', message: fill(s.uploadPartial, { ok: outcome.ok.length, failed: outcome.failed.length, names }), detail: outcome.failed[0]!.error };
  }
  if (outcome.failed.length) {
    return { kind: 'error', message: fill(s.uploadFailed, { names }), detail: outcome.failed[0]!.error };
  }
  if (outcome.ok.length === 1) return { kind: 'success', message: fill(s.uploaded, { name: outcome.ok[0]! }), detail: dir };
  return { kind: 'success', message: fill(s.uploadedMany, { count: outcome.ok.length }), detail: dir };
}

/** `null` when the file may go; otherwise the reason it may not. */
export function uploadSizeError(size: number, tooLarge: string): string | null {
  return size > UPLOAD_MAX_BYTES ? fill(tooLarge, { mb: UPLOAD_MAX_MB }) : null;
}
