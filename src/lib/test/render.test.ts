// The render tier: every `<Component>.render.ts` suite, ONE process, ONE warm
// Vite SSR server (board #178 — seven files were seven processes, each paying
// the same ~2.5 s CPU of fixed cost and compiling the same svelte runtime; a
// busy host stretched each to 35–42 s against a 60 s budget and all seven
// timed out together). A suite is a plain module of `test()` calls that loads
// its component through `renderHarness()`; it lives beside its component and
// is collected here — `harness.source.test.ts` pins that no `*.render.test.ts`
// process creeps back and that every suite is on this list.
import '../hub/Board.render.ts';
import '../hub/Composer.render.ts';
import '../hub/Drawer.render.ts';
import '../hub/Feed.render.ts';
import '../hub/Roster.render.ts';
import '../hub/Sidebar.render.ts';
import '../files/FilePreview.render.ts';
