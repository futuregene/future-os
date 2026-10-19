/** Producer envelope for the existing Remote v2 Noise exchange. */
export interface HandshakeRequest {
  type: string;
  message: string;
  id?: string;
  pairing?: boolean;
}
