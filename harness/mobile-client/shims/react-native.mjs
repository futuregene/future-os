/** Only `Platform.OS` is read by the pairing path. */
export const Platform = { OS: "ios", Version: "17.0", select: obj => obj.ios ?? obj.native ?? obj.default };
export default { Platform };
