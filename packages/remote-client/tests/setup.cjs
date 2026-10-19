// Standard Node WebCrypto supplies the same getRandomValues API as the host.
Object.defineProperty(globalThis, "crypto", { configurable: true, value: require("node:crypto").webcrypto });
