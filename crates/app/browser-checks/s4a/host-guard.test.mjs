// Run with `npm test` (node --test). No browser and no stack needed.
import test from 'node:test';
import assert from 'node:assert/strict';
import { checkedBase, isLoopbackHost } from './host-guard.mjs';

test('the default and loopback hosts pass', () => {
  assert.equal(checkedBase(undefined, undefined), 'http://127.0.0.1:3031');
  assert.equal(checkedBase('', undefined), 'http://127.0.0.1:3031');
  for (const b of ['http://127.0.0.1:3031/', 'http://localhost:3031', 'http://LOCALHOST:8080', 'http://[::1]:3031', 'http://127.0.0.2:3031']) {
    assert.equal(checkedBase(b, undefined), b.replace(/\/$/, ''));
  }
});

test('a non-loopback host is refused without ALLOW_REMOTE=1', () => {
  for (const b of ['https://scuffed.example', 'http://192.168.1.20:3031', 'http://10.0.0.1', 'http://127.0.0.1.example.com', 'http://localhost.example.com', 'http://0.0.0.0:3031']) {
    assert.throws(() => checkedBase(b, undefined), /not loopback/, b);
    assert.throws(() => checkedBase(b, '0'), /not loopback/, b);
    assert.throws(() => checkedBase(b, 'true'), /not loopback/, b);
  }
});

test('ALLOW_REMOTE=1 lets a remote host through', () => {
  assert.equal(checkedBase('https://stack.example/', '1'), 'https://stack.example');
});

test('a BASE that is not an http URL is refused', () => {
  assert.throws(() => checkedBase('127.0.0.1:3031', '1'), /http or https|not a URL/);
  assert.throws(() => checkedBase('not a url', '1'), /not a URL/);
  assert.throws(() => checkedBase('file:///etc', '1'), /http or https/);
});

test('loopback host matching', () => {
  assert.ok(isLoopbackHost('127.255.0.1'));
  assert.ok(!isLoopbackHost('127.0.0.1.nip.io'));
  assert.ok(!isLoopbackHost('::1'));
});
