import test from 'node:test';
import assert from 'node:assert/strict';
import { UPLOAD_MAX_BYTES, UPLOAD_MAX_MB, uploadProgress, uploadSizeError, uploadSummary } from './file-upload.ts';

// Owner 2026-09-20 (board #214): "文件上传要有个进度或者提示，让我知道传上去了没有".
const s = {
  uploading: 'Uploading {name} ({index}/{total})', uploadReading: 'Reading', uploadSending: 'Sending',
  uploaded: 'Uploaded {name}', uploadedMany: 'Uploaded {count} files',
  uploadFailed: 'Upload failed: {names}', uploadPartial: '{ok} uploaded, {failed} failed: {names}',
};

test('reading shows real bytes; sending is a discrete beat with no invented percentage', () => {
  const step = { name: 'a.bin', index: 2, total: 3, phase: 'reading' as const, loaded: 25, size: 100 };
  assert.deepEqual(uploadProgress(step, '/w', s),
    { kind: 'progress', message: 'Uploading a.bin (2/3)', detail: 'Reading · /w', progress: 25 });
  // One fs_upload RPC carries the whole file: nothing observes the send.
  assert.deepEqual(uploadProgress({ ...step, phase: 'sending', loaded: 100 }, '/w', s),
    { kind: 'progress', message: 'Uploading a.bin (2/3)', detail: 'Sending · /w', progress: null });
  // Unknown size (a native path read whole) has no percentage to show either.
  assert.equal(uploadProgress({ ...step, size: 0 }, '/w', s).progress, null);
  assert.equal(uploadProgress({ ...step, loaded: 400 }, '/w', s).progress, 100, 'never past the end');
});

test('the closing line counts a clean batch and NAMES every failed file', () => {
  assert.deepEqual(uploadSummary({ ok: ['a.bin'], failed: [] }, '/w', s),
    { kind: 'success', message: 'Uploaded a.bin', detail: '/w' });
  assert.deepEqual(uploadSummary({ ok: ['a', 'b', 'c'], failed: [] }, '/w', s),
    { kind: 'success', message: 'Uploaded 3 files', detail: '/w' });
  assert.deepEqual(uploadSummary({ ok: ['a'], failed: [{ name: 'b', error: 'EACCES' }, { name: 'c', error: 'EIO' }] }, '/w', s),
    { kind: 'error', message: '1 uploaded, 2 failed: b, c', detail: 'EACCES' });
  assert.deepEqual(uploadSummary({ ok: [], failed: [{ name: 'b', error: 'cannot read: b' }] }, '/w', s),
    { kind: 'error', message: 'Upload failed: b', detail: 'cannot read: b' });
});

test('the size guard mirrors the server cap: 80 MB message ÷ 4/3 base64 = 60 MB of file, and says so', () => {
  assert.equal(UPLOAD_MAX_BYTES, 60 * 1024 * 1024);
  assert.equal(UPLOAD_MAX_MB, 60);
  assert.equal(uploadSizeError(UPLOAD_MAX_BYTES, 'max {mb} MB'), null, 'at the cap still goes');
  assert.equal(uploadSizeError(UPLOAD_MAX_BYTES + 1, 'max {mb} MB'), 'max 60 MB');
});
