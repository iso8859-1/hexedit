// Thin wrappers around the Tauri commands.

const { invoke } = window.__TAURI__.core;

export const api = {
  docInfo: () => invoke('doc_info'),
  openFile: (path = null) => invoke('open_file', { path }),
  newFile: () => invoke('new_file'),
  saveFile: (saveAs, backup) => invoke('save_file', { saveAs, backup }),
  readBytes: async (offset, len) => new Uint8Array(await invoke('read_bytes', { offset, len })),
  writeBytes: (offset, bytes, insert, coalesce) =>
    invoke('write_bytes', { offset, bytes: Array.from(bytes), insert, coalesce }),
  deleteBytes: (offset, len) => invoke('delete_bytes', { offset, len }),
  undo: () => invoke('undo'),
  redo: () => invoke('redo'),
  find: (query, mode, encoding, littleEndian, from, backwards) =>
    invoke('find', { query, mode, encoding, littleEndian, from, backwards }),
  inspect: (offset, selLen, littleEndian, encoding) =>
    invoke('inspect', { offset, selLen, littleEndian, encoding }),
  inspectorWrite: (offset, key, text, littleEndian) =>
    invoke('inspector_write', { offset, key, text, littleEndian }),
  encodingList: () => invoke('encoding_list'),
  charTable: (encoding) => invoke('char_table', { encoding }),
  templateParse: (source) => invoke('template_parse', { source }),
  templateApply: (source, root, offset, littleEndian, encoding) =>
    invoke('template_apply', { source, root, offset, littleEndian, encoding }),
  templateOpen: () => invoke('template_open'),
  templateSave: (path, text) => invoke('template_save', { path, text }),
  templateExamples: () => invoke('template_examples'),
};

export function listen(event, fn) {
  return window.__TAURI__.event.listen(event, fn);
}
