import test from 'node:test';
import assert from 'node:assert/strict';
import { JSDOM } from 'jsdom';
import { activeModal } from './modal.ts';

test('modal ownership follows live visibility and DOM order, not opacity or a copied flag', () => {
  const dom = new JSDOM('<main><section id="page"><div id="first" aria-modal="true"></div></section></main>');
  try {
    const { document } = dom.window;
    const first = document.getElementById('first')!;
    assert.equal(activeModal(document), first);
    first.style.opacity = '0';
    assert.equal(activeModal(document), first, 'an entering modal owns keys before it paints');
    document.getElementById('page')!.style.visibility = 'hidden';
    assert.equal(activeModal(document), null);
    document.getElementById('page')!.style.visibility = 'visible';
    const second = document.createElement('div');
    second.setAttribute('aria-modal', 'true');
    document.body.append(second);
    assert.equal(activeModal(document), second);
    second.hidden = true;
    assert.equal(activeModal(document), first);
    document.getElementById('page')!.style.display = 'none';
    assert.equal(activeModal(document), null, 'a retained hidden page cannot claim the active page keys');
  } finally { dom.window.close(); }
});
