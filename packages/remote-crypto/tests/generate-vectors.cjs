// Deterministic public test keys ONLY. Print vectors; never write files.
// Run from a checkout with npm dependencies installed.
const Module = require('node:module');
const original = Module._load;
Module._load = function (id, ...args) {
  return original.call(this, id === 'sodium-universal' ? 'sodium-javascript' : id, ...args);
};
const sodium = require('sodium-javascript');
const Noise = require('noise-handshake');
const { chacha20poly1305 } = require('@noble/ciphers/chacha');
const hex = b => Buffer.from(b).toString('hex');
function key(byte) {
  const secretKey = Buffer.alloc(32, byte);
  const publicKey = Buffer.alloc(32);
  sodium.crypto_scalarmult_base(publicKey, secretKey);
  return { secretKey, publicKey };
}
function record(noise, counter, context, text) {
  const header = Buffer.alloc(28);
  header.write('FRE2'); Buffer.from(noise.hash).copy(header, 4, 0, 16); header.writeUInt32LE(counter, 20);
  const nonce = Buffer.alloc(12); nonce.writeUInt32LE(counter, 4);
  const ciphertext = chacha20poly1305(noise.tx, nonce, Buffer.concat([header, Buffer.from(context)])).encrypt(Buffer.from(text));
  return hex(Buffer.concat([header, ciphertext]));
}
const vectors = ['XXpsk0', 'IK'].map(pattern => {
  const a = key(1), b = key(2), psk = Buffer.alloc(32, 7);
  const prologue = Buffer.from('futureos-remote-v2\npair_1\ndesktop_1');
  const i = new Noise(pattern, true, a, pattern === 'XXpsk0' ? { psk } : {});
  const r = new Noise(pattern, false, b, pattern === 'XXpsk0' ? { psk } : {});
  i.e = key(3); r.e = key(4);
  i.initialise(prologue, pattern === 'IK' ? b.publicKey : undefined); r.initialise(prologue);
  const messages = [];
  let wire = i.send(); messages.push(hex(wire)); r.recv(wire);
  wire = r.send(); messages.push(hex(wire)); i.recv(wire);
  if (pattern === 'XXpsk0') { wire = i.send(); messages.push(hex(wire)); r.recv(wire); }
  const context = 'p.pair_1.cmd.session_1', plaintext = 'private prompt';
  const request = record(i, 0, context, plaintext);
  const replyContext = `reply:${context}:${request.slice(8, 56)}`;
  return { pattern, desktopPublicKey: hex(b.publicKey), messages, hash: hex(i.hash),
    context, plaintext, request, replyContext, reply: record(r, 0, replyContext, 'private reply') };
});
console.log(JSON.stringify(vectors, null, 2));
