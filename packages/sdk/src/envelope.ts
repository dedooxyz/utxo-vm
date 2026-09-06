export interface InscriptionEnvelope {
  protocol: string;
  version: number;
  contentType: string;
  payload: Uint8Array;
  metadata?: Record<string, any>;
}

export interface CallPayload {
  targetSeal: string;
  method: string;
  args: Record<string, any>;
}

export class EnvelopeBuilder {
  static PROTOCOL_TAG = "utxovm";
  static CURRENT_VERSION = 1;

  static buildDeployEnvelope(wasmBytecode: Uint8Array, metadata?: Record<string, any>): InscriptionEnvelope {
    return {
      protocol: this.PROTOCOL_TAG,
      version: this.CURRENT_VERSION,
      contentType: "application/wasm",
      payload: wasmBytecode,
      metadata,
    };
  }

  static buildCallEnvelope(targetSeal: string, method: string, args: Record<string, any> | Uint8Array): InscriptionEnvelope {
    let payloadBytes: Uint8Array;
    if (args instanceof Uint8Array) {
      payloadBytes = args;
    } else {
      const callData: CallPayload = { targetSeal, method, args };
      payloadBytes = new TextEncoder().encode(JSON.stringify(callData));
    }

    return {
      protocol: this.PROTOCOL_TAG,
      version: this.CURRENT_VERSION,
      contentType: "application/json",
      payload: payloadBytes,
      metadata: { targetSeal, method },
    };
  }

  /**
   * Serializes an inscription envelope into raw script envelope bytes:
   * OP_FALSE (0x00) OP_IF (0x63) PUSH "utxovm" PUSH 0x01 PUSH <contentType> PUSH <payload> OP_ENDIF (0x68)
   */
  static serializeTaprootEnvelope(envelope: InscriptionEnvelope): Uint8Array {
    const protoBytes = new TextEncoder().encode(envelope.protocol);
    const typeBytes = new TextEncoder().encode(envelope.contentType);

    const parts: number[] = [
      0x00, // OP_FALSE
      0x63, // OP_IF
      protoBytes.length,
      ...Array.from(protoBytes),
      0x01, // Version length
      envelope.version,
      typeBytes.length,
      ...Array.from(typeBytes),
    ];

    // Encode payload with length
    const payloadLen = envelope.payload.length;
    if (payloadLen < 0x4c) {
      parts.push(payloadLen);
    } else if (payloadLen <= 0xff) {
      parts.push(0x4c, payloadLen);
    } else if (payloadLen <= 0xffff) {
      parts.push(0x4d, payloadLen & 0xff, (payloadLen >> 8) & 0xff);
    } else {
      parts.push(
        0x4e,
        payloadLen & 0xff,
        (payloadLen >> 8) & 0xff,
        (payloadLen >> 16) & 0xff,
        (payloadLen >> 24) & 0xff
      );
    }
    parts.push(...Array.from(envelope.payload));
    parts.push(0x68); // OP_ENDIF

    return new Uint8Array(parts);
  }

  /**
   * Parses and decodes a Taproot/Witness inscription script looking for utxovm envelope
   */
  static deserializeTaprootEnvelope(script: Uint8Array): InscriptionEnvelope | null {
    try {
      const text = new TextDecoder().decode(script);
      const protoIndex = text.indexOf(this.PROTOCOL_TAG);
      if (protoIndex === -1) {
        return null;
      }

      // Simple header scanner
      const isJson = text.includes("application/json");
      const isWasm = text.includes("application/wasm");

      let contentType = isWasm ? "application/wasm" : "application/json";
      let payloadIndex = text.indexOf(contentType) + contentType.length + 1;
      let payload = script.slice(payloadIndex, script.length - 1);

      return {
        protocol: this.PROTOCOL_TAG,
        version: 1,
        contentType,
        payload,
      };
    } catch {
      return null;
    }
  }
}
