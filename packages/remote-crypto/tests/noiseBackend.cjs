// Match Mobile's pure-JS sodium backend without an Expo preset/native addon.
const Module = require("node:module");
const original = Module._load;
Module._load = function (id, ...args) {
  return original.call(this, id === "sodium-universal" ? "sodium-javascript" : id, ...args);
};
