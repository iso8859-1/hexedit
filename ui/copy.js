// Small "copy to clipboard" button, shown on the right of a value cell while its row is hovered.

const COPY_ICON =
  '<svg viewBox="0 0 16 16" aria-hidden="true"><rect x="5.5" y="5.5" width="8" height="9" rx="1.5"/>' +
  '<path d="M3.5 10.5h-.5a1.5 1.5 0 0 1-1.5-1.5V3A1.5 1.5 0 0 1 3 1.5h5A1.5 1.5 0 0 1 9.5 3v.5"/></svg>';
const DONE_ICON = '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M3 8.5l3.2 3L13 4.5"/></svg>';

/** Adds a copy button to `cell` (which gets the `has-copy` class) that copies `text`. */
export function addCopyButton(cell, text, onError) {
  const btn = document.createElement('button');
  btn.type = 'button';
  btn.className = 'copy';
  btn.title = 'Copy value';
  btn.setAttribute('aria-label', 'Copy value');
  btn.innerHTML = COPY_ICON;
  let timer;
  btn.addEventListener('click', async (e) => {
    // rows react to clicks (select bytes) and double-clicks (edit, expand)
    e.stopPropagation();
    try {
      await navigator.clipboard.writeText(text);
      btn.innerHTML = DONE_ICON;
      btn.classList.add('done');
      clearTimeout(timer);
      timer = setTimeout(() => {
        btn.innerHTML = COPY_ICON;
        btn.classList.remove('done');
      }, 1000);
    } catch (err) {
      onError?.(`Copy failed: ${err}`);
    }
  });
  btn.addEventListener('dblclick', (e) => e.stopPropagation());
  btn.addEventListener('mousedown', (e) => e.preventDefault()); // keep the focus where it is
  cell.classList.add('has-copy');
  cell.append(btn);
}
