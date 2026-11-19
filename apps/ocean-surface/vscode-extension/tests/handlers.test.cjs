const assert = require('node:assert/strict');
const { test } = require('node:test');
const { EventEmitter } = require('node:events');
const { readFileSync } = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const ts = require('typescript');

// Execute the production handlers with a minimal editor/provider seam.
function loadHandler(name, vscode, overrides = {}) {
  const filename = path.join(__dirname, '../src/handlers', `${name}.ts`);
  const output = ts.transpileModule(readFileSync(filename, 'utf8'), {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
    fileName: filename,
  }).outputText;
  const module = { exports: {} };
  const requireMock = (id) => {
    if (id === 'vscode') return vscode;
    if (id === '../logger') return { log() {}, logError() {} };
    return overrides[id] ?? require(id);
  };
  vm.runInThisContext(`(function(require, module, exports) {${output}\n})`, { filename })(requireMock, module, module.exports);
  return module.exports;
}

class FileSystemError extends Error {
  constructor(code) { super(code); this.code = code; }
}
function editor(options = {}) {
  const calls = { writes: [], edits: [], shown: [], terminalText: [] };
  const doc = {
    uri: { fsPath: '/file.txt' }, getText: () => 'original',
    lineCount: 1, lineAt: () => ({ text: 'original' }),
    save: async () => options.saved ?? true,
  };
  const vscode = {
    FileSystemError,
    Uri: { file: (fsPath) => ({ fsPath }) },
    Range: class { constructor(...args) { this.args = args; } },
    WorkspaceEdit: class { replace(...args) { calls.edits.push(args); } },
    EventEmitter: class { event() {} fire(text) { calls.terminalText.push(text); } },
    workspace: {
      textDocuments: options.open ? [doc] : [],
      applyEdit: async () => true,
      openTextDocument: async () => doc,
      fs: {
        readFile: async () => { if (options.readError) throw options.readError; return Buffer.from('original'); },
        writeFile: async (...args) => calls.writes.push(args),
      },
    },
    window: { showTextDocument: async (...args) => calls.shown.push(args), createTerminal() {} },
  };
  return { vscode, calls };
}

test('open-document save refusal never emits a persisted edit', async () => {
  const { vscode, calls } = editor({ open: true, saved: false });
  const applied = [];
  const { FileSystemHandler } = loadHandler('filesystem', vscode);
  await assert.rejects(new FileSystemHandler((edit) => applied.push(edit)).writeTextFile({ path: '/file.txt', content: 'replacement' }), /could not save/);
  assert.equal(calls.edits.length, 1);
  assert.deepEqual(applied, []);
  assert.deepEqual(calls.shown, []);
});

test('successful open-document save retains the original text', async () => {
  const { vscode } = editor({ open: true });
  const applied = [];
  const { FileSystemHandler } = loadHandler('filesystem', vscode);
  await new FileSystemHandler((edit) => applied.push(edit)).writeTextFile({ path: '/file.txt', content: 'replacement' });
  assert.deepEqual(applied, [{ path: '/file.txt', oldText: 'original', newText: 'replacement', created: false, saved: true }]);
});

for (const code of ['NoPermissions', 'Unavailable', 'Unknown']) {
  test(`existing-file ${code} read failure prevents overwrite`, async () => {
    const readError = new FileSystemError(code);
    const { vscode, calls } = editor({ readError });
    const applied = [];
    const { FileSystemHandler } = loadHandler('filesystem', vscode);
    await assert.rejects(new FileSystemHandler((edit) => applied.push(edit)).writeTextFile({ path: '/file.txt', content: 'replacement' }), (error) => error === readError);
    assert.deepEqual(calls.writes, []);
    assert.deepEqual(applied, []);
  });
}

test('known missing file can still be created', async () => {
  const { vscode, calls } = editor({ readError: new FileSystemError('FileNotFound') });
  const applied = [];
  const { FileSystemHandler } = loadHandler('filesystem', vscode);
  await new FileSystemHandler((edit) => applied.push(edit)).writeTextFile({ path: '/file.txt', content: 'new' });
  assert.equal(calls.writes.length, 1);
  assert.deepEqual(applied, [{ path: '/file.txt', oldText: null, newText: 'new', created: true, saved: true }]);
});

test('existing closed file captures original bytes before overwrite', async () => {
  const { vscode, calls } = editor();
  const applied = [];
  const { FileSystemHandler } = loadHandler('filesystem', vscode);
  await new FileSystemHandler((edit) => applied.push(edit)).writeTextFile({ path: '/file.txt', content: 'replacement' });
  assert.equal(calls.writes.length, 1);
  assert.equal(applied[0].oldText, 'original');
  assert.equal(applied[0].created, false);
});

test('terminal passes spaces, shell metacharacters and empty argv literally', async () => {
  const { vscode } = editor();
  const { TerminalHandler } = loadHandler('terminal', vscode);
  const handler = new TerminalHandler();
  const args = ['two words', '; echo unintended', '$HOME', '"quotes"', ''];
  const { terminalId } = await handler.createTerminal({ command: process.execPath, args: ['-e', 'process.stdout.write(JSON.stringify(process.argv.slice(1)))', ...args] });
  try {
    const exit = await handler.waitForTerminalExit({ terminalId });
    assert.equal(exit.exitCode, 0);
    assert.deepEqual(JSON.parse((await handler.terminalOutput({ terminalId })).output), args);
  } finally { handler.dispose(); }
});

function mockTerminal(outputByteLimit) {
  const { vscode, calls } = editor();
  const child = new EventEmitter();
  child.stdout = new EventEmitter();
  child.stderr = new EventEmitter();
  child.kill = () => true;
  const { TerminalHandler } = loadHandler('terminal', vscode, { 'node:child_process': { spawn: () => child } });
  const handler = new TerminalHandler();
  return { child, handler, calls, ready: handler.createTerminal({ command: 'fixture', outputByteLimit }) };
}

test('stdout and stderr preserve independently split UTF-8 in capture and display', async () => {
  const { child, handler, calls, ready } = mockTerminal();
  const { terminalId } = await ready;
  const first = Buffer.from('é');
  const second = Buffer.from('漢');
  child.stdout.emit('data', first.subarray(0, 1));
  child.stderr.emit('data', second.subarray(0, 2));
  child.stdout.emit('data', first.subarray(1));
  child.stderr.emit('data', second.subarray(2));
  child.stdout.emit('end'); child.stderr.emit('end'); child.emit('close', 0, null);
  assert.equal((await handler.terminalOutput({ terminalId })).output, 'é漢');
  assert.equal(calls.terminalText.join(''), 'é漢');
});

test('stream end flushes incomplete UTF-8 and byte truncation keeps whole characters', async () => {
  const { child, handler, ready } = mockTerminal(5);
  const { terminalId } = await ready;
  child.stdout.emit('data', Buffer.from('abcé漢'));
  child.stdout.emit('end'); child.stderr.emit('end'); child.emit('close', 0, null);
  const output = await handler.terminalOutput({ terminalId });
  assert.equal(output.output, 'é漢');
  assert.equal(output.truncated, true);
  assert.equal(Buffer.byteLength(output.output), 5);
});

test('truncated final code point is flushed once at stream end', async () => {
  const { child, handler, ready } = mockTerminal();
  const { terminalId } = await ready;
  child.stdout.emit('data', Buffer.from([0xc3]));
  assert.equal((await handler.terminalOutput({ terminalId })).output, '');
  child.stdout.emit('end'); child.stderr.emit('end'); child.emit('close', 0, null);
  assert.equal((await handler.terminalOutput({ terminalId })).output, '\ufffd');
});
